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
  let mut tx = db.begin().await?;

  let locked = lock_document_tx(&mut tx, workspace_id, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;
  let current_snapshot = latest_snapshot_tx(&mut tx, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;

  let current_payload = payload_from_value(current_snapshot.payload)
    .map_err(|error| ApiError::BadRequest(format!("invalid document snapshot: {error}")))?;
  let next_payload = apply_operations(current_payload, operations)
    .map_err(|error| ApiError::BadRequest(error.to_string()))?;
  let next_payload_value =
    serde_json::to_value(next_payload).map_err(|error| ApiError::Internal(error.to_string()))?;
  let operations_value =
    serde_json::to_value(operations).map_err(|error| ApiError::Internal(error.to_string()))?;
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
