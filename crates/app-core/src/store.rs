use chrono::{DateTime, Utc};
use mica_infra::{ApiError, ApiResult};
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::documents::{
  DocumentOperation, DocumentSnapshotPayload, apply_operations, payload_from_value,
};

/// Persistent document row. Shared by the REST and WebSocket write paths so
/// both go through exactly one storage representation.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct DocumentRecord {
  pub id: Uuid,
  pub workspace_id: Uuid,
  pub root_block_id: String,
  pub current_seq: i64,
  pub created_by: Uuid,
  pub created_at: DateTime<Utc>,
  pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct SnapshotRecord {
  pub id: Uuid,
  pub document_id: Uuid,
  pub version_seq: i64,
  pub schema_version: i32,
  pub payload: Value,
  pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct UpdateRecord {
  pub id: Uuid,
  pub document_id: Uuid,
  pub seq: i64,
  pub actor_id: Uuid,
  pub update_kind: String,
  pub payload: Value,
  pub created_at: DateTime<Utc>,
}

/// A named, restorable version pinned to a stored snapshot.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct VersionRecord {
  pub id: Uuid,
  pub document_id: Uuid,
  pub snapshot_id: Uuid,
  pub version_seq: i64,
  pub name: String,
  pub created_by: Uuid,
  pub created_at: DateTime<Utc>,
}

/// A yrs-native version-history row (see docs/version-history-plan.md). Metadata
/// only — the `state` blob is fetched separately so timeline listings stay cheap.
/// `label` NULL = an auto snapshot; set = a named user checkpoint. `expires_at`
/// NULL = kept forever (named rows); set = an auto row pruned after retention.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct YrsVersionMeta {
  pub id: Uuid,
  pub document_id: Uuid,
  pub rid: Option<i64>,
  pub label: Option<String>,
  pub created_by: Option<Uuid>,
  pub created_at: DateTime<Utc>,
}

/// A lightweight entry in the append-only change log. Excludes the operation
/// payload so history listings stay cheap.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct UpdateLogEntry {
  pub id: Uuid,
  pub seq: i64,
  pub actor_id: Uuid,
  pub update_kind: String,
  pub created_at: DateTime<Utc>,
}

/// Metadata for an uploaded file/object.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct FileRecord {
  pub id: Uuid,
  pub workspace_id: Uuid,
  pub uploaded_by: Uuid,
  pub object_key: String,
  pub original_name: String,
  pub mime_type: String,
  pub byte_size: i64,
  pub created_at: DateTime<Utc>,
}

/// Result of accepting and persisting a batch of document operations.
#[derive(Debug, Clone)]
pub struct AppliedUpdate {
  pub document: DocumentRecord,
  pub snapshot: SnapshotRecord,
  pub update: UpdateRecord,
  /// The SAME change expressed as a yrs update, folded into the document's
  /// base and appended to the workspace stream (see [`apply_derived_operations`]).
  /// Callers broadcast it so live editors converge without a rebootstrap.
  pub yrs: Option<AppliedYrs>,
}

/// The yrs half of an accepted op-model write: the stream `rid` it was assigned
/// and the encoded update that carries it.
#[derive(Debug, Clone)]
pub struct AppliedYrs {
  pub rid: i64,
  pub update: Vec<u8>,
}

/// Apply structural operations against the latest snapshot under a row lock,
/// append an immutable update, and store the resulting snapshot.
///
/// This is the single authoritative write path; REST and WebSocket callers both
/// route through it so permission-checked sequencing stays consistent.
pub async fn apply_document_operations(
  db: &PgPool,
  workspace_id: Uuid,
  document_id: Uuid,
  actor_id: Uuid,
  operations: &[DocumentOperation],
) -> ApiResult<AppliedUpdate> {
  apply_derived_operations(db, workspace_id, document_id, actor_id, |_| {
    Ok(operations.to_vec())
  })
  .await
}

