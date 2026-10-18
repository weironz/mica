
use axum::{
  Json,
  extract::{Path, Query, State},
  http::{HeaderMap, header},
  response::{IntoResponse, Response},
};
use mica_app_core::AppState;
use mica_infra::{ApiError, ApiResult};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::routes::auth::user_id_from_headers;
use super::*;

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
pub(super) fn views_etag(views: &[View], query: &ListViewsQuery) -> String {
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
pub(super) fn etag_matches(header_value: &str, etag: &str) -> bool {
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
pub(crate) struct SearchResult {
  pub(crate) view_id: Uuid,
  pub(crate) object_id: Uuid,
  /// Which workspace the hit lives in.
  ///
  /// Redundant on the per-workspace route (the caller named it in the URL) and
  /// load-bearing on the cross-workspace one: opening a page is workspace-scoped
  /// (`GET /workspaces/{workspace_id}/documents/{id}`), so a hit that does not
  /// say where it lives cannot be opened. The client cannot infer it either —
  /// it only holds page trees for workspaces it has actually visited.
  pub(crate) workspace_id: Uuid,
  pub(crate) name: String,
  pub(crate) snippet: String,
  pub(crate) title_match: bool,
  /// The containing folder, or null at the workspace root.
  ///
  /// Here because finding a page and then acting NEAR it — filing a sibling
  /// beside it, saying where it lives — otherwise costs a full workspace tree
  /// listing, orders of magnitude more than the hit itself on a large
  /// workspace. It is one column that was already on the row.
  pub(crate) parent_view_id: Option<Uuid>,
  /// True when the hit is a folder rather than a page. Can only be true if the
  /// caller passed `include_folders`.
  pub(crate) is_folder: bool,
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
pub(super) async fn search_views(
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
pub(super) fn like_pattern(needle: &str) -> String {
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
pub(super) fn snippet_for(text: &str, needle_lower: &str) -> Option<String> {
  const WINDOW: usize = 160;
  /// Chars of context to keep before the match so the hit isn't flush-left.
  const LEAD: usize = 40;
  let chars: Vec<char> = text.chars().collect();
  // Lowercase char by char, remembering which ORIGINAL char each lowered byte
  // came from. `to_lowercase` is not length-preserving: `İ` becomes two chars,
  // so counting chars in the lowered prefix used to over-shoot the original —
  // a page with ~100 `İ` before the hit pushed `start` past the end of `chars`
  // and the slice below panicked, dropping every search that matched it.
  let mut lower = String::with_capacity(text.len());
  let mut origin: Vec<usize> = Vec::with_capacity(text.len());
  for (index, c) in chars.iter().enumerate() {
    for l in c.to_lowercase() {
      lower.push(l);
      origin.resize(lower.len(), index);
    }
  }
  let byte_pos = lower.find(needle_lower)?;
  let match_char = origin[byte_pos];
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
pub(crate) struct Backlink {
  pub(crate) view_id: Uuid,
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
pub(crate) struct GraphNode {
  pub(crate) view_id: Uuid,
  pub(crate) name: String,
  /// How many edges touch this node (in + out). The client sizes nodes by it, so
  /// a hub reads as a hub without the client having to count edges itself.
  pub(crate) degree: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub(crate) struct GraphEdge {
  pub(crate) source: Uuid,
  pub(crate) target: Uuid,
}

#[derive(Debug, Serialize)]
pub struct GraphResponse {
  pub(crate) nodes: Vec<GraphNode>,
  pub(crate) edges: Vec<GraphEdge>,
  /// Live documents in the workspace that no link touches. Reported as a COUNT
  /// rather than as isolated nodes: on a real workspace most pages are unlinked
  /// (measured 2026-08-05 on a production snapshot — 798 documents, 136 with any
  /// link at all), and drawing 662 disconnected dots buries the structure the
  /// view exists to show. The number keeps that honest instead of silent.
  pub(crate) unlinked: i64,
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
pub(super) async fn workspace_graph(db: &PgPool, workspace_id: Uuid) -> ApiResult<GraphResponse> {
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
pub(super) async fn scan_backlinks(
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


#[cfg(test)]
mod tests {
  use super::*;
  use super::super::{ListViewsQuery, View};

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

  /// THE BUG: `İ` lowercases to TWO chars, so the char index counted in the
  /// lowered text over-shot the original and slicing it panicked — one planted
  /// page took down every search that matched it.
  #[test]
  fn a_snippet_survives_text_whose_lowercase_is_longer() {
    let body = format!("{}foo", "İ".repeat(100));
    let snip = snippet_for(&body, "foo").expect("the hit is still found");
    assert!(snip.ends_with("foo"), "window lands on the match: {snip}");
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

}
