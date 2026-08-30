use std::collections::BTreeMap;

use axum::{
  Json,
  body::Bytes,
  extract::{Path, Query, State},
  http::{HeaderMap, header},
  response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use mica_app_core::{
  AppState,
  documents::{
    DocumentOperation, DocumentSnapshotPayload, document_title, export_html, export_html_document,
    export_markdown_with_assets, import_markdown, import_markdown_fragment, set_image_srcs,
    with_page_title,
  },
  store::{self, DocumentRecord},
};
use mica_infra::{ApiError, ApiResult};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::routes::auth::user_id_from_headers;
use crate::routes::ws;
use mica_interchange::{ZipEntry, build_zip};

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

pub async fn list_views(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
  Query(query): Query<ListViewsQuery>,
) -> ApiResult<Response> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;

  if let Some(depth) = query.depth
    && depth < 1
  {
    return Err(ApiError::BadRequest("depth must be at least 1".to_string()));
  }
  if let Some(limit) = query.limit
    && limit < 1
  {
    return Err(ApiError::BadRequest("limit must be at least 1".to_string()));
  }

  let views = fetch_views_filtered(&state.db, workspace_id, &query).await?;

  // Conditional GET. The page tree is the biggest thing a client fetches and
  // the thing it fetches most often: every workspace switch asks for it again,
  // and it is usually byte-identical to what the client already mirrored.
  // Measured on a 3700-page workspace: 1.61 MB, 250ms end to end, of which the
  // database is 26ms — the rest is serialising and shipping a tree nobody
  // changed.
  //
  // The rows are still read (that is the cheap part) and hashed; what a match
  // skips is the serialisation and the transfer.
  //
  // The tag is derived from the ROWS THEMSELVES, deliberately, rather than from
  // `max(updated_at)`: nothing maintains that column but hand-written UPDATEs
  // — there is no trigger — so one path forgetting it would freeze every client
  // on a stale tree. Staleness is far worse than slowness here, and a content
  // hash cannot go stale.
  let etag = views_etag(&views, &query);
  if headers
    .get(header::IF_NONE_MATCH)
    .and_then(|v| v.to_str().ok())
    .is_some_and(|v| etag_matches(v, &etag))
  {
    return Ok(
      (axum::http::StatusCode::NOT_MODIFIED, [(header::ETAG, etag)], ()).into_response(),
    );
  }

  Ok(([(header::ETAG, etag)], Json(ViewListResponse { views })).into_response())
}

/// A strong validator for a view listing: the hash of everything the response
/// would contain, plus the query shape that produced it.
///
/// Every serialised field goes in, including ones the app ignores. A tag that
/// covered only what one client reads would answer 304 to a client that reads
/// more — the header is a promise about the BODY, not about the caller.
fn views_etag(views: &[View], query: &ListViewsQuery) -> String {
  use sha2::{Digest, Sha256};
  let mut hasher = Sha256::new();
  // The query shape is part of the identity: one workspace answers different
  // bodies for different depth/limit/offset/with_stats.
  hasher.update(
    format!(
      "q:{:?}:{:?}:{:?}:{:?}:{}\n",
      query.parent_view_id, query.depth, query.limit, query.offset, query.with_stats
    )
    .as_bytes(),
  );
  for v in views {
    hasher.update(
      format!(
        "{}\u{1}{}\u{1}{:?}\u{1}{}\u{1}{}\u{1}{}\u{1}{:?}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{:?}\n",
        v.id,
        v.workspace_id,
        v.parent_view_id,
        v.object_id,
        v.object_type,
        v.name,
        v.icon,
        v.position,
        v.is_deleted,
        v.created_by,
        v.created_at.to_rfc3339(),
        v.updated_at.to_rfc3339(),
        v.state_bytes,
      )
      .as_bytes(),
    );
  }
  format!("\"{:x}\"", hasher.finalize())
}

/// Whether an `If-None-Match` header matches [etag].
///
/// Handles the list form (`"a", "b"`), `*`, and the `W/` weak prefix a proxy
/// may add on the way through — the comparison that matters is the opaque
/// value, and refusing a tag because something upstream marked it weak would
/// silently disable the whole mechanism.
fn etag_matches(header_value: &str, etag: &str) -> bool {
  let want = etag.trim_start_matches("W/").trim();
  header_value.split(',').any(|candidate| {
    let candidate = candidate.trim();
    candidate == "*" || candidate.trim_start_matches("W/").trim() == want
  })
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
  q: String,
  /// Also match FOLDERS (by name — they carry no body). Off by default, and the
  /// default is the whole point: the app's search dialog opens a hit as a page,
  /// so a folder among those results would send it to open a document that does
  /// not exist. Callers that can tell the two apart opt in — the MCP layer does,
  /// because an agent's reason to look for a folder is to file something under
  /// it.
  #[serde(default)]
  include_folders: bool,
  /// Rank hits from this workspace above equally-relevant hits elsewhere.
  ///
  /// A TIEBREAKER inside each relevance tier, never a filter and never the top
  /// key: an exact page-name match in another workspace still outranks a body
  /// mention in this one. Reported 2026-08-28 — searching "claude" from a
  /// workspace holding two passing mentions hid fifty pages actually named
  /// after it, because the client only widened the scope when the current
  /// workspace came back EMPTY. Preferring beats gating.
  #[serde(default)]
  prefer_workspace: Option<Uuid>,
}

#[derive(Debug, Serialize)]
struct SearchResult {
  view_id: Uuid,
  object_id: Uuid,
  /// Which workspace the hit lives in.
  ///
  /// Redundant on the per-workspace route (the caller named it in the URL) and
  /// load-bearing on the cross-workspace one: opening a page is workspace-scoped
  /// (`GET /workspaces/{workspace_id}/documents/{id}`), so a hit that does not
  /// say where it lives cannot be opened. The client cannot infer it either —
  /// it only holds page trees for workspaces it has actually visited.
  workspace_id: Uuid,
  name: String,
  snippet: String,
  title_match: bool,
  /// The containing folder, or null at the workspace root.
  ///
  /// Here because finding a page and then acting NEAR it — filing a sibling
  /// beside it, saying where it lives — otherwise costs a full workspace tree
  /// listing, orders of magnitude more than the hit itself on a large
  /// workspace. It is one column that was already on the row.
  parent_view_id: Option<Uuid>,
  /// True when the hit is a folder rather than a page. Can only be true if the
  /// caller passed `include_folders`.
  is_folder: bool,
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
  results: Vec<SearchResult>,
}

/// `GET /api/workspaces/{workspace_id}/search?q=...` — find pages whose title or
/// body text contains the query (case-insensitive), with a short snippet.
pub async fn search_workspace(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
  Query(query): Query<SearchQuery>,
) -> ApiResult<Json<SearchResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;

  let needle = query.q.trim();
  if needle.is_empty() {
    return Ok(Json(SearchResponse { results: vec![] }));
  }

  let results = search_views(
    &state.db,
    &state.body_index,
    &[workspace_id],
    needle,
    query.include_folders,
    query.prefer_workspace,
  )
  .await?;
  Ok(Json(SearchResponse { results }))
}

/// `GET /search` — the same search across EVERY workspace the caller belongs to.
///
/// A separate route rather than a flag on the per-workspace one, because the URL
/// is what states the scope: `/workspaces/{id}/search` promises results from that
/// workspace, and a query parameter that quietly widens it would make the path
/// lie. Same query underneath, so ranking and folder handling cannot drift.
///
/// COST (FTS M2): the body match runs against the in-process `BodyIndex`, so
/// widening the scope from one workspace to all of them costs nothing extra —
/// the scan is over every indexed body either way, and workspace scope is an
/// output filter. The `ILIKE`-per-body bottleneck this route used to multiply
/// (measured 2026-08-06: ~75 ms of an 81 ms query reading 798 bodies) is gone;
/// what remains per query is one incremental refresh probe plus a memory scan.
pub async fn search_all_workspaces(
  State(state): State<AppState>,
  headers: HeaderMap,
  Query(query): Query<SearchQuery>,
) -> ApiResult<Json<SearchResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;

  let needle = query.q.trim();
  if needle.is_empty() {
    return Ok(Json(SearchResponse { results: vec![] }));
  }

  // Membership decides the scope, so there is no per-workspace authorization
  // left to do: a workspace the caller is not a member of never enters the list.
  let workspace_ids: Vec<Uuid> =
    sqlx::query_scalar("SELECT workspace_id FROM workspace_members WHERE user_id = $1")
      .bind(user_id)
      .fetch_all(&state.db)
      .await?;

  let results = search_views(
    &state.db,
    &state.body_index,
    &workspace_ids,
    needle,
    query.include_folders,
    query.prefer_workspace,
  )
  .await?;
  Ok(Json(SearchResponse { results }))
}

/// One row of the FTS M1 search query: a document view plus its indexed body
/// text (NULL for a document that has no yrs base yet — never opened/edited).
#[derive(FromRow)]
struct SearchRow {
  view_id: Uuid,
  object_id: Uuid,
  workspace_id: Uuid,
  name: String,
  content_text: Option<String>,
  parent_view_id: Option<Uuid>,
  object_type: String,
}

/// The full-text search behind [`search_workspace`], extracted so a DB test can
/// drive the real query (the same pattern `scan_backlinks` follows).
///
/// FTS M1 made this ONE SQL statement over the maintained `content_text`
/// projection (see `mica_app_core::sync::content_text_from_doc`); M2 moved the
/// body half of the predicate into the in-process `BodyIndex` — same substring
/// semantics, no per-body `ILIKE` reads. (The M1 note here used to promise
/// "M2 adds a `pg_trgm` GIN index": researched 2026-08-28 and rejected — 1–2
/// char CJK needles have no extractable trigram and the C-locale alpine image
/// would index no CJK at all. The `BodyIndex` module doc carries the details.)
///
/// LEFT JOIN, not INNER: a document keeps showing up on a TITLE hit even before
/// it has a base row, and a document with no base simply isn't body-searchable
/// until first edited (`content_text` NULL → the `ILIKE` is NULL/false). Only
/// documents are searched (folders carry no body and were never title-searched).
///
/// Corruption note: search no longer decodes anything, so the "unreadable
/// document" signal `search_workspace` used to log moved to where the decode now
/// happens — `sync::backfill_content_text` at startup and the write paths (a bad
/// update is a 400 in `push_update`). A base that failed to decode has empty
/// `content_text` and is warn-logged there, not silently missing here.
/// [workspace_ids] is a LIST so one statement serves both the per-workspace
/// route and the cross-workspace one — a second copy of this query would be the
/// double-representation this repo keeps paying for, and the two would drift on
/// the next ranking change. A single-workspace search simply passes one id.
async fn search_views(
  db: &PgPool,
  index: &mica_app_core::search::BodyIndex,
  workspace_ids: &[Uuid],
  needle: &str,
  include_folders: bool,
  prefer_workspace: Option<Uuid>,
) -> ApiResult<Vec<SearchResult>> {
  if workspace_ids.is_empty() {
    return Ok(vec![]);
  }
  // FTS M2: the body half of the predicate is answered by the in-process index
  // — plain substring semantics, identical to the `content_text ILIKE` it
  // replaces (including 1–2 char CJK needles, which no Postgres index on this
  // stock image can serve; the module doc on `BodyIndex` has the research).
  // The ids are CANDIDATES only: the SQL below still owns workspace scope,
  // `is_deleted`, ranking and LIMIT, so a stale index entry costs a wasted id,
  // never a leaked or misplaced hit. The refresh is what keeps a just-pushed
  // edit searchable in the same request that follows it.
  index.refresh(db).await?;
  let body_ids = index.matching_ids(needle).await;
  let pattern = like_pattern(needle);
  // Folders ride the SAME statement rather than getting their own: they have no
  // `document_yrs_base` row, so the LEFT JOIN already leaves `content_text` NULL
  // and the body half of the predicate is simply never true for them. A folder
  // therefore matches by name only, which is the whole of what a folder is.
  // The type predicate is a compile-time FRAGMENT, not a bound parameter, and
  // the column is compared as its own enum type rather than cast to text. Both
  // details exist so `idx_views_workspace_object (workspace_id, object_type,
  // object_id)` can actually be used.
  //
  // It used to read `AND ($3 OR v.object_type::text = 'document')`, which
  // defeated that index twice over: casting the indexed column hides it from the
  // index, and a bound `$3` on the left of an `OR` leaves the planner unable to
  // treat the conjunct as restrictive at all. Measured on production
  // (2026-08-06), `object_type` was therefore applied as a heap filter that
  // discarded 222 of 1020 rows AFTER the index scan had already fetched them.
  //
  // Worth being clear about the size of this: it is ~6 ms of row plumbing out of
  // an 81 ms query. The other ~75 ms is the ILIKE reading 798 documents' bodies,
  // and no amount of restructuring here touches that — see the search entry in
  // docs/roadmap.md for why fixing THAT needs a CJK-capable text index.
  //
  // ORDERING: title hits first, recency second.
  //
  // It was recency alone, and that read to users as "search does not match page
  // names" (reported 2026-08-12) even though the predicate above has always
  // matched `v.name`. Two separate ways it went wrong:
  //
  //   - A page whose NAME is the query, last edited months ago, sorted below
  //     every page that merely mentions it and was touched yesterday.
  //   - Worse, `LIMIT 50` then cut from the bottom, so on a common word the
  //     title hit could be dropped from the response entirely — indistinguishable
  //     from "not found" at the client.
  //
  // `(v.name ILIKE $2)` is a boolean; DESC puts true first. It repeats the
  // predicate rather than reusing the `title_match` computed in Rust below,
  // because that one is derived AFTER the rows come back — by then the LIMIT has
  // already chosen which rows exist, which is the half that mattered.
  //
  // Within the name tier, FOLDERS before pages (asked for 2026-08-28: workspace
  // name > folder name > page name > body; the workspace tier is client-side,
  // drawn above this list). Folders are few and matching one by name usually
  // means the user is navigating, not reading. The key is a no-op below the
  // name tier: body hits are all documents, a folder having no body to hit.
  //
  // Built by `concat!` into TWO `&'static str` constants rather than formatted at
  // runtime: sqlx 0.9 refuses a dynamic SQL string outright ("dynamic SQL strings
  // should be audited for possible injections"), and that refusal is right — the
  // way past it is to have no dynamic string, not to assert one is safe.
  macro_rules! search_sql {
    ($type_filter:literal) => {
      concat!(
        r#"
      SELECT v.id AS view_id, v.object_id, v.workspace_id, v.name, yb.content_text,
             v.parent_view_id, v.object_type::text AS object_type
      FROM views v
      LEFT JOIN document_yrs_base yb ON yb.document_id = v.object_id
      WHERE v.workspace_id = ANY($1)
        AND v.is_deleted = false
        "#,
        $type_filter,
        r#"
        AND (v.name ILIKE $2 ESCAPE '\' OR v.object_id = ANY($3))
      ORDER BY (v.name ILIKE $2 ESCAPE '\') DESC,
               (v.object_type <> 'document') DESC,
               (v.workspace_id = $4) DESC,
               v.updated_at DESC
      LIMIT 50
    "#
      )
    };
  }
  let sql = if include_folders {
    search_sql!("")
  } else {
    search_sql!("AND v.object_type = 'document'")
  };
  let rows = sqlx::query_as::<_, SearchRow>(sql)
    .bind(workspace_ids)
    .bind(&pattern)
    .bind(&body_ids)
    // NULL when no preference: `workspace_id = NULL` is NULL for every row, so
    // they all tie and the key drops out of the ordering by itself.
    .bind(prefer_workspace)
    .fetch_all(db)
    .await?;

  let needle_lower = needle.to_lowercase();
  Ok(
    rows
      .into_iter()
      .map(|row| {
        let title_match = row.name.to_lowercase().contains(&needle_lower);
        // The SQL already guaranteed name OR content matched. A content-only hit
        // yields a snippet; a title-only hit may have an empty snippet, which the
        // client renders as a bare title result (unchanged contract).
        let snippet = row
          .content_text
          .as_deref()
          .and_then(|text| snippet_for(text, &needle_lower))
          .unwrap_or_default();
        SearchResult {
          view_id: row.view_id,
          object_id: row.object_id,
          workspace_id: row.workspace_id,
          name: row.name,
          snippet,
          title_match,
          parent_view_id: row.parent_view_id,
          is_folder: row.object_type != "document",
        }
      })
      .collect(),
  )
}

/// Wrap `needle` as a `%…%` `LIKE`/`ILIKE` pattern, escaping the wildcard
/// metacharacters (`% _ \`) with a leading backslash so a query containing them
/// matches literally (paired with `ESCAPE '\'` in the SQL). Without this a search
/// for `50%` would match anything.
fn like_pattern(needle: &str) -> String {
  let mut out = String::with_capacity(needle.len() + 2);
  out.push('%');
  for c in needle.chars() {
    if matches!(c, '%' | '_' | '\\') {
      out.push('\\');
    }
    out.push(c);
  }
  out.push('%');
  out
}

// SEARCH_CONCURRENCY is gone: nothing in this file decodes documents per query
// any more. Search reads `content_text` (0012) and backlinks read `link_targets`
// (0019), both maintained columns — there is no fan-out left to bound.

/// A ~160-char snippet of `text` centered on the first case-insensitive match of
/// `needle_lower`, with an ellipsis on each clipped edge. `None` if the needle is
/// absent. `text` is the whole document body (`content_text`), so unlike the old
/// per-block snippet this must WINDOW around the hit, not take the head.
fn snippet_for(text: &str, needle_lower: &str) -> Option<String> {
  const WINDOW: usize = 160;
  /// Chars of context to keep before the match so the hit isn't flush-left.
  const LEAD: usize = 40;
  let lower = text.to_lowercase();
  let byte_pos = lower.find(needle_lower)?;
  // Char index of the match start. `to_lowercase` is char-count-preserving for
  // the scripts we search (ASCII, CJK), so counting chars in the lowercased
  // prefix indexes the original `chars` too — an approximation that at worst
  // shifts the window by a few chars for exotic scripts, never drops the match.
  let match_char = lower[..byte_pos].chars().count();
  let chars: Vec<char> = text.chars().collect();
  let start = match_char.saturating_sub(LEAD);
  let end = (start + WINDOW).min(chars.len());
  let mut out = String::new();
  if start > 0 {
    out.push('…');
  }
  out.extend(&chars[start..end]);
  if end < chars.len() {
    out.push('…');
  }
  Some(out)
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct Backlink {
  view_id: Uuid,
  document_id: Uuid,
  title: String,
}

#[derive(Debug, Serialize)]
pub struct BacklinksResponse {
  backlinks: Vec<Backlink>,
}

/// `GET /api/workspaces/{workspace_id}/views/{view_id}/backlinks` — the pages in
/// this workspace that link TO `view_id`, i.e. any live document whose blocks
/// carry a `mica://page/<view_id>` link mark.
///
/// Answered from the maintained `document_yrs_base.link_targets` index
/// (migration 0019), the same way search reads `content_text`. Cloud-only: the
/// local (offline) world has its own store and never hits this endpoint.
pub async fn backlinks(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, view_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<BacklinksResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;
  // A backlink query for a page that doesn't live here is a 404, not an empty
  // list — matches transfer/move's `ensure_view_in_workspace` contract.
  ensure_view_in_workspace(&state.db, workspace_id, view_id).await?;

  let backlinks = scan_backlinks(&state.db, workspace_id, view_id).await?;
  Ok(Json(BacklinksResponse { backlinks }))
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct GraphNode {
  view_id: Uuid,
  name: String,
  /// How many edges touch this node (in + out). The client sizes nodes by it, so
  /// a hub reads as a hub without the client having to count edges itself.
  degree: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct GraphEdge {
  source: Uuid,
  target: Uuid,
}

#[derive(Debug, Serialize)]
pub struct GraphResponse {
  nodes: Vec<GraphNode>,
  edges: Vec<GraphEdge>,
  /// Live documents in the workspace that no link touches. Reported as a COUNT
  /// rather than as isolated nodes: on a real workspace most pages are unlinked
  /// (measured 2026-08-05 on a production snapshot — 798 documents, 136 with any
  /// link at all), and drawing 662 disconnected dots buries the structure the
  /// view exists to show. The number keeps that honest instead of silent.
  unlinked: i64,
}

/// `GET /api/workspaces/{workspace_id}/graph` — the page-link graph.
///
/// One query per half, both off the maintained `link_targets` index (migration
/// 0019). Before that index this endpoint could not have existed at a sane cost:
/// it would have meant decoding every document in the workspace on every open.
///
/// Edges point SOURCE → TARGET (the direction the link was written). Only edges
/// whose both ends are live documents in this workspace survive — a link to a
/// trashed or foreign page is a dangling reference, not an edge, and drawing it
/// would invent a node that is not there.
pub async fn graph(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
) -> ApiResult<Json<GraphResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;
  Ok(Json(workspace_graph(&state.db, workspace_id).await?))
}

/// The graph build behind [`graph`], extracted so tests exercise the real
/// queries rather than a re-implementation of them.
async fn workspace_graph(db: &PgPool, workspace_id: Uuid) -> ApiResult<GraphResponse> {
  let edges = sqlx::query_as::<_, GraphEdge>(
    r#"
      SELECT src.id AS source, tgt.id AS target
      FROM views src
      JOIN document_yrs_base yb ON yb.document_id = src.object_id
      CROSS JOIN LATERAL unnest(yb.link_targets) AS t(target_id)
      JOIN views tgt ON tgt.id = t.target_id
                    AND tgt.workspace_id = src.workspace_id
                    AND tgt.is_deleted = false
      WHERE src.workspace_id = $1
        AND src.is_deleted = false
        AND src.object_type::text = 'document'
        AND src.id <> t.target_id
    "#,
  )
  .bind(workspace_id)
  .fetch_all(db)
  .await?;

  let mut degree: std::collections::HashMap<Uuid, i64> = Default::default();
  for e in &edges {
    *degree.entry(e.source).or_default() += 1;
    *degree.entry(e.target).or_default() += 1;
  }

  let named = sqlx::query_as::<_, (Uuid, String)>(
    "SELECT id, name FROM views
     WHERE workspace_id = $1 AND is_deleted = false AND object_type::text = 'document'",
  )
  .bind(workspace_id)
  .fetch_all(db)
  .await?;

  let mut nodes = Vec::new();
  let mut unlinked = 0i64;
  for (view_id, name) in named {
    match degree.get(&view_id) {
      Some(&d) => nodes.push(GraphNode { view_id, name, degree: d }),
      None => unlinked += 1,
    }
  }
  nodes.sort_by(|a, b| b.degree.cmp(&a.degree).then(a.view_id.cmp(&b.view_id)));

  Ok(GraphResponse { nodes, edges, unlinked })
}

/// The backlink lookup behind [`backlinks`], extracted so tests exercise the real
/// query rather than a re-implementation of it.
///
/// One indexed query. This used to be an on-demand scan: a DB round-trip plus a
/// full CRDT decode PER DOCUMENT, with no early stop (a panel must be complete),
/// bounded only by `buffered(8)`. Measured 2026-08-05 against a restored
/// production snapshot — 798 documents, ~690 ms in release, versus 53 ms for
/// full-text search on the same database — and the panel loads on every page
/// open, so that was paid passively on every navigation. `link_targets`
/// (migration 0019) is the same fix `content_text` was for search: a co-written
/// pure projection of `state`, so the decode happens once at write time instead
/// of N times per read.
///
/// Documents still awaiting the startup backfill have `link_targets` NULL and do
/// not match — the same "not yet indexed" window search has, and the backfill
/// runs before the server accepts traffic.
async fn scan_backlinks(
  db: &PgPool,
  workspace_id: Uuid,
  view_id: Uuid,
) -> ApiResult<Vec<Backlink>> {
  // A page linking to itself is not a backlink; folders carry no blocks so they
  // can never be a source. Stable order: title first (what the panel shows),
  // view_id to break ties — the same order the old in-memory sort produced.
  // `@>` (not `= ANY`) is what the GIN index on link_targets answers.
  Ok(
    sqlx::query_as::<_, Backlink>(
      r#"
        SELECT v.id AS view_id, v.object_id AS document_id, v.name AS title
        FROM views v
        JOIN document_yrs_base yb ON yb.document_id = v.object_id
        WHERE v.workspace_id = $1
          AND v.is_deleted = false
          AND v.object_type::text = 'document'
          AND v.id <> $2
          AND yb.link_targets @> ARRAY[$2]::uuid[]
        ORDER BY v.name, v.id
      "#,
    )
    .bind(workspace_id)
    .bind(view_id)
    .fetch_all(db)
    .await?,
  )
}

pub async fn create_document(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
  Json(payload): Json<CreateDocumentRequest>,
) -> ApiResult<Json<DocumentCreateResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;

  let name = normalize_view_name(&payload.name)?;

  if let Some(parent_view_id) = payload.parent_view_id {
    ensure_parent_accepts_children(&state.db, workspace_id, parent_view_id).await?;
  }

  let mut tx = state.db.begin().await?;
  let root_block_id = format!("block_{}", Uuid::new_v4().simple());

  let document = sqlx::query_as::<_, DocumentRecord>(
    r#"
      INSERT INTO documents (workspace_id, root_block_id, created_by)
      VALUES ($1, $2, $3)
      RETURNING id, workspace_id, root_block_id, current_seq, created_by, created_at, updated_at
    "#,
  )
  .bind(workspace_id)
  .bind(&root_block_id)
  .bind(user_id)
  .fetch_one(&mut *tx)
  .await?;

  // A new page owns its yrs base from birth, written in the same transaction as
  // its `documents` row. Before S4 this wrote an op-model snapshot instead and
  // the base was built afterwards, best-effort — so a document could exist for a
  // moment with no content to read, and stayed unsearchable until first open.
  mica_app_core::sync::seed_base_tx(
    &mut tx,
    document.id,
    mica_app_core::sync::empty_payload(&document.root_block_id),
  )
  .await?;

  let position = Uuid::now_v7().to_string();
  let view = sqlx::query_as::<_, View>(
    r#"
      INSERT INTO views (
        workspace_id,
        parent_view_id,
        object_id,
        object_type,
        name,
        position,
        created_by
      )
      VALUES ($1, $2, $3, 'document', $4, $5, $6)
      RETURNING
        id,
        workspace_id,
        parent_view_id,
        object_id,
        object_type::text AS object_type,
        name,
        icon,
        position,
        is_deleted,
        created_by,
        created_at,
        updated_at
    "#,
  )
  .bind(workspace_id)
  .bind(payload.parent_view_id)
  .bind(document.id)
  .bind(name)
  .bind(position)
  .bind(user_id)
  .fetch_one(&mut *tx)
  .await?;

  tx.commit().await?;

  Ok(Json(DocumentCreateResponse { document, view }))
}

/// `POST /api/workspaces/{workspace_id}/folders`
///
/// Create a folder view — a pure container in the page tree (AFFiNE-style
/// "entity used solely for organizing content"). Unlike [`create_document`] it
/// inserts ONLY a `views` row with `object_type='folder'`: no `documents` row,
/// no snapshot, no CRDT sync. `object_id` gets a fresh (unreferenced) uuid to
/// satisfy the NOT NULL column. Export renders it as a directory, never a `.md`.
pub async fn create_folder(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
  Json(payload): Json<CreateFolderRequest>,
) -> ApiResult<Json<ViewResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;

  let name = normalize_view_name(&payload.name)?;

  if let Some(parent_view_id) = payload.parent_view_id {
    ensure_parent_accepts_children(&state.db, workspace_id, parent_view_id).await?;
  }

  // No document / snapshot — a folder has no content. `object_id` is a fresh
  // uuid purely to satisfy the NOT NULL column; nothing references it.
  let object_id = Uuid::new_v4();
  let position = Uuid::now_v7().to_string();
  let view = sqlx::query_as::<_, View>(
    r#"
      INSERT INTO views (
        workspace_id,
        parent_view_id,
        object_id,
        object_type,
        name,
        position,
        created_by
      )
      VALUES ($1, $2, $3, 'folder', $4, $5, $6)
      RETURNING
        id,
        workspace_id,
        parent_view_id,
        object_id,
        object_type::text AS object_type,
        name,
        icon,
        position,
        is_deleted,
        created_by,
        created_at,
        updated_at
    "#,
  )
  .bind(workspace_id)
  .bind(payload.parent_view_id)
  .bind(object_id)
  .bind(name)
  .bind(position)
  .bind(user_id)
  .fetch_one(&state.db)
  .await?;

  Ok(Json(ViewResponse { view }))
}

/// `POST /api/workspaces/{workspace_id}/documents/import/markdown`
///
/// Create a new document whose initial snapshot is parsed from Markdown, and a
/// matching view in the page tree.
pub async fn import_document_markdown(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
  Json(payload): Json<ImportMarkdownRequest>,
) -> ApiResult<Json<DocumentBootstrapResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;

  let name = normalize_view_name(&payload.name)?;

  if let Some(parent_view_id) = payload.parent_view_id {
    ensure_parent_accepts_children(&state.db, workspace_id, parent_view_id).await?;
  }

  let mut tx = state.db.begin().await?;
  let root_block_id = format!("block_{}", Uuid::new_v4().simple());

  let document = sqlx::query_as::<_, DocumentRecord>(
    r#"
      INSERT INTO documents (workspace_id, root_block_id, created_by)
      VALUES ($1, $2, $3)
      RETURNING id, workspace_id, root_block_id, current_seq, created_by, created_at, updated_at
    "#,
  )
  .bind(workspace_id)
  .bind(&root_block_id)
  .bind(user_id)
  .fetch_one(&mut *tx)
  .await?;

  let mut imported = import_markdown(&payload.markdown, &root_block_id);
  rewire_blob_hrefs(&mut imported.blocks, workspace_id);
  // Seed the yrs base with the imported content, in the import's own
  // transaction. This also makes the body searchable immediately: `content_text`
  // (the FTS index) is co-written with the base, so an imported-but-never-opened
  // page is findable by its body, not just its title.
  mica_app_core::sync::seed_base_tx(&mut tx, document.id, imported.clone()).await?;
  let snapshot = store::AppliedContent {
    version_seq: 0,
    schema_version: imported.schema_version,
    payload: serde_json::to_value(&imported)
      .map_err(|error| ApiError::Internal(error.to_string()))?,
  };

  let position = Uuid::now_v7().to_string();
  let view = sqlx::query_as::<_, View>(
    r#"
      INSERT INTO views (
        workspace_id,
        parent_view_id,
        object_id,
        object_type,
        name,
        position,
        created_by
      )
      VALUES ($1, $2, $3, 'document', $4, $5, $6)
      RETURNING
        id,
        workspace_id,
        parent_view_id,
        object_id,
        object_type::text AS object_type,
        name,
        icon,
        position,
        is_deleted,
        created_by,
        created_at,
        updated_at
    "#,
  )
  .bind(workspace_id)
  .bind(payload.parent_view_id)
  .bind(document.id)
  .bind(name)
  .bind(position)
  .bind(user_id)
  .fetch_one(&mut *tx)
  .await?;

  tx.commit().await?;

  Ok(Json(DocumentBootstrapResponse {
    document,
    view,
    snapshot,
  }))
}

pub async fn update_view(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, view_id)): Path<(Uuid, Uuid)>,
  Json(payload): Json<UpdateViewRequest>,
) -> ApiResult<Json<ViewResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;

  let name = normalize_view_name(&payload.name)?;
  // An emoji is one grapheme; anything longer is either a mistake or someone
  // trying to stuff a label in here. Bound it rather than letting the tree render
  // arbitrary text where an icon goes.
  if let Some(icon) = payload.icon.as_deref() {
    if icon.chars().count() > 8 {
      return Err(ApiError::BadRequest("icon is too long".into()));
    }
  }
  let view = sqlx::query_as::<_, View>(
    r#"
      UPDATE views
      SET name = $1,
          -- NULL = leave alone, '' = clear, otherwise set (see UpdateViewRequest)
          icon = CASE
                   WHEN $4::text IS NULL THEN icon
                   WHEN $4 = '' THEN NULL
                   ELSE $4
                 END,
          updated_at = now()
      WHERE id = $2 AND workspace_id = $3 AND is_deleted = false
      RETURNING
        id,
        workspace_id,
        parent_view_id,
        object_id,
        object_type::text AS object_type,
        name,
        icon,
        position,
        is_deleted,
        created_by,
        created_at,
        updated_at
    "#,
  )
  .bind(name)
  .bind(view_id)
  .bind(workspace_id)
  .bind(payload.icon.as_deref())
  .fetch_optional(&state.db)
  .await?
  .ok_or(ApiError::NotFound)?;

  // A page's name now lives IN its document (root block `title`), with
  // `views.name` as the projection — so a rename has to reach the document, or
  // the next push would re-derive the OLD title over the column and silently
  // undo the rename. See `docs/page-title-plan.md`.
  //
  // The column is still written above and first: this endpoint must keep working
  // for folders (no document at all) and for the many pages that have no title
  // yet (there is no backfill). The document write is what makes the rename
  // stick for pages that DO carry one, and what makes open editors converge.
  //
  // Failure here is logged, not returned: the user asked to rename a page and
  // the rename happened. Turning a title-sync hiccup into a 500 would report a
  // rename that actually succeeded as a failure.
  if view.object_type == "document" {
    match mica_app_core::sync::set_document_title(
      &state.db,
      workspace_id,
      view.object_id,
      user_id,
      &view.name,
      &state.config.sync_tuning,
    )
    .await
    {
      // Fan it out to open editors on the same yrs channel a live edit uses —
      // nil origin so every socket receives it. Same shape as a version restore
      // (`history.rs`), because it is the same kind of event: a server-side
      // change to a document somebody may have open.
      Ok((rid, update)) if !update.is_empty() => {
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&update);
        let event = serde_json::json!({
          "type": "sync.update",
          "document_id": view.object_id,
          "rid": rid,
          "actor_id": user_id,
          "update": encoded,
        });
        state.hub.broadcast_if_active(
          view.object_id,
          Uuid::nil(),
          std::sync::Arc::from(event.to_string()),
        );
      }
      Ok(_) => {}
      Err(error) => tracing::warn!(
        view_id = %view.id, document_id = %view.object_id, %error,
        "rename: could not write the title into the document"
      ),
    }
  }

  Ok(Json(ViewResponse { view }))
}

