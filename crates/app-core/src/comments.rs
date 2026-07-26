//! Comment threads + replies — the store layer (docs/comments-plan.md).
//!
//! Anchors live HERE, in Postgres, never inside the yrs document: the Markdown
//! body stays byte-identical, so the round-trip invariant needs no changes and a
//! comment can never leak into an export. The anchor primitives themselves are
//! `mica_core::doc::{CommentAnchor, CommentRange}` (`sticky_for_range` to take
//! one, `resolve_range` to map it back onto the live text).
//!
//! Every query below is a literal string (sqlx 0.9 only accepts `&'static str`,
//! and column lists spelled out beat a `SELECT *` that drifts with the schema).

use chrono::{DateTime, Utc};
use mica_core::doc::CommentAnchor;
use mica_core::MicaDoc;
use mica_infra::{ApiError, ApiResult};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

/// Re-exported so callers (the API layer) can name an anchor/range without taking
/// a direct dependency on `mica-core` — this module is their door to comments.
pub use mica_core::doc::{CommentAnchor as Anchor, CommentRange};

pub const STATUS_OPEN: &str = "open";
pub const STATUS_RESOLVED: &str = "resolved";
/// The anchored text is gone. The discussion is kept and shown against its
/// `quote` — deleting a sentence should not silently delete the argument about it.
pub const STATUS_ORPHANED: &str = "orphaned";

#[derive(Debug, Clone, FromRow)]
pub struct ThreadRow {
  pub id: Uuid,
  pub document_id: Uuid,
  pub anchor_start_block: String,
  pub anchor_start_sticky: Vec<u8>,
  pub anchor_end_block: String,
  pub anchor_end_sticky: Vec<u8>,
  pub quote: String,
  pub status: String,
  pub created_by: Uuid,
  pub created_at: DateTime<Utc>,
  pub resolved_by: Option<Uuid>,
  pub resolved_at: Option<DateTime<Utc>>,
}

impl ThreadRow {
  pub fn anchor(&self) -> CommentAnchor {
    CommentAnchor {
      start_block: self.anchor_start_block.clone(),
      start_sticky: self.anchor_start_sticky.clone(),
      end_block: self.anchor_end_block.clone(),
      end_sticky: self.anchor_end_sticky.clone(),
    }
  }
}

#[derive(Debug, Clone, FromRow)]
pub struct CommentRow {
  pub id: Uuid,
  pub thread_id: Uuid,
  pub author_id: Uuid,
  pub body: String,
  pub created_at: DateTime<Utc>,
  pub edited_at: Option<DateTime<Utc>>,
}

/// The document's current CRDT state, for anchor work only (take a new sticky
/// index, or resolve a stored one). Read-only: it never writes the base back.
///
/// Goes through the same `ensure_base_tx` + panic-guarded decode as the sync
/// path, so a document that has only ever been written through the op model gets
/// its base derived on first use, and a corrupt base is an error rather than a
/// process-killing panic.
pub async fn load_doc(db: &PgPool, document_id: Uuid) -> ApiResult<MicaDoc> {
  let mut tx = db.begin().await?;
  let base = crate::sync::ensure_base_tx(&mut tx, document_id).await?;
  tx.commit().await?;
  crate::sync::guarded_from_update(&base.state)
    .map_err(|e| ApiError::Internal(format!("corrupt base state: {e}")))
}

/// Create a thread with its first comment, atomically — a thread with no comment
/// is a highlight nobody can read, so the two must never come apart.
pub async fn create_thread(
  db: &PgPool,
  document_id: Uuid,
  author_id: Uuid,
  anchor: &CommentAnchor,
  quote: &str,
  body: &str,
) -> ApiResult<(ThreadRow, CommentRow)> {
  let mut tx = db.begin().await?;
  let thread: ThreadRow = sqlx::query_as(
    r#"
      INSERT INTO comment_threads (id, document_id, anchor_start_block, anchor_start_sticky,
        anchor_end_block, anchor_end_sticky, quote, status, created_by)
      VALUES ($1, $2, $3, $4, $5, $6, $7, 'open', $8)
      RETURNING id, document_id, anchor_start_block, anchor_start_sticky, anchor_end_block,
        anchor_end_sticky, quote, status, created_by, created_at, resolved_by, resolved_at
    "#,
  )
  .bind(Uuid::new_v4())
  .bind(document_id)
  .bind(&anchor.start_block)
  .bind(&anchor.start_sticky)
  .bind(&anchor.end_block)
  .bind(&anchor.end_sticky)
  .bind(quote)
  .bind(author_id)
  .fetch_one(&mut *tx)
  .await?;

  let comment: CommentRow = sqlx::query_as(
    r#"
      INSERT INTO comments (id, thread_id, author_id, body)
      VALUES ($1, $2, $3, $4)
      RETURNING id, thread_id, author_id, body, created_at, edited_at
    "#,
  )
  .bind(Uuid::new_v4())
  .bind(thread.id)
  .bind(author_id)
  .bind(body)
  .fetch_one(&mut *tx)
  .await?;

  tx.commit().await?;
  Ok((thread, comment))
}

