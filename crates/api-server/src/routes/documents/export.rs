use std::collections::BTreeMap;

use axum::{
  Json,
  extract::{Path, Query, State},
  http::{HeaderMap, header},
  response::{IntoResponse, Response},
};
use mica_app_core::{
  AppState,
  documents::{
    DocumentSnapshotPayload, document_title, export_html_document,
    export_markdown_with_assets, set_image_srcs,
    with_page_title,
  },
  store::{self},
};
use mica_infra::{ApiError, ApiResult};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::routes::auth::user_id_from_headers;
use mica_interchange::{ZipEntry, build_zip};
use super::*;

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
      .get(storage.presign_get_server(&record.object_key))
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
  let _permit = tree_export_permit()?;
  let entries = build_tree_zip(&state, workspace_id, None).await?;
  Ok(zip_response(build_zip(&entries), "workspace.zip"))
}

/// v1 is the page-at-a-time protocol (including stable, file-id asset names).
/// Bump when Markdown/sidecar generation changes, so cached output is rebuilt.
const INCREMENTAL_EXPORT_VERSION: u32 = 1;

#[derive(Debug, Serialize)]
pub struct ExportPageRevision {
  pub(crate) view_id: Uuid,
  pub(crate) document_id: Uuid,
  pub(crate) revision: String,
}

#[derive(Debug, Serialize)]
pub struct ExportFileRevision {
  pub(crate) file_id: Uuid,
  pub(crate) revision: String,
}

#[derive(Debug, Serialize)]
pub struct ExportManifestResponse {
  pub(crate) version: u32,
  pub(crate) tree_revision: String,
  pub(crate) structure_revision: String,
  pub(crate) manifest: serde_json::Value,
  pub(crate) pages: Vec<ExportPageRevision>,
  pub(crate) files: Vec<ExportFileRevision>,
}

/// Inventory without CRDT decoding, Markdown generation or blob downloads.
/// Hashing the authoritative bytes also detects deletion-only yrs edits (whose
/// state vector need not change) and every writer, including imports/restores.
/// This reads the states in Postgres; it is not a metadata-only database scan.
pub async fn export_manifest(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
) -> ApiResult<Json<ExportManifestResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;
  Ok(Json(build_export_manifest(&state.db, workspace_id).await?))
}

pub(super) async fn build_export_manifest(
  db: &PgPool,
  workspace_id: Uuid,
) -> ApiResult<ExportManifestResponse> {
  // One repeatable-read snapshot keeps paths, body versions and asset metadata
  // consistent. The CLI also rechecks the inventory after fetching changed pages.
  let mut tx = db.begin().await?;
  sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
    .execute(&mut *tx)
    .await?;
  let views: Vec<View> = sqlx::query_as(
    "SELECT id, workspace_id, parent_view_id, object_id, object_type::text, name,
            position, icon, is_deleted, created_by, created_at, updated_at
     FROM views WHERE workspace_id = $1 AND is_deleted = false
     ORDER BY position ASC, id ASC",
  )
  .bind(workspace_id)
  .fetch_all(&mut *tx)
  .await?;
  let revisions: Vec<(Uuid, Uuid, String)> = sqlx::query_as(
    "SELECT v.id, v.object_id,
            coalesce(md5(y.state), 'missing') || ':' || md5(coalesce(d.root_block_id, ''))
     FROM views v
     LEFT JOIN documents d ON d.id = v.object_id
     LEFT JOIN document_yrs_base y ON y.document_id = v.object_id
     WHERE v.workspace_id = $1 AND v.is_deleted = false
       AND v.object_type::text = 'document'
     ORDER BY v.id",
  )
  .bind(workspace_id)
  .fetch_all(&mut *tx)
  .await?;
  // File ids serve immutable objects in normal operation. Including the actual
  // storage mapping also invalidates output after administrative repairs; do
  // not include GC timestamps, which do not affect exported bytes.
  let files: Vec<store::FileRecord> = sqlx::query_as(
    "SELECT * FROM files
     WHERE workspace_id = $1 ORDER BY id",
  )
  .bind(workspace_id)
  .fetch_all(&mut *tx)
  .await?;
  tx.commit().await?;
  let manifest = mica_interchange::markdown_tree_manifest(&export_tree_nodes(&views));
  let structure_revision = export_structure_revision(&manifest, &files)?;
  let tree_revision = export_revision_digest(&manifest)?;
  let file_revisions = files
    .iter()
    .map(|file| {
      Ok(ExportFileRevision {
        file_id: file.id,
        revision: export_revision_digest(&(
          file.id,
          &file.object_key,
          &file.original_name,
          &file.mime_type,
          file.byte_size,
        ))?,
      })
    })
    .collect::<ApiResult<Vec<_>>>()?;
  Ok(ExportManifestResponse {
    version: INCREMENTAL_EXPORT_VERSION,
    tree_revision,
    structure_revision,
    manifest,
    files: file_revisions,
    pages: revisions
      .into_iter()
      .map(|(view_id, document_id, revision)| ExportPageRevision {
        view_id,
        document_id,
        revision,
      })
      .collect(),
  })
}