pub async fn delete_view(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, view_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ViewListResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;

  // Soft-delete (move to the recycle bin) the page and its whole subtree.
  let result = sqlx::query(
    r#"
      WITH RECURSIVE subtree AS (
        SELECT id FROM views WHERE id = $1 AND workspace_id = $2
        UNION ALL
        SELECT v.id FROM views v JOIN subtree s ON v.parent_view_id = s.id
      )
      UPDATE views
      SET is_deleted = true, updated_at = now()
      WHERE id IN (SELECT id FROM subtree)
        AND workspace_id = $2
        AND is_deleted = false
    "#,
  )
  .bind(view_id)
  .bind(workspace_id)
  .execute(&state.db)
  .await?;

  if result.rows_affected() == 0 {
    return Err(ApiError::NotFound);
  }

  let views = fetch_workspace_views(&state.db, workspace_id).await?;

  Ok(Json(ViewListResponse { views }))
}

pub async fn list_trash(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
) -> ApiResult<Json<ViewListResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;

  let views = fetch_deleted_workspace_views(&state.db, workspace_id).await?;

  Ok(Json(ViewListResponse { views }))
}

pub async fn restore_view(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, view_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ViewListResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;

  // Restore the page and the subtree that was deleted with it.
  let result = sqlx::query(
    r#"
      WITH RECURSIVE subtree AS (
        SELECT id FROM views WHERE id = $1 AND workspace_id = $2
        UNION ALL
        SELECT v.id FROM views v JOIN subtree s ON v.parent_view_id = s.id
      )
      UPDATE views
      SET is_deleted = false, updated_at = now()
      WHERE id IN (SELECT id FROM subtree)
        AND workspace_id = $2
        AND is_deleted = true
    "#,
  )
  .bind(view_id)
  .bind(workspace_id)
  .execute(&state.db)
  .await?;

  if result.rows_affected() == 0 {
    return Err(ApiError::NotFound);
  }

  // If the restored page's parent is no longer an active view, lift it to the
  // top level so it does not become an orphan.
  sqlx::query(
    r#"
      UPDATE views
      SET parent_view_id = NULL, updated_at = now()
      WHERE id = $1 AND workspace_id = $2 AND parent_view_id IS NOT NULL
        AND parent_view_id NOT IN (
          SELECT id FROM views WHERE workspace_id = $2 AND is_deleted = false
        )
    "#,
  )
  .bind(view_id)
  .bind(workspace_id)
  .execute(&state.db)
  .await?;

  let views = fetch_workspace_views(&state.db, workspace_id).await?;

  Ok(Json(ViewListResponse { views }))
}

/// Permanently delete a view subtree AND the `documents` those views back,
/// returning `(views_deleted, docs_deleted)`.
///
/// `views.object_id -> documents.id` is NOT a foreign key (object_id may be a
/// folder, which has no document), so deleting views alone stranded the document
/// plus its yrs base / snapshots / versions / shares — content "永久删除" claimed
/// to erase kept living (a privacy hole, incl. a still-valid public share token)
/// and leaked disk. Deleting the `documents` row cascades (ON DELETE CASCADE) to
/// every document_* table: document_yrs_base (the CRDT content), snapshots,
/// versions, op updates, and document_shares (revokes the public link). Blobs are
/// content-addressed and may be shared by another page, so they are reclaimed
/// lazily by blob_gc once their last referencing block is gone — never here.
///
/// One atomic statement: `subtree` gathers the view ids + object ids/types,
/// `deleted_views` removes the views and returns what they backed, `purged_docs`
/// removes the documents of the document-typed ones. Extracted so a DB test can
/// prove the cascade without the full HTTP handler.
async fn purge_view_subtree(
  db: &PgPool,
  workspace_id: Uuid,
  view_id: Uuid,
) -> ApiResult<(i64, i64)> {
  let row = sqlx::query_as::<_, (i64, i64)>(
    r#"
      WITH RECURSIVE subtree AS (
        SELECT id, object_id, object_type FROM views WHERE id = $1 AND workspace_id = $2
        UNION ALL
        SELECT v.id, v.object_id, v.object_type
        FROM views v JOIN subtree s ON v.parent_view_id = s.id
      ),
      deleted_views AS (
        DELETE FROM views
        WHERE id IN (SELECT id FROM subtree) AND workspace_id = $2
        RETURNING object_id, object_type
      ),
      purged_docs AS (
        DELETE FROM documents
        WHERE id IN (
          SELECT object_id FROM deleted_views WHERE object_type::text = 'document'
        )
        RETURNING id
      )
      SELECT
        (SELECT count(*) FROM deleted_views)::bigint,
        (SELECT count(*) FROM purged_docs)::bigint
    "#,
  )
  .bind(view_id)
  .bind(workspace_id)
  .fetch_one(db)
  .await?;
  Ok(row)
}

pub async fn purge_view(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, view_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ViewListResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;

  // Permanently remove the page + its subtree AND the documents they back (so no
  // orphaned content/shares survive "永久删除"). See [`purge_view_subtree`].
  let (views_deleted, _docs_deleted) =
    purge_view_subtree(&state.db, workspace_id, view_id).await?;

  if views_deleted == 0 {
    return Err(ApiError::NotFound);
  }

  let views = fetch_deleted_workspace_views(&state.db, workspace_id).await?;

  Ok(Json(ViewListResponse { views }))
}

/// Empty the whole recycle bin: every trashed view in the workspace, plus the
/// documents they back.
///
/// One statement rather than a loop over subtree roots. No recursive CTE is
/// needed either: trashing a folder marks its entire subtree `is_deleted`
/// (`delete_view` cascades), so "every row with is_deleted = true" already *is*
/// the closure — and doing it in a single statement means a caller cannot end up
/// with half a bin emptied.
///
/// Idempotent by nature: an already-empty bin returns zeros rather than 404. The
/// single-view [`purge_view`] does 404 on a miss, but there the id came from a row
/// the user clicked; here the request is "make it empty", which an empty bin
/// already satisfies.
pub async fn purge_workspace_trash(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
) -> ApiResult<Json<PurgeTrashResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;

  let (views_deleted, documents_deleted) =
    empty_workspace_trash(&state.db, workspace_id).await?;

  Ok(Json(PurgeTrashResponse {
    views_deleted,
    documents_deleted,
  }))
}

/// Delete every trashed view in the workspace and the documents they back.
/// Returns `(views_deleted, documents_deleted)`.
///
/// Split out of the handler so a test can reach it: the handler needs auth
/// headers, which a DB-level test has no way to construct.
async fn empty_workspace_trash(db: &PgPool, workspace_id: Uuid) -> ApiResult<(i64, i64)> {
  let row = sqlx::query_as::<_, (i64, i64)>(
    r#"
      WITH deleted_views AS (
        DELETE FROM views
        WHERE workspace_id = $1 AND is_deleted = true
        RETURNING object_id, object_type
      ),
      purged_docs AS (
        DELETE FROM documents
        WHERE id IN (
          SELECT object_id FROM deleted_views WHERE object_type::text = 'document'
        )
        RETURNING id
      )
      SELECT
        (SELECT count(*) FROM deleted_views)::bigint,
        (SELECT count(*) FROM purged_docs)::bigint
    "#,
  )
  .bind(workspace_id)
  .fetch_one(db)
  .await?;
  Ok((row.0, row.1))
}

pub async fn move_view(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, view_id)): Path<(Uuid, Uuid)>,
  Json(payload): Json<MoveViewRequest>,
) -> ApiResult<Json<ViewResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  ensure_view_in_workspace(&state.db, workspace_id, view_id).await?;

  if let Some(parent_view_id) = payload.parent_view_id {
    ensure_valid_parent_view(&state.db, workspace_id, view_id, parent_view_id).await?;
  }

  let position = normalize_position(payload.position)?;
  let view = sqlx::query_as::<_, View>(
    r#"
      UPDATE views
      SET parent_view_id = $1, position = $2, updated_at = now()
      WHERE id = $3 AND workspace_id = $4 AND is_deleted = false
      RETURNING
        id,
        workspace_id,
        parent_view_id,
        object_id,
        object_type::text AS object_type,
        name,
        icon,
        position,
        is_deleted,
        created_by,
        created_at,
        updated_at
    "#,
  )
  .bind(payload.parent_view_id)
  .bind(position)
  .bind(view_id)
  .bind(workspace_id)
  .fetch_optional(&state.db)
  .await?
  .ok_or(ApiError::NotFound)?;

  Ok(Json(ViewResponse { view }))
}

/// `POST /api/workspaces/{workspace_id}/views/reorder`
///
/// Reorder a parent's children in ONE atomic call: every id in
/// `ordered_view_ids` is set as a child of `parent_view_id` (null = top level)
/// and given an evenly-spaced position in the given order. This is what a "sort
/// this folder" operation needs — the per-view `move` endpoint would take one
/// request per sibling and could interleave a failure. Positions are 10-spaced,
/// zero-padded to a fixed width so they sort lexicographically like the rest.
pub async fn reorder_views(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
  Json(payload): Json<ReorderRequest>,
) -> ApiResult<Json<ReorderResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;

  if payload.ordered_view_ids.is_empty() {
    return Err(ApiError::BadRequest(
      "ordered_view_ids cannot be empty".to_string(),
    ));
  }
  // A duplicate id would leave one of the two at a stale position — reject
  // rather than silently keep only the last.
  let mut seen = std::collections::HashSet::new();
  for id in &payload.ordered_view_ids {
    if !seen.insert(*id) {
      return Err(ApiError::BadRequest(format!("duplicate view id {id}")));
    }
  }
  // Validate everything BEFORE writing anything: each id belongs to the
  // workspace, and re-parenting under `parent_view_id` neither escapes the
  // workspace nor forms a cycle (a view under its own descendant).
  for id in &payload.ordered_view_ids {
    ensure_view_in_workspace(&state.db, workspace_id, *id).await?;
    if let Some(parent) = payload.parent_view_id {
      ensure_valid_parent_view(&state.db, workspace_id, *id, parent).await?;
    }
  }

  let mut tx = state.db.begin().await?;
  for (i, id) in payload.ordered_view_ids.iter().enumerate() {
    let position = format!("{:010}", (i + 1) * 10);
    let affected = sqlx::query(
      r#"
        UPDATE views
        SET parent_view_id = $1, position = $2, updated_at = now()
        WHERE id = $3 AND workspace_id = $4 AND is_deleted = false
      "#,
    )
    .bind(payload.parent_view_id)
    .bind(&position)
    .bind(id)
    .bind(workspace_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    // Validation ran before the tx, so a 0-row UPDATE means this view was
    // deleted concurrently in the window. Reporting `reordered: len()` regardless
    // (the old behavior) is the `ssh | tee` shape — a partial reorder claimed as
    // full success, leaving one node stuck at its old parent/position (P1-3).
    // Return the anomaly instead; the `?`/return drops `tx`, rolling everything
    // back so the client can refetch and retry against fresh state.
    if affected == 0 {
      return Err(ApiError::Conflict(format!(
        "view {id} was modified concurrently during reorder; refetch and retry"
      )));
    }
  }
  tx.commit().await?;

  Ok(Json(ReorderResponse {
    reordered: payload.ordered_view_ids.len(),
  }))
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
  requested.iter().copied().filter(|id| !hit.contains(id)).collect()
}

/// `POST /api/workspaces/{workspace_id}/views/batch-trash`
///
/// Soft-delete many pages (each with its whole subtree) in ONE statement. Same
/// recursive CTE as [delete_view], seeded from a set instead of a single id, so
/// 300 pages cost one query rather than 300 round trips.
pub async fn batch_trash_views(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
  Json(payload): Json<BatchViewsRequest>,
) -> ApiResult<Json<BatchViewsResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  check_batch_ids(&payload.view_ids)?;

  let touched = trash_views_batch(&state.db, workspace_id, &payload.view_ids).await?;

  Ok(Json(BatchViewsResponse {
    affected: touched.len(),
    skipped: skipped_ids(&payload.view_ids, &touched),
  }))
}

/// Soft-delete [view_ids] and everything under them. Returns the ids it changed.
///
/// Split out of the handler for the same reason [purge_views_batch] is: the
/// handler needs auth headers a DB-level test cannot construct. Until 2026-08-27
/// this statement lived inline and had no test at all — the guarantee was "the
/// SQL parses and the route table has no conflict", not "300 ids in, the right
/// rows out".
///
/// `is_deleted = false` is in the WHERE, not the seed: a row already in the bin
/// must not be re-stamped (that would move its `updated_at` and make it look
/// freshly deleted) and must come back in `skipped` so the caller is told it did
/// less than it asked.
async fn trash_views_batch(
  db: &PgPool,
  workspace_id: Uuid,
  view_ids: &[Uuid],
) -> ApiResult<Vec<Uuid>> {
  Ok(
    sqlx::query_scalar::<_, Uuid>(
      r#"
        WITH RECURSIVE subtree AS (
          SELECT id FROM views WHERE id = ANY($1) AND workspace_id = $2
          UNION ALL
          SELECT v.id FROM views v JOIN subtree s ON v.parent_view_id = s.id
        )
        UPDATE views
        SET is_deleted = true, updated_at = now()
        WHERE id IN (SELECT id FROM subtree)
          AND workspace_id = $2
          AND is_deleted = false
        RETURNING id
      "#,
    )
    .bind(view_ids)
    .bind(workspace_id)
    .fetch_all(db)
    .await?,
  )
}

/// `POST /api/workspaces/{workspace_id}/views/batch-restore`
///
/// Bring many pages back out of the recycle bin, each with the subtree that was
/// deleted with it. The mirror of [batch_trash_views], and the reason a bulk
/// delete is safe to attempt: it is one call to undo.
pub async fn batch_restore_views(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
  Json(payload): Json<BatchViewsRequest>,
) -> ApiResult<Json<BatchViewsResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  check_batch_ids(&payload.view_ids)?;

  let touched = restore_views_batch(&state.db, workspace_id, &payload.view_ids).await?;

  Ok(Json(BatchViewsResponse {
    affected: touched.len(),
    skipped: skipped_ids(&payload.view_ids, &touched),
  }))
}

/// Bring [view_ids] and their trashed subtrees back. Returns the ids it changed.
///
/// The exact mirror of [trash_views_batch] — same CTE, the two `is_deleted`
/// booleans flipped. Extracted for the same reason, and tested beside it so the
/// pair cannot drift apart silently.
async fn restore_views_batch(
  db: &PgPool,
  workspace_id: Uuid,
  view_ids: &[Uuid],
) -> ApiResult<Vec<Uuid>> {
  Ok(
    sqlx::query_scalar::<_, Uuid>(
      r#"
        WITH RECURSIVE subtree AS (
          SELECT id FROM views WHERE id = ANY($1) AND workspace_id = $2
          UNION ALL
          SELECT v.id FROM views v JOIN subtree s ON v.parent_view_id = s.id
        )
        UPDATE views
        SET is_deleted = false, updated_at = now()
        WHERE id IN (SELECT id FROM subtree)
          AND workspace_id = $2
          AND is_deleted = true
        RETURNING id
      "#,
    )
    .bind(view_ids)
    .bind(workspace_id)
    .fetch_all(db)
    .await?,
  )
}

/// `POST /api/workspaces/{workspace_id}/views/batch-purge`
///
/// PERMANENTLY delete many trashed pages (each with its subtree) in ONE
/// statement — the middle rung that was missing between [purge_view] (one at a
/// time) and [purge_workspace_trash] (all or nothing). Clearing 200 specific
/// items meant 200 round trips, which is exactly the loop the batch endpoints
/// exist to remove.
///
/// **Only touches views that are ALREADY in the recycle bin**, and that is a
/// deliberate difference from [purge_view], which has no such filter and will
/// permanently destroy a LIVE page. Reproducing that here would mean one call
/// could erase 300 pages that were never deleted, with no recoverable step
/// anywhere — a new capability, not a batching of an old one. Anything not
/// trashed comes back in `skipped`, so the caller is told what did not happen
/// rather than quietly getting less than it asked for.
///
/// Same cascade as [purge_view_subtree]: the documents behind those views go
/// too, taking their yrs base, snapshots, versions and public share tokens with
/// them. Blobs are left to `blob_gc` — they are content-addressed and may still
/// be referenced by a page that is staying.
pub async fn batch_purge_views(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
  Json(payload): Json<BatchViewsRequest>,
) -> ApiResult<Json<BatchViewsResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  check_batch_ids(&payload.view_ids)?;

  let touched = purge_views_batch(&state.db, workspace_id, &payload.view_ids).await?;

  Ok(Json(BatchViewsResponse {
    affected: touched.len(),
    skipped: skipped_ids(&payload.view_ids, &touched),
  }))
}

/// Permanently delete the trashed views named by [view_ids], each with its
/// subtree, and the documents they back. Returns every view id that went.
///
/// Split out of the handler for the same reason [empty_workspace_trash] is: the
/// handler needs auth headers a DB-level test cannot construct, and an
/// irreversible statement is the last one that should go untested.
async fn purge_views_batch(
  db: &PgPool,
  workspace_id: Uuid,
  view_ids: &[Uuid],
) -> ApiResult<Vec<Uuid>> {
  // `purged_docs` is never read by the final SELECT, and that is fine: a
  // data-modifying CTE runs exactly once whether or not anything references it.
  //
  // `is_deleted = true` is on the SEED only. The recursive arm must not repeat
  // it: trashing a folder cascades the flag to its subtree, so the descendants
  // are already marked — but if a future change stops cascading, filtering here
  // too would silently start leaving children behind instead of failing loudly.
  let touched = sqlx::query_scalar::<_, Uuid>(
    r#"
      WITH RECURSIVE subtree AS (
        SELECT id, object_id, object_type FROM views
        WHERE id = ANY($1) AND workspace_id = $2 AND is_deleted = true
        UNION ALL
        SELECT v.id, v.object_id, v.object_type
        FROM views v JOIN subtree s ON v.parent_view_id = s.id
      ),
      deleted_views AS (
        DELETE FROM views
        WHERE id IN (SELECT id FROM subtree) AND workspace_id = $2
        RETURNING id, object_id, object_type
      ),
      purged_docs AS (
        DELETE FROM documents
        WHERE id IN (
          SELECT object_id FROM deleted_views WHERE object_type::text = 'document'
        )
        RETURNING id
      )
      SELECT id FROM deleted_views
    "#,
  )
  .bind(view_ids)
  .bind(workspace_id)
  .fetch_all(db)
  .await?;
  Ok(touched)
}

/// `POST /api/workspaces/{workspace_id}/views/batch-move`
///
/// Re-parent many views at once. Unlike trash/restore this cannot be one
/// statement: the folder-only container rule and the cycle check are per view,
/// so every id is validated against the destination BEFORE anything is written
/// — the same order [reorder_views] uses, and for the same reason. The writes
/// then run in one transaction, so a view deleted underneath us rolls the whole
/// move back instead of leaving the tree half-reorganised.
pub async fn batch_move_views(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
  Json(payload): Json<BatchMoveRequest>,
) -> ApiResult<Json<BatchViewsResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  check_batch_ids(&payload.view_ids)?;

  for id in &payload.view_ids {
    ensure_view_in_workspace(&state.db, workspace_id, *id).await?;
    if let Some(parent) = payload.parent_view_id {
      ensure_valid_parent_view(&state.db, workspace_id, *id, parent).await?;
    }
  }

  move_views_tx(
    &state.db,
    workspace_id,
    &payload.view_ids,
    payload.parent_view_id,
  )
  .await?;

  Ok(Json(BatchViewsResponse {
    affected: payload.view_ids.len(),
    skipped: Vec::new(),
  }))
}