/// Every thread on a document, oldest first (creation order reads like a log).
pub async fn list_threads(db: &PgPool, document_id: Uuid) -> ApiResult<Vec<ThreadRow>> {
  Ok(
    sqlx::query_as(
      r#"
        SELECT id, document_id, anchor_start_block, anchor_start_sticky, anchor_end_block,
          anchor_end_sticky, quote, status, created_by, created_at, resolved_by, resolved_at
        FROM comment_threads WHERE document_id = $1 ORDER BY created_at
      "#,
    )
    .bind(document_id)
    .fetch_all(db)
    .await?,
  )
}

/// All replies for the given threads, in one query (no N+1 per thread).
pub async fn list_comments(db: &PgPool, thread_ids: &[Uuid]) -> ApiResult<Vec<CommentRow>> {
  if thread_ids.is_empty() {
    return Ok(Vec::new());
  }
  Ok(
    sqlx::query_as(
      r#"
        SELECT id, thread_id, author_id, body, created_at, edited_at
        FROM comments WHERE thread_id = ANY($1) ORDER BY thread_id, created_at
      "#,
    )
    .bind(thread_ids)
    .fetch_all(db)
    .await?,
  )
}

pub async fn fetch_thread(db: &PgPool, thread_id: Uuid) -> ApiResult<Option<ThreadRow>> {
  Ok(
    sqlx::query_as(
      r#"
        SELECT id, document_id, anchor_start_block, anchor_start_sticky, anchor_end_block,
          anchor_end_sticky, quote, status, created_by, created_at, resolved_by, resolved_at
        FROM comment_threads WHERE id = $1
      "#,
    )
    .bind(thread_id)
    .fetch_optional(db)
    .await?,
  )
}

pub async fn add_reply(
  db: &PgPool,
  thread_id: Uuid,
  author_id: Uuid,
  body: &str,
) -> ApiResult<CommentRow> {
  Ok(
    sqlx::query_as(
      r#"
        INSERT INTO comments (id, thread_id, author_id, body)
        VALUES ($1, $2, $3, $4)
        RETURNING id, thread_id, author_id, body, created_at, edited_at
      "#,
    )
    .bind(Uuid::new_v4())
    .bind(thread_id)
    .bind(author_id)
    .bind(body)
    .fetch_one(db)
    .await?,
  )
}

/// Resolve / re-open a thread.
///
/// The ANCHOR IS KEPT either way — "show resolved" can highlight it again, and a
/// resolved thread is history, not garbage. Resolving an orphaned thread is
/// allowed (the discussion is finished even though its text is gone); re-opening
/// returns it to `open` and the next list pass re-derives orphanhood from the
/// anchor, so the status cannot get stuck lying.
pub async fn set_resolved(
  db: &PgPool,
  thread_id: Uuid,
  actor_id: Uuid,
  resolved: bool,
) -> ApiResult<()> {
  if resolved {
    sqlx::query(
      r#"
        UPDATE comment_threads SET status = 'resolved', resolved_by = $2, resolved_at = now()
        WHERE id = $1
      "#,
    )
    .bind(thread_id)
    .bind(actor_id)
    .execute(db)
    .await?;
  } else {
    sqlx::query(
      r#"
        UPDATE comment_threads SET status = 'open', resolved_by = NULL, resolved_at = NULL
        WHERE id = $1
      "#,
    )
    .bind(thread_id)
    .execute(db)
    .await?;
  }
  Ok(())
}

/// Flag threads whose anchor no longer resolves. Best-effort bookkeeping done
/// while listing: `open` only, so it never overwrites a deliberate `resolved`.
pub async fn mark_orphaned(db: &PgPool, thread_ids: &[Uuid]) -> ApiResult<()> {
  if thread_ids.is_empty() {
    return Ok(());
  }
  sqlx::query(
    r#"
      UPDATE comment_threads SET status = 'orphaned'
      WHERE id = ANY($1) AND status = 'open'
    "#,
  )
  .bind(thread_ids)
  .execute(db)
  .await?;
  Ok(())
}

/// Delete a thread and (by cascade) its replies.
pub async fn delete_thread(db: &PgPool, thread_id: Uuid) -> ApiResult<()> {
  sqlx::query("DELETE FROM comment_threads WHERE id = $1")
    .bind(thread_id)
    .execute(db)
    .await?;
  Ok(())
}