/// Like [`apply_document_operations`], but the ops are DERIVED from the document
/// snapshot inside the same `FOR UPDATE` lock that applies them — so callers that
/// compute ops from current state (e.g. the markdown write path: append after the
/// end, insert after an anchor, delete existing) can't race a concurrent edit
/// between reading and applying. `derive` gets the locked snapshot; its `Err`
/// becomes a `BadRequest`.
pub async fn apply_derived_operations<F>(
  db: &PgPool,
  workspace_id: Uuid,
  document_id: Uuid,
  actor_id: Uuid,
  derive: F,
) -> ApiResult<AppliedUpdate>
where
  F: FnOnce(&DocumentSnapshotPayload) -> Result<Vec<DocumentOperation>, String>,
{
  let mut tx = db.begin().await?;

  let locked = lock_document_tx(&mut tx, workspace_id, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;
  let current_snapshot = latest_snapshot_tx(&mut tx, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;
  let snapshot_payload = payload_from_value(current_snapshot.payload)
    .map_err(|error| ApiError::BadRequest(format!("invalid document snapshot: {error}")))?;

  // Derive from the YRS base, not the op-model snapshot: once a document has a
  // yrs base (any document ever opened in the editor), [`current_payload`] —
  // i.e. every read, export and MCP fetch — returns the yrs blocks and ignores
  // the snapshot. Deriving from the snapshot meant ops were computed against a
  // baseline nobody reads, then written somewhere nobody reads: writes returned
  // ok, `current_seq` advanced, `document_snapshots` grew — and the content was
  // invisible forever. `ensure_base_tx` folds the snapshot into a base on first
  // touch, so documents that never had one keep working unchanged.
  let base = crate::sync::ensure_base_tx(&mut tx, document_id).await?;
  let mut doc = mica_core::MicaDoc::from_update(&base.state)
    .map_err(|error| ApiError::Internal(format!("corrupt yrs base for {document_id}: {error}")))?;
  let state_vector_before = doc.state_vector();
  // A yrs base whose `meta.root` was wiped (see `MicaDoc::set_blocks`) reports an
  // empty root. Propagating that turns every read and write into
  // `block not found: ` with an empty id, and writes the empty value straight
  // back out. The op-model snapshot still carries the document's real root, so
  // prefer it over an empty one rather than failing the whole batch.
  let yrs_root = doc.root_block_id();
  let root_block_id = if yrs_root.is_empty() {
    snapshot_payload.root_block_id.clone()
  } else {
    yrs_root
  };
  let mut current_payload = DocumentSnapshotPayload {
    schema_version: snapshot_payload.schema_version,
    root_block_id,
    blocks: doc.to_blocks().into_iter().map(md_block_from_core).collect(),
  };
  // Heal a lost root before deriving ops against it, otherwise every write to a
  // damaged document fails the whole batch on `block not found`. Unlike the read
  // path, the repair here IS persisted — `set_blocks` below writes the restored
  // root and children back to the base.
  ensure_root_block(&mut current_payload);

  let operations = derive(&current_payload).map_err(ApiError::BadRequest)?;
  if operations.is_empty() {
    return Err(ApiError::BadRequest("no operations to apply".to_string()));
  }
  let next_payload = apply_operations(current_payload, &operations)
    .map_err(|error| ApiError::BadRequest(error.to_string()))?;

  // Write the result THROUGH to yrs as forward operations, so the base every
  // read consults actually changes. `set_blocks` re-derives the whole block set
  // — the same coarse-but-exact primitive version restore uses. Deliberately
  // not a per-op yrs mapping: a second implementation of the op semantics is
  // precisely how the two stores drifted apart in the first place, whereas this
  // makes the yrs state equal `next_payload` by construction. The cost is a
  // full-document update per REST/MCP write (these are batch writes, not
  // keystrokes) and last-writer-wins against a concurrent editor.
  let next_blocks: Vec<mica_core::Block> = next_payload
    .blocks
    .iter()
    .cloned()
    .map(crate::sync::to_core_block)
    .collect();
  doc.set_blocks(&next_payload.root_block_id, &next_blocks);
  let yrs_update = doc
    .encode_diff(&state_vector_before)
    .map_err(|error| ApiError::Internal(format!("yrs diff failed: {error}")))?;
  let yrs_rid: i64 = sqlx::query_scalar(
    "INSERT INTO workspace_updates(workspace_id, document_id, actor_id, payload)
     VALUES ($1, $2, $3, $4) RETURNING rid",
  )
  .bind(workspace_id)
  .bind(document_id)
  .bind(actor_id)
  .bind(&yrs_update)
  .fetch_one(&mut *tx)
  .await?;
  let yrs_state = doc.encode_state();
  let yrs_state_vector = doc.state_vector();
  sqlx::query(
    "INSERT INTO document_yrs_base(document_id, state, state_vector, base_rid, updated_at)
     VALUES ($1, $2, $3, $4, now())
     ON CONFLICT (document_id) DO UPDATE SET
         state = excluded.state, state_vector = excluded.state_vector,
         base_rid = excluded.base_rid, updated_at = now()",
  )
  .bind(document_id)
  .bind(&yrs_state)
  .bind(&yrs_state_vector)
  .bind(yrs_rid)
  .execute(&mut *tx)
  .await?;

  let next_payload_value =
    serde_json::to_value(next_payload).map_err(|error| ApiError::Internal(error.to_string()))?;
  let operations_value =
    serde_json::to_value(&operations).map_err(|error| ApiError::Internal(error.to_string()))?;
  let next_seq = locked.current_seq + 1;

  let update = sqlx::query_as::<_, UpdateRecord>(
    r#"
      INSERT INTO document_updates (document_id, seq, actor_id, update_kind, payload)
      VALUES ($1, $2, $3, 'block_operations', $4)
      RETURNING id, document_id, seq, actor_id, update_kind, payload, created_at
    "#,
  )
  .bind(document_id)
  .bind(next_seq)
  .bind(actor_id)
  .bind(json!({ "operations": operations_value }))
  .fetch_one(&mut *tx)
  .await?;

  let snapshot = sqlx::query_as::<_, SnapshotRecord>(
    r#"
      INSERT INTO document_snapshots (document_id, version_seq, schema_version, payload)
      VALUES ($1, $2, 1, $3)
      RETURNING id, document_id, version_seq, schema_version, payload, created_at
    "#,
  )
  .bind(document_id)
  .bind(next_seq)
  .bind(next_payload_value)
  .fetch_one(&mut *tx)
  .await?;

  let document = sqlx::query_as::<_, DocumentRecord>(
    r#"
      UPDATE documents
      SET current_seq = $1, updated_at = now()
      WHERE id = $2 AND workspace_id = $3
      RETURNING id, workspace_id, root_block_id, current_seq, created_by, created_at, updated_at
    "#,
  )
  .bind(next_seq)
  .bind(document_id)
  .bind(workspace_id)
  .fetch_one(&mut *tx)
  .await?;

  tx.commit().await?;

  Ok(AppliedUpdate {
    document,
    snapshot,
    update,
    yrs: Some(AppliedYrs {
      rid: yrs_rid,
      update: yrs_update,
    }),
  })
}

pub async fn fetch_document(
  db: &PgPool,
  workspace_id: Uuid,
  document_id: Uuid,
) -> ApiResult<Option<DocumentRecord>> {
  sqlx::query_as::<_, DocumentRecord>(
    r#"
      SELECT id, workspace_id, root_block_id, current_seq, created_by, created_at, updated_at
      FROM documents
      WHERE id = $1 AND workspace_id = $2
    "#,
  )
  .bind(document_id)
  .bind(workspace_id)
  .fetch_optional(db)
  .await
  .map_err(ApiError::from)
}

pub async fn latest_snapshot(db: &PgPool, document_id: Uuid) -> ApiResult<Option<SnapshotRecord>> {
  sqlx::query_as::<_, SnapshotRecord>(
    r#"
      SELECT id, document_id, version_seq, schema_version, payload, created_at
      FROM document_snapshots
      WHERE document_id = $1
      ORDER BY version_seq DESC
      LIMIT 1
    "#,
  )
  .bind(document_id)
  .fetch_optional(db)
  .await
  .map_err(ApiError::from)
}

/// Convert a yrs-core block into the op-model/markdown block shape. The two
/// structs are field-identical; this is the inverse of `sync::to_core_block`.
fn md_block_from_core(b: mica_core::Block) -> crate::documents::Block {
  crate::documents::Block {
    id: b.id,
    kind: b.kind,
    text: b.text,
    data: b.data,
    children: b.children,
  }
}

/// Rebuild a root block that the yrs base lost, so a damaged document still
/// reads instead of erroring.
///
/// When `meta.root` gets wiped (see [`mica_core::MicaDoc::set_blocks`]) the root
/// block itself goes with it, leaving every remaining block parentless. Renders
/// then abort with `block not found: <root>` — the document is unreadable even
/// though all of its content is intact. Re-adopt the orphans (blocks nobody
/// lists as a child) under a fresh root, in the order the CRDT yields them,
/// which is the order they were created.
///
/// Read-path only: nothing is persisted here. This makes damaged documents
/// legible again without a migration, and is a no-op for healthy ones.
fn ensure_root_block(payload: &mut DocumentSnapshotPayload) {
  if payload.root_block_id.is_empty()
    || payload.blocks.iter().any(|b| b.id == payload.root_block_id)
  {
    return;
  }
  let claimed: std::collections::HashSet<&str> = payload
    .blocks
    .iter()
    .flat_map(|b| b.children.iter().map(String::as_str))
    .collect();
  let orphans: Vec<String> = payload
    .blocks
    .iter()
    .filter(|b| !claimed.contains(b.id.as_str()))
    .map(|b| b.id.clone())
    .collect();
  payload.blocks.push(crate::documents::Block {
    id: payload.root_block_id.clone(),
    kind: "page".to_string(),
    text: String::new(),
    data: serde_json::Value::Null,
    children: orphans,
  });
}

/// The document's CURRENT block payload for *reads* — bootstrap, export, outline,
/// search. Once a document is edited through the yrs sync path its live content
/// lives in `document_yrs_base`, while the op-model `document_snapshots` stays
/// frozen at the pre-yrs seed (P4①b dropped the periodic snapshot fold). Reading
/// the raw snapshot therefore returns near-empty content for any doc that's been
/// opened in the collaborative editor — the root cause of "cloud page exports
/// blank / re-opens blank". Prefer the folded yrs base (materialize blocks from
/// it) when present; fall back to the op-model snapshot for docs never touched
/// via yrs. `None` only when the document has neither.
///
/// Read-only: this never mutates state, so it can't corrupt data — it only
/// changes which representation a read returns. The op-model write path
/// ([`apply_derived_operations`]) deliberately still reads the raw snapshot; it
/// operates in the op-model world and is a separate concern.
pub async fn current_payload(
  db: &PgPool,
  document_id: Uuid,
) -> ApiResult<Option<DocumentSnapshotPayload>> {
  let Some(snapshot) = latest_snapshot(db, document_id).await? else {
    return Ok(None);
  };
  // op-model snapshot carries schema_version + the fallback content for docs
  // never touched via yrs.
  let mut payload = payload_from_value(snapshot.payload)
    .map_err(|error| ApiError::Internal(format!("invalid document snapshot: {error}")))?;
  if let Some(base) = crate::sync::document_base(db, document_id).await? {
    let doc = mica_core::MicaDoc::from_update(&base.state).map_err(|error| {
      ApiError::Internal(format!("corrupt yrs base for {document_id}: {error}"))
    })?;
    // Only adopt the yrs root when it actually has one. A base whose `meta.root`
    // was wiped (see `MicaDoc::set_blocks`) reports an empty id, and clobbering
    // the snapshot's still-correct root with it makes every read fail with
    // `block not found: ` — an empty id in the message is this exact case.
    let yrs_root = doc.root_block_id();
    if !yrs_root.is_empty() {
      payload.root_block_id = yrs_root;
    }
    payload.blocks = doc
      .to_blocks()
      .into_iter()
      .map(md_block_from_core)
      .collect();
    ensure_root_block(&mut payload);
  }
  Ok(Some(payload))
}

/// Decode a stored yrs version blob (from `document_yrs_versions.state`) into a
/// renderable payload — blocks in tree order — for a read-only preview of that
/// version. Same yrs→blocks path as [`current_payload`], on an arbitrary blob.
pub fn yrs_state_to_payload(state: &[u8]) -> ApiResult<DocumentSnapshotPayload> {
  let doc = mica_core::MicaDoc::from_update(state)
    .map_err(|error| ApiError::BadRequest(format!("corrupt version state: {error}")))?;
  Ok(DocumentSnapshotPayload {
    schema_version: 1,
    root_block_id: doc.root_block_id(),
    blocks: doc.to_blocks().into_iter().map(md_block_from_core).collect(),
  })
}

/// Insert the empty starting snapshot (`version_seq = 0`) for a new document.
pub async fn insert_initial_snapshot(
  tx: &mut Transaction<'_, Postgres>,
  document: &DocumentRecord,
) -> ApiResult<SnapshotRecord> {
  let payload = json!({
    "schema_version": 1,
    "root_block_id": document.root_block_id,
    "blocks": [
      {
        "id": document.root_block_id,
        "type": "paragraph",
        "text": "",
        "children": []
      }
    ]
  });

  sqlx::query_as::<_, SnapshotRecord>(
    r#"
      INSERT INTO document_snapshots (document_id, version_seq, schema_version, payload)
      VALUES ($1, 0, 1, $2)
      RETURNING id, document_id, version_seq, schema_version, payload, created_at
    "#,
  )
  .bind(document.id)
  .bind(payload)
  .fetch_one(&mut **tx)
  .await
  .map_err(ApiError::from)
}

async fn lock_document_tx(
  tx: &mut Transaction<'_, Postgres>,
  workspace_id: Uuid,
  document_id: Uuid,
) -> ApiResult<Option<DocumentRecord>> {
  sqlx::query_as::<_, DocumentRecord>(
    r#"
      SELECT id, workspace_id, root_block_id, current_seq, created_by, created_at, updated_at
      FROM documents
      WHERE id = $1 AND workspace_id = $2
      FOR UPDATE
    "#,
  )
  .bind(document_id)
  .bind(workspace_id)
  .fetch_optional(&mut **tx)
  .await
  .map_err(ApiError::from)
}

pub(crate) async fn latest_snapshot_tx(
  tx: &mut Transaction<'_, Postgres>,
  document_id: Uuid,
) -> ApiResult<Option<SnapshotRecord>> {
  sqlx::query_as::<_, SnapshotRecord>(
    r#"
      SELECT id, document_id, version_seq, schema_version, payload, created_at
      FROM document_snapshots
      WHERE document_id = $1
      ORDER BY version_seq DESC
      LIMIT 1
    "#,
  )
  .bind(document_id)
  .fetch_optional(&mut **tx)
  .await
  .map_err(ApiError::from)
}

/// Maximum number of change-log entries returned in a single history page.
pub const MAX_HISTORY_PAGE: i64 = 200;

/// List change-log entries newest first, optionally paging backwards from an
/// exclusive `before_seq` cursor.
pub async fn list_updates(
  db: &PgPool,
  document_id: Uuid,
  limit: i64,
  before_seq: Option<i64>,
) -> ApiResult<Vec<UpdateLogEntry>> {
  let limit = limit.clamp(1, MAX_HISTORY_PAGE);
  // A NULL cursor (bound below) matches every row, so the same query serves both
  // the first page and subsequent pages.
  sqlx::query_as::<_, UpdateLogEntry>(
    r#"
      SELECT id, seq, actor_id, update_kind, created_at
      FROM document_updates
      WHERE document_id = $1 AND ($2::bigint IS NULL OR seq < $2)
      ORDER BY seq DESC
      LIMIT $3
    "#,
  )
  .bind(document_id)
  .bind(before_seq)
  .bind(limit)
  .fetch_all(db)
  .await
  .map_err(ApiError::from)
}

/// List named versions newest first, resolving each to its snapshot's sequence.
pub async fn list_versions(db: &PgPool, document_id: Uuid) -> ApiResult<Vec<VersionRecord>> {
  sqlx::query_as::<_, VersionRecord>(
    r#"
      SELECT v.id, v.document_id, v.snapshot_id, s.version_seq, v.name, v.created_by, v.created_at
      FROM document_versions v
      INNER JOIN document_snapshots s ON s.id = v.snapshot_id
      WHERE v.document_id = $1
      ORDER BY v.created_at DESC
    "#,
  )
  .bind(document_id)
  .fetch_all(db)
  .await
  .map_err(ApiError::from)
}

// ── Public sharing (publish a page to a /s/{token} URL) ──────────────────────

/// An active public share of a document. `token` is the unguessable capability
/// that grants read-only access at the public route.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ShareRecord {
  pub id: Uuid,
  pub token: String,
  pub workspace_id: Uuid,
  pub document_id: Uuid,
  pub created_by: Uuid,
  pub created_at: DateTime<Utc>,
  pub allow_indexing: bool,
}

/// The document's active (non-revoked) share, if any.
pub async fn fetch_active_share_for_doc(
  db: &PgPool,
  workspace_id: Uuid,
  document_id: Uuid,
) -> ApiResult<Option<ShareRecord>> {
  sqlx::query_as::<_, ShareRecord>(
    "SELECT id, token, workspace_id, document_id, created_by, created_at, allow_indexing \
     FROM document_shares \
     WHERE workspace_id = $1 AND document_id = $2 AND revoked_at IS NULL",
  )
  .bind(workspace_id)
  .bind(document_id)
  .fetch_optional(db)
  .await
  .map_err(ApiError::from)
}

/// Return the document's active share, minting one (a fresh token) if none
/// exists. findOrCreate — re-sharing a page yields the SAME link, not a new one.
pub async fn create_or_get_share(
  db: &PgPool,
  workspace_id: Uuid,
  document_id: Uuid,
  user_id: Uuid,
) -> ApiResult<ShareRecord> {
  if let Some(existing) = fetch_active_share_for_doc(db, workspace_id, document_id).await? {
    return Ok(existing);
  }
  // 244 bits from two v4 UUIDs — the same recipe as a PAT (tokens.rs), no extra
  // crate. Unguessable is the whole security model, so this is not thrift.
  let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
  // `DO NOTHING` guards the race where two requests share at once: the partial
  // unique index (one active share per doc) makes the loser a no-op; we then
  // re-read the winner rather than erroring.
  let inserted = sqlx::query_as::<_, ShareRecord>(
    "INSERT INTO document_shares (token, workspace_id, document_id, created_by) \
     VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING \
     RETURNING id, token, workspace_id, document_id, created_by, created_at, allow_indexing",
  )
  .bind(&token)
  .bind(workspace_id)
  .bind(document_id)
  .bind(user_id)
  .fetch_optional(db)
  .await?;
  match inserted {
    Some(share) => Ok(share),
    None => fetch_active_share_for_doc(db, workspace_id, document_id)
      .await?
      .ok_or_else(|| ApiError::Internal("share vanished after conflict".to_string())),
  }
}

/// Soft-revoke the document's active share. Returns true if one was active.
/// Every public read re-checks `revoked_at IS NULL`, so this takes effect at
/// once — there is no token cache to invalidate.
pub async fn revoke_share(db: &PgPool, workspace_id: Uuid, document_id: Uuid) -> ApiResult<bool> {
  let result = sqlx::query(
    "UPDATE document_shares SET revoked_at = now() \
     WHERE workspace_id = $1 AND document_id = $2 AND revoked_at IS NULL",
  )
  .bind(workspace_id)
  .bind(document_id)
  .execute(db)
  .await?;
  Ok(result.rows_affected() > 0)
}

/// The public read path: resolve a token to its active share. The token is the
/// only credential; there is no workspace/auth context. A missing OR revoked
/// token both return None so the caller answers with an indistinguishable 404.
pub async fn fetch_share_by_token(db: &PgPool, token: &str) -> ApiResult<Option<ShareRecord>> {
  sqlx::query_as::<_, ShareRecord>(
    "SELECT id, token, workspace_id, document_id, created_by, created_at, allow_indexing \
     FROM document_shares WHERE token = $1 AND revoked_at IS NULL",
  )
  .bind(token)
  .fetch_optional(db)
  .await
  .map_err(ApiError::from)
}

pub async fn fetch_version(
  db: &PgPool,
  document_id: Uuid,
  version_id: Uuid,
) -> ApiResult<Option<VersionRecord>> {
  sqlx::query_as::<_, VersionRecord>(
    r#"
      SELECT v.id, v.document_id, v.snapshot_id, s.version_seq, v.name, v.created_by, v.created_at
      FROM document_versions v
      INNER JOIN document_snapshots s ON s.id = v.snapshot_id
      WHERE v.document_id = $1 AND v.id = $2
    "#,
  )
  .bind(document_id)
  .bind(version_id)
  .fetch_optional(db)
  .await
  .map_err(ApiError::from)
}

pub async fn fetch_snapshot(
  db: &PgPool,
  document_id: Uuid,
  snapshot_id: Uuid,
) -> ApiResult<Option<SnapshotRecord>> {
  sqlx::query_as::<_, SnapshotRecord>(
    r#"
      SELECT id, document_id, version_seq, schema_version, payload, created_at
      FROM document_snapshots
      WHERE document_id = $1 AND id = $2
    "#,
  )
  .bind(document_id)
  .bind(snapshot_id)
  .fetch_optional(db)
  .await
  .map_err(ApiError::from)
}

// ── yrs-native version history (docs/version-history-plan.md) ────────────────
// The op-model history below froze with P4①b; real history routes through these.

/// The document's version timeline, newest first. Metadata only (no `state`
/// blobs) so the panel loads cheaply. Auto rows and named rows are interleaved
/// by `created_at`.
pub async fn list_yrs_versions(
  db: &PgPool,
  document_id: Uuid,
) -> ApiResult<Vec<YrsVersionMeta>> {
  sqlx::query_as::<_, YrsVersionMeta>(
    r#"
      SELECT id, document_id, rid, label, created_by, created_at
      FROM document_yrs_versions
      WHERE document_id = $1
      ORDER BY created_at DESC
    "#,
  )
  .bind(document_id)
  .fetch_all(db)
  .await
  .map_err(ApiError::from)
}

/// One version's full yrs state blob (for read-only preview or restore). None if
/// the id doesn't belong to this document.
pub async fn fetch_yrs_version_state(
  db: &PgPool,
  document_id: Uuid,
  version_id: Uuid,
) -> ApiResult<Option<Vec<u8>>> {
  sqlx::query_scalar::<_, Vec<u8>>(
    "SELECT state FROM document_yrs_versions WHERE id = $1 AND document_id = $2",
  )
  .bind(version_id)
  .bind(document_id)
  .fetch_optional(db)
  .await
  .map_err(ApiError::from)
}

/// Pin the document's current folded state as a NAMED version (a manual
/// checkpoint). Named rows have `expires_at` NULL, so retention never prunes
/// them. Errors if the document has no yrs base yet (never edited).
pub async fn create_named_yrs_version(
  db: &PgPool,
  document_id: Uuid,
  label: &str,
  created_by: Uuid,
) -> ApiResult<YrsVersionMeta> {
  sqlx::query_as::<_, YrsVersionMeta>(
    r#"
      INSERT INTO document_yrs_versions (document_id, rid, label, created_by, state)
      SELECT $1, base_rid, $2, $3, state
      FROM document_yrs_base
      WHERE document_id = $1
      RETURNING id, document_id, rid, label, created_by, created_at
    "#,
  )
  .bind(document_id)
  .bind(label)
  .bind(created_by)
  .fetch_optional(db)
  .await?
  .ok_or(ApiError::NotFound)
}

pub async fn fetch_snapshot_by_version_seq(
  db: &PgPool,
  document_id: Uuid,
  version_seq: i64,
) -> ApiResult<Option<SnapshotRecord>> {
  sqlx::query_as::<_, SnapshotRecord>(
    r#"
      SELECT id, document_id, version_seq, schema_version, payload, created_at
      FROM document_snapshots
      WHERE document_id = $1 AND version_seq = $2
    "#,
  )
  .bind(document_id)
  .bind(version_seq)
  .fetch_optional(db)
  .await
  .map_err(ApiError::from)
}

/// Pin the document's current state as a named version. Because a snapshot is
/// stored for every accepted update, this points at the latest snapshot rather
/// than duplicating state.
pub async fn create_named_version(
  db: &PgPool,
  document_id: Uuid,
  name: &str,
  created_by: Uuid,
) -> ApiResult<VersionRecord> {
  let mut tx = db.begin().await?;

  let snapshot = latest_snapshot_tx(&mut tx, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;

  let version = sqlx::query_as::<_, VersionRecord>(
    r#"
      WITH inserted AS (
        INSERT INTO document_versions (document_id, snapshot_id, name, created_by)
        VALUES ($1, $2, $3, $4)
        RETURNING id, document_id, snapshot_id, name, created_by, created_at
      )
      SELECT i.id, i.document_id, i.snapshot_id, s.version_seq, i.name, i.created_by, i.created_at
      FROM inserted i
      INNER JOIN document_snapshots s ON s.id = i.snapshot_id
    "#,
  )
  .bind(document_id)
  .bind(snapshot.id)
  .bind(name)
  .bind(created_by)
  .fetch_one(&mut *tx)
  .await?;

  tx.commit().await?;

  Ok(version)
}

/// Restore a document to a prior snapshot's state. History is append-only: this
/// records a `restore_snapshot` update and a fresh snapshot at the next sequence
/// rather than rewriting past entries. The returned [`AppliedUpdate`] can be
/// broadcast to connected clients exactly like an ordinary edit.
pub async fn restore_snapshot(
  db: &PgPool,
  workspace_id: Uuid,
  document_id: Uuid,
  actor_id: Uuid,
  source_version_seq: i64,
) -> ApiResult<AppliedUpdate> {
  let mut tx = db.begin().await?;

  let locked = lock_document_tx(&mut tx, workspace_id, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;

  let source = sqlx::query_as::<_, SnapshotRecord>(
    r#"
      SELECT id, document_id, version_seq, schema_version, payload, created_at
      FROM document_snapshots
      WHERE document_id = $1 AND version_seq = $2
    "#,
  )
  .bind(document_id)
  .bind(source_version_seq)
  .fetch_optional(&mut *tx)
  .await?
  .ok_or(ApiError::NotFound)?;

  // Validate the restored payload before committing to it.
  let restored = payload_from_value(source.payload)
    .map_err(|error| ApiError::BadRequest(format!("invalid snapshot to restore: {error}")))?;
  let restored_value =
    serde_json::to_value(restored).map_err(|error| ApiError::Internal(error.to_string()))?;
  let next_seq = locked.current_seq + 1;

  let update = sqlx::query_as::<_, UpdateRecord>(
    r#"
      INSERT INTO document_updates (document_id, seq, actor_id, update_kind, payload)
      VALUES ($1, $2, $3, 'restore_snapshot', $4)
      RETURNING id, document_id, seq, actor_id, update_kind, payload, created_at
    "#,
  )
  .bind(document_id)
  .bind(next_seq)
  .bind(actor_id)
  .bind(json!({ "restored_from_version_seq": source_version_seq }))
  .fetch_one(&mut *tx)
  .await?;

  let snapshot = sqlx::query_as::<_, SnapshotRecord>(
    r#"
      INSERT INTO document_snapshots (document_id, version_seq, schema_version, payload)
      VALUES ($1, $2, 1, $3)
      RETURNING id, document_id, version_seq, schema_version, payload, created_at
    "#,
  )
  .bind(document_id)
  .bind(next_seq)
  .bind(restored_value)
  .fetch_one(&mut *tx)
  .await?;
  // (legacy path — see the `yrs: None` note at the end of this function)

  let document = sqlx::query_as::<_, DocumentRecord>(
    r#"
      UPDATE documents
      SET current_seq = $1, updated_at = now()
      WHERE id = $2 AND workspace_id = $3
      RETURNING id, workspace_id, root_block_id, current_seq, created_by, created_at, updated_at
    "#,
  )
  .bind(next_seq)
  .bind(document_id)
  .bind(workspace_id)
  .fetch_one(&mut *tx)
  .await?;

  tx.commit().await?;

  Ok(AppliedUpdate {
    document,
    snapshot,
    update,
    // Legacy op-model restore, superseded by `sync::restore_yrs_version` (the
    // yrs-native path history.rs actually calls). No yrs half to broadcast.
    yrs: None,
  })
}

/// Store an imported document's initial state as its `version_seq = 0` snapshot.
pub async fn insert_root_snapshot(
  tx: &mut Transaction<'_, Postgres>,
  document_id: Uuid,
  payload: &DocumentSnapshotPayload,
) -> ApiResult<SnapshotRecord> {
  let value =
    serde_json::to_value(payload).map_err(|error| ApiError::Internal(error.to_string()))?;

  sqlx::query_as::<_, SnapshotRecord>(
    r#"
      INSERT INTO document_snapshots (document_id, version_seq, schema_version, payload)
      VALUES ($1, 0, $2, $3)
      RETURNING id, document_id, version_seq, schema_version, payload, created_at
    "#,
  )
  .bind(document_id)
  .bind(payload.schema_version)
  .bind(value)
  .fetch_one(&mut **tx)
  .await
  .map_err(ApiError::from)
}

/// Record metadata for an uploaded object. The unique `object_key` makes a
/// duplicate completion a conflict rather than a silent overwrite.
pub async fn insert_file(
  db: &PgPool,
  workspace_id: Uuid,
  uploaded_by: Uuid,
  object_key: &str,
  original_name: &str,
  mime_type: &str,
  byte_size: i64,
) -> ApiResult<FileRecord> {
  // Object keys are content-addressed (sha256), so re-uploading identical bytes
  // hits the UNIQUE(object_key) constraint. Treat that as dedup: return the
  // existing row rather than failing. The no-op SET lets us use RETURNING.
  sqlx::query_as::<_, FileRecord>(
    r#"
      INSERT INTO files (workspace_id, uploaded_by, object_key, original_name, mime_type, byte_size)
      VALUES ($1, $2, $3, $4, $5, $6)
      ON CONFLICT (object_key) DO UPDATE SET object_key = EXCLUDED.object_key
      RETURNING id, workspace_id, uploaded_by, object_key, original_name, mime_type, byte_size, created_at
    "#,
  )
  .bind(workspace_id)
  .bind(uploaded_by)
  .bind(object_key)
  .bind(original_name)
  .bind(mime_type)
  .bind(byte_size)
  .fetch_one(db)
  .await
  .map_err(map_insert_file_error)
}

pub async fn fetch_file(
  db: &PgPool,
  workspace_id: Uuid,
  file_id: Uuid,
) -> ApiResult<Option<FileRecord>> {
  sqlx::query_as::<_, FileRecord>(
    r#"
      SELECT id, workspace_id, uploaded_by, object_key, original_name, mime_type, byte_size, created_at
      FROM files
      WHERE id = $1 AND workspace_id = $2
    "#,
  )
  .bind(file_id)
  .bind(workspace_id)
  .fetch_optional(db)
  .await
  .map_err(ApiError::from)
}

/// Fetch many files at once (for resolving image blocks on document load).
/// Silently skips ids that are absent or belong to another workspace.
pub async fn fetch_files(
  db: &PgPool,
  workspace_id: Uuid,
  file_ids: &[Uuid],
) -> ApiResult<Vec<FileRecord>> {
  sqlx::query_as::<_, FileRecord>(
    r#"
      SELECT id, workspace_id, uploaded_by, object_key, original_name, mime_type, byte_size, created_at
      FROM files
      WHERE workspace_id = $1 AND id = ANY($2)
    "#,
  )
  .bind(workspace_id)
  .bind(file_ids)
  .fetch_all(db)
  .await
  .map_err(ApiError::from)
}

/// Delete file metadata. Returns whether a row was removed. Object deletion in
/// the store is left to a separate lifecycle process for the MVP.
pub async fn delete_file(db: &PgPool, workspace_id: Uuid, file_id: Uuid) -> ApiResult<bool> {
  let result = sqlx::query(
    r#"
      DELETE FROM files
      WHERE id = $1 AND workspace_id = $2
    "#,
  )
  .bind(file_id)
  .bind(workspace_id)
  .execute(db)
  .await?;

  Ok(result.rows_affected() > 0)
}

fn map_insert_file_error(error: sqlx::Error) -> ApiError {
  if let sqlx::Error::Database(db_error) = &error
    && db_error.constraint() == Some("files_object_key_key")
  {
    return ApiError::Conflict("file already recorded for this object key".to_string());
  }

  ApiError::Database(error)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::documents::Block;

  fn block(id: &str, children: &[&str]) -> Block {
    Block {
      id: id.to_string(),
      kind: "paragraph".to_string(),
      text: String::new(),
      data: serde_json::Value::Null,
      children: children.iter().map(|s| s.to_string()).collect(),
    }
  }

  fn payload(root: &str, blocks: Vec<Block>) -> DocumentSnapshotPayload {
    DocumentSnapshotPayload {
      schema_version: 1,
      root_block_id: root.to_string(),
      blocks,
    }
  }

  /// The damaged-document shape seen in production: content intact, but the root
  /// block gone and every block parentless. Reads used to abort with
  /// `block not found: <root>`; now the orphans are re-adopted in order.
  #[test]
  fn rebuilds_a_missing_root_and_adopts_orphans_in_order() {
    let mut p = payload(
      "root_1",
      vec![block("a", &[]), block("b", &[]), block("c", &[])],
    );
    ensure_root_block(&mut p);

    let root = p
      .blocks
      .iter()
      .find(|b| b.id == "root_1")
      .expect("root rebuilt");
    assert_eq!(root.children, vec!["a", "b", "c"]);
    assert_eq!(root.kind, "page");
  }

  /// Blocks that already have a parent must not be re-parented onto the root,
  /// or a nested document would be flattened by the repair.
  #[test]
  fn only_unclaimed_blocks_become_root_children() {
    let mut p = payload(
      "root_1",
      vec![block("a", &["a1"]), block("a1", &[]), block("b", &[])],
    );
    ensure_root_block(&mut p);

    let root = p.blocks.iter().find(|b| b.id == "root_1").unwrap();
    assert_eq!(root.children, vec!["a", "b"], "a1 already belongs to a");
  }

  #[test]
  fn healthy_documents_are_untouched() {
    let mut p = payload("root_1", vec![block("root_1", &["a"]), block("a", &[])]);
    let before = p.blocks.len();
    ensure_root_block(&mut p);
    assert_eq!(p.blocks.len(), before);
  }

  /// Without a known root there is nothing to rebuild — don't invent one.
  #[test]
  fn an_empty_root_id_is_left_alone() {
    let mut p = payload("", vec![block("a", &[])]);
    ensure_root_block(&mut p);
    assert_eq!(p.blocks.len(), 1);
  }
}
