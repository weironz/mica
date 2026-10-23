use chrono::{DateTime, Utc};
use mica_app_core::{
  documents::DocumentOperation,
  store::{self, DocumentRecord},
};
use mica_infra::{ApiError, ApiResult};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct CreateDocumentRequest {
  name: String,
  parent_view_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct CreateFolderRequest {
  name: String,
  parent_view_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateViewRequest {
  name: String,
  /// The page/folder emoji. Three-way on purpose, because a rename must never be
  /// able to wipe an icon by omission:
  /// - absent (`null`) → leave the current icon alone
  /// - `""`            → clear it
  /// - `"📗"`          → set it
  #[serde(default)]
  icon: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MoveViewRequest {
  parent_view_id: Option<Uuid>,
  position: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReorderRequest {
  /// Parent that every listed view becomes a child of (null = top level).
  #[serde(default)]
  parent_view_id: Option<Uuid>,
  /// The COMPLETE desired order of that parent's children. Positions are
  /// reassigned evenly-spaced in this order; pass the full set so nothing keeps
  /// a stale position that interleaves with the reordered ones.
  ordered_view_ids: Vec<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct ReorderResponse {
  reordered: usize,
}

/// Ids for a batch view operation (trash / restore).
///
/// One request instead of N: reorganising a workspace means touching pages by
/// the hundred, and the per-view routes made that N round trips — enough that
/// one report gave up on the tools entirely and scripted raw HTTP instead.
#[derive(Debug, Deserialize)]
pub struct BatchViewsRequest {
  view_ids: Vec<Uuid>,
}

/// Ids plus the destination for a batch move.
#[derive(Debug, Deserialize)]
pub struct BatchMoveRequest {
  view_ids: Vec<Uuid>,
  /// Parent every listed view becomes a child of (null = top level).
  #[serde(default)]
  parent_view_id: Option<Uuid>,
}

/// What a batch operation actually did.
///
/// [affected] counts every row the statement touched — for trash and restore
/// that INCLUDES descendants pulled along with a folder, so it is normally
/// larger than `view_ids.len()`. [skipped] lists requested ids the statement
/// did not reach: already in that state, or gone. Echoing the requested count
/// back as the result would be the lie `reorder_views` was fixed for (P1-3) —
/// a partial batch reads as a full one and the caller stops looking.
#[derive(Debug, Serialize)]
pub struct BatchViewsResponse {
  affected: usize,
  skipped: Vec<Uuid>,
}

/// Ceiling on one batch. Guards the recursive CTE and the request body against a
/// runaway caller; well above the few hundred a real reorganisation needs.
const MAX_BATCH_VIEWS: usize = 1000;

#[derive(Debug, Deserialize)]
pub struct ApplyDocumentUpdateRequest {
  operations: Vec<DocumentOperation>,
}

#[derive(Debug, Deserialize)]
pub struct ImportMarkdownRequest {
  name: String,
  parent_view_id: Option<Uuid>,
  markdown: String,
}

#[derive(Debug, Serialize)]
pub struct ViewListResponse {
  views: Vec<View>,
}

/// What emptying the recycle bin actually removed.
///
/// Counts rather than a view list: the list is empty by construction afterwards,
/// and the numbers are what the client needs in order to confirm what it just did.
#[derive(Debug, Serialize)]
pub struct PurgeTrashResponse {
  views_deleted: i64,
  documents_deleted: i64,
}

#[derive(Debug, Serialize)]
pub struct DocumentCreateResponse {
  document: DocumentRecord,
  view: View,
}

#[derive(Debug, Serialize)]
pub struct DocumentBootstrapResponse {
  document: DocumentRecord,
  view: View,
  snapshot: store::AppliedContent,
}

#[derive(Debug, Serialize)]
pub struct DocumentUpdateResponse {
  document: DocumentRecord,
  snapshot: store::AppliedContent,
  update: store::AppliedOperations,
}

#[derive(Debug, Serialize)]
pub struct MarkdownExportResponse {
  markdown: String,
}

#[derive(Debug, Serialize)]
pub struct ViewResponse {
  view: View,
}

/// The views-table column list a [View] row decodes from. Spelled once and
/// concatenated into every SELECT/RETURNING below: the copies had already
/// drifted in shape (two textual orders, one unaliased cast), and adding a
/// column one place used to mean hunting the rest.
///
/// A macro rather than a `const` because `concat!` expands only literals and
/// macros, never const items: runtime formatting would cost the `&'static str`
/// shape every statement here relies on.
macro_rules! view_columns {
  () => {
    concat!(
      "id, workspace_id, parent_view_id, object_id, object_type::text AS object_type, ",
      "name, icon, position, is_deleted, created_by, created_at, updated_at",
    )
  };
}

/// The same list qualified `v.`, for statements that alias views.
macro_rules! prefixed_view_columns {
  () => {
    concat!(
      "v.id, v.workspace_id, v.parent_view_id, v.object_id, ",
      "v.object_type::text AS object_type, v.name, v.icon, v.position, v.is_deleted, ",
      "v.created_by, v.created_at, v.updated_at",
    )
  };
}

/// The `documents.root_block_id` every new page starts from.
pub(crate) fn new_root_block_id() -> String {
  format!("block_{}", Uuid::new_v4().simple())
}

/// The choices [`insert_page`] cannot make for its caller: an importer
/// pre-assigns view ids (so a page that arrives in the archive before its
/// parent can still refer to it) and may carry an icon; the plain create
/// routes want neither.
pub(crate) struct InsertPage<'a> {
  /// The new view's id. `None` = a fresh v4 — exactly what the column's
  /// `uuid_generate_v4()` default produced when the INSERT omitted it.
  pub view_id: Option<Uuid>,
  pub parent_view_id: Option<Uuid>,
  pub name: &'a str,
  pub icon: Option<&'a str>,
  /// The page's opening content; its `root_block_id` becomes the document's.
  /// A blank page seeds with `mica_app_core::sync::empty_payload`.
  pub seed: mica_app_core::documents::DocumentSnapshotPayload,
}

/// Insert a whole new page in ONE transaction: the `documents` row, its yrs
/// base seeded from `page.seed`, and the matching `views` row. A page that
/// existed for even a moment without content was unreadable and unsearchable
/// (before S4 the base was built afterwards, best-effort), and seeding the
/// base here co-writes `content_text` (the FTS index), so a just-created or
/// just-imported page is findable by its body before anyone opens it.
///
/// Three paths had each written this sequence out by hand ([create_document],
/// [import_document_markdown], and the import executor's `insert_page`); the
/// statements never differed, only the byproducts each caller kept.
pub(crate) async fn insert_page(
  db: &PgPool,
  workspace_id: Uuid,
  user_id: Uuid,
  page: InsertPage<'_>,
) -> ApiResult<(DocumentRecord, View)> {
  let mut tx = db.begin().await?;
  let document = sqlx::query_as::<_, DocumentRecord>(
    r#"
      INSERT INTO documents (workspace_id, root_block_id, created_by)
      VALUES ($1, $2, $3)
      RETURNING id, workspace_id, root_block_id, current_seq, created_by, created_at, updated_at
    "#,
  )
  .bind(workspace_id)
  .bind(&page.seed.root_block_id)
  .bind(user_id)
  .fetch_one(&mut *tx)
  .await?;

  mica_app_core::sync::seed_base_tx(&mut tx, document.id, page.seed).await?;

  let view = sqlx::query_as::<_, View>(concat!(
    r#"
      INSERT INTO views (
        id, workspace_id, parent_view_id, object_id, object_type, name, position, created_by, icon
      )
      VALUES ($1, $2, $3, $4, 'document', $5, $6, $7, $8)
      RETURNING
        "#,
    view_columns!(),
    "\n    ",
  ))
  .bind(page.view_id.unwrap_or_else(Uuid::new_v4))
  .bind(workspace_id)
  .bind(page.parent_view_id)
  .bind(document.id)
  .bind(page.name)
  .bind(Uuid::now_v7().to_string())
  .bind(user_id)
  .bind(page.icon)
  .fetch_one(&mut *tx)
  .await?;

  tx.commit().await?;
  Ok((document, view))
}

#[derive(Debug, Serialize, FromRow)]
pub struct View {
  id: Uuid,
  workspace_id: Uuid,
  parent_view_id: Option<Uuid>,
  object_id: Uuid,
  object_type: String,
  name: String,
  icon: Option<String>,
  position: String,
  is_deleted: bool,
  created_by: Uuid,
  created_at: DateTime<Utc>,
  updated_at: DateTime<Utc>,
  /// Size in bytes of this page's stored CRDT state, present only when the
  /// listing was asked for `with_stats`. Absent everywhere else, so every other
  /// response keeps its exact shape.
  ///
  /// Read it in ONE direction only: **small means nearly empty**, which is what
  /// makes "find the pages that are just a title" a listing instead of a read
  /// of every page. Large does NOT mean lots of text — a CRDT keeps tombstones,
  /// so a page whose content was deleted stays big. Anyone using this as a word
  /// count will be wrong.
  #[sqlx(default)]
  #[serde(skip_serializing_if = "Option::is_none")]
  state_bytes: Option<i64>,
}

/// Filters for [list_views]. All optional, and with none of them the response
/// is byte-for-byte what it always was.
///
/// Listing a 437-page workspace returned every page in one 125 KB body, which
/// is both more than a caller usually wants and enough to be truncated by
/// whatever sits in the middle — leaving no way to get the structure at all.
#[derive(Debug, Deserialize)]
pub struct ListViewsQuery {
  /// Only views under this one. Omitted, the listing starts at the top level.
  parent_view_id: Option<Uuid>,
  /// How many levels below the starting point to descend; 1 = direct children
  /// only. Omitted = all the way down.
  depth: Option<i32>,
  limit: Option<i64>,
  offset: Option<i64>,
  /// Include [View::state_bytes]. Off by default: it joins the content table,
  /// and a plain listing should not pay for what it does not ask for.
  #[serde(default)]
  with_stats: bool,
}

/// Reject an empty, oversized, or duplicate-bearing id list once, so each batch
/// handler below is just its statement. Duplicates are an error rather than
/// silently deduped: in a set operation they mean the caller's list is not what
/// it thinks it is, and the counts it gets back would not add up.
fn check_batch_ids(view_ids: &[Uuid]) -> ApiResult<()> {
  if view_ids.is_empty() {
    return Err(ApiError::BadRequest("view_ids cannot be empty".to_string()));
  }
  if view_ids.len() > MAX_BATCH_VIEWS {
    return Err(ApiError::BadRequest(format!(
      "at most {MAX_BATCH_VIEWS} view_ids per request, got {}",
      view_ids.len()
    )));
  }
  let mut seen = std::collections::HashSet::new();
  for id in view_ids {
    if !seen.insert(*id) {
      return Err(ApiError::BadRequest(format!("duplicate view id {id}")));
    }
  }
  Ok(())
}

/// The requested ids the statement did not reach, given the seed ids it did.
fn skipped_ids(requested: &[Uuid], touched: &[Uuid]) -> Vec<Uuid> {
  let hit: std::collections::HashSet<Uuid> = touched.iter().copied().collect();
  requested
    .iter()
    .copied()
    .filter(|id| !hit.contains(id))
    .collect()
}

/// [list_views] with its filters applied.
///
/// Two shapes on purpose. With no `parent_view_id` and no `depth` this is the
/// same flat SELECT the endpoint always ran — reachability is NOT introduced as
/// a new condition, so a view whose parent somehow went missing keeps showing
/// up exactly as before. Ask for a subtree and it becomes a recursive walk,
/// where reachability is the whole point.
async fn fetch_views_filtered(
  db: &PgPool,
  workspace_id: Uuid,
  query: &ListViewsQuery,
) -> ApiResult<Vec<View>> {
  // Both statements are built from `concat!`ed `'static` pieces rather than
  // assembled at runtime: sqlx only accepts `&'static str`, and the rule is a
  // good one to keep even when every piece would have been a constant.
  //
  // `$5` (with_stats) gates the join itself, so a plain listing never touches
  // the content table.
  const FLAT: &str = concat!(
    "\n      SELECT\n        ",
    prefixed_view_columns!(),
    r#",
        octet_length(b.state)::bigint AS state_bytes
      FROM views v
      LEFT JOIN document_yrs_base b ON $5 AND b.document_id = v.object_id
      WHERE v.workspace_id = $1 AND v.is_deleted = false
      ORDER BY v.parent_view_id NULLS FIRST, v.position ASC, v.id
      LIMIT $3 OFFSET $4
  "#,
  );
  const SUBTREE: &str = concat!(
    r#"
      WITH RECURSIVE subtree AS (
        SELECT id, 1 AS lvl
        FROM views
        WHERE workspace_id = $1 AND is_deleted = false
          AND parent_view_id IS NOT DISTINCT FROM $2
        UNION ALL
        SELECT c.id, s.lvl + 1
        FROM views c
        JOIN subtree s ON c.parent_view_id = s.id
        WHERE c.workspace_id = $1 AND c.is_deleted = false
          AND ($6::int IS NULL OR s.lvl < $6)
      ) CYCLE id SET is_cycle USING cycle_path
      SELECT
        "#,
    prefixed_view_columns!(),
    r#",
        octet_length(b.state)::bigint AS state_bytes
      FROM views v
      JOIN subtree ON subtree.id = v.id AND NOT subtree.is_cycle
      LEFT JOIN document_yrs_base b ON $5 AND b.document_id = v.object_id
      WHERE v.workspace_id = $1 AND v.is_deleted = false
      ORDER BY v.parent_view_id NULLS FIRST, v.position ASC, v.id
      LIMIT $3 OFFSET $4
  "#,
  );

  let filtered = query.parent_view_id.is_some() || query.depth.is_some();
  let sql = if filtered { SUBTREE } else { FLAT };

  let mut sqlx_query = sqlx::query_as::<_, View>(sql)
    .bind(workspace_id)
    .bind(query.parent_view_id)
    // Postgres reads `LIMIT NULL` as no limit, so an unasked-for `limit` stays
    // exactly the old "everything". A default ceiling here would have been a
    // silent truncation: the caller gets a short list that looks complete.
    .bind(query.limit)
    .bind(query.offset.unwrap_or(0))
    .bind(query.with_stats);
  if filtered {
    sqlx_query = sqlx_query.bind(query.depth);
  }

  Ok(sqlx_query.fetch_all(db).await?)
}

async fn fetch_workspace_views(db: &PgPool, workspace_id: Uuid) -> ApiResult<Vec<View>> {
  sqlx::query_as::<_, View>(concat!(
    "\n      SELECT\n        ",
    view_columns!(),
    r#"
      FROM views
      WHERE workspace_id = $1 AND is_deleted = false
      ORDER BY parent_view_id NULLS FIRST, position ASC, id
    "#,
  ))
  .bind(workspace_id)
  .fetch_all(db)
  .await
  .map_err(ApiError::from)
}

async fn fetch_deleted_workspace_views(db: &PgPool, workspace_id: Uuid) -> ApiResult<Vec<View>> {
  sqlx::query_as::<_, View>(concat!(
    "\n      SELECT\n        ",
    view_columns!(),
    r#"
      FROM views
      WHERE workspace_id = $1 AND is_deleted = true
      ORDER BY updated_at DESC
    "#,
  ))
  .bind(workspace_id)
  .fetch_all(db)
  .await
  .map_err(ApiError::from)
}

async fn fetch_document_view(
  db: &PgPool,
  workspace_id: Uuid,
  document_id: Uuid,
) -> ApiResult<Option<View>> {
  sqlx::query_as::<_, View>(concat!(
    "\n      SELECT\n        ",
    view_columns!(),
    r#"
      FROM views
      WHERE workspace_id = $1 AND object_id = $2 AND object_type = 'document' AND is_deleted = false
      LIMIT 1
    "#,
  ))
  .bind(workspace_id)
  .bind(document_id)
  .fetch_optional(db)
  .await
  .map_err(ApiError::from)
}

pub(crate) async fn ensure_document_in_workspace(
  db: &PgPool,
  workspace_id: Uuid,
  document_id: Uuid,
) -> ApiResult<()> {
  let exists = sqlx::query_scalar::<_, bool>(
    r#"
      SELECT EXISTS (
        SELECT 1
        FROM documents
        WHERE id = $1 AND workspace_id = $2
      )
    "#,
  )
  .bind(document_id)
  .bind(workspace_id)
  .fetch_one(db)
  .await?;

  if !exists {
    return Err(ApiError::NotFound);
  }

  Ok(())
}

/// Read/write/comment capabilities derived from a workspace role. Surfaced to
/// clients on bootstrap so the editor can enable or disable mutating actions.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct DocumentPermissions {
  pub can_read: bool,
  pub can_write: bool,
  pub can_comment: bool,
}

pub(crate) fn permissions_for_role(role: &str) -> DocumentPermissions {
  let can_write = matches!(role, "owner" | "admin" | "editor");
  let can_comment = can_write || role == "commenter";
  DocumentPermissions {
    can_read: true,
    can_write,
    can_comment,
  }
}

pub(crate) use super::workspaces::{ensure_workspace_member, workspace_role};

pub(crate) async fn ensure_workspace_editor(
  db: &PgPool,
  workspace_id: Uuid,
  user_id: Uuid,
) -> ApiResult<()> {
  let role = workspace_role(db, workspace_id, user_id)
    .await?
    .ok_or(ApiError::NotFound)?;

  if !matches!(role.as_str(), "owner" | "admin" | "editor") {
    return Err(ApiError::Forbidden);
  }

  Ok(())
}

async fn ensure_view_in_workspace(
  db: impl sqlx::PgExecutor<'_>,
  workspace_id: Uuid,
  view_id: Uuid,
) -> ApiResult<()> {
  let exists = sqlx::query_scalar::<_, bool>(
    r#"
      SELECT EXISTS (
        SELECT 1
        FROM views
        WHERE id = $1 AND workspace_id = $2 AND is_deleted = false
      )
    "#,
  )
  .bind(view_id)
  .bind(workspace_id)
  .fetch_one(db)
  .await?;

  if !exists {
    return Err(ApiError::NotFound);
  }

  Ok(())
}

/// A parent must be a live FOLDER in this workspace. Pages are leaves — only
/// folders contain (see `migrations/0011_pages_are_leaves.sql`, which repairs
/// the trees that predate this and enforces the same rule at the DB as a
/// backstop). Every path that sets `parent_view_id` goes through here so the
/// caller gets a 400 with a reason up front.
///
/// Up front is all it is: a long write (an import runs for minutes) can see
/// the folder trashed after this returned. The trigger re-checks the same rule
/// — live, folder, same workspace — at each write (migration 0033), and its
/// refusal reaches the caller as a 409 (`mica_infra::ApiError`).
pub(crate) async fn ensure_parent_accepts_children(
  db: impl sqlx::PgExecutor<'_>,
  workspace_id: Uuid,
  parent_view_id: Uuid,
) -> ApiResult<()> {
  let object_type = sqlx::query_scalar::<_, String>(
    r#"
      SELECT object_type::text
      FROM views
      WHERE id = $1 AND workspace_id = $2 AND is_deleted = false
    "#,
  )
  .bind(parent_view_id)
  .bind(workspace_id)
  .fetch_optional(db)
  .await?
  .ok_or(ApiError::NotFound)?;

  if object_type != "folder" {
    return Err(ApiError::BadRequest(
      "parent_view_id must be a folder — pages cannot contain pages".to_string(),
    ));
  }

  Ok(())
}

/// Serialize tree-reparenting within a workspace for the rest of the
/// transaction (gh#46). Two moves that swap a folder and one of its
/// descendants each passed the cycle check against a snapshot taken before the
/// other committed, then wrote — and produced a cycle no live-subtree walk
/// could ever return from. Taking this lock BEFORE validating means the second
/// transaction re-reads AFTER the first committed and the check sees the real
/// tree. Scoped to the workspace: moves in one workspace never wait on another.
pub(crate) async fn lock_workspace_tree(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  workspace_id: Uuid,
) -> ApiResult<()> {
  sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
    .bind(workspace_id)
    .execute(&mut **tx)
    .await?;
  Ok(())
}

/// gh#46 made this transaction-bound: both call sites reparent views inside a
/// transaction behind [lock_workspace_tree], and the cycle check must read the
/// SAME snapshot as the write that follows it — a pool read could validate
/// against a tree a concurrent move has since rewritten. A `&mut PgConnection`
/// reborrows per query; the two checks below share the caller's connection.
async fn ensure_valid_parent_view(
  db: &mut sqlx::PgConnection,
  workspace_id: Uuid,
  view_id: Uuid,
  parent_view_id: Uuid,
) -> ApiResult<()> {
  if parent_view_id == view_id {
    return Err(ApiError::BadRequest(
      "parent_view_id cannot be the same view".to_string(),
    ));
  }

  ensure_parent_accepts_children(&mut *db, workspace_id, parent_view_id).await?;

  // `CYCLE`: without it this walk has no way home if a cycle ever lands in
  // the table (one pre-cycle-guard row pair, one stray trigger, one dbadmin
  // fix-up), and the query would recurse until the session gives up. A
  // repeated row is NOT a descendant of `$1` — it was reached on a loop — so
  // it must not answer the cycle question either.
  let would_cycle = sqlx::query_scalar::<_, bool>(
    r#"
      WITH RECURSIVE descendants (id) AS (
        SELECT id
        FROM views
        WHERE id = $1 AND workspace_id = $2 AND is_deleted = false
        UNION ALL
        SELECT v.id
        FROM views v
        INNER JOIN descendants d ON v.parent_view_id = d.id
        WHERE v.workspace_id = $2 AND v.is_deleted = false
      ) CYCLE id SET is_cycle USING cycle_path
      SELECT EXISTS (
        SELECT 1
        FROM descendants
        WHERE id = $3 AND NOT is_cycle
      )
    "#,
  )
  .bind(view_id)
  .bind(workspace_id)
  .bind(parent_view_id)
  .fetch_one(&mut *db)
  .await?;

  if would_cycle {
    return Err(ApiError::BadRequest(
      "parent_view_id cannot be a descendant view".to_string(),
    ));
  }

  Ok(())
}

fn normalize_view_name(name: &str) -> ApiResult<String> {
  let name = name.trim().to_string();
  if name.is_empty() {
    return Err(ApiError::BadRequest("view name is required".to_string()));
  }

  Ok(name)
}

/// Server-side enforcement of the mangled-LaTeX guard (issue #83): it used to
/// live only in the MCP proxy layer, so REST and CLI writes walked straight past
/// it. The rules live in `mica-markdown`; here a collision is a 400 naming the
/// escape that decoded wrong and how to fix it. Any request field carrying
/// authored markdown (the body, a find_replace `replace`, …) must pass through
/// this before it is stored.
fn reject_mangled_latex(markdown: &str) -> ApiResult<()> {
  match mica_markdown::mangled_latex_collision(markdown) {
    Some((escape, example)) => Err(ApiError::BadRequest(mica_markdown::mangled_latex_message(
      escape, example,
    ))),
    None => Ok(()),
  }
}

fn normalize_position(position: Option<String>) -> ApiResult<String> {
  let Some(position) = position else {
    return Ok(Uuid::now_v7().to_string());
  };

  let position = position.trim().to_string();
  if position.is_empty() {
    return Err(ApiError::BadRequest("position cannot be empty".to_string()));
  }

  Ok(position)
}

// ── Cross-workspace transfer (move / copy) ───────────────────────────────────
// Move or copy a page (its whole subtree) or a folder into ANOTHER workspace on
// this server. No note app re-parents in place across workspaces: doc identity
// and blobs are both workspace-namespaced, so an in-place move invites doc-id
// collisions and lets the source's per-workspace blob GC reclaim images the
// moved page still needs. So this is copy-into-destination (new view + new
// document + blobs physically copied into the destination workspace), then a
// soft-delete of the source for a "move". Ordered blobs-first, so a half-failure
// leaves harmless orphan bytes in the destination (its GC reclaims them), never a
// page that lost its images — the thing Notion/AFFiNE get wrong and our Postgres

/// The shared shape of every batch view endpoint: authenticate the caller,
/// validate the id list, run the domain batch statement, and report affected
/// vs skipped. The three hand-written copies of this body were already only a
/// copy-paste apart; a macro cannot drift.
macro_rules! batch_view_handler {
  ($(#[$meta:meta])* $vis:vis async fn $name:ident, $inner:ident) => {
    $(#[$meta])*
    $vis async fn $name(
      State(state): State<AppState>,
      headers: HeaderMap,
      Path(workspace_id): Path<Uuid>,
      Json(payload): Json<BatchViewsRequest>,
    ) -> ApiResult<Json<BatchViewsResponse>> {
      let user_id = user_id_from_headers(&state, &headers).await?;
      ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
      check_batch_ids(&payload.view_ids)?;

      let touched = $inner(&state.db, workspace_id, &payload.view_ids).await?;

      Ok(Json(BatchViewsResponse {
        affected: touched.len(),
        skipped: skipped_ids(&payload.view_ids, &touched),
      }))
    }
  };
}

#[cfg(test)]
mod pg_tests;

#[cfg(test)]
mod transfer_flow_pg_tests;

#[cfg(test)]
mod contract_gold_tests;

mod crud;
mod export;
mod images;
mod search;
mod share;
mod transfer;
mod trash;

pub use crud::*;
pub use export::*;
pub use images::*;
pub use search::*;
pub use share::*;
pub use transfer::*;
pub use trash::*;
