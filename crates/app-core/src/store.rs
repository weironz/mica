use chrono::{DateTime, Utc};
use mica_infra::{ApiError, ApiResult};
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::documents::{DocumentOperation, DocumentSnapshotPayload, apply_operations};

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

/// A yrs-native version-history row (see docs/features/version-history-plan.md). Metadata
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
  pub snapshot: AppliedContent,
  pub update: AppliedOperations,
  /// The SAME change expressed as a yrs update, folded into the document's
  /// base and appended to the workspace stream (see [`apply_derived_operations`]).
  /// Callers broadcast it so live editors converge without a rebootstrap.
  pub yrs: Option<AppliedYrs>,
}

/// The content a write produced, in the JSON shape REST clients already parse
/// (`version_seq` / `schema_version` / `payload`).
///
/// Deliberately NOT a `document_snapshots` row: since S4 nothing writes that
/// table, so there is no id and no `created_at` to report — and no row to look
/// one up by. Reporting fields that address nothing would be the same
/// double-representation lie the op model was retired for.
#[derive(Debug, Clone, Serialize)]
pub struct AppliedContent {
  pub version_seq: i64,
  pub schema_version: i32,
  pub payload: Value,
}

/// The operations a write derived, in the JSON shape REST/WS clients parse.
/// Same reasoning as [`AppliedContent`]: no `document_updates` row backs it.
#[derive(Debug, Clone, Serialize)]
pub struct AppliedOperations {
  pub seq: i64,
  pub actor_id: Uuid,
  /// Always `block_operations` — the only kind the op path ever emitted. Kept
  /// on the wire because clients parse it.
  pub update_kind: &'static str,
  pub payload: Value,
}

/// The yrs half of an accepted op-model write: the stream `rid` it was assigned
/// and the encoded update that carries it.
#[derive(Debug, Clone)]
pub struct AppliedYrs {
  pub rid: i64,
  pub update: Vec<u8>,
}

/// Apply structural operations against the document's yrs base under a row lock,
/// fold the result back into that base, and append it to the workspace stream.
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
  apply_derived_operations(db, workspace_id, document_id, actor_id, None, |_| {
    Ok(operations.to_vec())
  })
  .await
}