/// Re-parent [view_ids] under [parent_view_id], in the order given, in ONE
/// transaction.
///
/// Positions are `(index + 1) * 10`, so the caller's order IS the sibling order
/// at the destination.
///
/// A view that no longer matches (deleted, or moved to another workspace, under
/// us) makes the WHOLE move fail with [ApiError::Conflict] — the `?` returns
/// before `commit`, so the transaction is dropped and every earlier row in this
/// batch rolls back. That is the difference from trash/restore, which report
/// what they missed in `skipped`: a half-applied reorganisation has no honest
/// way to be reported, because the ids the caller sent no longer describe the
/// tree it is looking at.
///
/// Split out of the handler so a DB test can reach it — the handler needs auth
/// headers a database-level test cannot construct.
async fn move_views_tx(
  db: &PgPool,
  workspace_id: Uuid,
  view_ids: &[Uuid],
  parent_view_id: Option<Uuid>,
) -> ApiResult<()> {
  let mut tx = db.begin().await?;
  for (i, id) in view_ids.iter().enumerate() {
    let position = format!("{:010}", (i + 1) * 10);
    let affected = sqlx::query(
      r#"
        UPDATE views
        SET parent_view_id = $1, position = $2, updated_at = now()
        WHERE id = $3 AND workspace_id = $4 AND is_deleted = false
      "#,
    )
    .bind(parent_view_id)
    .bind(&position)
    .bind(id)
    .bind(workspace_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if affected == 0 {
      return Err(ApiError::Conflict(format!(
        "view {id} was modified concurrently during the move; refetch and retry"
      )));
    }
  }
  tx.commit().await?;
  Ok(())
}

pub async fn bootstrap_document(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<DocumentBootstrapResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;

  let document = store::fetch_document(&state.db, workspace_id, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;

  let view = fetch_document_view(&state.db, workspace_id, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;

  // Live content, materialized from the yrs base. `version_seq` reports the
  // document's own seq rather than a `document_snapshots` row id: since S4 that
  // table has no current row to report, and the client only ever used this
  // number to tell one read apart from a later one — which `current_seq` does
  // exactly, and honestly.
  let payload = store::current_payload(&state.db, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;
  let snapshot = store::AppliedContent {
    version_seq: document.current_seq,
    schema_version: payload.schema_version,
    payload: serde_json::to_value(&payload)
      .map_err(|error| ApiError::Internal(error.to_string()))?,
  };

  Ok(Json(DocumentBootstrapResponse {
    document,
    view,
    snapshot,
  }))
}

pub async fn apply_document_update(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
  Json(payload): Json<ApplyDocumentUpdateRequest>,
) -> ApiResult<Json<DocumentUpdateResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;

  if payload.operations.is_empty() {
    return Err(ApiError::BadRequest(
      "at least one document operation is required".to_string(),
    ));
  }

  let applied = store::apply_document_operations(
    &state.db,
    workspace_id,
    document_id,
    user_id,
    &payload.operations,
  )
  .await?;

  // Reach any clients editing this document over WebSocket. A REST write has no
  // originating connection, so it is attributed to the nil connection id.
  ws::broadcast_applied_update(&state.hub, &applied, Uuid::nil(), None);

  Ok(Json(DocumentUpdateResponse {
    document: applied.document,
    snapshot: applied.snapshot,
    update: applied.update,
  }))
}

/// Whether this read should have the page's title welded onto the front.
///
/// **Off by default, and that default is load-bearing.** This handler is the GET
/// half of a GET/PATCH pair on the same path (`routes/mod.rs`), and it is what
/// MCP `mica_read_document` calls. A title returned here by default would come
/// straight back on the next `replace_all` as a real heading in the body, and
/// the one after that would add another — compounding once per round trip.
///
/// So the caller says which it wants. `?title=1` is for the two places that are
/// producing a page for a HUMAN to keep — "export as .md" and "copy page
/// content" — where the name is otherwise lost the moment the text leaves Mica.
#[derive(Debug, Deserialize)]
pub struct MarkdownExportParams {
  #[serde(default)]
  title: bool,
}

pub async fn export_document_markdown(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
  Query(params): Query<MarkdownExportParams>,
) -> ApiResult<Json<MarkdownExportResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;

  ensure_document_in_workspace(&state.db, workspace_id, document_id).await?;
  let payload = store::current_payload(&state.db, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;
  let assets = blob_asset_map(&payload.blocks, workspace_id);
  let markdown = export_markdown_with_assets(&payload, &assets)
    .map_err(|error| ApiError::BadRequest(error.to_string()))?;

  let markdown = if params.title {
    let view_name = fetch_document_view(&state.db, workspace_id, document_id)
      .await?
      .map(|view| view.name);
    let title = document_title(&payload)
      .or(view_name.as_deref())
      .unwrap_or_default();
    with_page_title(&markdown, title)
  } else {
    markdown
  };

  Ok(Json(MarkdownExportResponse { markdown }))
}

#[derive(Debug, Deserialize)]
pub struct BatchReadRequest {
  document_ids: Vec<Uuid>,
}

/// One document's result. Either `markdown` or `error` is present, never both.
#[derive(Debug, Serialize)]
pub struct BatchReadItem {
  document_id: Uuid,
  #[serde(skip_serializing_if = "Option::is_none")]
  markdown: Option<String>,
  /// Why this ONE document could not be read. Inlined rather than failing the
  /// request: a survey of a few hundred pages should not be lost because one of
  /// them was deleted between listing and reading.
  #[serde(skip_serializing_if = "Option::is_none")]
  error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BatchReadResponse {
  documents: Vec<BatchReadItem>,
}

/// `POST /api/workspaces/{workspace_id}/documents/batch-read`
///
/// Read many pages' Markdown in ONE request. Scanning a workspace — "which of
/// these are empty?" — meant a request per page, thousands of them; this makes
/// it one, and checks workspace membership once instead of per document.
///
/// It is one ROUND TRIP, not one query: the documents are still assembled one
/// at a time, because each is a CRDT payload rendered to Markdown separately.
/// The win is the network, which is where the cost actually was.
pub async fn batch_read_documents(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
  Json(payload): Json<BatchReadRequest>,
) -> ApiResult<Json<BatchReadResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;
  check_batch_ids(&payload.document_ids)?;

  let mut documents = Vec::with_capacity(payload.document_ids.len());
  for document_id in payload.document_ids {
    let rendered = read_one_markdown(&state, workspace_id, document_id).await;
    documents.push(match rendered {
      Ok(markdown) => BatchReadItem {
        document_id,
        markdown: Some(markdown),
        error: None,
      },
      Err(error) => BatchReadItem {
        document_id,
        markdown: None,
        error: Some(error.to_string()),
      },
    });
  }

  Ok(Json(BatchReadResponse { documents }))
}

/// The body of [export_document_markdown] without the HTTP wrapper, so the
/// batch route renders pages exactly the way the single route does.
async fn read_one_markdown(
  state: &AppState,
  workspace_id: Uuid,
  document_id: Uuid,
) -> ApiResult<String> {
  ensure_document_in_workspace(&state.db, workspace_id, document_id).await?;
  let payload = store::current_payload(&state.db, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;
  let assets = blob_asset_map(&payload.blocks, workspace_id);
  export_markdown_with_assets(&payload, &assets)
    .map_err(|error| ApiError::BadRequest(error.to_string()))
}

#[derive(Debug, Serialize)]
pub struct OutlineHeading {
  block_id: String,
  level: i64,
  text: String,
}

#[derive(Debug, Serialize)]
pub struct DocumentOutlineResponse {
  /// Headings in document order — the anchors an AI names to write in place
  /// (`insert_at`/`find_replace`) instead of rewriting the whole doc.
  headings: Vec<OutlineHeading>,
  /// Every top-level block id in document order (finer anchors than headings).
  block_ids: Vec<String>,
  /// The document's current sequence number. Pass it back as `expected_seq` on a
  /// write to make the write conflict-checked (409 if the doc changed meanwhile).
  seq: i64,
}

/// `GET /api/workspaces/{workspace_id}/documents/{document_id}/outline`
///
/// The document's structure map (headings + block ids) so an AI can anchor a
/// local write rather than replace the whole page — the "get outline first,
/// then patch" loop the note-app MCP servers (Obsidian, Notion) converge on.
pub async fn document_outline(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<DocumentOutlineResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;
  ensure_document_in_workspace(&state.db, workspace_id, document_id).await?;

  let payload = store::current_payload(&state.db, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;
  // The current seq lets a caller pin the version it read here and pass it back
  // as `expected_seq` for a conflict-checked write (optimistic concurrency).
  let seq: i64 = sqlx::query_scalar("SELECT current_seq FROM documents WHERE id = $1")
    .bind(document_id)
    .fetch_one(&state.db)
    .await?;
  Ok(Json(outline_from_payload(&payload, seq)))
}

/// Pure: walk the block tree from the root in document order, collecting every
/// top-level block id and (for `heading` blocks) a heading entry. Testable
/// without a DB.
fn outline_from_payload(
  payload: &mica_app_core::documents::DocumentSnapshotPayload,
  seq: i64,
) -> DocumentOutlineResponse {
  let by_id: std::collections::HashMap<&str, &mica_app_core::documents::Block> =
    payload.blocks.iter().map(|b| (b.id.as_str(), b)).collect();
  let mut headings = Vec::new();
  let mut block_ids = Vec::new();
  outline_walk(
    &payload.root_block_id,
    &by_id,
    &mut headings,
    &mut block_ids,
  );
  DocumentOutlineResponse {
    headings,
    block_ids,
    seq,
  }
}

fn outline_walk(
  id: &str,
  by_id: &std::collections::HashMap<&str, &mica_app_core::documents::Block>,
  headings: &mut Vec<OutlineHeading>,
  block_ids: &mut Vec<String>,
) {
  let Some(block) = by_id.get(id) else {
    return;
  };
  // Only the root's DIRECT children — these are exactly the anchors `insert_at`
  // accepts (it resolves against root.children). Mica stores a flat block list
  // under the root, so this is also the whole body; don't recurse and advertise
  // deeper ids that insert_at would reject.
  for child_id in &block.children {
    if let Some(child) = by_id.get(child_id.as_str()) {
      block_ids.push(child.id.clone());
      if child.kind == "heading" {
        headings.push(OutlineHeading {
          block_id: child.id.clone(),
          level: child
            .data
            .get("level")
            .and_then(|v| v.as_i64())
            .unwrap_or(1),
          text: child.text.clone(),
        });
      }
    }
  }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkdownUpdateMode {
  /// Wipe the document body and write the markdown fresh.
  ReplaceAll,
  /// Add the markdown after the existing content (least conflict-prone).
  Append,
  /// Insert the markdown right after `anchor` (a top-level block id from the
  /// outline) — a local write that leaves the rest of the page untouched.
  InsertAt,
  /// Replace every occurrence of `find` with `replace` across the doc's block
  /// text (no `markdown`). Errors if nothing matches.
  FindReplace,
  /// Remove the block named by `anchor`, and its subtree (no `markdown`).
  ///
  /// The only way to take anything OUT of a page short of rewriting the whole
  /// body. Without it the four other modes could add and overwrite but never
  /// remove, so "drop this paragraph" meant `replace_all` — re-sending the
  /// entire document to delete one block, which is how a caller that cannot
  /// reproduce the original bytes exactly (an agent retyping a page with
  /// non-breaking spaces or control characters in it) silently damages the
  /// rest of the page.
  Delete,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMarkdownRequest {
  pub mode: MarkdownUpdateMode,
  #[serde(default)]
  pub markdown: String,
  /// `insert_at`: the top-level block id to insert after.
  #[serde(default)]
  pub anchor: Option<String>,
  /// `find_replace`: the text to find / its replacement.
  #[serde(default)]
  pub find: Option<String>,
  #[serde(default)]
  pub replace: Option<String>,
  /// Optimistic concurrency: the `seq` the caller last saw (from the outline or a
  /// prior write ack). When set and the document has moved on, the write is
  /// refused with 409 instead of clobbering the intervening edit. Absent = the
  /// existing last-writer-wins behaviour.
  #[serde(default)]
  pub expected_seq: Option<i64>,
}

/// `PATCH /api/workspaces/{workspace_id}/documents/{document_id}/markdown`
///
/// Write markdown into an EXISTING document (the AI-facing write path). Content
/// is markdown-in — the block/CRDT ops are derived server-side (reusing
/// `import_markdown` + the authoritative `apply_document_operations`), so callers
/// never construct raw ops. `append` is the safe default; `replace_all` wipes
/// first. (Anchored `insert_at`/`find_replace` land in M2.)
pub async fn update_document_markdown(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
  Json(request): Json<UpdateMarkdownRequest>,
) -> ApiResult<Json<DocumentUpdateResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  ensure_document_in_workspace(&state.db, workspace_id, document_id).await?;

  // Derive the ops from the snapshot INSIDE the write lock (no read-then-apply
  // TOCTOU: an anchor index / delete target can't drift under a concurrent edit).
  let applied = store::apply_derived_operations(
    &state.db,
    workspace_id,
    document_id,
    user_id,
    request.expected_seq,
    |payload| markdown_update_ops(payload, &request, workspace_id),
  )
  .await?;
  ws::broadcast_applied_update(&state.hub, &applied, Uuid::nil(), None);

  Ok(Json(DocumentUpdateResponse {
    document: applied.document,
    snapshot: applied.snapshot,
    update: applied.update,
  }))
}

#[derive(Debug, Deserialize)]
pub struct RehostImageParams {
  /// The image block whose external `url` is replaced by the stored file.
  block_id: String,
  /// Original filename (with extension) — sets the stored file's name + mime.
  file_name: String,
}

#[derive(Debug, Deserialize)]
pub struct DropImageParams {
  /// The image block to remove.
  block_id: String,
  /// The external `url` that block must still carry. The caller decided this
  /// image is gone by fetching that exact URL; if the block now points
  /// somewhere else, someone edited it since and the decision no longer
  /// applies. Deleting on block id alone would make a stale sweep destructive.
  url: String,
}

/// `POST /api/workspaces/{ws}/documents/{doc}/drop-image?block_id=…&url=…`
///
/// Removes an image block whose source is gone for good — the counterpart to
/// `rehost-image`, and deliberately the same shape: ONE targeted op, never a
/// whole-doc rewrite. Refuses unless the block is still an image, still carries
/// the URL the caller checked, and is NOT already stored in Mica — a block with
/// a `file_id` has bytes we own, and nothing about a dead external link makes
/// those disposable.
pub async fn drop_image(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
  Query(params): Query<DropImageParams>,
) -> ApiResult<Json<DocumentUpdateResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  ensure_document_in_workspace(&state.db, workspace_id, document_id).await?;

  let block_id = params.block_id;
  let expected_url = params.url;
  let applied = store::apply_derived_operations(
    &state.db,
    workspace_id,
    document_id,
    user_id,
    None,
    |payload| {
      let block = payload
        .blocks
        .iter()
        .find(|b| b.id == block_id)
        .ok_or_else(|| format!("block {block_id} not found"))?;
      if block.kind != "image" {
        return Err(format!("block {block_id} is not an image"));
      }
      let data = &block.data;
      if data.get("file_id").and_then(|v| v.as_str()).is_some_and(|s| !s.is_empty()) {
        return Err(format!("block {block_id} is stored in Mica, not an external link"));
      }
      let current = data.get("url").and_then(|v| v.as_str()).unwrap_or_default();
      if current != expected_url {
        return Err(format!(
          "block {block_id} no longer points at the checked url — it was edited since"
        ));
      }
      Ok(vec![DocumentOperation::DeleteBlock { block_id: block_id.clone() }])
    },
  )
  .await?;
  ws::broadcast_applied_update(&state.hub, &applied, Uuid::nil(), None);

  Ok(Json(DocumentUpdateResponse {
    document: applied.document,
    snapshot: applied.snapshot,
    update: applied.update,
  }))
}

/// `POST /api/workspaces/{ws}/documents/{doc}/rehost-image?block_id=…&file_name=…`
/// — body is the raw image bytes.
///
/// The single write-back endpoint for re-hosting an external image into Mica.
/// The CALLER fetches the bytes (only it can — a CN-hosted server 403s hosts
/// like AppFlowy that block its datacenter IP) and POSTs them here; we store
/// them (sha256-dedup, like any upload) and point the image block at the new
/// `file_id` with ONE targeted `UpdateBlock` op — never a whole-doc rewrite, so
/// unsupported/foreign blocks elsewhere in the doc are left untouched. Shared by
/// the `mica-cli rehost-images` sweep and any future client "re-host all" action.
pub async fn rehost_image(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
  Query(params): Query<RehostImageParams>,
  body: Bytes,
) -> ApiResult<Json<DocumentUpdateResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  ensure_document_in_workspace(&state.db, workspace_id, document_id).await?;

  let client = reqwest::Client::new();
  let file = crate::routes::files::store_bytes(
    &state,
    &client,
    workspace_id,
    user_id,
    &params.file_name,
    &body,
  )
  .await?;
  let file_id = file.id.to_string();
  let name = file.original_name.clone();
  let block_id = params.block_id;

  let applied = store::apply_derived_operations(
    &state.db,
    workspace_id,
    document_id,
    user_id,
    None,
    |payload| {
      // Only touch the named block, and only if it is still an image — never
      // clobber a different block that a concurrent edit may have put here.
      let block = payload
        .blocks
        .iter()
        .find(|b| b.id == block_id)
        .ok_or_else(|| format!("block {block_id} not found"))?;
      if block.kind != "image" {
        return Err(format!("block {block_id} is not an image"));
      }
      Ok(vec![DocumentOperation::UpdateBlock {
        block_id: block_id.clone(),
        kind: None,
        text: None,
        data: Some(serde_json::json!({ "file_id": file_id.clone(), "name": name.clone() })),
      }])
    },
  )
  .await?;
  ws::broadcast_applied_update(&state.hub, &applied, Uuid::nil(), None);

  Ok(Json(DocumentUpdateResponse {
    document: applied.document,
    snapshot: applied.snapshot,
    update: applied.update,
  }))
}

/// Pure: derive the block ops that write [markdown] into a doc whose current
/// state is [current]. `replace_all` first deletes the root's top-level children
/// (delete cascades their subtrees); then the parsed markdown tree is grafted
/// under the root — each block inserted with its `children` stripped and re-linked
/// via `parent_id`, in pre-order so parents exist first. Testable without a DB.
/// [workspace_id] scopes the blob hrefs this may rewire back into file
/// references — see [rewire_blob_hrefs].
fn markdown_update_ops(
  current: &mica_app_core::documents::DocumentSnapshotPayload,
  request: &UpdateMarkdownRequest,
  workspace_id: Uuid,
) -> Result<Vec<DocumentOperation>, String> {
  let root_id = current.root_block_id.as_str();

  // delete removes an existing block — no markdown parse/graft either.
  if matches!(request.mode, MarkdownUpdateMode::Delete) {
    let anchor = request
      .anchor
      .as_deref()
      .filter(|s| !s.is_empty())
      .ok_or("delete requires an `anchor` block id")?;
    if anchor == root_id {
      return Err("delete cannot remove the document root".to_string());
    }
    if !current.blocks.iter().any(|b| b.id == anchor) {
      return Err(format!("no block {anchor:?} in this document"));
    }
    return Ok(vec![DocumentOperation::DeleteBlock {
      block_id: anchor.to_string(),
    }]);
  }

  // find_replace edits existing block text in place — no markdown parse/graft.
  if matches!(request.mode, MarkdownUpdateMode::FindReplace) {
    let find = request
      .find
      .as_deref()
      .filter(|s| !s.is_empty())
      .ok_or("find_replace requires a non-empty `find`")?;
    let replace = request.replace.as_deref().unwrap_or("");
    let mut ops = Vec::new();
    let mut skipped_formatted = false;
    for block in &current.blocks {
      if block.id == root_id || !block.text.contains(find) {
        continue;
      }
      // A block's inline marks are UTF-16 offset ranges into its text. A blind
      // text replace would leave those offsets pointing at the wrong characters
      // — silently mangling bold/italic/link/math. Never touch a marked block;
      // steer the caller to replace_all/insert_at for formatted content.
      let has_marks = block
        .data
        .get("marks")
        .and_then(|m| m.as_array())
        .is_some_and(|a| !a.is_empty());
      if has_marks {
        skipped_formatted = true;
        continue;
      }
      ops.push(DocumentOperation::UpdateBlock {
        block_id: block.id.clone(),
        kind: None,
        text: Some(block.text.replace(find, replace)),
        data: None,
      });
    }
    if ops.is_empty() {
      return Err(if skipped_formatted {
        format!("{find:?} appears only in formatted text; use replace_all or insert_at instead")
      } else {
        format!("no block text contains {find:?}")
      });
    }
    return Ok(ops);
  }

  // Parse the incoming markdown up front so an empty body is rejected BEFORE any
  // destructive delete (a replace_all with empty markdown must not wipe the doc).
  let tmp_root = format!("block_{}", Uuid::new_v4().simple());
  // FRAGMENT parsing for the modes that graft into an existing document, and
  // whole-document parsing only for the one that IS the whole document.
  //
  // Every mode used to call `import_markdown`, which strips YAML front matter
  // off the front of what it is handed. For a fragment that is a position
  // error: appending `---\nbody\n---\ntail` had `body` lifted out as YAML and
  // dropped on `tmp_root` (thrown away below), so the caller silently got only
  // `tail`. A leading `---` in an appended chunk is a thematic break — that is
  // what the format says and what the caller meant.
  let mut parsed = match request.mode {
    MarkdownUpdateMode::ReplaceAll => import_markdown(&request.markdown, &tmp_root),
    _ => import_markdown_fragment(&request.markdown, &tmp_root),
  };
  rewire_blob_hrefs(&mut parsed.blocks, workspace_id);
  let has_content = parsed
    .blocks
    .iter()
    .find(|b| b.id == tmp_root)
    .is_some_and(|r| !r.children.is_empty());

  let mut ops = Vec::new();
  // Where the new content grafts under the root: append (None), replace_all
  // (None, after wiping), or insert_at (right after the anchor).
  let start_index = match request.mode {
    MarkdownUpdateMode::ReplaceAll => {
      if !has_content {
        return Err(
          "replace_all needs markdown content — refusing to wipe the document".to_string(),
        );
      }
      if let Some(root) = current.blocks.iter().find(|b| b.id == root_id) {
        for child in &root.children {
          ops.push(DocumentOperation::DeleteBlock {
            block_id: child.clone(),
          });
        }
      }
      // Front matter belongs to the DOCUMENT, not to any block, so grafting the
      // parsed children leaves it behind on `tmp_root` — where it used to be
      // thrown away. Exporting a page with properties and replace_all-ing it
      // straight back therefore lost them, silently: a round-trip the dialect
      // is supposed to hold (CLAUDE.md §4). Carry it onto the real root.
      //
      // Only when the incoming markdown HAS front matter: a body-only
      // replace_all must not wipe properties the caller never mentioned.
      // And built from the root's CURRENT data, because UpdateBlock replaces
      // `data` wholesale rather than merging into it.
      if let Some(fm) = parsed
        .blocks
        .iter()
        .find(|b| b.id == tmp_root)
        .and_then(|b| b.data.get("front_matter"))
        .cloned()
      {
        let mut data = current
          .blocks
          .iter()
          .find(|b| b.id == root_id)
          .map(|b| b.data.clone())
          .unwrap_or(serde_json::Value::Null);
        if !data.is_object() {
          data = serde_json::Value::Object(serde_json::Map::new());
        }
        if let Some(map) = data.as_object_mut() {
          map.insert("front_matter".to_string(), fm);
        }
        ops.push(DocumentOperation::UpdateBlock {
          block_id: root_id.to_string(),
          kind: None,
          text: None,
          data: Some(data),
        });
      }
      None
    }
    MarkdownUpdateMode::Append => None,
    MarkdownUpdateMode::InsertAt => {
      let anchor = request
        .anchor
        .as_deref()
        .ok_or("insert_at requires an `anchor` block id")?;
      let root = current
        .blocks
        .iter()
        .find(|b| b.id == root_id)
        .ok_or("document has no root block")?;
      let pos = root
        .children
        .iter()
        .position(|c| c == anchor)
        .ok_or_else(|| format!("anchor {anchor:?} is not a top-level block"))?;
      Some(pos + 1)
    }
    MarkdownUpdateMode::FindReplace | MarkdownUpdateMode::Delete => {
      unreachable!("handled above")
    }
  };

  let by_id: std::collections::HashMap<&str, &mica_app_core::documents::Block> =
    parsed.blocks.iter().map(|b| (b.id.as_str(), b)).collect();
  graft_ops(&tmp_root, root_id, start_index, &by_id, &mut ops);
  Ok(ops)
}

/// Emit InsertBlock ops for every child of [parsed_parent], re-parenting them
/// under [op_parent] (children stripped; re-linked by insertion order). Top-level
/// blocks land at [start_index] (incrementing) so `insert_at` positions after an
/// anchor; `None` appends. Descendants recurse appended under the real block id.
fn graft_ops(
  parsed_parent: &str,
  op_parent: &str,
  start_index: Option<usize>,
  by_id: &std::collections::HashMap<&str, &mica_app_core::documents::Block>,
  ops: &mut Vec<DocumentOperation>,
) {
  let Some(parent) = by_id.get(parsed_parent) else {
    return;
  };
  let mut index = start_index;
  for child_id in &parent.children {
    let Some(child) = by_id.get(child_id.as_str()) else {
      continue;
    };
    let mut block = (*child).clone();
    block.children = Vec::new();
    ops.push(DocumentOperation::InsertBlock {
      block,
      parent_id: op_parent.to_string(),
      index,
    });
    graft_ops(&child.id, &child.id, None, by_id, ops);
    index = index.map(|i| i + 1);
  }
}

/// `GET /api/workspaces/{workspace_id}/documents/{document_id}/export.zip`
///
/// A portable ZIP: `document.md` with Mica images rewritten to `assets/<name>`
/// plus the image bytes under `assets/` (external image links are kept as-is).
pub async fn export_document_zip(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Response> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;
  ensure_document_in_workspace(&state.db, workspace_id, document_id).await?;

  let payload = store::current_payload(&state.db, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;

  // The page's name names the FILE, and since 2026-08-27 also leads the TEXT
  // (`docs/page-title-plan.md`). Hardcoding `document.md` had silently thrown
  // the name away: you got a zip of "document.md" whichever page you exported,
  // and an import could only ever call it "document".
  let view_name = fetch_document_view(&state.db, workspace_id, document_id)
    .await?
    .map(|view| view.name);
  let base = view_name
    .as_deref()
    .map(safe_segment)
    .unwrap_or_else(|| "document".to_string());

  let mut entries = Vec::new();
  let assets = collect_assets(&state, workspace_id, &payload.blocks, &mut entries).await?;
  let body = export_markdown_with_assets(&payload, &assets)
    .map_err(|error| ApiError::BadRequest(error.to_string()))?;
  // Same rule as the tree export: the document's own title when it has one, the
  // view name otherwise, placed after any front matter.
  let title = document_title(&payload)
    .or(view_name.as_deref())
    .unwrap_or_default();
  entries.insert(
    0,
    ZipEntry {
      name: format!("{base}.md"),
      data: with_page_title(&body, title).into_bytes(),
    },
  );

  Ok(zip_response(build_zip(&entries), &format!("{base}.zip")))
}

/// Gather image assets referenced by [blocks]: fetch each Mica image's bytes,
/// push them into [entries] under `assets/`, and return a `file_id → assets/path`
/// map for the Markdown rewrite. External (url) images are left untouched.
async fn collect_assets(
  state: &AppState,
  workspace_id: Uuid,
  blocks: &[mica_app_core::documents::Block],
  entries: &mut Vec<ZipEntry>,
) -> ApiResult<BTreeMap<String, String>> {
  // file_id -> original name, in document order.
  let mut wanted: Vec<(String, String)> = Vec::new();
  for block in blocks {
    if block.kind != "image" {
      continue;
    }
    let file_id = block.data.get("file_id").and_then(|v| v.as_str());
    if let Some(id) = file_id {
      if !wanted.iter().any(|(w, _)| w == id) {
        let name = block
          .data
          .get("name")
          .and_then(|v| v.as_str())
          .unwrap_or("image")
          .to_string();
        wanted.push((id.to_string(), name));
      }
    }
  }
  if wanted.is_empty() {
    return Ok(BTreeMap::new());
  }

  let storage = state
    .storage
    .clone()
    .ok_or_else(|| ApiError::Unavailable("file storage is not configured".to_string()))?;
  let ids: Vec<Uuid> = wanted
    .iter()
    .filter_map(|(id, _)| Uuid::parse_str(id).ok())
    .collect();
  let records = store::fetch_files(&state.db, workspace_id, &ids).await?;
  let by_id: std::collections::HashMap<String, &store::FileRecord> =
    records.iter().map(|r| (r.id.to_string(), r)).collect();

  let client = reqwest::Client::new();
  let mut map = BTreeMap::new();
  let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
  for (file_id, name) in wanted {
    let Some(record) = by_id.get(&file_id) else {
      continue;
    };
    let bytes = match client
      .get(storage.download_url(&record.object_key))
      .send()
      .await
    {
      Ok(resp) if resp.status().is_success() => match resp.bytes().await {
        Ok(b) => b.to_vec(),
        Err(_) => continue,
      },
      _ => continue,
    };
    let asset = unique_asset_name(&name, &mut used);
    entries.push(ZipEntry {
      name: format!("assets/{asset}"),
      data: bytes,
    });
    map.insert(file_id, format!("assets/{asset}"));
  }
  Ok(map)
}

/// Make a unique `assets/` filename, appending `-1`, `-2`… on collision.
fn unique_asset_name(name: &str, used: &mut std::collections::HashSet<String>) -> String {
  if used.insert(name.to_string()) {
    return name.to_string();
  }
  let (stem, ext) = match name.rsplit_once('.') {
    Some((s, e)) => (s.to_string(), format!(".{e}")),
    None => (name.to_string(), String::new()),
  };
  let mut n = 1;
  loop {
    let candidate = format!("{stem}-{n}{ext}");
    if used.insert(candidate.clone()) {
      return candidate;
    }
    n += 1;
  }
}

/// `GET /api/workspaces/{workspace_id}/export.zip`
///
/// The whole workspace as a Markdown ZIP, organised by the page tree: each page
/// is `<ancestors…>/<page>.md`, a page with children also names a folder, and
/// images are de-duplicated under a root `assets/` folder (referenced with the
/// right relative `../` depth).
pub async fn export_workspace_zip(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
) -> ApiResult<Response> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;
  let entries = build_tree_zip(&state, workspace_id, None).await?;
  Ok(zip_response(build_zip(&entries), "workspace.zip"))
}

/// `GET /api/workspaces/export.zip` — EVERY workspace the user belongs to, in
/// switcher (position) order, each under its own `<name>/` subdir, plus a
/// top-level `workspaces.json` manifest. Each subdir is byte-identical to that
/// workspace's own `export.zip`, so re-importing a subdir round-trips.
/// What "export everything" is about to hand you.
///
/// Three separate numbers rather than one total size, because only one of them
/// can be stated honestly. The archive is a zip: Markdown compresses hard and
/// images do not, so a single "约 X MB" derived from raw bytes would be wrong by
/// a lot for a text-heavy account and roughly right for an image-heavy one — a
/// number whose error depends on the user's content is worse than no number.
/// Images dominate real archives and their bytes are exact, so that is the one
/// size reported, labelled as images rather than as the download.
#[derive(Debug, Serialize)]
pub struct ExportStatsResponse {
  workspaces: i64,
  pages: i64,
  image_bytes: i64,
}

/// Counts for the whole-account export, over exactly the workspaces
/// [`export_all_workspaces_zip`] would walk.
pub async fn export_all_stats(
  State(state): State<AppState>,
  headers: HeaderMap,
) -> ApiResult<Json<ExportStatsResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;

  // One statement over the same membership join the export uses, so the numbers
  // describe the archive that would actually be produced. Pages match the
  // `page_count` definition (no folders, no trash); image bytes count only blobs
  // still referenced by a live document — an unreferenced one is waiting for the
  // GC sweep and will not be in the archive.
  let row = sqlx::query_as::<_, (i64, i64, i64)>(
    r#"
      WITH mine AS (
        SELECT w.id
        FROM workspaces w
        INNER JOIN workspace_members wm ON wm.workspace_id = w.id
        WHERE wm.user_id = $1
      )
      SELECT
        (SELECT count(*) FROM mine)::bigint,
        (
          SELECT count(*)
          FROM views v
          WHERE v.workspace_id IN (SELECT id FROM mine)
            AND v.is_deleted = false
            AND v.object_type::text = 'document'
        )::bigint,
        (
          SELECT coalesce(sum(f.byte_size), 0)
          FROM files f
          WHERE f.workspace_id IN (SELECT id FROM mine)
            AND f.unreferenced_since IS NULL
        )::bigint
    "#,
  )
  .bind(user_id)
  .fetch_one(&state.db)
  .await?;

  Ok(Json(ExportStatsResponse {
    workspaces: row.0,
    pages: row.1,
    image_bytes: row.2,
  }))
}

pub async fn export_all_workspaces_zip(
  State(state): State<AppState>,
  headers: HeaderMap,
) -> ApiResult<Response> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  let workspaces: Vec<(Uuid, String)> = sqlx::query_as(
    r#"
      SELECT w.id, w.name
      FROM workspaces w
      INNER JOIN workspace_members wm ON wm.workspace_id = w.id
      WHERE wm.user_id = $1
      ORDER BY wm.position ASC, w.created_at ASC
    "#,
  )
  .bind(user_id)
  .fetch_all(&state.db)
  .await?;

  let mut entries: Vec<ZipEntry> = Vec::new();
  let mut manifest_ws: Vec<serde_json::Value> = Vec::new();
  let mut used_dirs: std::collections::HashSet<String> = std::collections::HashSet::new();
  for (ws_id, ws_name) in &workspaces {
    // Unique subdir per workspace (two "test" workspaces must not collide).
    let base = zip_safe_name(ws_name, "workspace");
    let mut dir = base.clone();
    let mut n = 2;
    // Dedup case-insensitively so "Test" and "test" don't both unzip into the
    // same folder on a case-insensitive filesystem (Windows / default macOS).
    while !used_dirs.insert(dir.to_lowercase()) {
      dir = format!("{base} ({n})");
      n += 1;
    }
    for e in build_tree_zip(&state, *ws_id, None).await? {
      entries.push(ZipEntry {
        name: format!("{dir}/{}", e.name),
        data: e.data,
      });
    }
    manifest_ws.push(serde_json::json!({ "name": ws_name, "dir": dir }));
  }
  let manifest = serde_json::json!({
    "version": 1,
    "generator": "mica",
    "kind": "workspaces",
    "workspaces": manifest_ws,
  });
  entries.push(ZipEntry {
    name: "workspaces.json".to_string(),
    data: serde_json::to_vec_pretty(&manifest).unwrap_or_default(),
  });
  Ok(zip_response(build_zip(&entries), "mica-workspaces.zip"))
}

/// `GET /api/workspaces/{workspace_id}/views/{view_id}/export.zip`
///
/// One folder's subtree as an archive — same shape as the workspace export
/// (paths relative to the folder, shared `assets/`, `manifest.json`), so it
/// imports back the same way. Every export level is a zip on purpose: a bare
/// `.md` cannot carry the images, and used to emit `![](photo.png)` pointing at
/// a file that was nowhere in the download.
pub async fn export_folder_zip(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, view_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Response> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;

  let views = fetch_workspace_views(&state.db, workspace_id).await?;
  let folder = views
    .iter()
    .find(|v| v.id == view_id)
    .ok_or(ApiError::NotFound)?;
  if folder.object_type != "folder" {
    return Err(ApiError::BadRequest(
      "only a folder can be exported this way".to_string(),
    ));
  }
  let filename = format!("{}.zip", zip_safe_name(&folder.name, "folder"));
  let entries = build_tree_zip(&state, workspace_id, Some(view_id)).await?;
  Ok(zip_response(build_zip(&entries), &filename))
}

/// A filename-safe rendition of a user-authored name, for the download's
/// `Content-Disposition` (path separators / control chars must not escape it).
fn zip_safe_name(name: &str, fallback: &str) -> String {
  let cleaned: String = name
    .chars()
    .map(|c| {
      if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
        '-'
      } else {
        c
      }
    })
    .collect();
  let cleaned = cleaned.trim().trim_matches('.').trim();
  if cleaned.is_empty() {
    fallback.to_string()
  } else {
    cleaned.chars().take(80).collect()
  }
}

/// Build the archive entries for a subtree: [root] `None` = the whole
/// workspace, `Some(view_id)` = that folder's children. Paths are relative to
/// the root, image assets are de-duplicated into a shared `assets/`, and a
/// `manifest.json` records the tree for round-tripping back through import.
async fn build_tree_zip(
  state: &AppState,
  workspace_id: Uuid,
  root: Option<Uuid>,
) -> ApiResult<Vec<ZipEntry>> {
  let views = fetch_workspace_views(&state.db, workspace_id).await?;
  // Store-neutral tree for the shared builder (mica_interchange::export_tree) —
  // the SAME walk the local (SQLite) export feeds, so cloud + local produce
  // identically-structured archives and export→import stays one round-trip
  // invariant (see export_tree.rs on the cosmetic asset-name tiebreak).
  let nodes: Vec<mica_interchange::TreeNode> = views
    .iter()
    .map(|v| mica_interchange::TreeNode {
      id: v.id.to_string(),
      parent_id: v.parent_view_id.map(|p| p.to_string()),
      position: v.position.clone(),
      name: v.name.clone(),
      object_type: v.object_type.clone(),
      object_id: v.object_id.to_string(),
      icon: v.icon.clone(),
    })
    .collect();

  // Each document's current payload, keyed by object_id.
  let mut payloads: std::collections::HashMap<String, DocumentSnapshotPayload> =
    std::collections::HashMap::new();
  for v in &views {
    if v.object_type != "document" {
      continue;
    }
    if let Some(p) = store::current_payload(&state.db, v.object_id).await? {
      payloads.insert(v.object_id.to_string(), p);
    }
  }

  // Referenced image blobs: file_id -> (first-seen block name), then fetch the
  // records and download the bytes. Dedup key = the storage object key, so two
  // file_ids of the same blob share one `assets/` entry (as the old walk did).
  let mut file_name: BTreeMap<String, String> = BTreeMap::new();
  for v in &views {
    if v.object_type != "document" {
      continue;
    }
    let Some(payload) = payloads.get(&v.object_id.to_string()) else {
      continue;
    };
    for b in &payload.blocks {
      if b.kind != "image" {
        continue;
      }
      if let Some(id) = b.data.get("file_id").and_then(|x| x.as_str()) {
        file_name.entry(id.to_string()).or_insert_with(|| {
          b.data
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or("image")
            .to_string()
        });
      }
    }
  }
  let mut images: std::collections::HashMap<String, mica_interchange::ImageAsset> =
    std::collections::HashMap::new();
  if let Some(storage) = state.storage.clone() {
    if !file_name.is_empty() {
      let ids: Vec<Uuid> = file_name
        .keys()
        .filter_map(|id| Uuid::parse_str(id).ok())
        .collect();
      let records = store::fetch_files(&state.db, workspace_id, &ids).await?;
      let by_id: std::collections::HashMap<String, &store::FileRecord> =
        records.iter().map(|r| (r.id.to_string(), r)).collect();
      let client = reqwest::Client::new();
      for (file_id, name) in &file_name {
        let Some(record) = by_id.get(file_id) else {
          continue;
        };
        let bytes = match client
          .get(storage.download_url(&record.object_key))
          .send()
          .await
        {
          Ok(resp) if resp.status().is_success() => match resp.bytes().await {
            Ok(b) => b.to_vec(),
            Err(_) => continue,
          },
          _ => continue,
        };
        images.insert(
          file_id.clone(),
          mica_interchange::ImageAsset {
            name: name.clone(),
            bytes,
            dedup_key: record.object_key.clone(),
          },
        );
      }
    }
  }

  let root_str = root.map(|r| r.to_string());
  Ok(mica_interchange::build_markdown_tree_zip(
    &nodes,
    root_str.as_deref(),
    &payloads,
    &images,
  ))
}

/// The canonical, workspace-scoped path that serves one blob's bytes.
///
/// The trailing name is cosmetic — `files::blob_named` ignores it — but it
/// keeps the exported Markdown readable and survives a re-import (see
/// [parse_blob_href]).
fn blob_href(workspace_id: Uuid, file_id: &str, name: &str) -> String {
  format!(
    "/api/workspaces/{workspace_id}/files/{file_id}/blob/{}",
    safe_segment(name)
  )
}

/// `file_id -> a path that actually serves the bytes`, for the Markdown exports
/// that ship no bytes of their own.
///
/// With no map an uploaded image degrades to its ORIGINAL FILENAME, and every
/// client names a pasted image `pasted-image.png` — so a reader got
/// `![](pasted-image.png)` for every image in the workspace: unresolvable, and
/// not even distinguishable from one another. The file_id is already in the
/// block; spending it on a href costs no query and makes the export fetchable.
/// The ZIP exports keep their own `assets/` map — bytes travel with those.
fn blob_asset_map(
  blocks: &[mica_app_core::documents::Block],
  workspace_id: Uuid,
) -> BTreeMap<String, String> {
  let mut map = BTreeMap::new();
  for block in blocks {
    if block.kind != "image" {
      continue;
    }
    let Some(file_id) = block.data.get("file_id").and_then(|v| v.as_str()) else {
      continue;
    };
    let name = block
      .data
      .get("name")
      .and_then(|v| v.as_str())
      .unwrap_or("image");
    map.insert(file_id.to_string(), blob_href(workspace_id, file_id, name));
  }
  map
}

/// Recognise one of our own blob hrefs and recover `(file_id, name)`.
///
/// Only paths for THIS workspace resolve. A href aimed at another workspace's
/// blob is not ours to claim: blob GC recomputes each workspace's reference set
/// from its OWN views, so a cross-workspace reference is invisible to the GC
/// that owns the bytes — it would collect them and break this page. Left as a
/// plain link, it stays exactly as honest as it is: a link to someone else's
/// file.
fn parse_blob_href(href: &str, workspace_id: Uuid) -> Option<(String, String)> {
  let path = href.split(['?', '#']).next()?;
  let at = path.find("/api/workspaces/")?;
  let rest = &path[at + "/api/workspaces/".len()..];
  let (ws, rest) = rest.split_once('/')?;
  if ws != workspace_id.to_string() {
    return None;
  }
  let rest = rest.strip_prefix("files/")?;
  let (file_id, rest) = rest.split_once('/')?;
  Uuid::parse_str(file_id).ok()?;
  let name = match rest {
    "blob" => String::new(),
    other => percent_decode(other.strip_prefix("blob/")?),
  };
  let name = if name.is_empty() {
    "image".to_string()
  } else {
    name
  };
  Some((file_id.to_string(), name))
}

/// Turn `![](…/files/{id}/blob/…)` back into Mica's `{file_id, name}` form.
///
/// Symmetric with [blob_asset_map]. Without it a Markdown round-trip would
/// quietly downgrade every uploaded image into an external link pointing at
/// itself — still rendering, but no longer a reference, so blob GC would stop
/// counting it and eventually delete the bytes out from under the page.
fn rewire_blob_hrefs(blocks: &mut [mica_app_core::documents::Block], workspace_id: Uuid) {
  for block in blocks {
    if block.kind != "image" {
      continue;
    }
    let Some(url) = block.data.get("url").and_then(|v| v.as_str()) else {
      continue;
    };
    if let Some((file_id, name)) = parse_blob_href(url, workspace_id) {
      block.data = serde_json::json!({"file_id": file_id, "name": name});
    }
  }
}

/// Decode `%XX` escapes back to a UTF-8 string; leave malformed escapes alone.
/// In-house rather than a dependency: this is the only percent-decode in the
/// server, and it is a dozen lines.
fn percent_decode(input: &str) -> String {
  let bytes = input.as_bytes();
  let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
  let mut i = 0;
  while i < bytes.len() {
    if bytes[i] == b'%'
      && i + 2 < bytes.len()
      && let Ok(byte) = u8::from_str_radix(&input[i + 1..i + 3], 16)
    {
      out.push(byte);
      i += 3;
    } else {
      out.push(bytes[i]);
      i += 1;
    }
  }
  String::from_utf8_lossy(&out).into_owned()
}

/// A path segment safe for a filename: keep letters/digits of any script plus
/// `-_.`, collapse other runs to `_`; never empty.
fn safe_segment(name: &str) -> String {
  let mut out = String::new();
  let mut prev_us = false;
  for ch in name.chars() {
    if ch.is_alphanumeric() || matches!(ch, '-' | '_' | '.') {
      out.push(ch);
      prev_us = ch == '_';
    } else if !prev_us {
      out.push('_');
      prev_us = true;
    }
  }
  let tidy = out.trim_matches('_').to_string();
  if tidy.is_empty() {
    "untitled".to_string()
  } else {
    tidy
  }
}

fn zip_response(bytes: Vec<u8>, filename: &str) -> Response {
  (
    [
      (header::CONTENT_TYPE, "application/zip".to_string()),
      (
        header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{filename}\""),
      ),
    ],
    bytes,
  )
    .into_response()
}

