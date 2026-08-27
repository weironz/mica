//! Comment threads + replies — the store layer (docs/comments-plan.md).
//!
//! Anchors live HERE, in Postgres, never inside the yrs document: the Markdown
//! body stays byte-identical, so the round-trip invariant needs no changes and a
//! comment can never leak into an export. The anchor primitives themselves are
//! `mica_core::doc::{CommentAnchor, CommentRange}` (`sticky_for_range` to take
//! one, `resolve_range` to map it back onto the live text), plus
//! `mica_core::quote_match` for putting a dead anchor back on its text.
//!
//! [`anchor_state`] is the one place that decides what a thread's anchor means
//! today — highlight, re-anchor, or orphan — so a listing can never disagree with
//! what the document says.
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
pub use mica_core::QuoteIndex;

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

/// Try to bring a dead anchor back to life from the thread's saved `quote`.
///
/// Call ONLY when the stored anchor no longer resolves (or resolves empty). The
/// common cause is not a deletion at all: `set_blocks` — every REST/MCP write and
/// every version restore — rewrites each block's text object, so anchors die with
/// the text untouched. Matching the quote tells that apart from a real deletion,
/// where nothing is found and the thread rightly stays an orphan.
///
/// Returns the range it now covers plus a FRESH anchor for it; the caller
/// persists that with [`reanchor`]. None means "no confident match" — the rules
/// live in `mica_core::quote_match` and deliberately refuse ambiguity.
///
/// The document is never written: comments are a side store, and re-anchoring
/// only replaces bytes in `comment_threads`.
pub fn rematch(
  doc: &MicaDoc,
  index: &QuoteIndex,
  quote: &str,
  prefer_block: &str,
) -> Option<(CommentRange, CommentAnchor)> {
  let range = index.find(quote, Some(prefer_block))?;
  let anchor = doc.sticky_for_range(
    &range.start_block,
    range.start_offset,
    &range.end_block,
    range.end_offset,
  )?;
  Some((range, anchor))
}

/// What a listing must say about one thread, derived from the document as it is
/// now — never from the stored `status`, which is only a cache of this.
#[derive(Debug, Clone)]
pub struct AnchorState {
  /// Where to highlight now. None → orphaned: show the thread against its quote
  /// and draw nothing.
  pub range: Option<CommentRange>,
  /// Set when the thread was re-anchored from its quote — persist it with
  /// [`reanchor`], or the next listing repeats the work.
  pub fresh_anchor: Option<CommentAnchor>,
  /// The status to report (and to write, if it differs from the stored one).
  pub status: String,
}

/// Resolve one thread's anchor, re-anchoring from its quote if it died.
///
/// `index` is the listing's lazily-built quote index: pass `&mut None` and it is
/// built on the first thread that needs it, so a document whose anchors all still
/// resolve — the normal case — never pays for flattening its text.
pub fn anchor_state(
  doc: &MicaDoc,
  index: &mut Option<QuoteIndex>,
  row: &ThreadRow,
) -> AnchorState {
  // "Unresolvable" and "collapsed to nothing" are the same thing to a reader:
  // yrs keeps tombstones, so deleting the anchored text usually collapses the
  // range instead of failing to resolve.
  if let Some(range) = doc.resolve_range(&row.anchor()).filter(|r| !r.is_empty()) {
    return AnchorState {
      range: Some(range),
      fresh_anchor: None,
      status: row.status.clone(),
    };
  }
  let idx = index.get_or_insert_with(|| QuoteIndex::from_blocks(&doc.to_blocks()));
  if let Some((range, anchor)) = rematch(doc, idx, &row.quote, &row.anchor_start_block) {
    return AnchorState {
      range: Some(range),
      fresh_anchor: Some(anchor),
      // It has text under it again. A resolved thread stays resolved — getting
      // its anchor back is not a reason to re-open a finished discussion.
      status: if row.status == STATUS_ORPHANED {
        STATUS_OPEN.to_string()
      } else {
        row.status.clone()
      },
    };
  }
  AnchorState {
    range: None,
    fresh_anchor: None,
    status: if row.status == STATUS_OPEN {
      STATUS_ORPHANED.to_string()
    } else {
      row.status.clone()
    },
  }
}