fn export_structure_revision(
  manifest: &serde_json::Value,
  files: &[store::FileRecord],
) -> ApiResult<String> {
  let metadata: Vec<_> = files
    .iter()
    .map(|f| {
      (
        f.id,
        &f.object_key,
        &f.original_name,
        &f.mime_type,
        f.byte_size,
      )
    })
    .collect();
  export_revision_digest(&(manifest, metadata))
}

fn export_revision_digest(value: &impl Serialize) -> ApiResult<String> {
  use sha2::{Digest, Sha256};
  let mut hasher = Sha256::new();
  hasher.update(INCREMENTAL_EXPORT_VERSION.to_be_bytes());
  hasher.update(serde_json::to_vec(value).map_err(|e| ApiError::Internal(e.to_string()))?);
  Ok(format!("{:x}", hasher.finalize()))
}

#[derive(Debug, Default, Deserialize)]
pub struct ExportPageQuery {
  /// File UUIDs whose unchanged bytes the caller already has (up to 64).
  /// References stay in Markdown/sidecars; only the ZIP byte entries are omitted.
  skip_assets: Option<String>,
}

pub(super) fn parse_export_skip_assets(value: Option<&str>) -> ApiResult<std::collections::HashSet<String>> {
  let mut ids = std::collections::HashSet::new();
  let Some(value) = value.filter(|value| !value.is_empty()) else {
    return Ok(ids);
  };
  for (index, value) in value.split(',').enumerate() {
    if index >= 64 {
      return Err(ApiError::BadRequest(
        "skip_assets accepts at most 64 file ids".into(),
      ));
    }
    let id = Uuid::parse_str(value.trim())
      .map_err(|_| ApiError::BadRequest("skip_assets must contain comma-separated UUIDs".into()))?;
    ids.insert(id.to_string());
  }
  Ok(ids)
}

/// One page in the complete workspace's path namespace. Only this page is
/// decoded and only its missing assets are read, bounding memory by a single page.
pub async fn export_page_zip(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, view_id)): Path<(Uuid, Uuid)>,
  Query(query): Query<ExportPageQuery>,
) -> ApiResult<Response> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;
  let skip_assets = parse_export_skip_assets(query.skip_assets.as_deref())?;
  let mut tx = state.db.begin().await?;
  sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
    .execute(&mut *tx)
    .await?;
  let views: Vec<View> = sqlx::query_as(
    "SELECT id, workspace_id, parent_view_id, object_id, object_type::text, name,
       position, icon, is_deleted, created_by, created_at, updated_at
     FROM views WHERE workspace_id = $1 AND is_deleted = false ORDER BY position, id",
  )
  .bind(workspace_id)
  .fetch_all(&mut *tx)
  .await?;
  let files: Vec<store::FileRecord> =
    sqlx::query_as("SELECT * FROM files WHERE workspace_id = $1 ORDER BY id")
      .bind(workspace_id)
      .fetch_all(&mut *tx)
      .await?;
  tx.commit().await?;
  let nodes = export_tree_nodes(&views);
  let structure_revision =
    export_structure_revision(&mica_interchange::markdown_tree_manifest(&nodes), &files)?;
  let view = views
    .iter()
    .find(|v| v.id == view_id)
    .ok_or(ApiError::NotFound)?;
  if view.object_type != "document" {
    return Err(ApiError::BadRequest(
      "only a document can be exported this way".into(),
    ));
  }
  let (payload, revision) = store::current_payload_versioned(&state.db, view.object_id)
    .await?
    .ok_or(ApiError::NotFound)?;
  let payloads = std::collections::HashMap::from([(view.object_id.to_string(), payload)]);
  let images = fetch_export_images(
    &state,
    workspace_id,
    &payloads,
    true,
    Some(&files),
    &skip_assets,
  )
  .await?;
  let entries = mica_interchange::build_incremental_markdown_tree_zip(
    &nodes,
    &payloads,
    &images,
    &view_id.to_string(),
    &skip_assets,
  );
  let mut response = zip_response(build_zip(&entries), "page.zip");
  response.headers_mut().insert(
    "x-mica-export-structure-revision",
    structure_revision
      .parse()
      .map_err(|e| ApiError::Internal(format!("invalid export structure revision: {e}")))?,
  );
  response.headers_mut().insert(
    "x-mica-export-revision",
    revision
      .parse()
      .map_err(|e| ApiError::Internal(format!("invalid export revision: {e}")))?,
  );
  Ok(response)
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

  let _permit = tree_export_permit()?;
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
  let _permit = tree_export_permit()?;
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
/// How many whole-tree exports may build at once, server-wide.
///
/// A tree export holds every page payload and every image of the subtree in
/// memory (twice, briefly, while the archive is assembled), so a few
/// concurrent "export all workspaces" from any members — read-only ones
/// included — could stack gigabytes. Streaming the archive would bound one
/// export; this bounds how many there are. Excess callers get a 503 to retry.
static TREE_EXPORTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