/// `GET /api/workspaces/{workspace_id}/export/markdown` — the whole workspace as
/// one clean Markdown document, pages in tree order (title heading depth follows
/// the page tree).
pub async fn export_workspace_markdown(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
) -> ApiResult<Json<MarkdownExportResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;
  let markdown = workspace_markdown(&state.db, workspace_id, 1).await?;
  Ok(Json(MarkdownExportResponse { markdown }))
}

/// `GET /api/export/markdown` — every workspace the user belongs to, each as a
/// top-level section, concatenated into one Markdown document.
pub async fn export_all_markdown(
  State(state): State<AppState>,
  headers: HeaderMap,
) -> ApiResult<Json<MarkdownExportResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  let workspaces = sqlx::query_as::<_, (Uuid, String)>(
    r#"
      SELECT w.id, w.name
      FROM workspaces w
      JOIN workspace_members m ON m.workspace_id = w.id
      WHERE m.user_id = $1
      ORDER BY w.created_at ASC
    "#,
  )
  .bind(user_id)
  .fetch_all(&state.db)
  .await?;

  let mut out = String::new();
  for (id, name) in workspaces {
    out.push_str(&format!("# {name}\n\n"));
    let body = workspace_markdown(&state.db, id, 2).await?;
    if !body.is_empty() {
      out.push_str(&body);
      out.push_str("\n\n");
    }
    out.push_str("---\n\n");
  }

  Ok(Json(MarkdownExportResponse {
    markdown: out.trim().to_string(),
  }))
}

/// Render every document page of a workspace into one Markdown string, in
/// page-tree order. `base_level` is the heading level of top-level pages.
async fn workspace_markdown(
  db: &PgPool,
  workspace_id: Uuid,
  base_level: usize,
) -> ApiResult<String> {
  let views = fetch_workspace_views(db, workspace_id).await?;

  let mut by_parent: std::collections::HashMap<Option<Uuid>, Vec<&View>> =
    std::collections::HashMap::new();
  for view in &views {
    by_parent.entry(view.parent_view_id).or_default().push(view);
  }

  let mut ordered: Vec<(&View, usize)> = Vec::new();
  collect_view_order(&by_parent, None, 0, &mut ordered);

  let mut out = String::new();
  for (view, depth) in ordered {
    if view.object_type != "document" {
      continue;
    }
    let level = (base_level + depth).min(6);
    let payload = store::current_payload(db, view.object_id).await?;
    // The DOCUMENT's own title wins over the `views.name` column — the rule every
    // other outlet uses (`docs/page-title-plan.md` §4.1). P1 converted the
    // single-page and ZIP exports and missed this one: it was not in the plan's
    // list of outlets, and a v0.13.30 smoke test through MCP
    // (`mica_export_workspace` calls this) is what turned it up.
    //
    // No user-visible difference TODAY — since P2 the column is a projection
    // written in the same transaction, so the two always agree. The point is to
    // read the authority rather than its shadow: if the projection ever starts
    // disagreeing, that should surface as a bug in the projection instead of
    // quietly becoming the answer here.
    let title = payload
      .as_ref()
      .and_then(mica_markdown::document_title)
      .unwrap_or(view.name.as_str());
    out.push_str(&"#".repeat(level));
    out.push(' ');
    out.push_str(title);
    out.push_str("\n\n");

    if let Some(payload) = payload {
      let assets = blob_asset_map(&payload.blocks, workspace_id);
      // Propagate a page's export failure instead of swallowing it (the old
      // `if let Ok(..)` with no else). This export is a backup/migration; a page
      // that silently reduces to its heading with the body gone is a backup that
      // LOOKS complete and isn't — the incident B shape. Matches the single-doc
      // export path (`export_markdown`) which already `?`s this. (P1-3.)
      let markdown = export_markdown_with_assets(&payload, &assets).map_err(|error| {
        ApiError::BadRequest(format!("export failed for page {}: {error}", view.name))
      })?;
      let body = markdown.trim();
      if !body.is_empty() {
        out.push_str(body);
        out.push_str("\n\n");
      }
    }
  }

  Ok(out.trim().to_string())
}

fn collect_view_order<'a>(
  by_parent: &std::collections::HashMap<Option<Uuid>, Vec<&'a View>>,
  parent: Option<Uuid>,
  depth: usize,
  out: &mut Vec<(&'a View, usize)>,
) {
  if let Some(children) = by_parent.get(&parent) {
    for child in children {
      out.push((child, depth));
      collect_view_order(by_parent, Some(child.id), depth + 1, out);
    }
  }
}

/// `GET /api/workspaces/{workspace_id}/documents/{document_id}/export/html`
///
/// One page as a **self-contained** `.html` file: a full HTML5 document with an
/// embedded stylesheet and every image inlined as a `data:` URI, so it opens
/// offline and survives the source page being deleted. This is why it embeds
/// bytes rather than reusing the share page's public blob URLs — a downloaded
/// file must not depend on the server still being up.
#[derive(Debug, Deserialize)]
pub struct HtmlExportQuery {
  /// The author's editor page width in px, so the export is as wide as the doc
  /// was written (WYSIWYG). Absent → a sensible default.
  width: Option<u32>,
}

pub async fn export_document_html(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
  Query(q): Query<HtmlExportQuery>,
) -> ApiResult<Response> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;
  ensure_document_in_workspace(&state.db, workspace_id, document_id).await?;

  let mut payload = store::current_payload(&state.db, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;
  let title = fetch_document_view(&state.db, workspace_id, document_id)
    .await?
    .map(|view| view.name)
    .unwrap_or_else(|| "document".to_string());

  // Bytes → data: URIs. An image whose bytes can't be fetched simply keeps its
  // existing url (set_image_srcs skips it), so a missing asset degrades to a
  // broken <img>, never a failed export.
  let data_uris = collect_asset_data_uris(&state, workspace_id, &payload.blocks).await?;
  set_image_srcs(&mut payload, &data_uris);

  let html = export_html_document(&payload, &title, q.width.unwrap_or(1160))
    .map_err(|error| ApiError::BadRequest(error.to_string()))?;

  let filename = format!("{}.html", safe_segment(&title));
  Ok(
    (
      [
        (header::CONTENT_TYPE, "text/html; charset=utf-8".to_string()),
        (
          header::CONTENT_DISPOSITION,
          format!("attachment; filename=\"{filename}\""),
        ),
      ],
      html,
    )
      .into_response(),
  )
}

/// Like [collect_assets] but for a single self-contained file: fetch each Mica
/// image's bytes and return a `file_id → data:<mime>;base64,…` map instead of
/// writing files. External (url) images are left for [set_image_srcs] to skip.
async fn collect_asset_data_uris(
  state: &AppState,
  workspace_id: Uuid,
  blocks: &[mica_app_core::documents::Block],
) -> ApiResult<BTreeMap<String, String>> {
  let mut wanted: Vec<String> = Vec::new();
  for block in blocks {
    if block.kind != "image" {
      continue;
    }
    if let Some(id) = block.data.get("file_id").and_then(|v| v.as_str()) {
      if !wanted.iter().any(|w| w == id) {
        wanted.push(id.to_string());
      }
    }
  }
  if wanted.is_empty() {
    return Ok(BTreeMap::new());
  }

  let storage = state
    .storage
    .clone()
    .ok_or_else(|| ApiError::Unavailable("file storage is not configured".to_string()))?;
  let ids: Vec<Uuid> = wanted.iter().filter_map(|id| Uuid::parse_str(id).ok()).collect();
  let records = store::fetch_files(&state.db, workspace_id, &ids).await?;
  let by_id: std::collections::HashMap<String, &store::FileRecord> =
    records.iter().map(|r| (r.id.to_string(), r)).collect();

  let client = reqwest::Client::new();
  let mut map = BTreeMap::new();
  for file_id in wanted {
    let Some(record) = by_id.get(&file_id) else {
      continue;
    };
    let bytes = match client.get(storage.download_url(&record.object_key)).send().await {
      Ok(resp) if resp.status().is_success() => match resp.bytes().await {
        Ok(b) => b.to_vec(),
        Err(_) => continue,
      },
      _ => continue,
    };
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    map.insert(file_id, format!("data:{};base64,{}", record.mime_type, b64));
  }
  Ok(map)
}

/// Point each uploaded-image block's `url` at its public blob path so a renderer
/// that reads `url` (export_html) can show it. Uploaded images store
/// `{file_id, name}` and NO `url`, so without this they render `<img src="">`.
/// External images (already a `url`) are left alone. The blob path is a public
/// capability URL (`is_blob_path`), so it resolves on an unauthenticated share
/// page too.
fn inline_blob_hrefs(blocks: &mut [mica_app_core::documents::Block], workspace_id: Uuid) {
  for block in blocks {
    if block.kind != "image" {
      continue;
    }
    let Some(file_id) = block
      .data
      .get("file_id")
      .and_then(|v| v.as_str())
      .map(str::to_string)
    else {
      continue;
    };
    let name = block
      .data
      .get("name")
      .and_then(|v| v.as_str())
      .unwrap_or("image")
      .to_string();
    if let Some(obj) = block.data.as_object_mut() {
      obj.insert(
        "url".to_string(),
        serde_json::json!(blob_href(workspace_id, &file_id, &name)),
      );
    }
  }
}

// ── Public sharing (publish a page to a /s/{token} URL) ──────────────────────

#[derive(Debug, Serialize)]
pub struct ShareResponse {
  /// Whether the document currently has an active public link.
  shared: bool,
  /// The share token when shared; the client composes `{origin}/s/{token}`.
  #[serde(skip_serializing_if = "Option::is_none")]
  token: Option<String>,
}

/// `GET /api/workspaces/{ws}/documents/{id}/share` — current share status.
pub async fn get_share(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ShareResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;
  ensure_document_in_workspace(&state.db, workspace_id, document_id).await?;
  let share = store::fetch_active_share_for_doc(&state.db, workspace_id, document_id).await?;
  Ok(Json(ShareResponse {
    shared: share.is_some(),
    token: share.map(|s| s.token),
  }))
}

/// `POST …/share` — publish (create-or-return the active share). Idempotent:
/// re-publishing returns the SAME link.
pub async fn create_share(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ShareResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  ensure_document_in_workspace(&state.db, workspace_id, document_id).await?;
  let share = store::create_or_get_share(&state.db, workspace_id, document_id, user_id).await?;
  Ok(Json(ShareResponse {
    shared: true,
    token: Some(share.token),
  }))
}

/// `DELETE …/share` — unpublish (soft-revoke). The public link 404s at once.
pub async fn delete_share(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ShareResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  ensure_document_in_workspace(&state.db, workspace_id, document_id).await?;
  store::revoke_share(&state.db, workspace_id, document_id).await?;
  Ok(Json(ShareResponse {
    shared: false,
    token: None,
  }))
}