/// Store a re-anchored thread's new sticky pair.
///
/// An `orphaned` thread becomes `open` again — it has text under it once more.
/// A `resolved` one keeps its status: re-anchoring is bookkeeping, not a reason
/// to re-open a finished discussion.
pub async fn reanchor(db: &PgPool, thread_id: Uuid, anchor: &CommentAnchor) -> ApiResult<()> {
  sqlx::query(
    r#"
      UPDATE comment_threads
      SET anchor_start_block = $2, anchor_start_sticky = $3,
          anchor_end_block = $4, anchor_end_sticky = $5,
          status = CASE WHEN status = 'orphaned' THEN 'open' ELSE status END
      WHERE id = $1
    "#,
  )
  .bind(thread_id)
  .bind(&anchor.start_block)
  .bind(&anchor.start_sticky)
  .bind(&anchor.end_block)
  .bind(&anchor.end_sticky)
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

/// `anchor_state` is where a listing decides "highlight here", "re-anchor" or
/// "orphan". Those three answers are what the client renders, so they are pinned
/// here without a database — the Postgres tests (`tests/comments_pg.rs`) cover
/// the storage half.
#[cfg(test)]
mod tests {
  use super::*;
  use mica_core::Block;

  fn doc_with(text: &str) -> MicaDoc {
    MicaDoc::from_blocks(
      "r",
      &[
        Block::new("r", "page").with_children(vec!["a".into()]),
        Block::new("a", "paragraph").with_text(text.to_string()),
      ],
    )
  }

  fn row(anchor: &CommentAnchor, quote: &str, status: &str) -> ThreadRow {
    ThreadRow {
      id: Uuid::new_v4(),
      document_id: Uuid::new_v4(),
      anchor_start_block: anchor.start_block.clone(),
      anchor_start_sticky: anchor.start_sticky.clone(),
      anchor_end_block: anchor.end_block.clone(),
      anchor_end_sticky: anchor.end_sticky.clone(),
      quote: quote.to_string(),
      status: status.to_string(),
      created_by: Uuid::new_v4(),
      created_at: Utc::now(),
      resolved_by: None,
      resolved_at: None,
    }
  }

  /// A rewritten document: identical text, brand-new text objects — what every
  /// REST/MCP write and every version restore does to a document.
  fn rewrite(doc: &mut MicaDoc) {
    let blocks = doc.to_blocks();
    doc.set_blocks("r", &blocks);
  }

  /// **The acceptance test for `set_block_prop`** (`docs/page-title-plan.md`
  /// §5.1). Once the title lives on the root block, renaming a page has to write
  /// into the DOCUMENT — and the obvious way to do that, `set_blocks`, silently
  /// orphans every comment on the page: identical characters, brand-new text
  /// objects, dead sticky indexes.
  ///
  /// So the narrow primitive must leave `text` alone. Proven by contrast: the
  /// test right below performs the SAME rename through `set_blocks` and asserts
  /// that every anchor has to be RESCUED (re-anchored by quote, at the cost of
  /// flattening the document) — which makes "this one is untouched" a measured
  /// difference rather than a hopeful assertion about code that might be doing
  /// nothing at all.
  #[test]
  fn renaming_through_the_narrow_primitive_keeps_comment_anchors_alive() {
    let mut doc = doc_with("hello world");
    let anchor = doc.sticky_for_range("a", 6, "a", 11).unwrap();

    // What a rename will do: one key on the root block's props.
    assert!(doc.set_block_prop("r", "title", &serde_json::json!("新名字")));

    let mut index = None;
    let state = anchor_state(&doc, &mut index, &row(&anchor, "world", STATUS_OPEN));
    assert_eq!(
      state.range.map(|r| (r.start_offset, r.end_offset)),
      Some((6, 11)),
      "the anchor still resolves to the same words after a rename"
    );
    assert_eq!(state.status, STATUS_OPEN);
    assert!(state.fresh_anchor.is_none(), "nothing needed re-anchoring");

    // And the title really landed, so this is not passing by doing nothing.
    let root = doc.to_blocks().into_iter().find(|b| b.id == "r").unwrap();
    assert_eq!(
      root.data.get("title").and_then(|t| t.as_str()),
      Some("新名字")
    );
  }

  /// The contrast that gives the test above its teeth — and it is NOT
  /// "the comment disappears".
  ///
  /// Measured, after a first attempt asserted the wrong thing: a `set_blocks`
  /// rename does kill the sticky index (so the repo's "brand-new text objects"
  /// note is accurate), but the quote-based re-anchor then rescues it onto the
  /// same words. So the observable difference is the RESCUE, not the loss:
  /// every comment on the page has to be re-anchored, which costs a document
  /// flatten (the quote index) and writes a fresh anchor back.
  ///
  /// That is still a good reason to have the narrow primitive, because the
  /// rescue is best-effort by design: an ambiguous or deleted quote stays
  /// orphaned rather than guessing (`anchor_state`'s contract — a wrong anchor
  /// is worse than none). Renaming a page should not roll that dice on every
  /// thread it has.
  #[test]
  fn renaming_through_set_blocks_forces_every_anchor_to_be_rescued() {
    let mut doc = doc_with("hello world");
    let anchor = doc.sticky_for_range("a", 6, "a", 11).unwrap();

    let mut blocks = doc.to_blocks();
    let root = blocks.iter_mut().find(|b| b.id == "r").unwrap();
    root.data = serde_json::json!({"title": "新名字"});
    doc.set_blocks("r", &blocks);

    let mut index = None;
    let state = anchor_state(&doc, &mut index, &row(&anchor, "world", STATUS_OPEN));
    assert!(
      state.fresh_anchor.is_some(),
      "the original sticky index must have died and been re-anchored; if it \
       survived, set_block_prop is buying nothing and §5.1 of the plan is stale"
    );
    assert!(
      index.is_some(),
      "re-anchoring flattens the document to search for the quote — the cost \
       the narrow primitive avoids"
    );
    // The rescue landed correctly HERE because the quote is unique. It is not
    // guaranteed to: that is the risk a rename must not take.
    assert_eq!(
      state.range.map(|r| (r.start_offset, r.end_offset)),
      Some((6, 11))
    );
  }

  #[test]
  fn a_living_anchor_is_reported_as_is_and_costs_no_quote_index() {
    let doc = doc_with("hello world");
    let anchor = doc.sticky_for_range("a", 6, "a", 11).unwrap();
    let mut index = None;
    let state = anchor_state(&doc, &mut index, &row(&anchor, "world", STATUS_OPEN));
    assert_eq!(
      state.range.map(|r| (r.start_offset, r.end_offset)),
      Some((6, 11))
    );
    assert!(state.fresh_anchor.is_none(), "nothing to re-anchor");
    assert_eq!(state.status, STATUS_OPEN);
    assert!(
      index.is_none(),
      "flattening the document must stay off the normal path"
    );
  }

  #[test]
  fn a_dead_anchor_over_unchanged_text_is_re_anchored_and_reopened() {
    let mut doc = doc_with("hello world");
    let anchor = doc.sticky_for_range("a", 6, "a", 11).unwrap();
    rewrite(&mut doc);
    assert!(
      doc.resolve_range(&anchor).filter(|r| !r.is_empty()).is_none(),
      "the rewrite must kill the anchor — otherwise this proves nothing"
    );

    let state = anchor_state(&doc, &mut None, &row(&anchor, "world", STATUS_ORPHANED));
    assert_eq!(
      state.range.map(|r| (r.start_offset, r.end_offset)),
      Some((6, 11))
    );
    let fresh = state.fresh_anchor.expect("a fresh anchor to persist");
    assert_eq!(
      doc
        .resolve_range(&fresh)
        .map(|r| (r.start_offset, r.end_offset)),
      Some((6, 11)),
      "the fresh anchor must resolve to the same words"
    );
    assert_eq!(
      state.status, STATUS_OPEN,
      "it has text under it again, so it is no longer an orphan"
    );
  }

  #[test]
  fn re_anchoring_never_reopens_a_resolved_thread() {
    let mut doc = doc_with("hello world");
    let anchor = doc.sticky_for_range("a", 6, "a", 11).unwrap();
    rewrite(&mut doc);
    let state = anchor_state(&doc, &mut None, &row(&anchor, "world", STATUS_RESOLVED));
    assert!(state.fresh_anchor.is_some(), "re-anchored");
    assert_eq!(
      state.status, STATUS_RESOLVED,
      "recovering an anchor is bookkeeping, not a reason to re-open"
    );
  }

  #[test]
  fn text_that_is_really_gone_orphans_an_open_thread() {
    let mut doc = doc_with("hello world");
    let anchor = doc.sticky_for_range("a", 6, "a", 11).unwrap();
    doc.text_delete("a", 6, 5); // delete "world"
    let state = anchor_state(&doc, &mut None, &row(&anchor, "world", STATUS_OPEN));
    assert!(state.range.is_none(), "no highlight over unrelated words");
    assert!(state.fresh_anchor.is_none());
    assert_eq!(state.status, STATUS_ORPHANED);
  }

  #[test]
  fn a_resolved_thread_whose_text_is_gone_stays_resolved() {
    // `mark_orphaned` refuses to overwrite a resolve; the derived status must
    // agree with it, or the listing and the database would disagree forever.
    let mut doc = doc_with("hello world");
    let anchor = doc.sticky_for_range("a", 6, "a", 11).unwrap();
    doc.text_delete("a", 6, 5);
    let state = anchor_state(&doc, &mut None, &row(&anchor, "world", STATUS_RESOLVED));
    assert!(state.range.is_none());
    assert_eq!(state.status, STATUS_RESOLVED);
  }
}