/// Like [`apply_document_operations`], but the ops are DERIVED from the document's
/// current content inside the same `FOR UPDATE` lock that applies them — so
/// callers that compute ops from current state (e.g. the markdown write path:
/// append after the end, insert after an anchor, delete existing) can't race a
/// concurrent edit between reading and applying. `derive` gets that locked
/// content; its `Err` becomes a `BadRequest`.
pub async fn apply_derived_operations<F>(
  db: &PgPool,
  workspace_id: Uuid,
  document_id: Uuid,
  actor_id: Uuid,
  expected_seq: Option<i64>,
  derive: F,
) -> ApiResult<AppliedUpdate>
where
  F: FnOnce(&DocumentSnapshotPayload) -> Result<Vec<DocumentOperation>, String>,
{
  let mut tx = db.begin().await?;

  let locked = lock_document_tx(&mut tx, workspace_id, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;
  // Optimistic concurrency: when the caller pins the seq it last saw (from
  // mica_get_outline or a prior write ack) and the document has since moved on,
  // refuse (409) instead of silently clobbering the intervening edit. Checked
  // inside the FOR UPDATE lock, so the comparison can't race the next writer.
  if let Some(expected) = expected_seq
    && locked.current_seq != expected
  {
    return Err(ApiError::Conflict(format!(
      "document changed since seq {expected} (now at seq {}); re-read before writing",
      locked.current_seq
    )));
  }
  // Derive from the YRS base — the one representation every read consults.
  // This used to derive from the op-model snapshot, which meant ops were
  // computed against a baseline nobody reads, then written somewhere nobody
  // reads: the write returned ok, `current_seq` advanced, `document_snapshots`
  // grew — and the content was invisible forever. `ensure_base_tx` builds the
  // base on first touch (folding a leftover snapshot, or empty), so documents
  // from either era keep working unchanged.
  let base = crate::sync::ensure_base_tx(&mut tx, document_id).await?;
  let mut doc = crate::sync::guarded_from_update(&base.state)
    .map_err(|error| ApiError::Internal(format!("corrupt yrs base for {document_id}: {error}")))?;
  let state_vector_before = doc.state_vector();
  // A yrs base whose `meta.root` was wiped (see `MicaDoc::set_blocks`) reports an
  // empty root. Propagating that turns every read and write into
  // `block not found: ` with an empty id, and writes the empty value straight
  // back out. `documents.root_block_id` — already in hand from the FOR UPDATE
  // lock — is the authoritative copy, so prefer it over an empty one rather
  // than failing the whole batch.
  let yrs_root = doc.root_block_id();
  let root_block_id = if yrs_root.is_empty() {
    locked.root_block_id.clone()
  } else {
    yrs_root
  };
  let mut current_payload = DocumentSnapshotPayload {
    schema_version: 1,
    root_block_id,
    blocks: doc.to_blocks(),
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
  // makes the yrs state equal `next_payload` by construction. `set_blocks`
  // diffs block by block, so blocks this write did not change keep their yrs
  // objects and a concurrent editor's edits to them merge; only a block both
  // sides changed is decided by the CRDT (text splices interleave, the same
  // prop key is last-writer-wins).
  let next_blocks: Vec<mica_core::Block> = next_payload.blocks.clone();
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
  // Flatten ONCE and write the base exactly the way the collaborative push
  // path does: same statement, same derived columns from the SAME flatten
  // (red line #1 — the projections cannot diverge from the state they index).
  // These helpers used to be duplicated here as a separate upsert that
  // flattened three more times (one per projection) and shipped the whole
  // state to Postgres on every write for its version-window check.
  let flat = crate::sync::FlatBase::from_doc(&doc);
  crate::sync::write_base_tx(&mut tx, document_id, yrs_rid, &flat).await?;
  // `views.name` rides every base write, or the sidebar name drifts from the
  // title — this path used to skip it, the push path did not.
  crate::sync::sync_view_name_tx(&mut tx, document_id, &flat).await?;
  // Version history: archive the just-folded state as an AUTO snapshot, exactly
  // as the collaborative sync path does (`sync::push_update`). REST/MCP writes
  // land here and never touch that path, so without this a document maintained
  // purely over MCP/REST accrues no user-visible version history at all — and
  // this only writes the archive from the yrs `state` already in hand: no
  // second representation of the content.
  crate::sync::archive_auto_version_tx(&mut tx, document_id, yrs_rid, &flat.state).await?;

  let next_payload_value =
    serde_json::to_value(next_payload).map_err(|error| ApiError::Internal(error.to_string()))?;
  let operations_value =
    serde_json::to_value(&operations).map_err(|error| ApiError::Internal(error.to_string()))?;
  let next_seq = locked.current_seq + 1;

  // S4: the op-model tables are no longer written. `document_updates` and
  // `document_snapshots` used to be INSERTed here alongside the yrs write above
  // — a second representation of the same change, which is exactly how the two
  // stores drifted apart (red line #1). The yrs base + `workspace_updates` are
  // now the sole record; what the caller gets back is that same change reported
  // in the shape REST/WS clients already parse, not a row read back from a
  // table. `document_yrs_versions` (written above) carries version history.
  let update = AppliedOperations {
    seq: next_seq,
    actor_id,
    update_kind: "block_operations",
    payload: json!({ "operations": operations_value }),
  };
  let snapshot = AppliedContent {
    version_seq: next_seq,
    schema_version: 1,
    payload: next_payload_value,
  };

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
/// search. Materialized from the folded yrs base, which since S5 is the ONE place
/// a document's content lives.
///
/// `None` means the document has no base. Before S5 that fell back to the
/// op-model snapshot; there is no second copy to consult now, and there is
/// nothing for one to hold either — a base-less document is a content-less
/// document, because every write path seeds the base in the same transaction as
/// the `documents` row.
pub async fn current_payload(
  db: &PgPool,
  document_id: Uuid,
) -> ApiResult<Option<DocumentSnapshotPayload>> {
  let row: Option<(Vec<u8>, String)> = sqlx::query_as(
    "SELECT b.state, coalesce(d.root_block_id, '') FROM document_yrs_base b
     LEFT JOIN documents d ON d.id = b.document_id WHERE b.document_id = $1",
  )
  .bind(document_id)
  .fetch_optional(db)
  .await?;
  row
    .map(|(state, root)| payload_from_state(document_id, &state, root))
    .transpose()
}

/// Export the payload together with a revision of the EXACT bytes decoded.
/// One statement avoids marking a concurrent edit as already backed up.
/// Many documents' current payloads in ONE query — a bulk pre-scan (transfer /
/// clone of a large subtree) otherwise round-trips per document with
/// [current_payload], paying N sequential DB awaits before any copy work can
/// start. Documents without a yrs base are ABSENT from the map, exactly as
/// [current_payload] returns None for them; then map lookup is the caller's
/// "no payload" branch.
pub async fn current_payloads(
  db: &PgPool,
  document_ids: &[Uuid],
) -> ApiResult<std::collections::HashMap<Uuid, DocumentSnapshotPayload>> {
  let rows: Vec<(Uuid, Vec<u8>, String)> = sqlx::query_as(
    "SELECT b.document_id, b.state, coalesce(d.root_block_id, '') FROM document_yrs_base b
     LEFT JOIN documents d ON d.id = b.document_id WHERE b.document_id = ANY($1)",
  )
  .bind(document_ids)
  .fetch_all(db)
  .await?;
  rows
    .into_iter()
    .map(|(id, state, root)| payload_from_state(id, &state, root).map(|p| (id, p)))
    .collect()
}

pub async fn current_payload_versioned(
  db: &PgPool,
  document_id: Uuid,
) -> ApiResult<Option<(DocumentSnapshotPayload, String)>> {
  let row: Option<(Vec<u8>, String, String)> = sqlx::query_as(
    "SELECT b.state, md5(b.state) || ':' || md5(d.root_block_id), d.root_block_id
     FROM document_yrs_base b JOIN documents d ON d.id = b.document_id
     WHERE b.document_id = $1",
  )
  .bind(document_id)
  .fetch_optional(db)
  .await?;
  row
    .map(|(state, revision, root)| {
      payload_from_state(document_id, &state, root).map(|payload| (payload, revision))
    })
    .transpose()
}

fn payload_from_state(
  document_id: Uuid,
  state: &[u8],
  fallback_root: String,
) -> ApiResult<DocumentSnapshotPayload> {
  let doc = crate::sync::guarded_from_update(state)
    .map_err(|error| ApiError::Internal(format!("corrupt yrs base for {document_id}: {error}")))?;
  // A base whose `meta.root` was wiped (see `MicaDoc::set_blocks`) reports an
  // empty root, and an empty root makes every read fail with `block not found: `
  // — an empty id in that message is this exact case. `documents.root_block_id`
  // is the authoritative copy, so heal from it rather than propagate the hole.
  let mut root_block_id = doc.root_block_id();
  if root_block_id.is_empty() {
    root_block_id = fallback_root;
  }
  let mut payload = DocumentSnapshotPayload {
    schema_version: 1,
    root_block_id,
    blocks: doc.to_blocks(),
  };
  ensure_root_block(&mut payload);
  Ok(payload)
}

/// Decode a stored yrs version blob (from `document_yrs_versions.state`) into a
/// renderable payload — blocks in tree order — for a read-only preview of that
/// version. Same yrs→blocks path as [`current_payload`], on an arbitrary blob.
pub fn yrs_state_to_payload(state: &[u8]) -> ApiResult<DocumentSnapshotPayload> {
  let doc = crate::sync::guarded_from_update(state)
    .map_err(|error| ApiError::BadRequest(format!("corrupt version state: {error}")))?;
  Ok(DocumentSnapshotPayload {
    schema_version: 1,
    root_block_id: doc.root_block_id(),
    blocks: doc.to_blocks(),
  })
}

/// Take the `documents` row lock, returning the record or `None` if there is no
/// such document in this workspace.
///
/// `pub(crate)` because BOTH write paths must take it, in this order, before
/// touching `document_yrs_base`: `apply_derived_operations` here, and
/// `sync::push_update` for the WebSocket route. A lock only mutual excludes if
/// everyone agrees to take it — one path locking and the other not is the same
/// lost update as nobody locking, plus a deadlock risk from the disagreement in
/// ordering.
pub(crate) async fn lock_document_tx(
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
/// Maximum number of change-log entries returned in a single history page.
pub const MAX_HISTORY_PAGE: i64 = 200;

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

// ── yrs-native version history (docs/features/version-history-plan.md) ────────────────
// The op-model history below froze with P4①b; real history routes through these.

/// The document's version timeline, newest first. Metadata only (no `state`
/// blobs) so the panel loads cheaply. Auto rows and named rows are interleaved
/// by `created_at`.
pub async fn list_yrs_versions(db: &PgPool, document_id: Uuid) -> ApiResult<Vec<YrsVersionMeta>> {
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

/// Record metadata for an uploaded object. The unique `object_key` makes a
/// duplicate completion a conflict rather than a silent overwrite.
/// Bytes of stored files this workspace already occupies, for the quota check.
///
/// Counts every `files` row, including ones the blob GC has marked unreferenced:
/// those bytes are still on the disk until the sweep removes them, and a quota
/// that ignored them would let a delete-and-reupload loop exceed it indefinitely.
/// Deduplication is already reflected — `insert_file` returns the existing row for
/// identical bytes, so re-uploading the same image adds a reference and no size.
pub async fn workspace_bytes_used(db: &PgPool, workspace_id: Uuid) -> ApiResult<i64> {
  workspace_bytes_used_excluding(db, workspace_id, None).await
}

/// [workspace_bytes_used] without the pending reservation of `exclude_key` —
/// for `complete`, whose own upload is that reservation and must not be
/// counted twice against itself.
pub async fn workspace_bytes_used_excluding(
  db: &PgPool,
  workspace_id: Uuid,
  exclude_key: Option<&str>,
) -> ApiResult<i64> {
  // Registered files PLUS browser uploads issued but never registered. The
  // latter count until the orphan sweep removes their ledger row — whether or
  // not the bytes were actually PUT, which the database cannot know; an
  // abandoned upload costs its size for the grace period rather than letting
  // never-completed uploads fill the store unmetered.
  sqlx::query_scalar::<_, i64>(
    "SELECT (
       (SELECT coalesce(sum(byte_size), 0) FROM files WHERE workspace_id = $1)
       + (SELECT coalesce(sum(p.byte_size), 0) FROM pending_file_uploads p
          WHERE p.workspace_id = $1
            AND p.object_key IS DISTINCT FROM $2
            AND NOT EXISTS (SELECT 1 FROM files f WHERE f.object_key = p.object_key))
     )::bigint",
  )
  .bind(workspace_id)
  .bind(exclude_key)
  .fetch_one(db)
  .await
  .map_err(ApiError::from)
}

/// Bytes stored across every workspace owned by `workspace_id`'s owner — the
/// per-owner quota (B7). Same counting rules as [workspace_bytes_used_excluding],
/// summed over the owner's workspaces.
pub async fn owner_bytes_used_excluding(
  db: &PgPool,
  workspace_id: Uuid,
  exclude_key: Option<&str>,
) -> ApiResult<i64> {
  sqlx::query_scalar::<_, i64>(
    "WITH owned AS (
       SELECT w.id FROM workspaces w
       WHERE w.owner_id = (SELECT owner_id FROM workspaces WHERE id = $1)
     )
     SELECT (
       (SELECT coalesce(sum(byte_size), 0) FROM files WHERE workspace_id IN (SELECT id FROM owned))
       + (SELECT coalesce(sum(p.byte_size), 0) FROM pending_file_uploads p
          WHERE p.workspace_id IN (SELECT id FROM owned)
            AND p.object_key IS DISTINCT FROM $2
            AND NOT EXISTS (SELECT 1 FROM files f WHERE f.object_key = p.object_key))
     )::bigint",
  )
  .bind(workspace_id)
  .bind(exclude_key)
  .fetch_one(db)
  .await
  .map_err(ApiError::from)
}

pub async fn insert_file(
  db: &PgPool,
  workspace_id: Uuid,
  uploaded_by: Uuid,
  object_key: &str,
  original_name: &str,
  mime_type: &str,
  byte_size: i64,
) -> ApiResult<FileRecord> {
  let mut tx = db.begin().await?;
  lock_file_object_key(&mut tx, object_key).await?;
  let file = insert_file_tx(
    &mut tx,
    workspace_id,
    uploaded_by,
    object_key,
    original_name,
    mime_type,
    byte_size,
  )
  .await?;
  tx.commit().await?;
  Ok(file)
}

/// Serialize object deletion and all paths that register the same key. The
/// namespace separates this lock from unrelated advisory-lock users. A hash
/// collision only serializes two unrelated keys; it cannot authorize deletion.
pub async fn lock_file_object_key(
  tx: &mut Transaction<'_, Postgres>,
  object_key: &str,
) -> ApiResult<()> {
  sqlx::query("SELECT pg_advisory_xact_lock(1296646977, hashtext($1))")
    .bind(object_key)
    .execute(&mut **tx)
    .await?;
  Ok(())
}

/// Remember the latest URL expiry for this key. Presign callers must hold
/// [lock_file_object_key] in the same transaction as their decision. Workspace
/// deletion holds the workspace row lock instead, then stages every `files`
/// key atomically with the cascade; GC's registered-row cleanup only removes
/// the pending version it observed before a concurrent expiry extension.
pub async fn record_pending_file_upload_tx(
  tx: &mut Transaction<'_, Postgres>,
  workspace_id: Uuid,
  object_key: &str,
  ttl_seconds: u64,
) -> ApiResult<()> {
  let ttl_seconds = i64::try_from(ttl_seconds)
    .map_err(|_| ApiError::BadRequest("upload URL lifetime is too long".to_string()))?;
  sqlx::query(
    r#"
      INSERT INTO pending_file_uploads (object_key, workspace_id, expires_at)
      VALUES ($1, $2, now() + make_interval(secs => $3::double precision))
      ON CONFLICT (object_key) DO UPDATE
      SET expires_at = GREATEST(pending_file_uploads.expires_at, EXCLUDED.expires_at)
    "#,
  )
  .bind(object_key)
  .bind(workspace_id)
  .bind(ttl_seconds)
  .execute(&mut **tx)
  .await?;
  Ok(())
}

/// Record the size a browser upload URL allows, for the quota
/// ([workspace_bytes_used]). Kept apart from [record_pending_file_upload_tx] so
/// its many server-side callers — which meter themselves — are unaffected.
pub async fn reserve_pending_upload_bytes_tx(
  tx: &mut Transaction<'_, Postgres>,
  object_key: &str,
  byte_size: i64,
) -> ApiResult<()> {
  sqlx::query(
    "UPDATE pending_file_uploads SET byte_size = GREATEST(byte_size, $2) WHERE object_key = $1",
  )
  .bind(object_key)
  .bind(byte_size)
  .execute(&mut **tx)
  .await?;
  Ok(())
}

/// Server-side PUTs also need a durable ledger entry *before* sending bytes.
/// A transaction held across PUT is not enough: a process crash would roll it
/// back and leave an object that no sweep can discover.
/// Phase one of a server-side store (D5): under the key lock, in a short
/// transaction of its own, make both collectors leave `object_key` alone while
/// the caller PUTs with NO transaction open —
/// - the orphan sweep: a fresh ledger expiry (it waits a day past expiry);
/// - the registered sweep: an existing row's unreferenced mark is cleared (it
///   re-reads the mark under this lock before deleting, and a cleared mark
///   restarts its 30-day clock). A sweep already mid-delete holds the lock, so
///   this waits for it and the PUT that follows restores the bytes.
///
/// Phase two is [register_put_file], under the lock again.
///
/// The PUT used to run inside the locked transaction instead: a pooled
/// connection and a held lock for a whole upload, and `complete` did the same
/// for its HEAD.
pub async fn record_pending_file_upload(
  db: &PgPool,
  workspace_id: Uuid,
  object_key: &str,
  ttl_seconds: u64,
) -> ApiResult<()> {
  let mut tx = db.begin().await?;
  lock_file_object_key(&mut tx, object_key).await?;
  record_pending_file_upload_tx(&mut tx, workspace_id, object_key, ttl_seconds).await?;
  sqlx::query(
    "UPDATE files SET unreferenced_since = NULL WHERE object_key = $1 AND unreferenced_since IS NOT NULL",
  )
  .bind(object_key)
  .execute(&mut *tx)
  .await?;
  tx.commit().await?;
  Ok(())
}

/// Phase two of a server-side store: register the PUT object under the key
/// lock, in a short transaction. See [record_pending_file_upload].
pub async fn register_put_file(
  db: &PgPool,
  workspace_id: Uuid,
  uploaded_by: Uuid,
  object_key: &str,
  original_name: &str,
  mime_type: &str,
  byte_size: i64,
) -> ApiResult<FileRecord> {
  let mut tx = db.begin().await?;
  lock_file_object_key(&mut tx, object_key).await?;
  let file = insert_file_tx(
    &mut tx,
    workspace_id,
    uploaded_by,
    object_key,
    original_name,
    mime_type,
    byte_size,
  )
  .await?;
  tx.commit().await?;
  Ok(file)
}

pub async fn pending_file_upload_exists_tx(
  tx: &mut Transaction<'_, Postgres>,
  workspace_id: Uuid,
  object_key: &str,
) -> ApiResult<bool> {
  sqlx::query_scalar(
    "SELECT EXISTS (SELECT 1 FROM pending_file_uploads WHERE workspace_id = $1 AND object_key = $2)",
  )
  .bind(workspace_id)
  .bind(object_key)
  .fetch_one(&mut **tx)
  .await
  .map_err(ApiError::from)
}

/// Insert a file while the caller's key lock is held across its storage HEAD
/// or PUT and this row write. `complete` needs that larger critical section:
/// locking only the INSERT leaves a gap in which GC can delete the object.
pub async fn insert_file_tx(
  tx: &mut Transaction<'_, Postgres>,
  workspace_id: Uuid,
  uploaded_by: Uuid,
  object_key: &str,
  original_name: &str,
  mime_type: &str,
  byte_size: i64,
) -> ApiResult<FileRecord> {
  // Object keys are content-addressed (sha256), so re-uploading identical bytes
  // hits the UNIQUE(object_key) constraint. Treat that as dedup: return the
  // existing row rather than failing. The SET also clears the GC mark — see
  // [reclaim_file_tx] for why handing out an existing row must.
  sqlx::query_as::<_, FileRecord>(
    r#"
      INSERT INTO files (workspace_id, uploaded_by, object_key, original_name, mime_type, byte_size)
      VALUES ($1, $2, $3, $4, $5, $6)
      ON CONFLICT (object_key) DO UPDATE SET unreferenced_since = NULL
      RETURNING id, workspace_id, uploaded_by, object_key, original_name, mime_type, byte_size, created_at
    "#,
  )
  .bind(workspace_id)
  .bind(uploaded_by)
  .bind(object_key)
  .bind(original_name)
  .bind(mime_type)
  .bind(byte_size)
  .fetch_one(&mut **tx)
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

/// Transactional lookup for the presign/GC key-locked critical section.
/// Clear the blob-GC mark on a row that is being handed out again by dedup.
///
/// GC's "fresh upload" guard is `MIN_OBJECT_AGE` against the row's ORIGINAL
/// `created_at`, so a row unreferenced for 30+ days and then reused (the same
/// image uploaded again, an import's prefetch hitting it) had no protection:
/// a sweep landing before the new page saved deleted the object, and the page
/// kept a dangling `file_id`. Clearing the mark restarts the full grace period.
/// Callers hold [lock_file_object_key], the same lock GC's collect takes.
pub async fn reclaim_file_tx(tx: &mut Transaction<'_, Postgres>, file_id: Uuid) -> ApiResult<()> {
  sqlx::query(
    "UPDATE files SET unreferenced_since = NULL WHERE id = $1 AND unreferenced_since IS NOT NULL",
  )
  .bind(file_id)
  .execute(&mut **tx)
  .await?;
  Ok(())
}

pub async fn fetch_file_by_key_tx(
  tx: &mut Transaction<'_, Postgres>,
  workspace_id: Uuid,
  object_key: &str,
) -> ApiResult<Option<FileRecord>> {
  sqlx::query_as::<_, FileRecord>(
    r#"
      SELECT id, workspace_id, uploaded_by, object_key, original_name, mime_type, byte_size, created_at
      FROM files
      WHERE workspace_id = $1 AND object_key = $2
    "#,
  )
  .bind(workspace_id)
  .bind(object_key)
  .fetch_optional(&mut **tx)
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