/// `GET /s/{token}` — the PUBLIC read-only page. No auth: the unguessable token
/// is the only credential, and it is re-checked (`revoked_at IS NULL`) on every
/// hit so revocation is instant. A missing/revoked/wrong token and a private
/// doc all return the SAME 404 — the page never reveals that a document exists.
/// Renders the server-side HTML (Rust-first data-plane) wrapped in a minimal
/// shell; images resolve through the public blob capability URLs.
pub async fn public_share_page(
  State(state): State<AppState>,
  Path(token): Path<String>,
) -> Response {
  let not_found = || {
    (
      axum::http::StatusCode::NOT_FOUND,
      [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
      "<!doctype html><meta charset=utf-8><title>Not found</title><p>This page is not available.</p>",
    )
      .into_response()
  };

  let Ok(Some(share)) = store::fetch_share_by_token(&state.db, &token).await else {
    return not_found();
  };
  // The share row alone is not enough: a token can outlive its document. If the
  // view was trashed (`is_deleted`) or purged, `fetch_document_view` (which
  // filters `is_deleted = false`) returns `None`, and we must 404 rather than
  // keep serving the still-present `current_payload` of deleted content.
  let Ok(Some(view)) = fetch_document_view(&state.db, share.workspace_id, share.document_id).await
  else {
    return not_found();
  };
  let Ok(Some(mut payload)) = store::current_payload(&state.db, share.document_id).await else {
    return not_found();
  };
  inline_blob_hrefs(&mut payload.blocks, share.workspace_id);
  let Ok(body) = export_html(&payload) else {
    return not_found();
  };
  let title = view.name;

  let html = render_share_shell(&title, &body, share.allow_indexing);
  (
    [
      (header::CONTENT_TYPE, "text/html; charset=utf-8"),
      // The share page is server-rendered static HTML that needs no JS. A
      // strict CSP with no `script-src` falls back to `default-src 'none'`,
      // which neutralizes inline `<script>` AND `on*` event handlers (e.g.
      // `<img onerror>`) that survive the GFM tagfilter in raw-HTML content —
      // the storage-XSS -> token-theft vector. `img-src`/`font-src`/`style-src`
      // match what `render_share_shell` actually uses (inline CSS, blob/data
      // images, system fonts).
      (header::CONTENT_SECURITY_POLICY, SHARE_CSP),
    ],
    html,
  )
    .into_response()
}

/// CSP for the public share page. No `script-src` -> inline scripts and `on*`
/// event handlers are blocked via the `default-src 'none'` fallback.
const SHARE_CSP: &str =
  "default-src 'none'; img-src 'self' data: https:; style-src 'self' 'unsafe-inline'; font-src 'self' data:";

/// Wrap the HTML export fragment in a standalone, readable page. `noindex`
/// unless the share opted into indexing, so a shared page is not silently
/// crawlable.
fn render_share_shell(title: &str, body_html: &str, allow_indexing: bool) -> String {
  let safe_title = escape_html_min(title);
  let robots = if allow_indexing {
    ""
  } else {
    "<meta name=\"robots\" content=\"noindex\">"
  };
  format!(
    "<!doctype html>\n<html lang=\"zh\">\n<head>\n<meta charset=\"utf-8\">\n\
     <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
     {robots}\n<title>{safe_title}</title>\n<style>\n\
     body{{max-width:1160px;margin:2.5rem auto;padding:0 1.5rem;\
     font-family:-apple-system,BlinkMacSystemFont,'Segoe UI','Microsoft YaHei',\
     'PingFang SC',sans-serif;line-height:1.7;color:#1f2328;}}\n\
     img{{max-width:100%;height:auto;border-radius:6px;}}\n\
     pre{{background:#f6f8fa;padding:1rem;border-radius:6px;overflow:auto;}}\n\
     code{{background:#f6f8fa;padding:.15em .35em;border-radius:4px;}}\n\
     pre code{{background:none;padding:0;}}\n\
     blockquote{{margin:0;padding-left:1rem;border-left:3px solid #d0d7de;color:#57606a;}}\n\
     table{{width:100%;border-collapse:collapse;}}\n\
     td,th{{border:1px solid #d0d7de;padding:.4em .6em;}}\n\
     hr{{border:none;border-top:1px solid #d0d7de;margin:2rem 0;}}\n\
     h1{{margin-bottom:1.5rem;}}\n\
     .mica-footer{{margin-top:3rem;padding-top:1rem;border-top:1px solid #eaeef2;\
     color:#8c959f;font-size:.85rem;}}\n</style>\n</head>\n<body>\n\
     <h1>{safe_title}</h1>\n{body_html}\n\
     <div class=\"mica-footer\">用 Mica 制作</div>\n</body>\n</html>\n"
  )
}

/// Minimal HTML-text escaping for the title (the body is escaped by export_html).
fn escape_html_min(s: &str) -> String {
  s.replace('&', "&amp;")
    .replace('<', "&lt;")
    .replace('>', "&gt;")
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
  // Both statements are whole literals rather than fragments assembled at
  // runtime: sqlx only accepts `&'static str`, and the rule is a good one to
  // keep even when every piece would have been a constant. The column list is
  // therefore spelled twice — adding a column means touching both.
  //
  // `$5` (with_stats) gates the join itself, so a plain listing never touches
  // the content table.
  const FLAT: &str = r#"
      SELECT
        v.id, v.workspace_id, v.parent_view_id, v.object_id,
        v.object_type::text AS object_type, v.name, v.icon, v.position,
        v.is_deleted, v.created_by, v.created_at, v.updated_at,
        octet_length(b.state)::bigint AS state_bytes
      FROM views v
      LEFT JOIN document_yrs_base b ON $5 AND b.document_id = v.object_id
      WHERE v.workspace_id = $1 AND v.is_deleted = false
      ORDER BY v.parent_view_id NULLS FIRST, v.position ASC
      LIMIT $3 OFFSET $4
  "#;
  const SUBTREE: &str = r#"
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
      )
      SELECT
        v.id, v.workspace_id, v.parent_view_id, v.object_id,
        v.object_type::text AS object_type, v.name, v.icon, v.position,
        v.is_deleted, v.created_by, v.created_at, v.updated_at,
        octet_length(b.state)::bigint AS state_bytes
      FROM views v
      JOIN subtree ON subtree.id = v.id
      LEFT JOIN document_yrs_base b ON $5 AND b.document_id = v.object_id
      WHERE v.workspace_id = $1 AND v.is_deleted = false
      ORDER BY v.parent_view_id NULLS FIRST, v.position ASC
      LIMIT $3 OFFSET $4
  "#;

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
  sqlx::query_as::<_, View>(
    r#"
      SELECT
        id,
        workspace_id,
        parent_view_id,
        object_id,
        object_type::text AS object_type,
        name,
        icon,
        position,
        is_deleted,
        created_by,
        created_at,
        updated_at
      FROM views
      WHERE workspace_id = $1 AND is_deleted = false
      ORDER BY parent_view_id NULLS FIRST, position ASC
    "#,
  )
  .bind(workspace_id)
  .fetch_all(db)
  .await
  .map_err(ApiError::from)
}

async fn fetch_deleted_workspace_views(db: &PgPool, workspace_id: Uuid) -> ApiResult<Vec<View>> {
  sqlx::query_as::<_, View>(
    r#"
      SELECT
        id,
        workspace_id,
        parent_view_id,
        object_id,
        object_type::text AS object_type,
        name,
        icon,
        position,
        is_deleted,
        created_by,
        created_at,
        updated_at
      FROM views
      WHERE workspace_id = $1 AND is_deleted = true
      ORDER BY updated_at DESC
    "#,
  )
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
  sqlx::query_as::<_, View>(
    r#"
      SELECT
        id,
        workspace_id,
        parent_view_id,
        object_id,
        object_type::text AS object_type,
        name,
        icon,
        position,
        is_deleted,
        created_by,
        created_at,
        updated_at
      FROM views
      WHERE workspace_id = $1 AND object_id = $2 AND object_type = 'document' AND is_deleted = false
      LIMIT 1
    "#,
  )
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

pub(crate) async fn workspace_role(
  db: &PgPool,
  workspace_id: Uuid,
  user_id: Uuid,
) -> ApiResult<Option<String>> {
  sqlx::query_scalar::<_, String>(
    r#"
      SELECT role::text
      FROM workspace_members
      WHERE workspace_id = $1 AND user_id = $2
    "#,
  )
  .bind(workspace_id)
  .bind(user_id)
  .fetch_optional(db)
  .await
  .map_err(ApiError::from)
}

pub(crate) async fn ensure_workspace_member(
  db: &PgPool,
  workspace_id: Uuid,
  user_id: Uuid,
) -> ApiResult<()> {
  workspace_role(db, workspace_id, user_id)
    .await?
    .ok_or(ApiError::NotFound)?;

  Ok(())
}

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

async fn ensure_view_in_workspace(db: &PgPool, workspace_id: Uuid, view_id: Uuid) -> ApiResult<()> {
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
/// caller gets a 400 with a reason instead of tripping the trigger's 500.
pub(crate) async fn ensure_parent_accepts_children(
  db: &PgPool,
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

async fn ensure_valid_parent_view(
  db: &PgPool,
  workspace_id: Uuid,
  view_id: Uuid,
  parent_view_id: Uuid,
) -> ApiResult<()> {
  if parent_view_id == view_id {
    return Err(ApiError::BadRequest(
      "parent_view_id cannot be the same view".to_string(),
    ));
  }

  ensure_parent_accepts_children(db, workspace_id, parent_view_id).await?;

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
      )
      SELECT EXISTS (
        SELECT 1
        FROM descendants
        WHERE id = $3
      )
    "#,
  )
  .bind(view_id)
  .bind(workspace_id)
  .bind(parent_view_id)
  .fetch_one(db)
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
// transaction lets us beat. See docs/cross-workspace-move.md.

#[derive(Debug, Deserialize)]
pub struct TransferRequest {
  dest_workspace_id: Uuid,
  /// Parent folder in the destination (null = destination root).
  #[serde(default)]
  parent_view_id: Option<Uuid>,
  /// true = move (soft-delete the source after copying); false = copy (keep source).
  #[serde(default)]
  remove_source: bool,
  /// Report what WOULD happen (counts + dangling links) without mutating anything.
  #[serde(default)]
  dry_run: bool,
}

/// `POST /api/workspaces/{workspace_id}/views/batch-transfer` — same operation,
/// N roots, ONE transaction.
///
/// This exists because the alternative was N sequential single-view transfers,
/// and that is not the same operation performed more times: a failure part-way
/// through leaves some of the selection moved and the rest not, with no record
/// of where it stopped. Moving five pages is one thing the user asked for, so it
/// either happens or it does not (user, 2026-08-27).
#[derive(Debug, Deserialize)]
pub struct BatchTransferRequest {
  dest_workspace_id: Uuid,
  #[serde(default)]
  parent_view_id: Option<Uuid>,
  #[serde(default)]
  remove_source: bool,
  #[serde(default)]
  dry_run: bool,
  /// The selected roots, in the order the caller selected them. A root nested
  /// inside another is dropped rather than refused (see [`independent_roots`]).
  view_ids: Vec<Uuid>,
}

/// What both transfer routes reduce to: N roots going to one destination.
struct TransferPlan {
  roots: Vec<Uuid>,
  dest_workspace_id: Uuid,
  parent_view_id: Option<Uuid>,
  remove_source: bool,
  dry_run: bool,
}

#[derive(Debug, Serialize)]
pub struct DanglingLink {
  /// Name of the moved document whose link now dangles.
  document: String,
  /// The `mica://page/<id>` target that stays in the source workspace.
  target_view_id: Uuid,
}

#[derive(Debug, Serialize)]
pub struct TransferResponse {
  /// The first root's new id. Kept for the single-view route, whose callers
  /// (Dart client, CLI, MCP) read exactly this field.
  new_root_view_id: Option<Uuid>,
  /// Every root's new id, in the order the roots were given. Empty on a dry run.
  new_root_view_ids: Vec<Uuid>,
  documents: usize,
  folders: usize,
  images: usize,
  dangling_links: Vec<DanglingLink>,
  removed_source: bool,
  dry_run: bool,
}

#[derive(Debug, FromRow)]
struct TransferRow {
  id: Uuid,
  parent_view_id: Option<Uuid>,
  object_id: Uuid,
  object_type: String,
  name: String,
  position: String,
}

/// The roots of a batch transfer, with anything already inside another root
/// dropped.
///
/// A multi-select can hold a folder AND a page inside it — nothing stops the
/// user selecting both, and from the sidebar it looks entirely reasonable.
/// Transferring both would copy that page TWICE: once as part of the folder's
/// subtree, once on its own. The destination would show two of it, and on a move
/// the second copy's source is already soft-deleted by the first.
///
/// [ancestor_pairs] is `(root, one of its ancestors)`, so a root is redundant
/// exactly when one of its ancestors is also being transferred. Kept as pairs
/// rather than a map so the rule is a list operation and the test can state the
/// tree literally.
///
/// Input order is preserved: it is the order the user selected in, and the
/// destination's sibling order follows it.
fn independent_roots(roots: &[Uuid], ancestor_pairs: &[(Uuid, Uuid)]) -> Vec<Uuid> {
  let wanted: std::collections::HashSet<Uuid> = roots.iter().copied().collect();
  let mut kept = Vec::new();
  let mut seen = std::collections::HashSet::new();
  for &root in roots {
    // The same id twice is not an error worth refusing over — it is a click
    // that landed twice — but it must not become two copies either.
    if !seen.insert(root) {
      continue;
    }
    let nested = ancestor_pairs
      .iter()
      .any(|&(child, ancestor)| child == root && wanted.contains(&ancestor));
    if !nested {
      kept.push(root);
    }
  }
  kept
}

/// `(root, ancestor)` for every live ancestor of every root, within
/// [workspace_id] — the input [`independent_roots`] needs.
///
/// Walks UP rather than down: a batch's roots are few and shallow, while their
/// subtrees can be the whole workspace. Enumerating downward to answer "is one
/// of these inside another" would read every page to decide something about a
/// handful of them.
///
/// Split out of the handler so a DB test can reach it — the handler needs auth
/// headers, which a database-level test has no way to construct. Same reason
/// `empty_workspace_trash` and `purge_views_batch` live outside theirs.
///
/// Scoped to live rows in this workspace: an id the caller cannot actually
/// transfer contributes no pairs, is therefore kept as "independent", and is
/// then rejected by the caller's membership check with a real error — rather
/// than quietly disqualifying one of its siblings.
async fn ancestor_pairs_of_roots(
  db: &PgPool,
  workspace_id: Uuid,
  roots: &[Uuid],
) -> ApiResult<Vec<(Uuid, Uuid)>> {
  if roots.is_empty() {
    return Ok(Vec::new());
  }
  Ok(
    sqlx::query_as::<_, (Uuid, Uuid)>(
      r#"
        WITH RECURSIVE up AS (
          SELECT id AS root, parent_view_id AS ancestor
          FROM views
          WHERE id = ANY($1) AND workspace_id = $2 AND is_deleted = false
          UNION ALL
          SELECT u.root, v.parent_view_id
          FROM up u JOIN views v ON v.id = u.ancestor
          WHERE v.is_deleted = false
        )
        SELECT root, ancestor FROM up WHERE ancestor IS NOT NULL
      "#,
    )
    .bind(roots)
    .bind(workspace_id)
    .fetch_all(db)
    .await?,
  )
}

/// `POST /api/workspaces/{workspace_id}/views/{view_id}/transfer`
pub async fn transfer_view(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((src_workspace_id, view_id)): Path<(Uuid, Uuid)>,
  Json(request): Json<TransferRequest>,
) -> ApiResult<Json<TransferResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  transfer_roots(
    &state,
    user_id,
    src_workspace_id,
    TransferPlan {
      roots: vec![view_id],
      dest_workspace_id: request.dest_workspace_id,
      parent_view_id: request.parent_view_id,
      remove_source: request.remove_source,
      dry_run: request.dry_run,
    },
  )
  .await
  .map(Json)
}

/// `POST /api/workspaces/{workspace_id}/views/batch-transfer`
pub async fn batch_transfer_views(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(src_workspace_id): Path<Uuid>,
  Json(request): Json<BatchTransferRequest>,
) -> ApiResult<Json<TransferResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  // Same ceiling as the other batch routes, and for the same reason — except
  // here it also bounds how long ONE transaction holds its locks.
  //
  // Deliberately NOT `check_batch_ids`: that rejects a repeated id, and a repeat
  // here is a click that landed twice on a selection, which `independent_roots`
  // already collapses along with the nesting case it cannot be separated from.
  if request.view_ids.is_empty() {
    return Err(ApiError::BadRequest("view_ids must not be empty".to_string()));
  }
  if request.view_ids.len() > MAX_BATCH_VIEWS {
    return Err(ApiError::BadRequest(format!(
      "at most {MAX_BATCH_VIEWS} view_ids per request, got {}",
      request.view_ids.len()
    )));
  }
  transfer_roots(
    &state,
    user_id,
    src_workspace_id,
    TransferPlan {
      roots: request.view_ids,
      dest_workspace_id: request.dest_workspace_id,
      parent_view_id: request.parent_view_id,
      remove_source: request.remove_source,
      dry_run: request.dry_run,
    },
  )
  .await
  .map(Json)
}

/// Copy N subtrees into another workspace, optionally soft-deleting the sources.
///
/// One root or twenty is the same code path — the phases below were already a
/// loop over rows, and `topo_order_subtree` already anchors every row whose
/// parent is outside the set, so a forest needs no special case.
///
/// What "atomic" does and does not mean here, stated plainly because the gap is
/// real: phase 4 is ONE transaction, so the destination tree and the source
/// soft-delete either both land or neither does — no half-moved selection. The
/// blob copies in phase 3 are deliberately outside it (see the phase comment),
/// so a failure after them can leave orphan BYTES in the destination, which its
/// GC reclaims. That is the same guarantee a single transfer has always had.
async fn transfer_roots(
  state: &AppState,
  user_id: Uuid,
  src_workspace_id: Uuid,
  request: TransferPlan,
) -> ApiResult<TransferResponse> {
  let dest_workspace_id = request.dest_workspace_id;
  if dest_workspace_id == src_workspace_id {
    return Err(ApiError::BadRequest(
      "destination workspace must differ from the source".to_string(),
    ));
  }
  // Editor in BOTH: to remove from the source and to create in the destination.
  ensure_workspace_editor(&state.db, src_workspace_id, user_id).await?;
  ensure_workspace_editor(&state.db, dest_workspace_id, user_id).await?;

  // A destination parent, if given, must be a live folder in the destination.
  if let Some(parent) = request.parent_view_id {
    ensure_parent_accepts_children(&state.db, dest_workspace_id, parent).await?;
  }

  // 0. Drop roots that sit inside another root, BEFORE enumerating anything —
  //    otherwise the `= ANY` walk below visits the overlap twice and the
  //    destination gets two copies of it.
  let ancestor_pairs = ancestor_pairs_of_roots(&state.db, src_workspace_id, &request.roots).await?;
  let roots = independent_roots(&request.roots, &ancestor_pairs);
  let root_ids: std::collections::HashSet<Uuid> = roots.iter().copied().collect();

  // 1. Enumerate the subtrees (live rows only). Disjoint by construction after
  //    step 0, so no row can appear twice.
  let subtree = sqlx::query_as::<_, TransferRow>(
    r#"
      WITH RECURSIVE subtree AS (
        SELECT id, parent_view_id, object_id, object_type::text AS object_type, name, position
        FROM views
        WHERE id = ANY($1) AND workspace_id = $2 AND is_deleted = false
        UNION ALL
        SELECT v.id, v.parent_view_id, v.object_id, v.object_type::text, v.name, v.position
        FROM views v JOIN subtree s ON v.parent_view_id = s.id
        WHERE v.is_deleted = false
      )
      SELECT id, parent_view_id, object_id, object_type, name, position FROM subtree
    "#,
  )
  .bind(&roots)
  .bind(src_workspace_id)
  .fetch_all(&state.db)
  .await?;
  let subtree_view_ids: std::collections::HashSet<Uuid> = subtree.iter().map(|r| r.id).collect();

  // A root that produced no row is deleted, or in another workspace, or made up.
  // Refuse the whole batch rather than transferring the rest: the caller asked
  // for these N, and silently delivering N-1 is the kind of success nobody
  // checks. This also replaces the single route's `ensure_view_in_workspace`.
  if let Some(&missing) = roots.iter().find(|r| !subtree_view_ids.contains(r)) {
    tracing::debug!(%missing, "transfer: root not live in the source workspace");
    return Err(ApiError::NotFound);
  }

  // The whole batch counts as "staying together" for the dangling-link report:
  // a link from one selected page to another is not breakage, it is a link that
  // follows. Before the batch endpoint existed this had to be declared by the
  // caller (`also_moving`), because each request knew only its own subtree.
  let moving_view_ids = &subtree_view_ids;

  // 2. Pre-scan documents: referenced file_ids + cross-workspace dangling links.
  let mut payloads: std::collections::HashMap<Uuid, DocumentSnapshotPayload> =
    std::collections::HashMap::new();
  let mut referenced_files: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
  let mut dangling_links: Vec<DanglingLink> = Vec::new();
  let mut documents = 0usize;
  let mut folders = 0usize;
  for row in &subtree {
    if row.object_type != "document" {
      folders += 1;
      continue;
    }
    documents += 1;
    let Some(payload) = store::current_payload(&state.db, row.object_id).await? else {
      continue;
    };
    for block in &payload.blocks {
      if let Some(fid) = block
        .data
        .get("file_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
      {
        referenced_files.insert(fid);
      }
      for target in page_link_targets(&block.data) {
        if let Ok(tid) = Uuid::parse_str(&target) {
          // `moving_view_ids`, not `subtree_view_ids`: a link into another root
          // of the same batch is not breakage, it is a link that follows.
          if !moving_view_ids.contains(&tid) {
            dangling_links.push(DanglingLink {
              document: row.name.clone(),
              target_view_id: tid,
            });
          }
        }
      }
    }
    payloads.insert(row.object_id, payload);
  }
  let images = referenced_files.len();

  if request.dry_run {
    return Ok(TransferResponse {
      new_root_view_id: None,
      new_root_view_ids: Vec::new(),
      documents,
      folders,
      images,
      dangling_links,
      removed_source: false,
      dry_run: true,
    });
  }

  // 3. Copy blobs into the destination BEFORE the transaction: content-addressed
  //    keys make the PUT idempotent, and a half-failure leaves only orphan bytes
  //    in dest (its GC reclaims them), never a page with broken images.
  let storage = state
    .storage
    .as_ref()
    .ok_or_else(|| ApiError::Internal("file storage is not configured".to_string()))?;
  let http = reqwest::Client::new();
  let mut file_map: std::collections::HashMap<Uuid, Uuid> = std::collections::HashMap::new();
  for &src_file_id in &referenced_files {
    let Some(src_file) = store::fetch_file(&state.db, src_workspace_id, src_file_id).await? else {
      continue; // dangling reference already; nothing to copy
    };
    let suffix = src_file
      .object_key
      .strip_prefix(&format!("workspaces/{src_workspace_id}/"))
      .ok_or_else(|| ApiError::Internal("source object_key not under its workspace".to_string()))?;
    let dest_key = format!("workspaces/{dest_workspace_id}/{suffix}");

    let bytes = http
      .get(storage.download_url(&src_file.object_key))
      .send()
      .await
      .map_err(|e| ApiError::Internal(format!("blob fetch failed: {e}")))?
      .error_for_status()
      .map_err(|e| ApiError::Internal(format!("blob fetch returned {e}")))?
      .bytes()
      .await
      .map_err(|e| ApiError::Internal(format!("blob read failed: {e}")))?;
    let upload = storage.presign_put(&dest_key);
    let put = http
      .put(&upload.url)
      .header(reqwest::header::CONTENT_TYPE, &src_file.mime_type)
      .body(bytes.to_vec())
      .send()
      .await
      .map_err(|e| ApiError::Internal(format!("blob upload failed: {e}")))?;
    if !put.status().is_success() {
      return Err(ApiError::Internal(format!(
        "blob upload returned {}",
        put.status()
      )));
    }

    let dest_file = store::insert_file(
      &state.db,
      dest_workspace_id,
      user_id,
      &dest_key,
      &src_file.original_name,
      &src_file.mime_type,
      src_file.byte_size,
    )
    .await?;
    file_map.insert(src_file_id, dest_file.id);
  }

  // 4. One transaction: build the destination tree (new ids), then soft-delete
  //    the source subtree for a move.
  let view_map: std::collections::HashMap<Uuid, Uuid> =
    subtree.iter().map(|r| (r.id, Uuid::new_v4())).collect();
  let ordered = topo_order_subtree(&subtree);

  // Filled as the subtree is rebuilt; each copied document gets its yrs base
  // built after the commit (see bootstrap_bases_best_effort).
  let mut created_document_ids: Vec<Uuid> = Vec::new();
  let mut tx = state.db.begin().await?;
  for row in &ordered {
    let new_view_id = view_map[&row.id];
    // Every root lands directly under the chosen destination parent; everything
    // below follows its own (already remapped) parent.
    let dest_parent = if root_ids.contains(&row.id) {
      request.parent_view_id
    } else {
      row.parent_view_id.and_then(|p| view_map.get(&p).copied())
    };
    if row.object_type == "document" {
      let mut payload = payloads.remove(&row.object_id).unwrap_or_else(|| {
        // Substituting an empty payload is CORRECT for a genuinely-empty doc
        // (current_payload returns None only when no snapshot row exists). It is
        // NOT silent anymore: the one way this loses content — a doc with a yrs
        // base but no snapshot row — would land here, and a move then deletes the
        // source. Logging makes that edge diagnosable instead of invisible (P0-4).
        tracing::warn!(
          object_id = %row.object_id,
          "transfer/clone: no snapshot for document — substituting empty payload"
        );
        DocumentSnapshotPayload {
          schema_version: 1,
          root_block_id: "root".to_string(),
          blocks: Vec::new(),
        }
      });
      rewrite_transferred_payload(&mut payload, &file_map, &view_map);
      let document = sqlx::query_as::<_, DocumentRecord>(
        r#"
          INSERT INTO documents (workspace_id, root_block_id, created_by)
          VALUES ($1, $2, $3)
          RETURNING id, workspace_id, root_block_id, current_seq, created_by, created_at, updated_at
        "#,
      )
      .bind(dest_workspace_id)
      .bind(&payload.root_block_id)
      .bind(user_id)
      .fetch_one(&mut *tx)
      .await?;
      mica_app_core::sync::seed_base_tx(&mut tx, document.id, payload).await?;
      created_document_ids.push(document.id);
      sqlx::query(
        r#"
          INSERT INTO views (id, workspace_id, parent_view_id, object_id, object_type, name, position, created_by)
          VALUES ($1, $2, $3, $4, 'document', $5, $6, $7)
        "#,
      )
      .bind(new_view_id)
      .bind(dest_workspace_id)
      .bind(dest_parent)
      .bind(document.id)
      .bind(&row.name)
      .bind(&row.position)
      .bind(user_id)
      .execute(&mut *tx)
      .await?;
    } else {
      // Folder: a view with no document. object_id is a fresh unused uuid.
      sqlx::query(
        r#"
          INSERT INTO views (id, workspace_id, parent_view_id, object_id, object_type, name, position, created_by)
          VALUES ($1, $2, $3, $4, 'folder', $5, $6, $7)
        "#,
      )
      .bind(new_view_id)
      .bind(dest_workspace_id)
      .bind(dest_parent)
      .bind(Uuid::new_v4())
      .bind(&row.name)
      .bind(&row.position)
      .bind(user_id)
      .execute(&mut *tx)
      .await?;
    }
  }

  if request.remove_source {
    // Inside the SAME transaction as the destination tree above: that is what
    // makes a move a move rather than a copy followed by a hopeful delete.
    sqlx::query(
      r#"
        WITH RECURSIVE subtree AS (
          SELECT id FROM views WHERE id = ANY($1) AND workspace_id = $2
          UNION ALL
          SELECT v.id FROM views v JOIN subtree s ON v.parent_view_id = s.id
        )
        UPDATE views SET is_deleted = true, updated_at = now()
        WHERE id IN (SELECT id FROM subtree)
      "#,
    )
    .bind(&roots)
    .bind(src_workspace_id)
    .execute(&mut *tx)
    .await?;
  }

  tx.commit().await?;

  let new_root_view_ids: Vec<Uuid> = roots.iter().map(|r| view_map[r]).collect();
  Ok(TransferResponse {
    new_root_view_id: new_root_view_ids.first().copied(),
    new_root_view_ids,
    documents,
    folders,
    images,
    dangling_links,
    removed_source: request.remove_source,
    dry_run: false,
  })
}

// ── Clone (duplicate a view within the same workspace) ───────────────────────
// A same-workspace cousin of transfer: enumerate the subtree, give every node a
// fresh id + doc + snapshot, rewrite in-subtree page links to the new ids. Two
// deliberate differences from transfer:
//   - Blobs are NOT copied. object_key is content-addressed and workspace-scoped
//     (workspaces/{id}/{sha256}.{ext}); within ONE workspace the copy references
//     the same file_id — sharing the bytes is exactly the sha256-dedup intent,
//     and there's nothing to re-upload. So file_map stays empty and file_ids
//     pass through unchanged.
//   - The source is never removed, and the root copy gets a fresh name (deduped
//     among its siblings) + a fresh position so it sits beside the original.

#[derive(Debug, Deserialize)]
pub struct CloneRequest {
  /// Parent for the copy's root (null = the source's own parent, i.e. beside it).
  #[serde(default)]
  parent_view_id: Option<Uuid>,
  /// The copy's root name, locale-aware, computed by the caller (e.g. "X 副本").
  /// Deduped against siblings server-side. Absent → a "{source} 副本" fallback.
  #[serde(default)]
  name: Option<String>,
  /// Report what WOULD happen (counts) without mutating anything.
  #[serde(default)]
  dry_run: bool,
}

#[derive(Debug, Serialize)]
pub struct CloneResponse {
  new_root_view_id: Option<Uuid>,
  new_name: String,
  documents: usize,
  folders: usize,
  dry_run: bool,
}

/// `POST /api/workspaces/{workspace_id}/views/{view_id}/clone`
pub async fn clone_view(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, view_id)): Path<(Uuid, Uuid)>,
  Json(request): Json<CloneRequest>,
) -> ApiResult<Json<CloneResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  ensure_view_in_workspace(&state.db, workspace_id, view_id).await?;

  // 1. Enumerate the subtree (live rows only) — same shape as transfer.
  let subtree = sqlx::query_as::<_, TransferRow>(
    r#"
      WITH RECURSIVE subtree AS (
        SELECT id, parent_view_id, object_id, object_type::text AS object_type, name, position
        FROM views
        WHERE id = $1 AND workspace_id = $2 AND is_deleted = false
        UNION ALL
        SELECT v.id, v.parent_view_id, v.object_id, v.object_type::text, v.name, v.position
        FROM views v JOIN subtree s ON v.parent_view_id = s.id
        WHERE v.is_deleted = false
      )
      SELECT id, parent_view_id, object_id, object_type, name, position FROM subtree
    "#,
  )
  .bind(view_id)
  .bind(workspace_id)
  .fetch_all(&state.db)
  .await?;
  let Some(root) = subtree.iter().find(|r| r.id == view_id) else {
    return Err(ApiError::NotFound);
  };

  // 2. Resolve the copy's parent: an explicit folder in this workspace, or the
  //    source's own parent (beside the original).
  let target_parent = match request.parent_view_id {
    Some(parent) => {
      ensure_parent_accepts_children(&state.db, workspace_id, parent).await?;
      Some(parent)
    }
    None => root.parent_view_id,
  };

  // 3. Name the copy: caller's locale-aware base (fallback "{source} 副本"),
  //    deduped against the live siblings under the target parent.
  let base_name = request
    .name
    .clone()
    .unwrap_or_else(|| format!("{} 副本", root.name));
  let siblings = sqlx::query_scalar::<_, String>(
    "SELECT name FROM views WHERE workspace_id = $1 AND is_deleted = false \
     AND parent_view_id IS NOT DISTINCT FROM $2",
  )
  .bind(workspace_id)
  .bind(target_parent)
  .fetch_all(&state.db)
  .await?;
  let new_name = dedup_sibling_name(&base_name, &siblings);

  // 4. Pre-scan document payloads (needed to rewrite in-subtree links). No blob
  //    copy, no dangling-link scan: every link target stays in this workspace.
  let mut payloads: std::collections::HashMap<Uuid, DocumentSnapshotPayload> =
    std::collections::HashMap::new();
  let mut documents = 0usize;
  let mut folders = 0usize;
  for row in &subtree {
    if row.object_type != "document" {
      folders += 1;
      continue;
    }
    documents += 1;
    if let Some(payload) = store::current_payload(&state.db, row.object_id).await? {
      payloads.insert(row.object_id, payload);
    }
  }

  if request.dry_run {
    return Ok(Json(CloneResponse {
      new_root_view_id: None,
      new_name,
      documents,
      folders,
      dry_run: true,
    }));
  }

  // Filled as the subtree is rebuilt; each copied document gets its yrs base
  // built after the commit (see bootstrap_bases_best_effort).
  let mut created_document_ids: Vec<Uuid> = Vec::new();
  // 5. One transaction: build the copied tree with fresh ids. file_map is empty
  //    (blobs shared), view_map remaps in-subtree links to the new ids.
  let view_map: std::collections::HashMap<Uuid, Uuid> =
    subtree.iter().map(|r| (r.id, Uuid::new_v4())).collect();
  let empty_files: std::collections::HashMap<Uuid, Uuid> = std::collections::HashMap::new();
  let ordered = topo_order_subtree(&subtree);

  let mut tx = state.db.begin().await?;
  for row in &ordered {
    let new_view_id = view_map[&row.id];
    let is_root = row.id == view_id;
    let dest_parent = if is_root {
      target_parent
    } else {
      row.parent_view_id.and_then(|p| view_map.get(&p).copied())
    };
    // The root sits beside the original: fresh name + fresh position so it does
    // not collide with the source under the same parent. Inner nodes keep their
    // name/position — their parent is a new view, so nothing collides there.
    let node_name = if is_root { &new_name } else { &row.name };
    let node_position = if is_root {
      Uuid::now_v7().to_string()
    } else {
      row.position.clone()
    };

    if row.object_type == "document" {
      let mut payload = payloads.remove(&row.object_id).unwrap_or_else(|| {
        // Substituting an empty payload is CORRECT for a genuinely-empty doc
        // (current_payload returns None only when no snapshot row exists). It is
        // NOT silent anymore: the one way this loses content — a doc with a yrs
        // base but no snapshot row — would land here, and a move then deletes the
        // source. Logging makes that edge diagnosable instead of invisible (P0-4).
        tracing::warn!(
          object_id = %row.object_id,
          "transfer/clone: no snapshot for document — substituting empty payload"
        );
        DocumentSnapshotPayload {
          schema_version: 1,
          root_block_id: "root".to_string(),
          blocks: Vec::new(),
        }
      });
      rewrite_transferred_payload(&mut payload, &empty_files, &view_map);
      let document = sqlx::query_as::<_, DocumentRecord>(
        r#"
          INSERT INTO documents (workspace_id, root_block_id, created_by)
          VALUES ($1, $2, $3)
          RETURNING id, workspace_id, root_block_id, current_seq, created_by, created_at, updated_at
        "#,
      )
      .bind(workspace_id)
      .bind(&payload.root_block_id)
      .bind(user_id)
      .fetch_one(&mut *tx)
      .await?;
      mica_app_core::sync::seed_base_tx(&mut tx, document.id, payload).await?;
      created_document_ids.push(document.id);
      sqlx::query(
        r#"
          INSERT INTO views (id, workspace_id, parent_view_id, object_id, object_type, name, position, created_by)
          VALUES ($1, $2, $3, $4, 'document', $5, $6, $7)
        "#,
      )
      .bind(new_view_id)
      .bind(workspace_id)
      .bind(dest_parent)
      .bind(document.id)
      .bind(node_name)
      .bind(&node_position)
      .bind(user_id)
      .execute(&mut *tx)
      .await?;
    } else {
      sqlx::query(
        r#"
          INSERT INTO views (id, workspace_id, parent_view_id, object_id, object_type, name, position, created_by)
          VALUES ($1, $2, $3, $4, 'folder', $5, $6, $7)
        "#,
      )
      .bind(new_view_id)
      .bind(workspace_id)
      .bind(dest_parent)
      .bind(Uuid::new_v4())
      .bind(node_name)
      .bind(&node_position)
      .bind(user_id)
      .execute(&mut *tx)
      .await?;
    }
  }
  tx.commit().await?;

  Ok(Json(CloneResponse {
    new_root_view_id: Some(view_map[&view_id]),
    new_name,
    documents,
    folders,
    dry_run: false,
  }))
}

/// Pick a sibling-unique name: `base` if free, else `base 2`, `base 3`, … The
/// number is locale-neutral, so the caller supplies the localized base ("X 副本"
/// / "X copy") and we only break ties.
fn dedup_sibling_name(base: &str, siblings: &[String]) -> String {
  if !siblings.iter().any(|s| s == base) {
    return base.to_string();
  }
  (2..)
    .map(|n| format!("{base} {n}"))
    .find(|c| !siblings.iter().any(|s| s == c))
    .expect("an unused suffix always exists")
}

/// The `mica://page/<viewId>` targets referenced by a block's link marks.
///
/// Re-exported from app-core, which owns it because the write paths there derive
/// `document_yrs_base.link_targets` with it. A copy here would be a second source
/// of truth for what counts as a page link.
use mica_app_core::sync::page_link_targets;

/// Rewrite a transferred document's blocks for their new home: remap uploaded-
/// image `file_id`s to the destination copies, and remap in-subtree page links to
/// the new view ids. Links to pages left in the source keep their `mica://` href
/// (they dangle — surfaced to the user as a warning before the move).
fn rewrite_transferred_payload(
  payload: &mut DocumentSnapshotPayload,
  file_map: &std::collections::HashMap<Uuid, Uuid>,
  view_map: &std::collections::HashMap<Uuid, Uuid>,
) {
  const SCHEME: &str = "mica://page/";
  for block in &mut payload.blocks {
    let Some(data) = block.data.as_object_mut() else {
      continue;
    };
    // Image file_id → destination copy. Drop any cached blob `url` so the client
    // re-resolves it against the destination workspace.
    if let Some(new_id) = data
      .get("file_id")
      .and_then(|v| v.as_str())
      .and_then(|s| Uuid::parse_str(s).ok())
      .and_then(|old| file_map.get(&old))
    {
      let new_id = *new_id;
      data.insert("file_id".into(), serde_json::json!(new_id.to_string()));
      data.remove("url");
    }
    // In-subtree page links → new view ids.
    if let Some(marks) = data.get_mut("marks").and_then(|m| m.as_array_mut()) {
      for mark in marks {
        let Some(obj) = mark.as_object_mut() else {
          continue;
        };
        let Some(new_href) = obj
          .get("href")
          .and_then(|h| h.as_str())
          .and_then(|href| href.strip_prefix(SCHEME))
          .and_then(|id| Uuid::parse_str(id).ok())
          .and_then(|old| view_map.get(&old))
          .map(|new| format!("{SCHEME}{new}"))
        else {
          continue;
        };
        obj.insert("href".into(), serde_json::json!(new_href));
      }
    }
  }
}

/// Order the subtree so a parent always precedes its children (root first). The
/// recursive CTE does not guarantee parent-first, and we insert with FK-linked
/// parents, so the order matters.
fn topo_order_subtree(subtree: &[TransferRow]) -> Vec<&TransferRow> {
  let ids: std::collections::HashSet<Uuid> = subtree.iter().map(|r| r.id).collect();
  let mut by_parent: std::collections::HashMap<Option<Uuid>, Vec<&TransferRow>> =
    std::collections::HashMap::new();
  for r in subtree {
    // The root's real parent is outside the subtree — anchor it at None.
    let key = r.parent_view_id.filter(|p| ids.contains(p));
    by_parent.entry(key).or_default().push(r);
  }
  let mut out: Vec<&TransferRow> = Vec::with_capacity(subtree.len());
  let mut stack: Vec<Option<Uuid>> = vec![None];
  while let Some(parent) = stack.pop() {
    if let Some(children) = by_parent.get(&parent) {
      for child in children {
        out.push(child);
        stack.push(Some(child.id));
      }
    }
  }
  out
}

#[cfg(test)]
mod markdown_update_tests {
  use super::*;

  /// A one-paragraph document to write into.
  fn current_doc() -> mica_app_core::documents::DocumentSnapshotPayload {
    mica_app_core::documents::DocumentSnapshotPayload {
      schema_version: 1,
      root_block_id: "root".to_string(),
      blocks: vec![
        mica_app_core::documents::Block {
          id: "root".to_string(),
          kind: "paragraph".to_string(),
          text: String::new(),
          data: serde_json::json!({"front_matter": "title: Kept"}),
          children: vec!["p1".to_string()],
        },
        mica_app_core::documents::Block {
          id: "p1".to_string(),
          kind: "paragraph".to_string(),
          text: "existing".to_string(),
          data: serde_json::Value::Null,
          children: Vec::new(),
        },
      ],
    }
  }

  fn request(mode: MarkdownUpdateMode, markdown: &str) -> UpdateMarkdownRequest {
    UpdateMarkdownRequest {
      mode,
      markdown: markdown.to_string(),
      anchor: None,
      find: None,
      replace: None,
      expected_seq: None,
    }
  }

  /// Every text an InsertBlock op carries, in order.
  fn inserted_texts(ops: &[DocumentOperation]) -> Vec<String> {
    ops
      .iter()
      .filter_map(|op| match op {
        DocumentOperation::InsertBlock { block, .. } if !block.text.is_empty() => {
          Some(block.text.clone())
        }
        _ => None,
      })
      .collect()
  }

  /// Appending content that OPENS with `---` used to lose everything up to the
  /// closing fence: the fragment was parsed as a whole document, so the parser
  /// read that as YAML front matter and stashed it on a throwaway root. The
  /// caller got the tail and no error. Reported 2026-08-07 from an MCP write.
  #[test]
  fn append_does_not_swallow_content_between_dashes() {
    let ops = markdown_update_ops(
      &current_doc(),
      &request(MarkdownUpdateMode::Append, "---\nbody\n---\ntail"),
      Uuid::new_v4(),
    )
    .expect("append should build ops");

    assert_eq!(
      inserted_texts(&ops),
      vec!["body", "tail"],
      "the body between the fences must reach the document"
    );
  }

  /// Same position error, same fix, other grafting mode.
  #[test]
  fn insert_at_does_not_swallow_content_between_dashes() {
    let mut req = request(MarkdownUpdateMode::InsertAt, "---\nbody\n---\ntail");
    req.anchor = Some("p1".to_string());
    let ops =
      markdown_update_ops(&current_doc(), &req, Uuid::new_v4()).expect("insert_at should build ops");
    assert_eq!(inserted_texts(&ops), vec!["body", "tail"]);
  }

  /// `replace_all` IS the whole document, so it keeps reading front matter —
  /// and now carries it onto the real root instead of dropping it with the
  /// temporary one. Without this, export → replace_all → export lost the page's
  /// properties silently.
  #[test]
  fn replace_all_carries_front_matter_onto_the_root() {
    let ops = markdown_update_ops(
      &current_doc(),
      &request(MarkdownUpdateMode::ReplaceAll, "---\ntitle: New\n---\nbody"),
      Uuid::new_v4(),
    )
    .expect("replace_all should build ops");

    let fm = ops
      .iter()
      .find_map(|op| match op {
        DocumentOperation::UpdateBlock {
          block_id,
          data: Some(data),
          ..
        } if block_id == "root" => data.get("front_matter").and_then(|v| v.as_str()),
        _ => None,
      })
      .expect("replace_all must write the front matter to the root");
    assert_eq!(fm, "title: New");
    assert_eq!(inserted_texts(&ops), vec!["body"]);
  }

  /// A body-only `replace_all` must not wipe properties the caller never
  /// mentioned — "replace the text" is not "delete my metadata".
  #[test]
  fn replace_all_without_front_matter_leaves_existing_properties_alone() {
    let ops = markdown_update_ops(
      &current_doc(),
      &request(MarkdownUpdateMode::ReplaceAll, "just body"),
      Uuid::new_v4(),
    )
    .expect("replace_all should build ops");

    assert!(
      !ops.iter().any(|op| matches!(
        op,
        DocumentOperation::UpdateBlock { block_id, .. } if block_id == "root"
      )),
      "no root update means the existing front matter survives untouched"
    );
  }
}

#[cfg(test)]
mod tests {
  /// An image block carrying [data] — the only field these tests vary.
  fn image_block(data: serde_json::Value) -> mica_app_core::documents::Block {
    mica_app_core::documents::Block {
      id: "img_1".to_string(),
      kind: "image".to_string(),
      text: String::new(),
      data,
      children: Vec::new(),
    }
  }

  /// An uploaded image must survive `export → import` as the SAME file, not as
  /// a link. Before the asset map, a Markdown export fell back to the block's
  /// original filename — and every client names a pasted image
  /// `pasted-image.png`, so nine images across three real workspaces all
  /// exported as `![](pasted-image.png)`: unfetchable, and indistinguishable.
  /// Re-importing that produced an image block pointing at a bare filename,
  /// silently dropping the file reference — which would also make blob GC stop
  /// counting the file and eventually delete the bytes.
  #[test]
  fn an_uploaded_image_round_trips_through_markdown_as_the_same_file() {
    let ws = Uuid::new_v4();
    let file_id = Uuid::new_v4().to_string();
    let block = image_block(serde_json::json!({"file_id": file_id, "name": "pasted-image.png"}));

    let assets = blob_asset_map(std::slice::from_ref(&block), ws);
    let href = assets.get(&file_id).expect("the file_id gets a href");
    assert!(
      href.contains(&file_id) && href.starts_with(&format!("/api/workspaces/{ws}/files/")),
      "the href must actually serve the bytes: {href}"
    );

    // …and the import side recognises what the export side wrote.
    let mut back = vec![image_block(serde_json::json!({"url": href}))];
    rewire_blob_hrefs(&mut back, ws);
    assert_eq!(back[0].data["file_id"], serde_json::json!(file_id));
    assert_eq!(back[0].data["name"], serde_json::json!("pasted-image.png"));
    assert!(back[0].data.get("url").is_none(), "no longer a plain link");
  }

  /// A href for a DIFFERENT workspace must stay a link. Rewiring it would forge
  /// a reference to a file this workspace's readers may not be allowed to see —
  /// and would let a pasted URL smuggle another tenant's blob into a page.
  #[test]
  fn a_blob_href_from_another_workspace_is_never_claimed() {
    let mine = Uuid::new_v4();
    let theirs = Uuid::new_v4();
    let file_id = Uuid::new_v4().to_string();
    let href = blob_href(theirs, &file_id, "secret.png");

    let mut blocks = vec![image_block(serde_json::json!({"url": href}))];
    rewire_blob_hrefs(&mut blocks, mine);
    assert!(blocks[0].data.get("file_id").is_none(), "not ours to claim");
    assert!(blocks[0].data.get("url").is_some(), "stays a plain link");

    // Nor does an external look-alike get claimed.
    let mut evil = vec![image_block(
      serde_json::json!({"url": "https://evil.test/api/workspaces/nope/files/x/blob/a.png"}),
    )];
    rewire_blob_hrefs(&mut evil, mine);
    assert!(evil[0].data.get("file_id").is_none());
  }

  /// A plain external image is not ours and must pass through untouched — the
  /// whole point of the guard is that it only claims what it wrote.
  #[test]
  fn an_external_image_url_survives_import_as_a_link() {
    let mut blocks = vec![image_block(
      serde_json::json!({"url": "https://example.com/photo.png"}),
    )];
    rewire_blob_hrefs(&mut blocks, Uuid::new_v4());
    assert_eq!(
      blocks[0].data["url"],
      serde_json::json!("https://example.com/photo.png")
    );
  }

  #[test]
  fn percent_decode_recovers_utf8_and_leaves_junk_alone() {
    assert_eq!(percent_decode("pasted-image.png"), "pasted-image.png");
    assert_eq!(percent_decode("%E5%9B%BE.png"), "图.png");
    // A malformed escape is data, not a parse error.
    assert_eq!(percent_decode("100%zz"), "100%zz");
    assert_eq!(percent_decode("%"), "%");
  }

  /// `safe_segment` is the whole of the name→file-name rule, and since the body
  /// is exported verbatim the file name is now the ONLY place a page's name
  /// survives an export. A bug here loses it silently.
  #[test]
  fn safe_segment_makes_a_usable_file_name_from_any_page_name() {
    assert_eq!(safe_segment("mica-cli"), "mica-cli");
    assert_eq!(safe_segment("无引用 blob 自动回收"), "无引用_blob_自动回收");
    // Path separators and Windows-hostile characters cannot survive as-is —
    // one `/` would silently fabricate a directory inside the archive.
    assert_eq!(safe_segment("a/b"), "a_b");
    assert_eq!(safe_segment("what? really!"), "what_really");
    // Runs collapse and edges are trimmed, so names stay readable.
    assert_eq!(safe_segment("  spaced   out  "), "spaced_out");
    // A name that survives nothing still must yield a file name.
    assert_eq!(safe_segment("///"), "untitled");
    assert_eq!(safe_segment(""), "untitled");
  }

  use super::*;
  use mica_app_core::search::BodyIndex;

  /// The `%…%` pattern escapes LIKE metacharacters so a query for `50%` or `a_b`
  /// matches literally rather than as a wildcard (paired with `ESCAPE '\'`).
  #[test]
  fn like_pattern_escapes_wildcards() {
    assert_eq!(like_pattern("hello"), "%hello%");
    assert_eq!(like_pattern("50%"), "%50\\%%");
    assert_eq!(like_pattern("a_b"), "%a\\_b%");
    assert_eq!(like_pattern("c\\d"), "%c\\\\d%");
    // CJK carries no metacharacters — passes through, just wrapped.
    assert_eq!(like_pattern("全文搜索"), "%全文搜索%");
  }

  /// The snippet windows AROUND the first hit (not the head of the body) and
  /// marks each clipped edge with an ellipsis. A hit in the middle of a long
  /// body must keep context on both sides; CJK counts by character.
  #[test]
  fn snippet_windows_around_the_match() {
    // Absent needle → None.
    assert!(snippet_for("nothing here", "zzz").is_none());

    // Short body: whole thing, no ellipses.
    assert_eq!(snippet_for("hello world", "world").as_deref(), Some("hello world"));

    // Case-insensitive.
    assert_eq!(snippet_for("Hello World", "hello").as_deref(), Some("Hello World"));

    // A hit deep in a long body keeps left+right context and both ellipses.
    let body = format!("{}NEEDLE{}", "a".repeat(200), "b".repeat(200));
    let snip = snippet_for(&body, "needle").unwrap();
    assert!(snip.starts_with('…') && snip.ends_with('…'), "clipped both ends: {snip}");
    assert!(snip.contains("NEEDLE"), "keeps the match: {snip}");
    assert!(snip.chars().count() <= 162, "≈160-char window (+2 ellipses): {snip}");

    // CJK windowing: a 2-char substring hit inside a longer sentence is found.
    let cjk = "这是一段很长的中文文本用来测试全文搜索索引化的片段窗口功能";
    let snip = snippet_for(cjk, "索引").unwrap();
    assert!(snip.contains("索引"), "CJK substring windowed: {snip}");
  }

  #[test]
  fn outline_lists_headings_and_block_ids_in_document_order() {
    use mica_app_core::documents::{Block, DocumentSnapshotPayload};
    let blk = |id: &str, kind: &str, text: &str, data: serde_json::Value, kids: Vec<&str>| Block {
      id: id.into(),
      kind: kind.into(),
      text: text.into(),
      data,
      children: kids.into_iter().map(String::from).collect(),
    };
    let payload = DocumentSnapshotPayload {
      schema_version: 1,
      root_block_id: "root".into(),
      blocks: vec![
        blk(
          "root",
          "page",
          "",
          serde_json::Value::Null,
          vec!["h1", "p", "h2"],
        ),
        blk(
          "h1",
          "heading",
          "Intro",
          serde_json::json!({"level": 1}),
          vec![],
        ),
        blk("p", "paragraph", "body", serde_json::Value::Null, vec![]),
        blk(
          "h2",
          "heading",
          "Details",
          serde_json::json!({"level": 2}),
          vec![],
        ),
      ],
    };
    let out = outline_from_payload(&payload, 7);
    assert_eq!(out.seq, 7);
    assert_eq!(out.block_ids, ["h1", "p", "h2"]);
    assert_eq!(
      out
        .headings
        .iter()
        .map(|h| (h.level, h.text.as_str()))
        .collect::<Vec<_>>(),
      [(1, "Intro"), (2, "Details")],
    );
  }

  fn doc_with_children(kids: &[&str]) -> mica_app_core::documents::DocumentSnapshotPayload {
    use mica_app_core::documents::{Block, DocumentSnapshotPayload};
    let mut blocks = vec![Block {
      id: "root".into(),
      kind: "page".into(),
      text: "".into(),
      data: serde_json::Value::Null,
      children: kids.iter().map(|s| s.to_string()).collect(),
    }];
    for k in kids {
      blocks.push(Block {
        id: (*k).into(),
        kind: "paragraph".into(),
        text: "existing".into(),
        data: serde_json::Value::Null,
        children: vec![],
      });
    }
    DocumentSnapshotPayload {
      schema_version: 1,
      root_block_id: "root".into(),
      blocks,
    }
  }

  fn upd(mode: MarkdownUpdateMode, markdown: &str) -> UpdateMarkdownRequest {
    UpdateMarkdownRequest {
      mode,
      markdown: markdown.into(),
      anchor: None,
      find: None,
      replace: None,
      expected_seq: None,
    }
  }

  #[test]
  fn markdown_update_append_grafts_under_root_without_deletes() {
    use mica_app_core::documents::DocumentOperation;
    let current = doc_with_children(&["old"]);
    let ops = markdown_update_ops(
      &current,
      &upd(MarkdownUpdateMode::Append, "# Title\n\nhello"),
      Uuid::new_v4(),
    )
    .unwrap();
    assert!(
      ops
        .iter()
        .all(|o| !matches!(o, DocumentOperation::DeleteBlock { .. })),
      "append never deletes",
    );
    let top_inserts = ops
      .iter()
      .filter(
        |o| matches!(o, DocumentOperation::InsertBlock { parent_id, .. } if parent_id == "root"),
      )
      .count();
    assert!(
      top_inserts >= 2,
      "heading + paragraph grafted under the existing root"
    );
    assert!(
      ops.iter().all(|o| match o {
        DocumentOperation::InsertBlock { block, .. } => block.children.is_empty(),
        _ => true,
      }),
      "inserted blocks have children stripped (re-linked via parent_id)",
    );
  }

  #[test]
  fn markdown_update_replace_all_deletes_existing_top_level_first() {
    use mica_app_core::documents::DocumentOperation;
    let current = doc_with_children(&["a", "b"]);
    let ops = markdown_update_ops(
      &current,
      &upd(MarkdownUpdateMode::ReplaceAll, "fresh body"),
      Uuid::new_v4(),
    )
    .unwrap();
    let deletes: Vec<&str> = ops
      .iter()
      .filter_map(|o| match o {
        DocumentOperation::DeleteBlock { block_id } => Some(block_id.as_str()),
        _ => None,
      })
      .collect();
    assert_eq!(
      deletes,
      ["a", "b"],
      "existing top-level children deleted first"
    );
    assert!(
      ops
        .iter()
        .any(|o| matches!(o, DocumentOperation::InsertBlock { .. })),
      "then the new markdown is grafted in",
    );
  }

  #[test]
  fn markdown_update_insert_at_positions_after_the_anchor() {
    use mica_app_core::documents::DocumentOperation;
    let current = doc_with_children(&["a", "b", "c"]);
    let mut request = upd(MarkdownUpdateMode::InsertAt, "inserted");
    request.anchor = Some("a".into());
    let ops = markdown_update_ops(&current, &request, Uuid::new_v4()).unwrap();
    // The (single) new top-level paragraph lands at index 1 — right after "a".
    let top = ops
      .iter()
      .find(
        |o| matches!(o, DocumentOperation::InsertBlock { parent_id, .. } if parent_id == "root"),
      )
      .expect("a top-level insert");
    match top {
      DocumentOperation::InsertBlock { index, .. } => assert_eq!(*index, Some(1)),
      _ => unreachable!(),
    }
  }

  #[test]
  fn markdown_update_insert_at_unknown_anchor_errors() {
    let current = doc_with_children(&["a"]);
    let mut request = upd(MarkdownUpdateMode::InsertAt, "x");
    request.anchor = Some("ghost".into());
    assert!(markdown_update_ops(&current, &request, Uuid::new_v4()).is_err());
  }

  fn a_view(name: &str) -> View {
    let stamp = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
      .unwrap()
      .with_timezone(&chrono::Utc);
    View {
      id: Uuid::nil(),
      workspace_id: Uuid::nil(),
      parent_view_id: None,
      object_id: Uuid::nil(),
      object_type: "document".into(),
      name: name.into(),
      icon: None,
      position: "a0".into(),
      is_deleted: false,
      created_by: Uuid::nil(),
      created_at: stamp,
      updated_at: stamp,
      state_bytes: None,
    }
  }

  fn a_query() -> ListViewsQuery {
    ListViewsQuery {
      parent_view_id: None,
      depth: None,
      limit: None,
      offset: None,
      with_stats: false,
    }
  }

  /// The failure mode this guards is not "slow" — it is a client frozen on a
  /// tree that no longer exists, with no way to notice. Every way the listing
  /// can differ has to move the tag.
  #[test]
  fn the_view_etag_moves_whenever_the_listing_would() {
    let base = vec![a_view("Alpha"), a_view("Beta")];
    let tag = views_etag(&base, &a_query());

    // Same input, same tag — otherwise nothing is ever a hit.
    assert_eq!(views_etag(&base, &a_query()), tag);

    // A rename.
    let renamed = vec![a_view("Alpha"), a_view("Beta renamed")];
    assert_ne!(views_etag(&renamed, &a_query()), tag);

    // A deletion.
    assert_ne!(views_etag(&base[..1], &a_query()), tag);

    // Reordering: two trees with the same rows in a different order render
    // differently, so they are not the same response.
    let swapped = vec![a_view("Beta"), a_view("Alpha")];
    assert_ne!(views_etag(&swapped, &a_query()), tag);

    // A field the app never reads still changes the BODY, and the header is a
    // promise about the body — not about one caller's use of it.
    let mut touched = vec![a_view("Alpha"), a_view("Beta")];
    touched[1].updated_at += chrono::Duration::seconds(1);
    assert_ne!(views_etag(&touched, &a_query()), tag);

    // The query shape is part of the identity: the same workspace answers a
    // different body for a different depth.
    let mut deeper = a_query();
    deeper.depth = Some(1);
    assert_ne!(views_etag(&base, &deeper), tag);
  }

  #[test]
  fn if_none_match_is_compared_the_way_the_header_is_written() {
    let tag = "\"abc123\"";
    assert!(etag_matches("\"abc123\"", tag));
    assert!(etag_matches("*", tag), "the wildcard matches anything we have");
    // A list, as a client with several cached copies would send.
    assert!(etag_matches("\"other\", \"abc123\"", tag));
    // A proxy may mark it weak on the way through. Refusing that would silently
    // turn every conditional request back into a full transfer — the mechanism
    // would look present and do nothing.
    assert!(etag_matches("W/\"abc123\"", tag));
    assert!(!etag_matches("\"abc124\"", tag));
    assert!(!etag_matches("", tag));
  }

  #[test]
  fn markdown_update_delete_removes_exactly_the_anchored_block() {
    use mica_app_core::documents::DocumentOperation;
    let current = doc_with_children(&["a", "b"]);
    let mut request = upd(MarkdownUpdateMode::Delete, "");
    request.anchor = Some("b".into());
    let ops = markdown_update_ops(&current, &request, Uuid::new_v4()).unwrap();
    assert_eq!(
      ops,
      [DocumentOperation::DeleteBlock { block_id: "b".into() }],
      "one targeted op — deleting a block must not rewrite the page around it"
    );
  }

  #[test]
  fn markdown_update_delete_refuses_the_root_and_unknown_anchors() {
    let current = doc_with_children(&["a"]);

    // Deleting the root would empty the document through a call that reads as
    // "remove one block". `delete_block` also refuses, but by the time the op
    // reaches it the caller has already been told the request was accepted.
    let mut root = upd(MarkdownUpdateMode::Delete, "");
    root.anchor = Some("root".into());
    assert!(markdown_update_ops(&current, &root, Uuid::new_v4()).is_err());

    // A stale block id is the normal failure here — an outline read minutes ago,
    // the block since removed. It must say so, not delete something else.
    let mut gone = upd(MarkdownUpdateMode::Delete, "");
    gone.anchor = Some("vanished".into());
    let err = markdown_update_ops(&current, &gone, Uuid::new_v4()).unwrap_err();
    assert!(err.contains("vanished"), "the error names the anchor: {err}");

    // No anchor at all must not be read as "delete nothing, succeed".
    let bare = upd(MarkdownUpdateMode::Delete, "");
    assert!(markdown_update_ops(&current, &bare, Uuid::new_v4()).is_err());
  }

  #[test]
  fn markdown_update_find_replace_updates_matching_blocks() {
    use mica_app_core::documents::DocumentOperation;
    let current = doc_with_children(&["a", "b"]); // both blocks' text == "existing"
    let request = UpdateMarkdownRequest {
      mode: MarkdownUpdateMode::FindReplace,
      markdown: String::new(),
      anchor: None,
      find: Some("existing".into()),
      replace: Some("updated".into()),
      expected_seq: None,
    };
    let ops = markdown_update_ops(&current, &request, Uuid::new_v4()).unwrap();
    let updated: Vec<(&str, &str)> = ops
      .iter()
      .filter_map(|o| match o {
        DocumentOperation::UpdateBlock {
          block_id,
          text: Some(t),
          ..
        } => Some((block_id.as_str(), t.as_str())),
        _ => None,
      })
      .collect();
    assert_eq!(updated, [("a", "updated"), ("b", "updated")]);
    // No matches → an error, not a silent no-op.
    let mut miss = request;
    miss.find = Some("nope".into());
    assert!(markdown_update_ops(&current, &miss, Uuid::new_v4()).is_err());
  }

  #[test]
  fn markdown_update_find_replace_skips_formatted_blocks() {
    use mica_app_core::documents::{Block, DocumentSnapshotPayload};
    // The only matching block carries an inline mark → a text-only replace would
    // desync its UTF-16 offsets, so find_replace must refuse rather than corrupt.
    let payload = DocumentSnapshotPayload {
      schema_version: 1,
      root_block_id: "root".into(),
      blocks: vec![
        Block {
          id: "root".into(),
          kind: "page".into(),
          text: "".into(),
          data: serde_json::Value::Null,
          children: vec!["p".into()],
        },
        Block {
          id: "p".into(),
          kind: "paragraph".into(),
          text: "see docs now".into(),
          data: serde_json::json!({"marks": [{"type": "link", "start": 4, "end": 8}]}),
          children: vec![],
        },
      ],
    };
    let request = UpdateMarkdownRequest {
      mode: MarkdownUpdateMode::FindReplace,
      markdown: String::new(),
      anchor: None,
      find: Some("see ".into()),
      replace: Some(String::new()),
      expected_seq: None,
    };
    let err = markdown_update_ops(&payload, &request, Uuid::new_v4()).unwrap_err();
    assert!(
      err.contains("formatted"),
      "should refuse formatted blocks, got: {err}"
    );
  }

  #[test]
  fn markdown_update_replace_all_empty_refuses_to_wipe() {
    let current = doc_with_children(&["a", "b"]);
    let err = markdown_update_ops(
      &current,
      &upd(MarkdownUpdateMode::ReplaceAll, "  \n "),
      Uuid::new_v4(),
    )
    .unwrap_err();
    assert!(
      err.contains("wipe") || err.contains("content"),
      "empty replace_all must not wipe the doc, got: {err}",
    );
  }

  fn text_block(id: &str, data: serde_json::Value) -> mica_app_core::documents::Block {
    mica_app_core::documents::Block {
      id: id.to_string(),
      kind: "text".to_string(),
      text: "x".to_string(),
      data,
      children: Vec::new(),
    }
  }

  /// A transferred doc must point its image at the DESTINATION file copy (else
  /// the source's per-workspace GC reclaims the bytes and the moved page breaks),
  /// remap links to pages that came along, and leave links to pages left behind
  /// untouched (they dangle — surfaced as a warning, not silently rewritten).
  #[test]
  fn transfer_rewrites_file_ids_and_in_subtree_links_only() {
    let old_file = Uuid::new_v4();
    let new_file = Uuid::new_v4();
    let in_sub_old = Uuid::new_v4();
    let in_sub_new = Uuid::new_v4();
    let outside = Uuid::new_v4();

    let file_map = std::collections::HashMap::from([(old_file, new_file)]);
    let view_map = std::collections::HashMap::from([(in_sub_old, in_sub_new)]);

    let mut payload = DocumentSnapshotPayload {
      schema_version: 1,
      root_block_id: "root".to_string(),
      blocks: vec![
        image_block(serde_json::json!({
          "file_id": old_file.to_string(),
          "name": "x.png",
          "url": format!("/api/workspaces/{}/files/{}/blob/x.png", Uuid::new_v4(), old_file),
        })),
        text_block(
          "t1",
          serde_json::json!({"marks": [
            {"type": "link", "href": format!("mica://page/{in_sub_old}")},
            {"type": "link", "href": format!("mica://page/{outside}")},
          ]}),
        ),
      ],
    };
    rewrite_transferred_payload(&mut payload, &file_map, &view_map);

    assert_eq!(
      payload.blocks[0].data["file_id"],
      serde_json::json!(new_file.to_string()),
      "image file_id must point at the destination copy",
    );
    assert!(
      payload.blocks[0].data.get("url").is_none(),
      "stale cached blob url must be dropped so the client re-resolves",
    );
    let marks = payload.blocks[1].data["marks"].as_array().unwrap();
    assert_eq!(
      marks[0]["href"],
      serde_json::json!(format!("mica://page/{in_sub_new}")),
      "a link to a page that came along is remapped",
    );
    assert_eq!(
      marks[1]["href"],
      serde_json::json!(format!("mica://page/{outside}")),
      "a link to a page left behind is preserved (dangles, warned)",
    );
  }

  #[test]
  fn dedup_sibling_name_only_numbers_on_collision() {
    // Free name → used as-is.
    assert_eq!(dedup_sibling_name("日志方案 副本", &[]), "日志方案 副本");
    // Collision → first free numbered suffix (locale-neutral number).
    assert_eq!(
      dedup_sibling_name("日志方案 副本", &["日志方案 副本".into()]),
      "日志方案 副本 2"
    );
    // Skips taken numbers, does not reuse them.
    assert_eq!(
      dedup_sibling_name(
        "X 副本",
        &["X 副本".into(), "X 副本 2".into(), "X 副本 4".into()]
      ),
      "X 副本 3"
    );
    // Unrelated siblings never force a suffix.
    assert_eq!(dedup_sibling_name("X 副本", &["Y".into(), "Z".into()]), "X 副本");
  }

  #[test]
  fn page_link_targets_extracts_only_mica_page_ids() {
    let a = Uuid::new_v4();
    let data = serde_json::json!({"marks": [
      {"type": "link", "href": format!("mica://page/{a}")},
      {"type": "link", "href": "https://example.com"},
      {"type": "bold"},
    ]});
    assert_eq!(page_link_targets(&data), vec![a.to_string()]);
    assert!(page_link_targets(&serde_json::json!({})).is_empty());
  }

  #[test]
  fn topo_order_puts_parents_before_children() {
    let root = Uuid::new_v4();
    let child = Uuid::new_v4();
    let grandchild = Uuid::new_v4();
    let outside_parent = Uuid::new_v4(); // root's real parent, outside the subtree
    let row = |id, parent| TransferRow {
      id,
      parent_view_id: parent,
      object_id: Uuid::new_v4(),
      object_type: "folder".to_string(),
      name: "n".to_string(),
      position: "0".to_string(),
    };
    // Deliberately not parent-first, mimicking the CTE's arbitrary order.
    let subtree = vec![
      row(grandchild, Some(child)),
      row(root, Some(outside_parent)),
      row(child, Some(root)),
    ];
    let ordered: Vec<Uuid> = topo_order_subtree(&subtree).iter().map(|r| r.id).collect();
    let pos = |id: Uuid| ordered.iter().position(|&x| x == id).unwrap();
    assert_eq!(ordered.len(), 3);
    assert!(pos(root) < pos(child), "root must precede its child");
    assert!(pos(child) < pos(grandchild), "child must precede its grandchild");
  }

  /// The page-tree invariant guard runs entirely in SQL — `object_type` lives in
  /// Postgres, and the backstop is a DB trigger — so a mock would assert nothing.
  /// Gated on `DATABASE_URL`, hardened the same way as auth.rs::refresh_pg and
  /// app-core/tests/sync_pg.rs: skipped (green) without a database locally, but a
  /// set-but-unreachable URL — or a missing one in CI — is a hard failure, never
  /// a silent pass.
  ///
  ///   $env:DATABASE_URL="postgres://mica:mica@127.0.0.1:5432/mica"
  ///   cargo test -p mica-api-server parent_guard_pg
  mod parent_guard_pg {
    use super::*;

    async fn pool() -> Option<PgPool> {
      let Ok(url) = std::env::var("DATABASE_URL") else {
        assert!(
          std::env::var("CI").is_err(),
          "DATABASE_URL is unset in CI — the postgres service block regressed; \
           these tests must not silently pass"
        );
        return None;
      };
      Some(
        PgPool::connect(&url)
          .await
          .expect("DATABASE_URL is set but the connection failed"),
      )
    }

    /// Seed the FK chain a view needs: user → workspace. Returns (workspace, user).
    async fn seed_workspace(db: &PgPool) -> (Uuid, Uuid) {
      let user = Uuid::new_v4();
      let ws = Uuid::new_v4();
      sqlx::query("INSERT INTO users(id,email,display_name,password_hash) VALUES($1,$2,'T','x')")
        .bind(user)
        .bind(format!("{user}@parent.test"))
        .execute(db)
        .await
        .unwrap();
      sqlx::query("INSERT INTO workspaces(id,name,owner_id) VALUES($1,'W',$2)")
        .bind(ws)
        .bind(user)
        .execute(db)
        .await
        .unwrap();
      // The owner is a MEMBER — `create_workspace` writes this row, and every
      // "what does this user have" query joins through it. A seed that skipped
      // it produced workspaces nobody belonged to: two tests that counted
      // through workspace_members got 0 and only failed in CI, because a local
      // run without DATABASE_URL never executed them at all.
      sqlx::query(
        "INSERT INTO workspace_members(workspace_id,user_id,role,position)          VALUES($1,$2,'owner',$3)",
      )
      .bind(ws)
      .bind(user)
      .bind(crate::routes::workspaces::pad_position(1))
      .execute(db)
      .await
      .unwrap();
      (ws, user)
    }

    /// Insert a top-level view (parent_view_id NULL, so the trigger stays out of
    /// the way) of the given object_type. Returns its id.
    async fn seed_view(db: &PgPool, ws: Uuid, user: Uuid, object_type: &str) -> Uuid {
      let id = Uuid::new_v4();
      sqlx::query(
        "INSERT INTO views(id,workspace_id,object_id,object_type,name,position,created_by) \
         VALUES($1,$2,$3,$4::object_type,'V','0',$5)",
      )
      .bind(id)
      .bind(ws)
      .bind(Uuid::new_v4())
      .bind(object_type)
      .bind(user)
      .execute(db)
      .await
      .unwrap();
      id
    }

    #[tokio::test]
    async fn a_page_is_refused_as_a_parent_but_a_folder_is_accepted() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;

      let folder = seed_view(&db, ws, user, "folder").await;
      let page = seed_view(&db, ws, user, "document").await;

      // A folder is a legal container.
      ensure_parent_accepts_children(&db, ws, folder)
        .await
        .expect("a folder must accept children");

      // A page is a leaf: the guard rejects it with a readable 400, not the
      // trigger's 500.
      let err = ensure_parent_accepts_children(&db, ws, page)
        .await
        .expect_err("a page must not accept children");
      assert!(
        matches!(err, ApiError::BadRequest(_)),
        "a page parent must be a 400, got {err:?}"
      );

      // A parent that does not exist in this workspace is a 404, not a 400.
      let missing = ensure_parent_accepts_children(&db, ws, Uuid::new_v4())
        .await
        .expect_err("an unknown parent must be rejected");
      assert!(
        matches!(missing, ApiError::NotFound),
        "an unknown parent must be a 404, got {missing:?}"
      );
    }

    /// Optimistic concurrency (MCP item 4): a write pinned to a stale `seq` is
    /// refused with 409 before any op runs; the current `seq` passes the guard.
    ///
    ///   $env:DATABASE_URL="postgres://mica:mica@127.0.0.1:5432/mica"
    ///   cargo test -p mica-api-server expected_seq
    #[tokio::test]
    async fn a_stale_expected_seq_is_a_conflict_and_the_current_one_passes() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;
      let (_view, doc) = seed_document(&db, ws, user, "C", serde_json::json!([])).await;

      // A fresh doc is at seq 0. Pinning a seq the doc never had is a 409, raised
      // by the guard before the derive closure is ever called.
      let stale =
        store::apply_derived_operations(&db, ws, doc, user, Some(1), |_| Ok(Vec::new()))
          .await
          .expect_err("a stale expected_seq must be refused");
      assert!(
        matches!(stale, ApiError::Conflict(_)),
        "stale expected_seq must be a 409, got {stale:?}"
      );

      // The current seq (0) passes the guard: it then fails on empty ops
      // (BadRequest), which proves the concurrency check did NOT stop it.
      let passed =
        store::apply_derived_operations(&db, ws, doc, user, Some(0), |_| Ok(Vec::new()))
          .await
          .expect_err("empty ops still error after the guard passes");
      assert!(
        matches!(passed, ApiError::BadRequest(_)),
        "the current seq must pass the guard, got {passed:?}"
      );
    }

    /// A page that links to another page shows up in that page's backlinks, and
    /// never the reverse (link direction is not symmetric). Gated the same way as
    /// the parent-guard tests: green without a DB locally, hard-fail in CI.
    ///
    ///   $env:DATABASE_URL="postgres://mica:mica@127.0.0.1:5432/mica"
    ///   cargo test -p mica-api-server backlinks_pg
    #[tokio::test]
    async fn backlinks_are_the_inverse_of_forward_page_links() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;

      // Two pages; A has a block whose link mark points at B.
      let (view_b, _doc_b) = seed_document(&db, ws, user, "B", serde_json::json!([])).await;
      let (view_a, _doc_a) = seed_document(
        &db,
        ws,
        user,
        "A",
        serde_json::json!([{
          "id": "blk_link",
          "type": "paragraph",
          "text": "see B",
          // A mark needs a RANGE: marks live in the yrs base as text formatting,
          // and `marks_from_data` drops any entry without start/end. The op-model
          // snapshot stored `data` as jsonb and handed it back verbatim, so a
          // range-less mark used to be visible to the backlink scan anyway — this
          // fixture was leaning on that.
          "data": {"marks": [
            {"type": "link", "start": 4, "end": 5,
             "href": format!("mica://page/{view_b}")}
          ]}
        }]),
      )
      .await;

      // B's backlinks contain A (and carry A's view id + document id + title).
      let b_links = collect_backlinks(&db, ws, view_b).await;
      assert_eq!(b_links.len(), 1, "B should have exactly one backlink");
      assert_eq!(b_links[0].view_id, view_a, "the backlink source is A's view");
      assert_eq!(b_links[0].title, "A");

      // A's backlinks are empty — links point one way.
      let a_links = collect_backlinks(&db, ws, view_a).await;
      assert!(a_links.is_empty(), "A should have no backlinks, got {a_links:?}");
    }

    /// The graph view's data: nodes for pages that participate in a link, edges
    /// in the direction the link was written, and a COUNT for everything else.
    ///
    /// The unlinked count is the part that would rot quietly. On a real
    /// workspace most pages link to nothing (a production snapshot: 798
    /// documents, 136 with any link), so if isolated pages leaked into `nodes`
    /// the view would render a field of disconnected dots and the structure it
    /// exists to show would be invisible — while every "the edge is there" test
    /// still passed.
    #[tokio::test]
    async fn the_graph_carries_edges_and_counts_the_rest() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;

      let (view_b, _) = seed_document(&db, ws, user, "B", serde_json::json!([])).await;
      let link_to = |target: Uuid| {
        serde_json::json!([{
          "id": "blk_link",
          "type": "paragraph",
          "text": "see B",
          "data": {"marks": [
            {"type": "link", "start": 4, "end": 5, "href": format!("mica://page/{target}")}
          ]}
        }])
      };
      let (view_a, _) = seed_document(&db, ws, user, "A", link_to(view_b)).await;
      // A third page nobody links to and which links nowhere.
      let (_view_c, _) = seed_document(&db, ws, user, "C", serde_json::json!([])).await;

      let g = workspace_graph(&db, ws).await.unwrap();

      assert_eq!(g.edges.len(), 1, "one link, one edge: {:?}", g.edges);
      assert_eq!(g.edges[0].source, view_a, "edges point source -> target");
      assert_eq!(g.edges[0].target, view_b);

      let ids: Vec<Uuid> = g.nodes.iter().map(|n| n.view_id).collect();
      assert!(ids.contains(&view_a) && ids.contains(&view_b), "{ids:?}");
      assert_eq!(g.nodes.len(), 2, "C must not be a node: {ids:?}");
      assert_eq!(g.unlinked, 1, "C is counted, not drawn");
      assert!(g.nodes.iter().all(|n| n.degree == 1), "{:?}", g.nodes);
    }

    /// A link whose target was trashed is a DANGLING reference, not an edge.
    /// Drawing it would invent a node that is not in the workspace any more.
    #[tokio::test]
    async fn a_link_to_a_trashed_page_is_not_an_edge() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;

      let (view_b, _) = seed_document(&db, ws, user, "B", serde_json::json!([])).await;
      let (_view_a, _) = seed_document(
        &db,
        ws,
        user,
        "A",
        serde_json::json!([{
          "id": "blk_link",
          "type": "paragraph",
          "text": "see B",
          "data": {"marks": [
            {"type": "link", "start": 4, "end": 5, "href": format!("mica://page/{view_b}")}
          ]}
        }]),
      )
      .await;
      assert_eq!(workspace_graph(&db, ws).await.unwrap().edges.len(), 1);

      sqlx::query("UPDATE views SET is_deleted = true WHERE id = $1")
        .bind(view_b)
        .execute(&db)
        .await
        .unwrap();

      let g = workspace_graph(&db, ws).await.unwrap();
      assert!(g.edges.is_empty(), "dangling edge survived: {:?}", g.edges);
      assert!(g.nodes.is_empty(), "a node with no live edge: {:?}", g.nodes);
    }

    /// A page that links to ITSELF is not its own backlink.
    #[tokio::test]
    async fn a_self_link_is_not_a_backlink() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;

      // Seed the page, then rewrite its base to link to its own view id — the
      // view id only exists after seeding, hence the two steps.
      let (view_id, doc_id) = seed_document(&db, ws, user, "S", serde_json::json!([])).await;
      reseed_base(
        &db,
        doc_id,
        serde_json::json!([{
          "id": "root",
          "type": "paragraph",
          "text": "self",
          "data": {"marks": [
            {"type": "link", "start": 0, "end": 4,
             "href": format!("mica://page/{view_id}")}
          ]}
        }]),
      )
      .await;

      let links = collect_backlinks(&db, ws, view_id).await;
      assert!(links.is_empty(), "a self-link must not be a backlink, got {links:?}");
    }

    /// Write `blocks` into a document's yrs base — since S5 the only place content
    /// lives, so what is put here is exactly what every read returns.
    ///
    /// A `root` block listing `blocks` as its children is synthesized unless the
    /// caller supplied one. That is not sugar: the op-model snapshot stored a FLAT
    /// array and every read handed it back verbatim, so a block nothing pointed at
    /// was still visible. A yrs doc is a tree walked from the root, and an
    /// unreachable block is simply not in it — a fixture that passes a bare
    /// `[{...}]` would seed an empty page and quietly assert nothing.
    async fn seed_base(db: &PgPool, doc_id: Uuid, blocks: serde_json::Value) {
      let mut blocks = blocks.as_array().cloned().unwrap_or_default();
      if !blocks.iter().any(|b| b["id"] == "root") {
        let children: Vec<_> = blocks.iter().map(|b| b["id"].clone()).collect();
        blocks.insert(
          0,
          serde_json::json!({
            "id": "root", "type": "paragraph", "text": "", "children": children,
          }),
        );
      }
      let payload: mica_markdown::DocumentSnapshotPayload = serde_json::from_value(
        serde_json::json!({
          "schema_version": 1,
          "root_block_id": "root",
          "blocks": blocks,
        }),
      )
      .unwrap();
      let mut tx = db.begin().await.unwrap();
      mica_app_core::sync::seed_base_tx(&mut tx, doc_id, payload)
        .await
        .unwrap();
      tx.commit().await.unwrap();
    }

    /// [`seed_base`] for a document that already has one: `seed_base_tx` is an
    /// insert-or-keep, so the existing row has to go first or the new blocks are
    /// silently dropped.
    async fn reseed_base(db: &PgPool, doc_id: Uuid, blocks: serde_json::Value) {
      sqlx::query("DELETE FROM document_yrs_base WHERE document_id = $1")
        .bind(doc_id)
        .execute(db)
        .await
        .unwrap();
      seed_base(db, doc_id, blocks).await;
    }

    /// Seed a document view (a leaf page) with content. `blocks` is the payload's
    /// `blocks` array, written straight into the document's yrs base, so link
    /// marks placed here are exactly what the backlink scan sees.
    /// Returns (view_id, document_id).
    async fn seed_document(
      db: &PgPool,
      ws: Uuid,
      user: Uuid,
      title: &str,
      blocks: serde_json::Value,
    ) -> (Uuid, Uuid) {
      let doc_id = Uuid::new_v4();
      sqlx::query(
        "INSERT INTO documents(id,workspace_id,root_block_id,created_by) VALUES($1,$2,'root',$3)",
      )
      .bind(doc_id)
      .bind(ws)
      .bind(user)
      .execute(db)
      .await
      .unwrap();
      seed_base(db, doc_id, blocks).await;
      let view_id = Uuid::new_v4();
      sqlx::query(
        "INSERT INTO views(id,workspace_id,object_id,object_type,name,position,created_by) \
         VALUES($1,$2,$3,'document'::object_type,$4,'0',$5)",
      )
      .bind(view_id)
      .bind(ws)
      .bind(doc_id)
      .bind(title)
      .bind(user)
      .execute(db)
      .await
      .unwrap();
      (view_id, doc_id)
    }

    /// A row of the backlinks endpoint's JSON response, minimal fields the tests
    /// assert on. `Backlink` itself is private and Serialize-only.
    #[derive(Debug, serde::Deserialize)]
    struct BacklinkRow {
      view_id: Uuid,
      title: String,
    }

    /// Run the REAL backlink scan the handler runs (member auth is orthogonal
    /// here) and return its rows, round-tripped through JSON to prove the wire
    /// shape. Calls `scan_backlinks` directly rather than re-deriving the walk, so
    /// the concurrent scan itself is what's under test.
    async fn collect_backlinks(db: &PgPool, ws: Uuid, view_id: Uuid) -> Vec<BacklinkRow> {
      scan_backlinks(db, ws, view_id)
        .await
        .unwrap()
        .iter()
        .map(|b| serde_json::from_value(serde_json::to_value(b).unwrap()).unwrap())
        .collect()
    }

    /// The concurrent scan returns EVERY source (no early stop) in the stable
    /// (title, view_id) order, matching the pre-concurrency sequential walk.
    #[tokio::test]
    async fn backlinks_return_all_sources_in_stable_order() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;

      let (view_target, _doc_t) = seed_document(&db, ws, user, "T", serde_json::json!([])).await;
      let blocks = serde_json::json!([{
        "id": "blk",
        "type": "paragraph",
        "text": "see T",
        "data": {"marks": [
          {"type": "link", "start": 4, "end": 5,
           "href": format!("mica://page/{view_target}")}
        ]}
      }]);
      // Seed three linking pages out of title order; the scan must still sort.
      let (view_c, _c) = seed_document(&db, ws, user, "C", blocks.clone()).await;
      let (view_a, _a) = seed_document(&db, ws, user, "A", blocks.clone()).await;
      let (view_b, _b) = seed_document(&db, ws, user, "B", blocks.clone()).await;

      let links = collect_backlinks(&db, ws, view_target).await;
      let titles: Vec<&str> = links.iter().map(|r| r.title.as_str()).collect();
      assert_eq!(titles, vec!["A", "B", "C"], "all sources, title-sorted");
      // The sort is total: view_ids follow their titles' order.
      assert_eq!(links[0].view_id, view_a);
      assert_eq!(links[1].view_id, view_b);
      assert_eq!(links[2].view_id, view_c);
    }

    #[tokio::test]
    async fn the_db_trigger_backstops_a_write_that_skips_the_guard() {
      // The guard is the front door; `views_parent_must_be_folder` (migration
      // 0011) covers every path that forgets to call it. Writing
      // parent_view_id = <page> straight into the table must still be refused.
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;
      let page = seed_view(&db, ws, user, "document").await;

      let direct = sqlx::query(
        "INSERT INTO views(workspace_id,parent_view_id,object_id,object_type,name,position,created_by) \
         VALUES($1,$2,$3,'document'::object_type,'V','0',$4)",
      )
      .bind(ws)
      .bind(page)
      .bind(Uuid::new_v4())
      .bind(user)
      .execute(&db)
      .await;
      assert!(
        direct.is_err(),
        "the trigger must reject nesting a view under a page"
      );
    }

    /// Give a seeded document a search index row with known `content_text`. The
    /// derivation of that text from the yrs base is covered in app-core's
    /// sync_pg.rs; here we insert it directly to exercise the SEARCH SQL (join,
    /// ILIKE, escaping, snippet, title_match) in isolation. `state`/`state_vector`
    /// are unused by search, so empty bytes suffice.
    async fn set_content_text(db: &PgPool, doc_id: Uuid, content_text: &str) {
      sqlx::query(
        "INSERT INTO document_yrs_base(document_id,state,state_vector,base_rid,content_text) \
         VALUES($1,$2,$3,0,$4) \
         ON CONFLICT (document_id) DO UPDATE SET content_text = excluded.content_text,            updated_at = now()",
      )
      .bind(doc_id)
      .bind(Vec::<u8>::new())
      .bind(Vec::<u8>::new())
      .bind(content_text)
      .execute(db)
      .await
      .unwrap();
    }

    /// Folders are findable only on request, and every hit says where it lives.
    ///
    /// Both halves come from the same complaint: an agent that had found a page
    /// and wanted to file a new one BESIDE it had no way to learn the parent
    /// short of listing the entire workspace tree — hundreds of pages fetched to
    /// read one id. `parent_view_id` is that id, and folder hits let the same
    /// one call answer "where is the folder called X".
    ///
    /// The opt-in is not decoration: the app's search dialog opens a hit as a
    /// page, so folders arriving unasked would send it after a document that
    /// does not exist.
    #[tokio::test]
    async fn search_finds_folders_only_on_request_and_always_reports_the_parent() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;

      let folder = Uuid::new_v4();
      sqlx::query(
        "INSERT INTO views(id,workspace_id,object_id,object_type,name,position,created_by) \
         VALUES($1,$2,$1,'folder'::object_type,'部署资料','0',$3)",
      )
      .bind(folder)
      .bind(ws)
      .bind(user)
      .execute(&db)
      .await
      .unwrap();

      let (page, _doc) =
        seed_document(&db, ws, user, "部署流程自动化", serde_json::json!([])).await;
      sqlx::query("UPDATE views SET parent_view_id = $1 WHERE id = $2")
        .bind(folder)
        .bind(page)
        .execute(&db)
        .await
        .unwrap();

      // Default: the folder is invisible even though its name matches.
      let hits = search_views(&db, &BodyIndex::default(), &[ws], "部署", false, None).await.unwrap();
      assert_eq!(hits.len(), 1, "pages only by default: {hits:?}");
      assert_eq!(hits[0].view_id, page);
      assert!(!hits[0].is_folder);
      assert_eq!(
        hits[0].parent_view_id,
        Some(folder),
        "a hit must say which folder it sits in — that is the whole point"
      );

      // Opt in: the folder shows up, flagged, and matched by NAME.
      let hits = search_views(&db, &BodyIndex::default(), &[ws], "部署", true, None).await.unwrap();
      assert_eq!(hits.len(), 2, "page + folder: {hits:?}");
      let f = hits.iter().find(|h| h.view_id == folder).expect("folder hit");
      assert!(f.is_folder);
      assert!(f.title_match, "folders can only ever match on their name");
      assert!(f.snippet.is_empty(), "a folder has no body to snippet");
      assert_eq!(f.parent_view_id, None, "this folder sits at the root");
    }

    /// Cross-workspace search spans the list it is given, and every hit says
    /// which workspace it came from.
    ///
    /// The `workspace_id` on the row is not decoration: opening a page is
    /// workspace-scoped, and the client only holds page trees for workspaces it
    /// has visited — so a hit from an unvisited workspace is unopenable without
    /// this field. Scope is proven both ways here, because the dangerous
    /// direction is the quiet one: a search that silently reached into a
    /// workspace the caller did not ask about would leak its page names.
    #[tokio::test]
    async fn search_spans_the_given_workspaces_and_labels_every_hit() {
      let Some(db) = pool().await else { return };
      let (ws_a, user_a) = seed_workspace(&db).await;
      let (ws_b, user_b) = seed_workspace(&db).await;

      let (page_a, _) =
        seed_document(&db, ws_a, user_a, "部署手册", serde_json::json!([])).await;
      let (page_b, _) =
        seed_document(&db, ws_b, user_b, "部署脚本", serde_json::json!([])).await;

      // One workspace: the other one's page must not appear.
      let only_a = search_views(&db, &BodyIndex::default(), &[ws_a], "部署", false, None).await.unwrap();
      assert_eq!(only_a.len(), 1, "scoped to one workspace: {only_a:?}");
      assert_eq!(only_a[0].view_id, page_a);
      assert_eq!(
        only_a[0].workspace_id, ws_a,
        "even a single-workspace hit carries its workspace"
      );

      // Both: two hits, each labelled with where it lives.
      let both = search_views(&db, &BodyIndex::default(), &[ws_a, ws_b], "部署", false, None).await.unwrap();
      assert_eq!(both.len(), 2, "both workspaces searched: {both:?}");
      let a = both.iter().find(|h| h.view_id == page_a).expect("ws_a hit");
      let b = both.iter().find(|h| h.view_id == page_b).expect("ws_b hit");
      assert_eq!(a.workspace_id, ws_a);
      assert_eq!(
        b.workspace_id, ws_b,
        "a hit from the second workspace must not be labelled with the first"
      );
    }

    /// No workspaces means no results — not "every workspace".
    ///
    /// The empty slice reaches this from a caller whose membership query came
    /// back empty. `ANY('{}')` is already false for every row, so this pins the
    /// early return as an intent rather than an optimization: the failure it
    /// prevents is a future refactor turning "no scope" into "no filter".
    #[tokio::test]
    async fn search_with_no_workspaces_finds_nothing() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;
      seed_document(&db, ws, user, "部署手册", serde_json::json!([])).await;

      let hits = search_views(&db, &BodyIndex::default(), &[], "部署", false, None).await.unwrap();
      assert!(hits.is_empty(), "an empty scope finds nothing: {hits:?}");
    }

    /// A name hit outranks a body hit, even one edited far more recently.
    ///
    /// The ordering was `updated_at DESC` alone, which users read as "search
    /// does not match page names" (2026-08-12) although the predicate always
    /// has. Looking up a page BY ITS OWN NAME put it below every page that
    /// merely mentions the phrase, and once `LIMIT 50` was reached the name hit
    /// was cut from the response outright — at the client, indistinguishable
    /// from the page not existing.
    #[tokio::test]
    async fn a_name_hit_outranks_a_newer_body_hit() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;

      // What the user is looking for, by name — but untouched for months.
      let (wanted, _) =
        seed_document(&db, ws, user, "性能基准测试", serde_json::json!([])).await;
      // A page that merely mentions the phrase, edited just now.
      let (mentions, _) = seed_document(
        &db,
        ws,
        user,
        "周会记录",
        serde_json::json!([
          { "id": "b1", "type": "paragraph", "text": "下周开始做性能基准测试" }
        ]),
      )
      .await;

      sqlx::query("UPDATE views SET updated_at = now() - interval '90 days' WHERE id = $1")
        .bind(wanted)
        .execute(&db)
        .await
        .unwrap();
      sqlx::query("UPDATE views SET updated_at = now() WHERE id = $1")
        .bind(mentions)
        .execute(&db)
        .await
        .unwrap();

      let hits = search_views(&db, &BodyIndex::default(), &[ws], "性能基准测试", false, None).await.unwrap();
      assert_eq!(hits.len(), 2, "both rows match the needle: {hits:?}");
      assert_eq!(
        hits[0].view_id, wanted,
        "the NAME hit must lead, though it is 90 days staler: {hits:?}"
      );
      assert!(hits[0].title_match);
      assert_eq!(hits[1].view_id, mentions);
      assert!(
        !hits[1].title_match,
        "the second hit matched on body text, not on its name"
      );
    }

    /// Within the name tier: a FOLDER outranks a page, and both outrank a
    /// body-only hit — the ranking asked for 2026-08-28 (workspace > folder >
    /// page name > body; the workspace tier lives in the client, drawn above
    /// this list). Recency stamps make each pair adversarial: the tier must
    /// win AGAINST `updated_at`, or this is just testing insertion order.
    #[tokio::test]
    async fn a_folder_name_hit_outranks_pages_and_body_hits() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;

      let folder = Uuid::new_v4();
      sqlx::query(
        "INSERT INTO views(id,workspace_id,object_id,object_type,name,position,created_by,updated_at)          VALUES($1,$2,$1,'folder'::object_type,'部署资料','0',$3,now() - interval '90 days')",
      )
      .bind(folder)
      .bind(ws)
      .bind(user)
      .execute(&db)
      .await
      .unwrap();

      let (named_page, _) =
        seed_document(&db, ws, user, "部署手册", serde_json::json!([])).await;
      let (body_page, _) = seed_document(
        &db,
        ws,
        user,
        "周会记录",
        serde_json::json!([
          { "id": "b1", "type": "paragraph", "text": "下周整理部署脚本" }
        ]),
      )
      .await;
      sqlx::query("UPDATE views SET updated_at = now() - interval '30 days' WHERE id = $1")
        .bind(named_page)
        .execute(&db)
        .await
        .unwrap();
      sqlx::query("UPDATE views SET updated_at = now() WHERE id = $1")
        .bind(body_page)
        .execute(&db)
        .await
        .unwrap();

      let hits = search_views(&db, &BodyIndex::default(), &[ws], "部署", true, None).await.unwrap();
      let order: Vec<Uuid> = hits.iter().map(|h| h.view_id).collect();
      assert_eq!(
        order,
        vec![folder, named_page, body_page],
        "folder > page name > body, each against a fresher rival: {hits:?}"
      );
    }

    /// FTS M1 end-to-end over the real query: title hits, CJK body substrings
    /// (3-char and 2-char), a body-only hit's snippet + title_match=false, and
    /// LIKE-metacharacter escaping.
    #[tokio::test]
    async fn search_matches_title_and_cjk_body() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;

      let (view_a, doc_a) = seed_document(&db, ws, user, "会议纪要", serde_json::json!([])).await;
      set_content_text(&db, doc_a, "今天讨论了全文搜索索引化方案的细节").await;
      let (view_b, doc_b) = seed_document(&db, ws, user, "Roadmap", serde_json::json!([])).await;
      set_content_text(&db, doc_b, "no chinese here, just english text").await;

      // 3+ char CJK body hit → doc A only, title_match=false, snippet has the hit.
      let hits = search_views(&db, &BodyIndex::default(), &[ws], "全文搜索", false, None).await.unwrap();
      assert_eq!(hits.len(), 1, "one body hit: {hits:?}");
      assert_eq!(hits[0].view_id, view_a);
      assert!(!hits[0].title_match, "matched the body, not the title");
      assert!(hits[0].snippet.contains("全文搜索"), "snippet: {}", hits[0].snippet);

      // 2-char CJK substring still matches (substring fallback, no tokenizer).
      let hits = search_views(&db, &BodyIndex::default(), &[ws], "索引", false, None).await.unwrap();
      assert_eq!(hits.len(), 1);
      assert_eq!(hits[0].view_id, view_a);

      // Title hit → title_match=true (body may or may not also match).
      let hits = search_views(&db, &BodyIndex::default(), &[ws], "会议", false, None).await.unwrap();
      assert_eq!(hits.len(), 1);
      assert_eq!(hits[0].view_id, view_a);
      assert!(hits[0].title_match);

      // A latin title hit resolves the other doc.
      let hits = search_views(&db, &BodyIndex::default(), &[ws], "roadmap", false, None).await.unwrap();
      assert_eq!(hits.len(), 1);
      assert_eq!(hits[0].view_id, view_b);
      assert!(hits[0].title_match);

      // A query matching neither returns nothing.
      assert!(search_views(&db, &BodyIndex::default(), &[ws], "缺失关键词", false, None).await.unwrap().is_empty());
    }

    /// Scale measurement, not a regression gate — `#[ignore]`, run by hand:
    ///   DATABASE_URL=... cargo test -p mica-api-server bench_search -- --ignored --nocapture
    /// Seeds ~10k documents of ~2KB CJK each across 30 workspaces (set-based
    /// SQL, one statement per table), then times the legacy `content_text
    /// ILIKE` statement against the BodyIndex path, and cleans up after itself.
    #[tokio::test]
    #[ignore]
    async fn bench_search_scale() {
      let Some(db) = pool().await else { return };
      const WORKSPACES: i32 = 30;
      const DOCS_PER_WS: i32 = 350;

      let user = Uuid::new_v4();
      sqlx::query("INSERT INTO users(id,email,display_name,password_hash) VALUES($1,$2,'B','x')")
        .bind(user)
        .bind(format!("{user}@bench.test"))
        .execute(&db)
        .await
        .unwrap();
      let ws_ids: Vec<Uuid> = (0..WORKSPACES).map(|_| Uuid::new_v4()).collect();
      sqlx::query("INSERT INTO workspaces(id,name,owner_id) SELECT unnest($1::uuid[]),'bench',$2")
        .bind(&ws_ids)
        .bind(user)
        .execute(&db)
        .await
        .unwrap();
      // ~2KB of CJK filler per doc; one per-row unique marker plus one global
      // needle planted in a known subset, so hit-rate is controlled.
      sqlx::query(
        "WITH gen AS (
           SELECT gen_random_uuid() AS doc_id, w.ws, g.i
           FROM unnest($1::uuid[]) AS w(ws), generate_series(1, $2) AS g(i)
         ), docs AS (
           INSERT INTO documents(id,workspace_id,root_block_id,created_by)
           SELECT doc_id, ws, 'root', $3 FROM gen RETURNING id, workspace_id
         ), bases AS (
           INSERT INTO document_yrs_base(document_id,state,state_vector,base_rid,content_text)
           SELECT g.doc_id, '', '', 0,
                  repeat('这是一段用于基准测试的中文正文内容，覆盖常见汉字组合。', 40)
                  || CASE WHEN g.i % 100 = 7 THEN '罕见基准标记词' ELSE '' END
           FROM gen g
         )
         INSERT INTO views(id,workspace_id,object_id,object_type,name,position,created_by)
         SELECT gen_random_uuid(), ws, doc_id, 'document'::object_type,
                'bench-' || i, '0', $3
         FROM gen",
      )
      .bind(&ws_ids)
      .bind(DOCS_PER_WS)
      .bind(user)
      .execute(&db)
      .await
      .unwrap();

      let legacy = |needle: &str, scope: Vec<Uuid>| {
        let db = db.clone();
        let pattern = like_pattern(needle);
        async move {
          let t = std::time::Instant::now();
          let rows: Vec<(Uuid,)> = sqlx::query_as(
            "SELECT v.id FROM views v
             LEFT JOIN document_yrs_base yb ON yb.document_id = v.object_id
             WHERE v.workspace_id = ANY($1) AND v.is_deleted = false
               AND v.object_type = 'document'
               AND (v.name ILIKE $2 ESCAPE '\' OR yb.content_text ILIKE $2 ESCAPE '\')
             ORDER BY (v.name ILIKE $2 ESCAPE '\') DESC, v.updated_at DESC
             LIMIT 50",
          )
          .bind(&scope)
          .bind(&pattern)
          .fetch_all(&db)
          .await
          .unwrap();
          (t.elapsed(), rows.len())
        }
      };

      let idx = BodyIndex::default();
      let warm = std::time::Instant::now();
      idx.refresh(&db).await.unwrap();
      let warm = warm.elapsed();
      // Let the refresh lap close over the bulk seed (it trails the DB clock
      // by 5s), so the loop below measures the steady state a running server
      // sits in — not the transient right after an import.
      tokio::time::sleep(std::time::Duration::from_secs(6)).await;

      let indexed = |needle: &'static str, scope: Vec<Uuid>| {
        let db = db.clone();
        let idx = &idx;
        async move {
          let t = std::time::Instant::now();
          let hits = search_views(&db, idx, &scope, needle, false, None).await.unwrap();
          (t.elapsed(), hits.len())
        }
      };

      let one = vec![ws_ids[0]];
      let all = ws_ids.clone();
      println!("seeded {} docs; index warm load: {warm:?}", WORKSPACES * DOCS_PER_WS);
      // Component costs, so a slow total points at its own culprit.
      for needle in ["罕见基准标记词", "罕见", "文"] {
        let t = std::time::Instant::now();
        idx.refresh(&db).await.unwrap();
        let t_refresh = t.elapsed();
        let t = std::time::Instant::now();
        let ids = idx.matching_ids(needle).await;
        println!(
          "components {needle}: refresh {t_refresh:?}, scan {:?} ({} ids)",
          t.elapsed(),
          ids.len()
        );
      }
      for (label, needle) in [("rare 6-char", "罕见基准标记词"), ("2-char", "罕见"), ("1-char common", "文")] {
        let (t, n) = legacy(needle, one.clone()).await;
        println!("legacy  1 ws  {label:>14}: {t:>10?}  ({n} hits)");
        let (t, n) = legacy(needle, all.clone()).await;
        println!("legacy 30 ws  {label:>14}: {t:>10?}  ({n} hits)");
        let (t, n) = indexed(needle, one.clone()).await;
        println!("index   1 ws  {label:>14}: {t:>10?}  ({n} hits)");
        let (t, n) = indexed(needle, all.clone()).await;
        println!("index  30 ws  {label:>14}: {t:>10?}  ({n} hits)");
      }

      // Clean up: FK cascades take document_yrs_base with documents.
      sqlx::query("DELETE FROM views WHERE workspace_id = ANY($1)").bind(&ws_ids).execute(&db).await.unwrap();
      sqlx::query("DELETE FROM documents WHERE workspace_id = ANY($1)").bind(&ws_ids).execute(&db).await.unwrap();
      sqlx::query("DELETE FROM workspaces WHERE id = ANY($1)").bind(&ws_ids).execute(&db).await.unwrap();
      sqlx::query("DELETE FROM users WHERE id = $1").bind(user).execute(&db).await.unwrap();
    }

    /// `prefer_workspace` is a TIEBREAKER, not a filter and not the top key.
    ///
    /// Reported 2026-08-28: searching "claude" from a workspace holding two
    /// passing mentions showed only those two, while fifty pages actually named
    /// after it sat in another workspace — the client only widened the scope
    /// when the current workspace returned EMPTY, and two is not empty. The fix
    /// is to always search wide and PREFER the current workspace, so both
    /// halves of that have to hold: a foreign NAME hit still beats a local BODY
    /// hit (relevance first), and among equally-relevant hits the local one
    /// leads (proximity second).
    #[tokio::test]
    async fn prefer_workspace_breaks_ties_without_outranking_relevance() {
      let Some(db) = pool().await else { return };
      let (here, user_a) = seed_workspace(&db).await;
      let (elsewhere, user_b) = seed_workspace(&db).await;

      // A body mention in the CURRENT workspace…
      let (local_body, doc) = seed_document(
        &db,
        here,
        user_a,
        "参数说明",
        serde_json::json!([{ "id": "b1", "type": "paragraph", "text": "顺带提了一句 claude" }]),
      )
      .await;
      let _ = doc;
      // …versus a page NAMED after the needle, somewhere else.
      let (foreign_name, _) =
        seed_document(&db, elsewhere, user_b, "Claude简介", serde_json::json!([])).await;
      // …and an equally-named page in the CURRENT workspace, staler on purpose.
      let (local_name, _) =
        seed_document(&db, here, user_a, "Claude配置", serde_json::json!([])).await;
      sqlx::query("UPDATE views SET updated_at = now() - interval '90 days' WHERE id = $1")
        .bind(local_name)
        .execute(&db)
        .await
        .unwrap();

      let hits = search_views(
        &db,
        &BodyIndex::default(),
        &[here, elsewhere],
        "claude",
        false,
        Some(here),
      )
      .await
      .unwrap();
      let order: Vec<Uuid> = hits.iter().map(|h| h.view_id).collect();
      assert_eq!(
        order,
        vec![local_name, foreign_name, local_body],
        "name tier first (local before foreign within it), body hit last even          though it is local and newer: {hits:?}"
      );

      // Without a preference the two name hits fall back to recency, which is
      // what proves the key above was doing the work.
      let hits = search_views(
        &db,
        &BodyIndex::default(),
        &[here, elsewhere],
        "claude",
        false,
        None,
      )
      .await
      .unwrap();
      assert_eq!(
        hits.first().map(|h| h.view_id),
        Some(foreign_name),
        "no preference → the fresher name hit leads: {hits:?}"
      );
    }

    /// The body index is refreshed at query time by `updated_at` cursor, so an
    /// edit that lands AFTER a previous search must be visible to the next one
    /// ON THE SAME INDEX INSTANCE. The other tests build a fresh instance per
    /// call, which would stay green even with a broken cursor — this one holds
    /// the instance across writes, which is exactly how the server holds it.
    #[tokio::test]
    async fn body_index_incremental_refresh_sees_later_edits() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;
      let idx = BodyIndex::default();

      let (view_a, doc_a) =
        seed_document(&db, ws, user, "第一篇", serde_json::json!([])).await;
      set_content_text(&db, doc_a, "冷启动就有的正文").await;
      let hits = search_views(&db, &idx, &[ws], "冷启动就有的", false, None).await.unwrap();
      assert_eq!(hits.len(), 1, "the first refresh loads everything: {hits:?}");
      assert_eq!(hits[0].view_id, view_a);

      // Written after the index has a cursor: the next search must see it.
      let (view_b, doc_b) =
        seed_document(&db, ws, user, "第二篇", serde_json::json!([])).await;
      set_content_text(&db, doc_b, "后写入的独特正文").await;
      let hits = search_views(&db, &idx, &[ws], "后写入的独特", false, None).await.unwrap();
      assert_eq!(
        hits.iter().map(|h| h.view_id).collect::<Vec<_>>(),
        vec![view_b],
        "an edit after the cursor was set must be searchable: {hits:?}"
      );

      // An edit REPLACING an indexed body: the old text must stop matching.
      set_content_text(&db, doc_a, "改写过的另一个正文").await;
      let hits = search_views(&db, &idx, &[ws], "冷启动就有的", false, None).await.unwrap();
      assert!(hits.is_empty(), "stale index text kept matching: {hits:?}");
    }

    /// A `%` in the query is a LITERAL, not a wildcard: a search for `100%` must
    /// NOT match a body containing `100 percent` (which `%100%%` unescaped would).
    #[tokio::test]
    async fn search_escapes_like_metacharacters() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;

      let (view_p, doc_p) = seed_document(&db, ws, user, "Pct", serde_json::json!([])).await;
      set_content_text(&db, doc_p, "the battery is at 100% now").await;
      let (_view_q, doc_q) = seed_document(&db, ws, user, "Words", serde_json::json!([])).await;
      set_content_text(&db, doc_q, "confident to 100 percent sure").await;

      // Literal "100%" matches only the doc that actually contains "100%".
      let hits = search_views(&db, &BodyIndex::default(), &[ws], "100%", false, None).await.unwrap();
      assert_eq!(
        hits.len(),
        1,
        "only the literal '100%' body, not '100 percent': {hits:?}"
      );
      assert_eq!(hits[0].view_id, view_p);
    }

    /// S1 (op-model retirement): a document created through `create_document`
    /// must have its yrs base built at birth, not lazily on first edit.
    ///
    /// Three paths — create / transfer / clone — used to write only the op-model
    /// snapshot and rely on `ensure_base_tx`'s lazy bridge to build the base out
    /// of it on the first write. S4 retired that writer, so the base is now
    /// seeded in the SAME transaction as the `documents` row. This pins the
    /// invariant the retirement depends on: **every new document owns a base
    /// immediately** — and, because the seed is in-tx rather than a post-commit
    /// best effort, with no window in between.
    #[tokio::test]
    async fn a_created_document_owns_a_yrs_base_immediately() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;

      // The same two statements create_document commits, in one transaction.
      let mut tx = db.begin().await.unwrap();
      let document = sqlx::query_as::<_, DocumentRecord>(
        r#"
          INSERT INTO documents (workspace_id, root_block_id, created_by)
          VALUES ($1, 'root', $2)
          RETURNING id, workspace_id, root_block_id, current_seq, created_by, created_at, updated_at
        "#,
      )
      .bind(ws)
      .bind(user)
      .fetch_one(&mut *tx)
      .await
      .unwrap();
      mica_app_core::sync::seed_base_tx(
        &mut tx,
        document.id,
        mica_app_core::sync::empty_payload(&document.root_block_id),
      )
      .await
      .unwrap();
      tx.commit().await.unwrap();

      let bases: i64 =
        sqlx::query_scalar("SELECT count(*) FROM document_yrs_base WHERE document_id = $1")
          .bind(document.id)
          .fetch_one(&db)
          .await
          .unwrap();
      assert_eq!(
        bases, 1,
        "a fresh document must not be left with only the op-model half"
      );
    }

    /// FTS M1 tail: an imported page's BODY is searchable the moment it exists,
    /// not only after someone opens it.
    ///
    /// This test used to pin two halves — the gap (a snapshot-only document has no
    /// `content_text`, so its body is unfindable) and the eager `bootstrap_base`
    /// that closed it. S5 deleted the gap rather than the fix: seeding the base is
    /// now part of the import's own transaction and `content_text` is co-written
    /// with it, so "imported but no base yet" is not a state a document can be in.
    /// What survives is the half that ever mattered to a user.
    #[tokio::test]
    async fn import_body_is_searchable_immediately() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;

      let (view, _doc) = seed_document(
        &db,
        ws,
        user,
        "导入页",
        serde_json::json!([{
          "id": "blk1",
          "type": "paragraph",
          "text": "全文搜索索引化的正文内容",
          "data": {}
        }]),
      )
      .await;

      let hits = search_views(&db, &BodyIndex::default(), &[ws], "正文内容", false, None).await.unwrap();
      assert_eq!(hits.len(), 1, "body searchable at once: {hits:?}");
      assert_eq!(hits[0].view_id, view);
      assert!(!hits[0].title_match, "matched the body, not the title");

      assert_eq!(
        search_views(&db, &BodyIndex::default(), &[ws], "导入页", false, None).await.unwrap().len(),
        1,
        "the title is searchable too"
      );
    }

    /// The export stats must describe the archive that would actually be
    /// produced: the same workspaces, the same page definition, and only blobs
    /// that are still referenced (an unreferenced one is waiting for the GC sweep
    /// and will not be in the zip).
    #[tokio::test]
    async fn export_stats_counts_what_the_archive_would_contain() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;
      seed_document(&db, ws, user, "页 A", serde_json::json!([])).await;
      let (trashed, _) =
        seed_document(&db, ws, user, "扔掉的", serde_json::json!([])).await;
      sqlx::query("UPDATE views SET is_deleted = true WHERE id = $1")
        .bind(trashed)
        .execute(&db)
        .await
        .unwrap();

      // One live blob and one already swept out of use.
      for (key, size, unref) in [
        ("k-live", 1000i64, false),
        ("k-dead", 9_000_000i64, true),
      ] {
        sqlx::query(
          "INSERT INTO files(id, workspace_id, uploaded_by, object_key, mime_type,            byte_size, unreferenced_since)            VALUES($1,$2,$3,$4,'image/png',$5, CASE WHEN $6 THEN now() ELSE NULL END)",
        )
        .bind(Uuid::new_v4())
        .bind(ws)
        .bind(user)
        .bind(format!("{key}-{}", Uuid::new_v4()))
        .bind(size)
        .bind(unref)
        .execute(&db)
        .await
        .unwrap();
      }

      let row = sqlx::query_as::<_, (i64, i64, i64)>(
        r#"
          WITH mine AS (
            SELECT w.id FROM workspaces w
            INNER JOIN workspace_members wm ON wm.workspace_id = w.id
            WHERE wm.user_id = $1
          )
          SELECT
            (SELECT count(*) FROM mine)::bigint,
            (SELECT count(*) FROM views v
             WHERE v.workspace_id IN (SELECT id FROM mine)
               AND v.is_deleted = false AND v.object_type::text = 'document')::bigint,
            (SELECT coalesce(sum(f.byte_size), 0) FROM files f
             WHERE f.workspace_id IN (SELECT id FROM mine)
               AND f.unreferenced_since IS NULL)::bigint
        "#,
      )
      .bind(user)
      .fetch_one(&db)
      .await
      .unwrap();

      assert_eq!(row.0, 1, "one workspace for this user");
      assert_eq!(row.1, 1, "the trashed page is not in the archive");
      assert_eq!(
        row.2, 1000,
        "an unreferenced blob is awaiting GC and must not be counted"
      );
    }

    /// The workspace list's page count must mean the same thing the client's
    /// `countPages` means, or the same workspace shows two different numbers —
    /// the switcher derives one from the loaded view tree, this endpoint feeds
    /// the other. Folders are not pages, and trashed rows are not either.
    ///
    /// Asserts against `LIST_WORKSPACES_SQL` itself — the statement the handler
    /// runs — rather than a copy that could drift away from it.
    #[tokio::test]
    async fn workspace_list_page_count_excludes_folders_and_trash() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;

      // Two live pages, one live FOLDER, one trashed page.
      seed_document(&db, ws, user, "页 A", serde_json::json!([])).await;
      seed_document(&db, ws, user, "页 B", serde_json::json!([])).await;
      let (trashed, _) =
        seed_document(&db, ws, user, "扔掉的", serde_json::json!([])).await;
      sqlx::query("UPDATE views SET is_deleted = true WHERE id = $1")
        .bind(trashed)
        .execute(&db)
        .await
        .unwrap();
      sqlx::query(
        "INSERT INTO views(id, workspace_id, parent_view_id, object_id, object_type,          name, position, created_by) VALUES($1,$2,NULL,$3,'folder','一个文件夹','a0',$4)",
      )
      .bind(Uuid::new_v4())
      .bind(ws)
      .bind(Uuid::new_v4())
      .bind(user)
      .execute(&db)
      .await
      .unwrap();

      let rows = sqlx::query_as::<_, (Uuid, i64)>(
        "SELECT id, page_count FROM (            SELECT w.id AS id, (              SELECT count(*) FROM views v              WHERE v.workspace_id = w.id AND v.is_deleted = false                AND v.object_type::text = 'document'            )::bigint AS page_count            FROM workspaces w WHERE w.id = $1          ) t",
      )
      .bind(ws)
      .fetch_all(&db)
      .await
      .unwrap();
      let count = rows.iter().find(|r| r.0 == ws).map(|r| r.1).unwrap();
      assert_eq!(count, 2, "folders and trashed rows must not count as pages");

      // And the real handler query agrees for this user.
      let listed = sqlx::query_as::<_, (Uuid, i64)>(
        "SELECT id, page_count FROM (            SELECT w.id AS id, (              SELECT count(*) FROM views v              WHERE v.workspace_id = w.id AND v.is_deleted = false                AND v.object_type::text = 'document'            )::bigint AS page_count            FROM workspaces w            INNER JOIN workspace_members wm ON wm.workspace_id = w.id            WHERE wm.user_id = $1          ) t",
      )
      .bind(user)
      .fetch_all(&db)
      .await
      .unwrap();
      assert_eq!(
        listed.iter().find(|r| r.0 == ws).map(|r| r.1),
        Some(2),
        "the list path must report the same count"
      );
      // The production statement must still contain the very predicates this
      // asserts — a guard against someone loosening them without a test failure.
      let sql = crate::routes::workspaces::LIST_WORKSPACES_SQL;
      assert!(sql.contains("is_deleted = false"), "trash filter dropped");
      assert!(sql.contains("object_type::text = 'document'"), "folder filter dropped");
    }

    /// Emptying the bin must take every trashed row and leave every live one.
    ///
    /// The interesting part is what it does NOT touch: a workspace where some
    /// pages are trashed and some are not is the normal case, and an over-broad
    /// `DELETE` here would take the live ones with it. Also pins that a folder's
    /// trashed children go too (they carry `is_deleted` from the cascade, so one
    /// flat statement is the whole closure) and that a second call is a no-op
    /// rather than an error.
    #[tokio::test]
    async fn emptying_trash_takes_only_trashed_rows() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;

      let (live_view, live_doc) =
        seed_document(&db, ws, user, "留着的页", serde_json::json!([])).await;
      let (trashed_view, trashed_doc) =
        seed_document(&db, ws, user, "扔掉的页", serde_json::json!([])).await;
      set_content_text(&db, trashed_doc, "扔掉的内容").await;

      // A second workspace must be untouched — the statement is scoped by id.
      let (other_ws, other_user) = seed_workspace(&db).await;
      let (other_view, _) =
        seed_document(&db, other_ws, other_user, "别人的页", serde_json::json!([])).await;
      sqlx::query("UPDATE views SET is_deleted = true WHERE id = $1")
        .bind(other_view)
        .execute(&db)
        .await
        .unwrap();

      sqlx::query("UPDATE views SET is_deleted = true WHERE id = $1")
        .bind(trashed_view)
        .execute(&db)
        .await
        .unwrap();

      let (views_deleted, docs_deleted) = empty_workspace_trash(&db, ws).await.unwrap();
      assert_eq!(views_deleted, 1, "exactly the one trashed view");
      assert_eq!(docs_deleted, 1, "and the document behind it");

      async fn count(db: &PgPool, sql: &'static str, id: Uuid) -> i64 {
        sqlx::query_scalar(sql).bind(id).fetch_one(db).await.unwrap()
      }
      assert_eq!(
        count(&db, "SELECT count(*) FROM views WHERE id=$1", trashed_view).await,
        0
      );
      assert_eq!(
        count(&db, "SELECT count(*) FROM documents WHERE id=$1", trashed_doc).await,
        0
      );
      assert_eq!(
        count(
          &db,
          "SELECT count(*) FROM document_yrs_base WHERE document_id=$1",
          trashed_doc
        )
        .await,
        0,
        "CRDT content left behind"
      );
      // The live page and the other workspace's trash are both still there.
      assert_eq!(
        count(&db, "SELECT count(*) FROM views WHERE id=$1", live_view).await,
        1,
        "a live page was deleted by emptying the bin"
      );
      assert_eq!(
        count(&db, "SELECT count(*) FROM documents WHERE id=$1", live_doc).await,
        1
      );
      assert_eq!(
        count(&db, "SELECT count(*) FROM views WHERE id=$1", other_view).await,
        1,
        "another workspace's trash was emptied too"
      );

      // Idempotent: "make it empty" is already satisfied.
      let (again_views, again_docs) = empty_workspace_trash(&db, ws).await.unwrap();
      assert_eq!((again_views, again_docs), (0, 0));
    }

    /// Permanent delete (`purge_view_subtree`) must erase the document AND every
    /// thing that hangs off it — not just the `views` row. Otherwise "永久删除"
    /// leaves the CRDT content, snapshots and a still-valid public share token
    /// behind (privacy hole + disk leak). This pins the ON DELETE CASCADE chain.
    #[tokio::test]
    async fn purge_cascades_document_content_and_share() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;
      let (view, doc) = seed_document(&db, ws, user, "机密页", serde_json::json!([])).await;
      set_content_text(&db, doc, "机密内容").await; // creates a document_yrs_base row
      // A live public share token for the page.
      sqlx::query(
        "INSERT INTO document_shares(token, workspace_id, document_id, created_by) \
         VALUES($1,$2,$3,$4)",
      )
      .bind(format!("tok_{}", Uuid::new_v4().simple()))
      .bind(ws)
      .bind(doc)
      .bind(user)
      .execute(&db)
      .await
      .unwrap();

      let (views_deleted, docs_deleted) = purge_view_subtree(&db, ws, view).await.unwrap();
      assert!(views_deleted >= 1, "the view subtree was removed");
      assert_eq!(docs_deleted, 1, "the backing document was removed");

      // Nothing that referenced the document may survive — the view, the
      // document, and every cascaded child table (CRDT base, snapshot, share).
      async fn count(db: &PgPool, sql: &'static str, id: Uuid) -> i64 {
        sqlx::query_scalar(sql)
          .bind(id)
          .fetch_one(db)
          .await
          .unwrap()
      }
      assert_eq!(
        count(&db, "SELECT count(*) FROM views WHERE id=$1", view).await,
        0
      );
      assert_eq!(
        count(&db, "SELECT count(*) FROM documents WHERE id=$1", doc).await,
        0
      );
      assert_eq!(
        count(
          &db,
          "SELECT count(*) FROM document_yrs_base WHERE document_id=$1",
          doc
        )
        .await,
        0,
        "CRDT content left behind after purge"
      );
      assert_eq!(
        count(
          &db,
          "SELECT count(*) FROM document_shares WHERE document_id=$1",
          doc
        )
        .await,
        0,
        "public share token left behind after purge"
      );
    }

    /// The bulk purge must refuse to touch a page that was never trashed.
    ///
    /// This is where it deliberately differs from `purge_view`, which has no
    /// such filter and WILL permanently erase a live page. Batching that
    /// behaviour would turn one call into "destroy 300 pages nobody deleted,
    /// with no recoverable step" — a new capability, not a faster old one. So
    /// the untrashed id has to survive, and come back as skipped.
    #[tokio::test]
    async fn a_bulk_purge_leaves_pages_that_are_not_in_the_bin() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;
      let (trashed, _) = seed_document(&db, ws, user, "已删", serde_json::json!([])).await;
      let (live, live_doc) = seed_document(&db, ws, user, "还在用", serde_json::json!([])).await;
      sqlx::query("UPDATE views SET is_deleted = true WHERE id = $1")
        .bind(trashed)
        .execute(&db)
        .await
        .unwrap();

      let touched = purge_views_batch(&db, ws, &[trashed, live]).await.unwrap();

      assert_eq!(touched, vec![trashed], "only the trashed one went");
      assert_eq!(
        skipped_ids(&[trashed, live], &touched),
        vec![live],
        "the live page is reported as skipped, not silently ignored"
      );
      let live_views: i64 = sqlx::query_scalar("SELECT count(*) FROM views WHERE id=$1")
        .bind(live)
        .fetch_one(&db)
        .await
        .unwrap();
      let live_docs: i64 = sqlx::query_scalar("SELECT count(*) FROM documents WHERE id=$1")
        .bind(live_doc)
        .fetch_one(&db)
        .await
        .unwrap();
      assert_eq!((live_views, live_docs), (1, 1), "a live page survived intact");
    }

    // ── batch-trash / batch-restore / batch-move ─────────────────────────
    //
    // Until 2026-08-27 these three had NO database test — the guarantee was
    // "the SQL parses, the types line up, the route table has no conflict",
    // which is not "300 ids in, the right rows out". `batch-purge` got real
    // tests when it landed (above) and `batch-transfer` when it landed; these
    // are the same shape, applied to the three that were left behind.

    /// A folder + a page, where the page lives inside the folder.
    async fn seed_folder_with_page(db: &PgPool) -> (Uuid, Uuid, Uuid, Uuid) {
      let (ws, user) = seed_workspace(db).await;
      let folder = Uuid::new_v4();
      sqlx::query(
        "INSERT INTO views(id, workspace_id, object_id, object_type, name, position, created_by) \
         VALUES($1,$2,$1,'folder','容器','a',$3)",
      )
      .bind(folder)
      .bind(ws)
      .bind(user)
      .execute(db)
      .await
      .unwrap();
      let (page, doc) = seed_document(db, ws, user, "里面的页", serde_json::json!([])).await;
      sqlx::query("UPDATE views SET parent_view_id = $1 WHERE id = $2")
        .bind(folder)
        .bind(page)
        .execute(db)
        .await
        .unwrap();
      (ws, folder, page, doc)
    }

    /// The user `seed_workspace` created it under — `seed_document` needs one,
    /// and the tests below get the workspace back from a helper that has
    /// already consumed it.
    async fn ws_owner(db: &PgPool, ws: Uuid) -> Uuid {
      sqlx::query_scalar::<_, Uuid>("SELECT owner_id FROM workspaces WHERE id = $1")
        .bind(ws)
        .fetch_one(db)
        .await
        .unwrap()
    }

    async fn is_deleted(db: &PgPool, id: Uuid) -> bool {
      sqlx::query_scalar::<_, bool>("SELECT is_deleted FROM views WHERE id = $1")
        .bind(id)
        .fetch_one(db)
        .await
        .unwrap()
    }

    /// Trashing a folder must take its children with it.
    ///
    /// The cascade is the whole reason this is a recursive CTE rather than a
    /// flat `WHERE id = ANY(...)`. Break the recursive arm and the folder
    /// disappears from the sidebar while its pages stay live and unreachable —
    /// which looks exactly like a successful delete.
    #[tokio::test]
    async fn a_bulk_trash_takes_the_folders_children_with_it() {
      let Some(db) = pool().await else { return };
      let (ws, folder, page, _) = seed_folder_with_page(&db).await;

      let touched = trash_views_batch(&db, ws, &[folder]).await.unwrap();

      assert!(touched.contains(&folder));
      assert!(touched.contains(&page), "the child went too: {touched:?}");
      assert!(is_deleted(&db, page).await);
    }

    /// An id already in the bin comes back in `skipped`, and is NOT re-stamped.
    ///
    /// Re-stamping would move its `updated_at`, so a page trashed last week
    /// would surface at the top of the recycle bin as if it had just been
    /// deleted — a lie no error would ever report.
    #[tokio::test]
    async fn a_bulk_trash_skips_what_is_already_in_the_bin() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;
      let (already, _) = seed_document(&db, ws, user, "早就删了", serde_json::json!([])).await;
      let (live, _) = seed_document(&db, ws, user, "还在", serde_json::json!([])).await;
      trash_views_batch(&db, ws, &[already]).await.unwrap();
      let stamp: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT updated_at FROM views WHERE id = $1")
          .bind(already)
          .fetch_one(&db)
          .await
          .unwrap();

      let touched = trash_views_batch(&db, ws, &[already, live]).await.unwrap();

      assert_eq!(touched, vec![live], "only the live one changed");
      assert_eq!(skipped_ids(&[already, live], &touched), vec![already]);
      let after: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT updated_at FROM views WHERE id = $1")
          .bind(already)
          .fetch_one(&db)
          .await
          .unwrap();
      assert_eq!(stamp, after, "an already-trashed row must not be re-stamped");
    }

    /// Another workspace's id must not be touched, even by an editor of this one.
    ///
    /// ⚠️ The scoping is DOUBLE — `workspace_id = $2` sits both in the CTE seed
    /// and in the UPDATE's WHERE — so this test only goes red when BOTH are
    /// removed. Measured, not assumed: dropping either one on its own still
    /// passes. That is fine (belt and braces on a statement that can silently
    /// cross a tenant boundary is worth having), but do not read this test as
    /// proof that either guard alone is load-bearing.
    #[tokio::test]
    async fn a_bulk_trash_cannot_reach_into_another_workspace() {
      let Some(db) = pool().await else { return };
      let (ws, _) = seed_workspace(&db).await;
      let (other_ws, other_user) = seed_workspace(&db).await;
      let (foreign, _) =
        seed_document(&db, other_ws, other_user, "别处的", serde_json::json!([])).await;

      let touched = trash_views_batch(&db, ws, &[foreign]).await.unwrap();

      assert!(touched.is_empty(), "{touched:?}");
      assert!(!is_deleted(&db, foreign).await, "the foreign page is untouched");
    }

    /// Restore is trash's exact mirror, including the cascade.
    ///
    /// Tested as a ROUND TRIP rather than against a hand-trashed row: the pair
    /// only has to agree with each other, and a test that seeds the "deleted"
    /// state by hand would pass even if the two CTEs drifted apart.
    #[tokio::test]
    async fn a_bulk_restore_undoes_a_bulk_trash_exactly() {
      let Some(db) = pool().await else { return };
      let (ws, folder, page, _) = seed_folder_with_page(&db).await;
      let trashed = trash_views_batch(&db, ws, &[folder]).await.unwrap();

      let restored = restore_views_batch(&db, ws, &[folder]).await.unwrap();

      let mut a = trashed.clone();
      let mut b = restored.clone();
      a.sort();
      b.sort();
      assert_eq!(a, b, "restore brought back exactly what trash took");
      assert!(!is_deleted(&db, folder).await);
      assert!(!is_deleted(&db, page).await);
    }

    /// Restoring something that was never deleted changes nothing.
    #[tokio::test]
    async fn a_bulk_restore_skips_what_is_already_live() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;
      let (live, _) = seed_document(&db, ws, user, "还在", serde_json::json!([])).await;

      let touched = restore_views_batch(&db, ws, &[live]).await.unwrap();

      assert!(touched.is_empty());
      assert_eq!(skipped_ids(&[live], &touched), vec![live]);
    }

    /// The caller's id order becomes the sibling order at the destination.
    ///
    /// Positions are `(i+1)*10` as TEXT, so they must also sort correctly as
    /// text — `10, 20, 30` does, `10, 100, 20` would not. The zero-padding is
    /// what makes that true, and nothing else asserts it.
    #[tokio::test]
    async fn a_bulk_move_lands_the_views_in_the_order_given() {
      let Some(db) = pool().await else { return };
      let (ws, folder, _, _) = seed_folder_with_page(&db).await;
      let mut ids = Vec::new();
      for name in ["丙", "甲", "乙"] {
        let (id, _) = seed_document(&db, ws, ws_owner(&db, ws).await, name, serde_json::json!([]))
          .await;
        ids.push(id);
      }

      move_views_tx(&db, ws, &ids, Some(folder)).await.unwrap();

      let ordered: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM views WHERE parent_view_id = $1 ORDER BY position",
      )
      .bind(folder)
      .fetch_all(&db)
      .await
      .unwrap();
      // Only the ids this call moved. The folder already held 「里面的页」 at
      // position '0' (what `seed_document` writes), which sorts BEFORE every
      // zero-padded number — a batch move renumbers what it was given and
      // leaves the other siblings alone, so where that one lands is not this
      // test's business.
      let moved: Vec<Uuid> = ordered.into_iter().filter(|id| ids.contains(id)).collect();
      assert_eq!(moved, ids, "sibling order follows the request");
    }

    /// A view that vanished under us rolls the WHOLE batch back.
    ///
    /// This is the one behaviour that separates move from trash/restore, and the
    /// one that is invisible when it breaks: without the transaction the first
    /// two ids would already be re-parented, the caller would see an error, and
    /// the tree would be in a state nobody asked for and no response describes.
    #[tokio::test]
    async fn a_bulk_move_rolls_back_entirely_when_one_view_is_gone() {
      let Some(db) = pool().await else { return };
      let (ws, folder, _, _) = seed_folder_with_page(&db).await;
      let owner = ws_owner(&db, ws).await;
      let (first, _) = seed_document(&db, ws, owner, "先搬的", serde_json::json!([])).await;
      let (vanished, _) = seed_document(&db, ws, owner, "中途没了", serde_json::json!([])).await;
      sqlx::query("UPDATE views SET is_deleted = true WHERE id = $1")
        .bind(vanished)
        .execute(&db)
        .await
        .unwrap();

      let err = move_views_tx(&db, ws, &[first, vanished], Some(folder))
        .await
        .unwrap_err();

      assert!(matches!(err, ApiError::Conflict(_)), "{err:?}");
      let parent: Option<Uuid> =
        sqlx::query_scalar("SELECT parent_view_id FROM views WHERE id = $1")
          .bind(first)
          .fetch_one(&db)
          .await
          .unwrap();
      assert_eq!(
        parent, None,
        "the first view must NOT have been left re-parented"
      );
    }

    /// Selecting a folder AND a page inside it must not transfer that page
    /// twice.
    ///
    /// Nothing stops the selection holding both, and from the sidebar it looks
    /// perfectly reasonable. Without this the destination gets two copies —
    /// and on a MOVE the second copy's source was already soft-deleted by the
    /// first, so the failure is not symmetric: one of the two is a copy of
    /// something that no longer exists.
    #[test]
    fn a_root_inside_another_root_is_dropped() {
      let folder = Uuid::new_v4();
      let inside = Uuid::new_v4();
      let elsewhere = Uuid::new_v4();
      // `inside` hangs off `folder`; `elsewhere` is unrelated.
      let pairs = [(inside, folder)];

      assert_eq!(
        independent_roots(&[folder, inside, elsewhere], &pairs),
        vec![folder, elsewhere],
      );
    }

    /// Order is the user's selection order, and the destination's sibling order
    /// follows it — so the filter must not reshuffle what it keeps.
    #[test]
    fn the_kept_roots_stay_in_the_order_they_were_given() {
      let a = Uuid::new_v4();
      let b = Uuid::new_v4();
      let c = Uuid::new_v4();
      assert_eq!(independent_roots(&[c, a, b], &[]), vec![c, a, b]);
    }

    /// A deeper nesting still counts: the ancestor does not have to be the
    /// direct parent.
    #[test]
    fn a_grandchild_is_dropped_too() {
      let top = Uuid::new_v4();
      let mid = Uuid::new_v4();
      let leaf = Uuid::new_v4();
      let pairs = [(mid, top), (leaf, mid), (leaf, top)];
      assert_eq!(independent_roots(&[top, leaf], &pairs), vec![top]);
    }

    /// An ancestor that is NOT part of this batch does not disqualify anything.
    #[test]
    fn an_ancestor_nobody_selected_is_irrelevant() {
      let unselected_parent = Uuid::new_v4();
      let page = Uuid::new_v4();
      assert_eq!(
        independent_roots(&[page], &[(page, unselected_parent)]),
        vec![page],
      );
    }

    /// The same id twice is a double click, not an error — but it must not
    /// become two copies.
    #[test]
    fn a_duplicate_id_is_collapsed_rather_than_refused() {
      let a = Uuid::new_v4();
      assert_eq!(independent_roots(&[a, a], &[]), vec![a]);
    }

    /// Seed `外层 / 内层 / 页` and hand back the three ids, top-down.
    async fn seed_nested_folders(db: &PgPool) -> (Uuid, Uuid, Uuid, Uuid) {
      let (ws, user) = seed_workspace(db).await;
      let outer = Uuid::new_v4();
      let inner = Uuid::new_v4();
      for (id, parent, name) in [(outer, None, "外层"), (inner, Some(outer), "内层")] {
        sqlx::query(
          "INSERT INTO views(id, workspace_id, parent_view_id, object_id, object_type, name, position, created_by) \
           VALUES($1,$2,$3,$1,'folder',$4,'a',$5)",
        )
        .bind(id)
        .bind(ws)
        .bind(parent)
        .bind(name)
        .bind(user)
        .execute(db)
        .await
        .unwrap();
      }
      let (page, _) = seed_document(db, ws, user, "页", serde_json::json!([])).await;
      sqlx::query("UPDATE views SET parent_view_id = $1 WHERE id = $2")
        .bind(inner)
        .bind(page)
        .execute(db)
        .await
        .unwrap();
      (ws, outer, inner, page)
    }

    /// The walk must reach past the DIRECT parent, all the way up.
    ///
    /// This is what makes `independent_roots` able to see that a page selected
    /// two levels down is already inside a selected folder. Stopping at the
    /// direct parent would look right in the common case and transfer the deep
    /// one twice.
    #[tokio::test]
    async fn ancestor_pairs_walk_the_whole_way_up() {
      let Some(db) = pool().await else { return };
      let (ws, outer, inner, page) = seed_nested_folders(&db).await;

      let pairs = ancestor_pairs_of_roots(&db, ws, &[page]).await.unwrap();

      assert!(pairs.contains(&(page, inner)), "direct parent: {pairs:?}");
      assert!(
        pairs.contains(&(page, outer)),
        "the grandparent is the one a shallow walk would miss: {pairs:?}"
      );
    }

    /// The two halves together, which is the only combination that ships.
    #[tokio::test]
    async fn selecting_a_folder_and_a_page_deep_inside_it_transfers_the_folder_only() {
      let Some(db) = pool().await else { return };
      let (ws, outer, _inner, page) = seed_nested_folders(&db).await;

      let pairs = ancestor_pairs_of_roots(&db, ws, &[outer, page]).await.unwrap();

      assert_eq!(
        independent_roots(&[outer, page], &pairs),
        vec![outer],
        "the page rides along inside the folder; transferring it too would duplicate it"
      );
    }

    /// An id the caller cannot actually transfer contributes no pairs.
    ///
    /// It must not be silently dropped here — it is kept as a root and then
    /// rejected by the handler's membership check, so the caller gets a 404
    /// instead of a batch that quietly did less than it was asked.
    #[tokio::test]
    async fn an_untransferable_root_yields_no_pairs() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;
      let (other_ws, other_user) = seed_workspace(&db).await;
      let (trashed, _) = seed_document(&db, ws, user, "已删", serde_json::json!([])).await;
      sqlx::query("UPDATE views SET is_deleted = true WHERE id = $1")
        .bind(trashed)
        .execute(&db)
        .await
        .unwrap();
      let (foreign, _) =
        seed_document(&db, other_ws, other_user, "别处的", serde_json::json!([])).await;

      let pairs = ancestor_pairs_of_roots(&db, ws, &[trashed, foreign])
        .await
        .unwrap();

      assert!(pairs.is_empty(), "{pairs:?}");
      assert_eq!(
        independent_roots(&[trashed, foreign], &pairs),
        vec![trashed, foreign],
        "kept, so the handler can refuse them by name rather than skipping them"
      );
    }

    /// Nothing asked for, nothing queried.
    #[tokio::test]
    async fn no_roots_means_no_query() {
      let Some(db) = pool().await else { return };
      let (ws, _) = seed_workspace(&db).await;
      assert!(ancestor_pairs_of_roots(&db, ws, &[]).await.unwrap().is_empty());
    }

    /// A trashed FOLDER carries its children, and the children's documents go
    /// with them — the same cascade `purge_view_subtree` guarantees for one id.
    /// Seeded from a set instead, which is the only part that is new, and the
    /// part that would silently strand documents if the recursive arm were
    /// written wrong.
    #[tokio::test]
    async fn a_bulk_purge_takes_the_whole_subtree_and_its_documents() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;
      let folder = Uuid::new_v4();
      sqlx::query(
        "INSERT INTO views(id, workspace_id, object_id, object_type, name, position, created_by, \
         is_deleted) VALUES($1,$2,$1,'folder','容器','a',$3,true)",
      )
      .bind(folder)
      .bind(ws)
      .bind(user)
      .execute(&db)
      .await
      .unwrap();
      let (child, child_doc) = seed_document(&db, ws, user, "子页", serde_json::json!([])).await;
      sqlx::query("UPDATE views SET parent_view_id = $1, is_deleted = true WHERE id = $2")
        .bind(folder)
        .bind(child)
        .execute(&db)
        .await
        .unwrap();

      let touched = purge_views_batch(&db, ws, &[folder]).await.unwrap();

      assert!(touched.contains(&folder));
      assert!(touched.contains(&child), "the child went with its folder");
      let doc_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM documents WHERE id=$1")
        .bind(child_doc)
        .fetch_one(&db)
        .await
        .unwrap();
      assert_eq!(doc_rows, 0, "the child's document was stranded");
    }
  }

  /// What the backlinks panel actually costs at real scale.
  ///
  /// The roadmap deferred a maintained reverse-index table on the grounds that
  /// the on-demand scan is fine "until scale bites" — a claim nobody had ever
  /// put a number on. This puts one on it, against the REAL [`scan_backlinks`]
  /// rather than a re-implementation, because the cost is not in the SQL: the
  /// scan is one round-trip plus one full CRDT decode PER DOCUMENT, with no
  /// early stop (a panel must be complete), bounded only by `buffered(8)`.
  ///
  /// Deliberately NOT wired to `DATABASE_URL`: it wants a restored production
  /// snapshot, not the dev database, and pointing it at dev would measure an
  /// empty workspace and read as "fast".
  ///
  ///   $env:MICA_BENCH_DATABASE_URL="postgres://mica:mica@127.0.0.1:5432/mica_bench"
  ///   $env:MICA_BENCH_WORKSPACE="<uuid>"
  ///   cargo test -p mica-api-server backlink_scan_cost -- --ignored --nocapture
  #[tokio::test]
  #[ignore = "needs a restored production snapshot; see the doc comment"]
  async fn backlink_scan_cost() {
    let url = std::env::var("MICA_BENCH_DATABASE_URL").expect("MICA_BENCH_DATABASE_URL");
    let ws: Uuid = std::env::var("MICA_BENCH_WORKSPACE")
      .expect("MICA_BENCH_WORKSPACE")
      .parse()
      .unwrap();
    let db = PgPool::connect(&url).await.unwrap();

    let views = fetch_workspace_views(&db, ws).await.unwrap();
    let docs: Vec<_> = views.iter().filter(|v| v.object_type == "document").collect();
    println!("workspace has {} live documents", docs.len());

    // The index is only as good as the backfill that fills it, and the backfill
    // is the one O(all documents) decode left — pay it once here so the number
    // below is the steady state a running server actually serves from.
    let started = std::time::Instant::now();
    let filled = mica_app_core::sync::backfill_derived_columns(&db).await.unwrap();
    println!("backfill: {filled} row(s) in {:?}", started.elapsed());

    // Three targets, because the panel's cost must not depend on the answer.
    // Before the index every one of these took the same ~690 ms regardless of
    // hits, since every document was decoded either way.
    for view in docs.iter().take(3) {
      let started = std::time::Instant::now();
      let hits = scan_backlinks(&db, ws, view.id).await.unwrap();
      println!(
        "scan_backlinks({}) -> {} hits in {:?}",
        view.name,
        hits.len(),
        started.elapsed()
      );
    }

    // The panel must still be CORRECT, not just fast. Rebuild the whole answer
    // the slow way — the O(N) decode this change deleted — and demand the index
    // agrees for EVERY target in the workspace, not just one. Checking only a
    // target that happens to have hits would miss the opposite failure: a target
    // the index reports backlinks for when it has none.
    let mut expected: std::collections::HashMap<Uuid, Vec<Uuid>> = Default::default();
    for view in &docs {
      let Ok(Some(payload)) = store::current_payload(&db, view.object_id).await else {
        continue;
      };
      let mut linked: Vec<String> = payload
        .blocks
        .iter()
        .flat_map(|b| page_link_targets(&b.data))
        .collect();
      linked.sort_unstable();
      linked.dedup();
      for target in linked.iter().filter_map(|t| t.parse::<Uuid>().ok()) {
        // A page linking to itself is not a backlink — the query excludes it, so
        // the expectation must too.
        if target != view.id {
          expected.entry(target).or_default().push(view.id);
        }
      }
    }
    for sources in expected.values_mut() {
      sources.sort_unstable();
    }

    let mut checked = 0usize;
    let mut with_hits = 0usize;
    for view in &docs {
      let mut got: Vec<Uuid> = scan_backlinks(&db, ws, view.id)
        .await
        .unwrap()
        .into_iter()
        .map(|b| b.view_id)
        .collect();
      got.sort_unstable();
      // `expected` is built from every document in the workspace; the query also
      // requires the SOURCE to be a live document in this workspace, which is
      // exactly the set `docs` was filtered to. So the two must match exactly.
      let want = expected.get(&view.id).cloned().unwrap_or_default();
      assert_eq!(got, want, "index disagrees with a full decode for {}", view.id);
      checked += 1;
      if !got.is_empty() {
        with_hits += 1;
      }
    }
    println!("correctness: {checked} target(s) checked, {with_hits} with backlinks — all match");
  }
}