fn tree_export_permit() -> ApiResult<tokio::sync::SemaphorePermit<'static>> {
  TREE_EXPORTS.try_acquire().map_err(|_| {
    ApiError::Unavailable("another large export is running; try again in a moment".to_string())
  })
}

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
  let nodes = export_tree_nodes(&views);

  // Only the legacy interactive workspace download gathers every payload.
  // Scheduled exports use export_page_zip, keeping this map to a single page.
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
  let images = fetch_export_images(
    state,
    workspace_id,
    &payloads,
    false,
    None,
    &std::collections::HashSet::new(),
  )
  .await?;
  let root_str = root.map(|r| r.to_string());
  Ok(mica_interchange::build_markdown_tree_zip(
    &nodes,
    root_str.as_deref(),
    &payloads,
    &images,
  ))
}

fn export_tree_nodes(views: &[View]) -> Vec<mica_interchange::TreeNode> {
  views
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
    .collect()
}

pub(super) async fn fetch_export_images(
  state: &AppState,
  workspace_id: Uuid,
  payloads: &std::collections::HashMap<String, DocumentSnapshotPayload>,
  strict: bool,
  file_snapshot: Option<&[store::FileRecord]>,
  skip_assets: &std::collections::HashSet<String>,
) -> ApiResult<std::collections::HashMap<String, mica_interchange::ImageAsset>> {
  // Referenced image blobs: file_id -> (first-seen block name), then fetch the
  // records and download the bytes. Dedup key = the storage object key, so two
  // file_ids of the same blob share one `assets/` entry (as the old walk did).
  let mut file_name: BTreeMap<String, String> = BTreeMap::new();
  // HashMap iteration must not choose a different cosmetic asset name on each
  // full export when multiple pages use one file under different names.
  let mut document_ids: Vec<_> = payloads.keys().collect();
  document_ids.sort();
  for document_id in document_ids {
    let payload = &payloads[document_id];
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
  if !file_name.is_empty() {
    let storage = state.storage.clone();
    if strict && storage.is_none() && file_name.keys().any(|id| !skip_assets.contains(id)) {
      return Err(ApiError::Unavailable(
        "file storage is not configured".into(),
      ));
    }
    let ids: Vec<Uuid> = file_name
      .keys()
      .filter_map(|id| Uuid::parse_str(id).ok())
      .collect();
    let records = match file_snapshot {
      Some(records) => records
        .iter()
        .filter(|r| ids.contains(&r.id))
        .cloned()
        .collect(),
      None => store::fetch_files(&state.db, workspace_id, &ids).await?,
    };
    let by_id: std::collections::HashMap<String, &store::FileRecord> =
      records.iter().map(|r| (r.id.to_string(), r)).collect();
    let client = reqwest::Client::builder()
      .connect_timeout(std::time::Duration::from_secs(10))
      .timeout(std::time::Duration::from_secs(120))
      .build()
      .map_err(|e| ApiError::Internal(e.to_string()))?;
    for (file_id, name) in &file_name {
      let Some(record) = by_id.get(file_id) else {
        if strict {
          return Err(ApiError::Unavailable(format!(
            "export image {file_id} is missing"
          )));
        }
        continue;
      };
      let bytes = if skip_assets.contains(file_id) {
        Vec::new()
      } else {
        let Some(storage) = storage.as_ref() else {
          continue;
        };
        match client
          .get(storage.presign_get_server(&record.object_key))
          .send()
          .await
        {
          Ok(resp) if resp.status().is_success() => match resp.bytes().await {
            Ok(b) => b.to_vec(),
            Err(e) => {
              if strict {
                return Err(ApiError::Unavailable(format!(
                  "export image {file_id}: {e}"
                )));
              }
              continue;
            }
          },
          _ => {
            if strict {
              return Err(ApiError::Unavailable(format!(
                "failed to download export image {file_id}"
              )));
            }
            continue;
          }
        }
      };
      images.insert(
        file_id.clone(),
        mica_interchange::ImageAsset {
          name: if strict {
            record.original_name.clone()
          } else {
            name.clone()
          },
          bytes,
          dedup_key: record.object_key.clone(),
        },
      );
    }
  }

  Ok(images)
}

/// The canonical, workspace-scoped path that serves one blob's bytes.
///
/// The trailing name is cosmetic — `files::blob_named` ignores it — but it
/// keeps the exported Markdown readable and survives a re-import (see
/// [parse_blob_href]).
pub(super) fn blob_href(workspace_id: Uuid, file_id: &str, name: &str) -> String {
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
pub(super) fn blob_asset_map(
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
pub(super) fn rewire_blob_hrefs(blocks: &mut [mica_app_core::documents::Block], workspace_id: Uuid) {
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
pub(super) fn percent_decode(input: &str) -> String {
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
pub(super) fn safe_segment(name: &str) -> String {
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
    let bytes = match client.get(storage.presign_get_server(&record.object_key)).send().await {
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
pub(super) fn inline_blob_hrefs(blocks: &mut [mica_app_core::documents::Block], workspace_id: Uuid) {
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


#[cfg(test)]
mod tests {
  use super::*;

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

}
