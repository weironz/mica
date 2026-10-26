// Postgres-backed FLOW tests for the cross-workspace transfer and same-workspace
// clone routes (issue #86): the full `transfer_view` / `batch_transfer_views` /
// `clone_view` handlers driven with real extractor arguments against a real
// database — pg_tests.rs covers the helper layer (ancestor_pairs_of_roots,
// remove_moved_source, …), this file covers what the handlers assemble out of
// them.
//
// A new sibling file rather than an addition to pg_tests.rs because that file
// is at the file-size gate's ratchet limit: it may not grow.
//
// Gated on `DATABASE_URL`, hardened the same way as pg_tests.rs: skipped
// (green) without a database locally, but a set-but-unreachable URL — or a
// missing one in CI — is a hard failure, never a silent pass.
//
//   $env:DATABASE_URL="postgres://mica:mica@127.0.0.1:5432/mica"
//   cargo test -p mica-api-server transfer_flow_pg
use super::transfer::{
  BatchTransferRequest, CloneRequest, TransferRequest, batch_transfer_views, clone_view,
  transfer_view,
};
use axum::{
  Json,
  extract::{Path, State},
  http::{HeaderMap, header::AUTHORIZATION},
};
use mica_app_core::{AppState, store};
use mica_infra::ApiError;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

const JWT_SECRET: &str = "transfer-flow-pg-test-secret";

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

/// Seed just the user row; two-workspace scenarios share one owner.
async fn seed_user(db: &PgPool) -> Uuid {
  let user = Uuid::new_v4();
  sqlx::query("INSERT INTO users(id,email,display_name,password_hash) VALUES($1,$2,'T','x')")
    .bind(user)
    .bind(format!("{user}@transfer-flow.test"))
    .execute(db)
    .await
    .unwrap();
  user
}

/// Seed a workspace owned by [user], INCLUDING the membership row — every
/// "is this user an editor here" check joins through workspace_members (same
/// lesson pg_tests::seed_workspace documents).
async fn seed_workspace(db: &PgPool, user: Uuid) -> Uuid {
  let ws = Uuid::new_v4();
  sqlx::query("INSERT INTO workspaces(id,name,owner_id) VALUES($1,'W',$2)")
    .bind(ws)
    .bind(user)
    .execute(db)
    .await
    .unwrap();
  sqlx::query(
    "INSERT INTO workspace_members(workspace_id,user_id,role,position) VALUES($1,$2,'owner',$3)",
  )
  .bind(ws)
  .bind(user)
  .bind(crate::routes::workspaces::pad_position(1))
  .execute(db)
  .await
  .unwrap();
  ws
}

/// Seed a folder view; [parent] must itself be a folder (the page-tree trigger
/// enforces it). Returns its id.
async fn seed_folder(db: &PgPool, ws: Uuid, user: Uuid, parent: Option<Uuid>, name: &str) -> Uuid {
  let id = Uuid::new_v4();
  sqlx::query(
    "INSERT INTO views(id,workspace_id,parent_view_id,object_id,object_type,name,position,created_by) \
       VALUES($1,$2,$3,$4,'folder'::object_type,$5,'0',$6)",
  )
  .bind(id)
  .bind(ws)
  .bind(parent)
  .bind(Uuid::new_v4())
  .bind(name)
  .bind(user)
  .execute(db)
  .await
  .unwrap();
  id
}

/// Write `blocks` into a document's yrs base — since S5 the only place content
/// lives, so what is put here is exactly what every read returns. Copied from
/// pg_tests.rs: a `root` block listing `blocks` as its children is synthesized
/// unless the caller supplied one, because a yrs doc is a tree walked from the
/// root and a bare `[{...}]` fixture would seed an empty page.
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
  let payload: mica_markdown::DocumentSnapshotPayload = serde_json::from_value(serde_json::json!({
    "schema_version": 1,
    "root_block_id": "root",
    "blocks": blocks,
  }))
  .unwrap();
  let mut tx = db.begin().await.unwrap();
  mica_app_core::sync::seed_base_tx(&mut tx, doc_id, payload)
    .await
    .unwrap();
  tx.commit().await.unwrap();
}

/// Seed a document view (a leaf page) with content, optionally under [parent].
/// Returns (view_id, document_id).
async fn seed_document(
  db: &PgPool,
  ws: Uuid,
  user: Uuid,
  parent: Option<Uuid>,
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
    "INSERT INTO views(id,workspace_id,parent_view_id,object_id,object_type,name,position,created_by) \
       VALUES($1,$2,$3,$4,'document'::object_type,$5,'0',$6)",
  )
  .bind(view_id)
  .bind(ws)
  .bind(parent)
  .bind(doc_id)
  .bind(title)
  .bind(user)
  .execute(db)
  .await
  .unwrap();
  (view_id, doc_id)
}

/// The AppState the handlers need: a non-empty jwt_secret (AppState::new
/// refuses an empty one), and storage set to Some — `transfer_roots` checks
/// `state.storage` BEFORE its blob loop even when no document references a
/// file, so a None would fail every transfer with
/// "file storage is not configured". The endpoint is unreachable on purpose:
/// these fixtures reference no blobs, so nothing is ever fetched.
fn test_state(db: &PgPool) -> AppState {
  let mut config = mica_infra::AppConfig::from_env().unwrap();
  config.jwt_secret = JWT_SECRET.into();
  config.environment = mica_infra::Environment::Test;
  let mut state = AppState::new(
    config,
    db.clone(),
    std::sync::Arc::new(mica_infra::LogMailer),
  );
  state.storage = Some(std::sync::Arc::new(mica_infra::S3Config {
    endpoint: "http://127.0.0.1:1/unreachable".to_string(),
    internal_endpoint: None,
    region: "us-east-1".to_string(),
    bucket: "transfer-flow-test".to_string(),
    access_key: "x".to_string(),
    secret_key: "x".to_string(),
    presign_ttl_seconds: 900,
    max_upload_bytes: 0,
    public_base_url: None,
    force_path_style: true,
  }));
  state
}

/// A real JWT, minted the way `auth_response` does (HS256, `{sub, exp}`), so
/// `user_id_from_headers` resolves [user] from the bearer header.
fn bearer_headers(user: Uuid) -> HeaderMap {
  #[derive(serde::Serialize)]
  struct TestClaims {
    sub: String,
    exp: usize,
  }
  let claims = TestClaims {
    sub: user.to_string(),
    exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
  };
  let token = jsonwebtoken::encode(
    &jsonwebtoken::Header::default(),
    &claims,
    &jsonwebtoken::EncodingKey::from_secret(JWT_SECRET.as_bytes()),
  )
  .unwrap();
  let mut headers = HeaderMap::new();
  headers.insert(AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
  headers
}

/// Live views of a workspace: (id, parent, name, object_id), ordered by name
/// for deterministic assertions.
async fn live_views(db: &PgPool, ws: Uuid) -> Vec<(Uuid, Option<Uuid>, String, Uuid)> {
  sqlx::query_as::<_, (Uuid, Option<Uuid>, String, Uuid)>(
    "SELECT id, parent_view_id, name, object_id FROM views \
       WHERE workspace_id = $1 AND is_deleted = false ORDER BY name",
  )
  .bind(ws)
  .fetch_all(db)
  .await
  .unwrap()
}

async fn is_deleted(db: &PgPool, view: Uuid) -> bool {
  sqlx::query_scalar::<_, bool>("SELECT is_deleted FROM views WHERE id = $1")
    .bind(view)
    .fetch_one(db)
    .await
    .unwrap()
}

/// A document's blocks as the server hands them back — compared before/after
/// to prove a transfer/clone carried the payload across byte-for-byte.
async fn current_blocks_json(db: &PgPool, doc: Uuid) -> serde_json::Value {
  let payload = store::current_payload(db, doc)
    .await
    .unwrap()
    .expect("document must have a payload");
  serde_json::to_value(&payload.blocks).unwrap()
}

/// `POST /api/workspaces/{ws}/views/{view}/transfer` with remove_source: a
/// folder + the page inside it leaves the source (soft-deleted) and arrives in
/// the destination under the requested parent, payload intact.
#[tokio::test]
async fn transfer_view_moves_a_subtree_between_workspaces() {
  let Some(db) = pool().await else { return };
  let user = seed_user(&db).await;
  let src = seed_workspace(&db, user).await;
  let dest = seed_workspace(&db, user).await;

  let folder = seed_folder(&db, src, user, None, "Folder").await;
  let (page, doc) = seed_document(
    &db,
    src,
    user,
    Some(folder),
    "Page",
    json!([{"id": "b1", "type": "paragraph", "text": "hello transfer", "data": {}}]),
  )
  .await;
  let before = current_blocks_json(&db, doc).await;
  let dest_folder = seed_folder(&db, dest, user, None, "Inbox").await;

  let request: TransferRequest = serde_json::from_value(json!({
    "dest_workspace_id": dest,
    "parent_view_id": dest_folder,
    "remove_source": true,
  }))
  .unwrap();
  let Json(response) = transfer_view(
    State(test_state(&db)),
    bearer_headers(user),
    Path((src, folder)),
    Json(request),
  )
  .await
  .expect("transfer should succeed");
  let response = serde_json::to_value(&response).unwrap();
  assert_eq!(response["documents"], json!(1));
  assert_eq!(response["folders"], json!(1));
  assert_eq!(response["removed_source"], json!(true));
  assert_eq!(response["dry_run"], json!(false));
  let new_root: Uuid = response["new_root_view_id"]
    .as_str()
    .expect("a move reports its new root")
    .parse()
    .unwrap();
  assert_ne!(new_root, folder, "the destination tree gets FRESH ids");

  // Source: the whole subtree is in the bin — no live rows left under src.
  assert!(is_deleted(&db, folder).await, "moved folder must be binned");
  assert!(is_deleted(&db, page).await, "moved page must be binned");
  assert!(
    live_views(&db, src).await.is_empty(),
    "the source workspace must no longer list the moved tree as live"
  );

  // Destination: the folder copy under the requested parent, the page copy
  // under the folder copy, payload preserved.
  let dest_views = live_views(&db, dest).await;
  assert_eq!(
    dest_views.len(),
    3,
    "dest should hold Inbox + the moved folder + its page, got {dest_views:?}"
  );
  let moved_folder = dest_views
    .iter()
    .find(|(_, _, name, _)| name == "Folder")
    .expect("the folder must arrive in the destination");
  assert_eq!(moved_folder.0, new_root);
  assert_eq!(
    moved_folder.1,
    Some(dest_folder),
    "the moved root must sit under the requested destination parent"
  );
  let moved_page = dest_views
    .iter()
    .find(|(_, _, name, _)| name == "Page")
    .expect("the page must arrive in the destination");
  assert_eq!(
    moved_page.1,
    Some(new_root),
    "the child must follow the folder's NEW id, not the old one"
  );
  assert_ne!(moved_page.0, page, "the destination page gets a fresh id");
  assert_eq!(
    current_blocks_json(&db, moved_page.3).await,
    before,
    "the moved page's content must be preserved"
  );
}

/// Same route with `remove_source: false` — a copy: the source tree stays
/// live, untouched, and the destination holds a duplicate with fresh ids.
#[tokio::test]
async fn transfer_view_copy_mode_keeps_the_source_intact() {
  let Some(db) = pool().await else { return };
  let user = seed_user(&db).await;
  let src = seed_workspace(&db, user).await;
  let dest = seed_workspace(&db, user).await;

  let folder = seed_folder(&db, src, user, None, "Orig").await;
  let (page, doc) = seed_document(
    &db,
    src,
    user,
    Some(folder),
    "Leaf",
    json!([{"id": "b1", "type": "paragraph", "text": "keep me", "data": {}}]),
  )
  .await;
  let before = current_blocks_json(&db, doc).await;

  let request: TransferRequest = serde_json::from_value(json!({
    "dest_workspace_id": dest,
    "remove_source": false,
  }))
  .unwrap();
  let Json(response) = transfer_view(
    State(test_state(&db)),
    bearer_headers(user),
    Path((src, folder)),
    Json(request),
  )
  .await
  .expect("copy transfer should succeed");
  let response = serde_json::to_value(&response).unwrap();
  assert_eq!(response["removed_source"], json!(false));

  // Source untouched: both views still live, content unchanged.
  assert!(
    !is_deleted(&db, folder).await,
    "copy must not bin the source"
  );
  assert!(!is_deleted(&db, page).await, "copy must not bin the source");
  assert_eq!(current_blocks_json(&db, doc).await, before);

  // Destination got its own copy, at the destination ROOT (no parent given).
  let dest_views = live_views(&db, dest).await;
  assert_eq!(dest_views.len(), 2, "everything in dest is the copy");
  let copied_folder = dest_views
    .iter()
    .find(|(_, _, name, _)| name == "Orig")
    .unwrap();
  assert_eq!(copied_folder.1, None, "no parent given → destination root");
  assert_ne!(copied_folder.0, folder);
  let copied_leaf = dest_views
    .iter()
    .find(|(_, _, name, _)| name == "Leaf")
    .unwrap();
  assert_eq!(copied_leaf.1, Some(copied_folder.0));
  assert_eq!(current_blocks_json(&db, copied_leaf.3).await, before);
}

/// `POST /api/workspaces/{ws}/views/batch-transfer` where the selection holds
/// a folder AND a page inside it: the nested root is dropped, the page is
/// transferred exactly ONCE — no duplicate, no orphan in the destination.
#[tokio::test]
async fn batch_transfer_transfers_nested_roots_only_once() {
  let Some(db) = pool().await else { return };
  let user = seed_user(&db).await;
  let src = seed_workspace(&db, user).await;
  let dest = seed_workspace(&db, user).await;

  let folder = seed_folder(&db, src, user, None, "F").await;
  let (page, _doc) = seed_document(
    &db,
    src,
    user,
    Some(folder),
    "P",
    json!([{"id": "b1", "type": "paragraph", "text": "nested", "data": {}}]),
  )
  .await;

  let request: BatchTransferRequest = serde_json::from_value(json!({
    "dest_workspace_id": dest,
    "remove_source": true,
    "view_ids": [folder, page],
  }))
  .unwrap();
  let Json(response) = batch_transfer_views(
    State(test_state(&db)),
    bearer_headers(user),
    Path(src),
    Json(request),
  )
  .await
  .expect("batch transfer should succeed");
  let response = serde_json::to_value(&response).unwrap();

  // The page rode along inside its folder — counted once, one root reported.
  assert_eq!(response["documents"], json!(1));
  assert_eq!(response["folders"], json!(1));
  let new_roots = response["new_root_view_ids"]
    .as_array()
    .expect("the batch reports every root's new id");
  assert_eq!(
    new_roots.len(),
    1,
    "a nested selection collapses to the folder alone"
  );
  let new_root: Uuid = new_roots[0].as_str().unwrap().parse().unwrap();

  // Destination: exactly the folder and its page — no second "P", nothing
  // parentless but the folder copy.
  let dest_views = live_views(&db, dest).await;
  assert_eq!(
    dest_views.len(),
    2,
    "destination must hold exactly F + P, no duplicates: {dest_views:?}"
  );
  let pages: Vec<_> = dest_views.iter().filter(|(_, _, n, _)| n == "P").collect();
  assert_eq!(pages.len(), 1, "the page must not be copied twice");
  assert_eq!(pages[0].1, Some(new_root), "P hangs under the F copy");
  let folders: Vec<_> = dest_views.iter().filter(|(_, _, n, _)| n == "F").collect();
  assert_eq!(folders.len(), 1);
  assert_eq!(folders[0].0, new_root);
  assert_eq!(folders[0].1, None, "the moved root sits at the dest root");

  // Source fully binned — a move that copied twice would have failed here
  // (the second write would hit an already-deleted source).
  assert!(is_deleted(&db, folder).await);
  assert!(is_deleted(&db, page).await);
}

/// A batch containing a root that is not live in the source (soft-deleted
/// here) is refused WHOLESALE: NotFound, and not one view written to the
/// destination.
#[tokio::test]
async fn batch_transfer_refuses_a_root_not_live_in_the_source() {
  let Some(db) = pool().await else { return };
  let user = seed_user(&db).await;
  let src = seed_workspace(&db, user).await;
  let dest = seed_workspace(&db, user).await;

  let (live_page, _doc_a) = seed_document(&db, src, user, None, "Live", json!([])).await;
  let (trashed_page, _doc_b) = seed_document(&db, src, user, None, "Trashed", json!([])).await;
  sqlx::query("UPDATE views SET is_deleted = true WHERE id = $1")
    .bind(trashed_page)
    .execute(&db)
    .await
    .unwrap();

  let request: BatchTransferRequest = serde_json::from_value(json!({
    "dest_workspace_id": dest,
    "remove_source": true,
    "view_ids": [live_page, trashed_page],
  }))
  .unwrap();
  let err = batch_transfer_views(
    State(test_state(&db)),
    bearer_headers(user),
    Path(src),
    Json(request),
  )
  .await
  .expect_err("a root that is not live in the source must refuse the batch");
  assert!(
    matches!(err, ApiError::NotFound),
    "expected 404 for a non-live root, got {err:?}"
  );

  // Nothing leaked into the destination, and the live page was left alone.
  assert!(
    live_views(&db, dest).await.is_empty(),
    "a refused batch must not transfer ANY of the selection"
  );
  assert!(
    !is_deleted(&db, live_page).await,
    "the live root must be untouched when its sibling is refused"
  );
}

/// `POST /api/workspaces/{ws}/views/{view}/clone`: same-workspace duplicate —
/// fresh ids everywhere, renamed root, repositioned beside the original, same
/// payload, and the original left exactly as it was.
#[tokio::test]
async fn clone_view_duplicates_a_subtree_in_the_same_workspace() {
  let Some(db) = pool().await else { return };
  let user = seed_user(&db).await;
  let ws = seed_workspace(&db, user).await;

  let folder = seed_folder(&db, ws, user, None, "Docs").await;
  let (page, doc) = seed_document(
    &db,
    ws,
    user,
    Some(folder),
    "P1",
    json!([{"id": "b1", "type": "paragraph", "text": "clone me", "data": {}}]),
  )
  .await;
  let before = current_blocks_json(&db, doc).await;

  let request: CloneRequest = serde_json::from_value(json!({
    "name": "Docs copy",
  }))
  .unwrap();
  let Json(response) = clone_view(
    State(test_state(&db)),
    bearer_headers(user),
    Path((ws, folder)),
    Json(request),
  )
  .await
  .expect("clone should succeed");
  let response = serde_json::to_value(&response).unwrap();
  assert_eq!(response["documents"], json!(1));
  assert_eq!(response["folders"], json!(1));
  assert_eq!(response["new_name"], json!("Docs copy"));
  assert_eq!(response["dry_run"], json!(false));
  let new_root: Uuid = response["new_root_view_id"]
    .as_str()
    .expect("a clone reports its new root")
    .parse()
    .unwrap();
  assert_ne!(new_root, folder, "a clone is a NEW view");

  // The copy: renamed root at top level beside the original, repositioned
  // (a clone never reuses its source's position), child remapped under it.
  let (new_parent, new_name, new_position) = sqlx::query_as::<_, (Option<Uuid>, String, String)>(
    "SELECT parent_view_id, name, position FROM views WHERE id = $1 AND is_deleted = false",
  )
  .bind(new_root)
  .fetch_one(&db)
  .await
  .unwrap();
  assert_eq!(new_parent, None, "no parent given → beside the source root");
  assert_eq!(new_name, "Docs copy");
  let (_, _, src_position) = sqlx::query_as::<_, (Option<Uuid>, String, String)>(
    "SELECT parent_view_id, name, position FROM views WHERE id = $1",
  )
  .bind(folder)
  .fetch_one(&db)
  .await
  .unwrap();
  assert_ne!(
    new_position, src_position,
    "the copy must be repositioned, not stacked on the source's position"
  );

  let cloned: Vec<_> = live_views(&db, ws)
    .await
    .into_iter()
    .filter(|(id, _, _, _)| *id != folder && *id != page)
    .collect();
  assert_eq!(cloned.len(), 2, "exactly the two copies are new");
  let cloned_page = cloned
    .iter()
    .find(|(id, parent, name, _)| *id != new_root && parent == &Some(new_root) && name == "P1")
    .expect("the cloned page hangs under the cloned root");
  assert_eq!(
    current_blocks_json(&db, cloned_page.3).await,
    before,
    "the clone carries the same payload"
  );

  // The original is untouched: still live, same name, same position, same doc.
  assert!(!is_deleted(&db, folder).await);
  assert!(!is_deleted(&db, page).await);
  let (orig_parent, orig_name, orig_position) =
    sqlx::query_as::<_, (Option<Uuid>, String, String)>(
      "SELECT parent_view_id, name, position FROM views WHERE id = $1",
    )
    .bind(folder)
    .fetch_one(&db)
    .await
    .unwrap();
  assert_eq!(orig_parent, None);
  assert_eq!(orig_name, "Docs");
  assert_eq!(orig_position, src_position);
  assert_eq!(current_blocks_json(&db, doc).await, before);
}

// ── clone root title (#101) ──────────────────────────────────────────────────

/// The page title a client shows (and the exporter names its file after) is the
/// ROOT BLOCK's `title`, which the view's name does not touch. A clone that
/// renames only the view therefore opens under the source's old title — and
/// because the copy's payload still says "Original", the next body write
/// persists that stale title right back.
#[tokio::test]
async fn clone_writes_the_new_name_into_the_copy_root_title() {
  let Some(db) = pool().await else { return };
  let user = seed_user(&db).await;
  let ws = seed_workspace(&db, user).await;

  let (page, doc) = seed_document(
    &db,
    ws,
    user,
    None,
    "Original",
    json!([
      {"id": "root", "type": "page", "text": "", "children": ["b1"],
       "data": {"title": "Original"}},
      {"id": "b1", "type": "paragraph", "text": "original body", "data": {}},
    ]),
  )
  .await;

  let request: CloneRequest = serde_json::from_value(json!({ "name": "Copy" })).unwrap();
  let Json(response) = clone_view(
    State(test_state(&db)),
    bearer_headers(user),
    Path((ws, page)),
    Json(request),
  )
  .await
  .expect("clone should succeed");
  let response = serde_json::to_value(&response).unwrap();
  let new_root: Uuid = response["new_root_view_id"]
    .as_str()
    .unwrap()
    .parse()
    .unwrap();
  let copied_doc: Uuid = sqlx::query_scalar("SELECT object_id FROM views WHERE id = $1")
    .bind(new_root)
    .fetch_one(&db)
    .await
    .unwrap();

  let payload = store::current_payload(&db, copied_doc)
    .await
    .unwrap()
    .expect("the copy has a body");
  assert_eq!(
    mica_markdown::document_title(&payload),
    Some("Copy"),
    "the copy's root title must be the name the user sees on the sidebar"
  );
  assert!(
    payload.blocks.iter().any(|b| b.text == "original body"),
    "the body is copied unchanged"
  );

  // A later body write must not resurrect the old title.
  let _ = super::update_document_markdown(
    State(test_state(&db)),
    bearer_headers(user),
    Path((ws, copied_doc)),
    Json(
      serde_json::from_value(json!({
        "mode": "append",
        "markdown": "written after the clone",
      }))
      .unwrap(),
    ),
  )
  .await
  .expect("a body write on the copy should succeed");
  let payload = store::current_payload(&db, copied_doc)
    .await
    .unwrap()
    .expect("the copy still has a body");
  assert_eq!(
    mica_markdown::document_title(&payload),
    Some("Copy"),
    "a body write must not bring the source's title back"
  );

  let source = store::current_payload(&db, doc).await.unwrap().unwrap();
  assert_eq!(
    mica_markdown::document_title(&source),
    Some("Original"),
    "the source keeps its own title"
  );
}

// ── icons (#156) ─────────────────────────────────────────────────────────────

/// An icon is its own `views` column — it is not in the document payload, so a
/// copy that rewrites payloads and forgets the column drops every emoji.
async fn set_icon(db: &PgPool, view: Uuid, icon: &str) {
  sqlx::query("UPDATE views SET icon = $2 WHERE id = $1")
    .bind(view)
    .bind(icon)
    .execute(db)
    .await
    .unwrap();
}

/// (name, icon) of a workspace's live views, ordered by name.
async fn live_icons(db: &PgPool, ws: Uuid) -> Vec<(String, Option<String>)> {
  sqlx::query_as(
    "SELECT name, icon FROM views WHERE workspace_id = $1 AND is_deleted = false ORDER BY name",
  )
  .bind(ws)
  .fetch_all(db)
  .await
  .unwrap()
}

/// A cross-workspace move carries the icon of every node it copies: the ROOT,
/// a descendant page and a descendant folder. A node that never had one keeps
/// none — the fix must not invent an icon where there was none.
#[tokio::test]
async fn transfer_preserves_icons_of_root_page_and_folder() {
  let Some(db) = pool().await else { return };
  let user = seed_user(&db).await;
  let src = seed_workspace(&db, user).await;
  let dest = seed_workspace(&db, user).await;

  let folder = seed_folder(&db, src, user, None, "Icons").await;
  let (page, _doc) = seed_document(
    &db,
    src,
    user,
    Some(folder),
    "Emoji page",
    json!([{"id": "b1", "type": "paragraph", "text": "hello", "data": {}}]),
  )
  .await;
  let sub = seed_folder(&db, src, user, Some(folder), "Emoji folder").await;
  let (plain, _plain_doc) = seed_document(&db, src, user, Some(folder), "Plain", json!([])).await;
  set_icon(&db, folder, "📁").await;
  set_icon(&db, page, "📄").await;
  set_icon(&db, sub, "🗂").await;
  let _ = plain;

  let request: TransferRequest = serde_json::from_value(json!({
    "dest_workspace_id": dest,
    "remove_source": true,
  }))
  .unwrap();
  let _ = transfer_view(
    State(test_state(&db)),
    bearer_headers(user),
    Path((src, folder)),
    Json(request),
  )
  .await
  .expect("transfer should succeed");

  assert_eq!(
    live_icons(&db, dest).await,
    vec![
      ("Emoji folder".to_string(), Some("🗂".to_string())),
      ("Emoji page".to_string(), Some("📄".to_string())),
      ("Icons".to_string(), Some("📁".to_string())),
      ("Plain".to_string(), None),
    ],
    "every copied node keeps its icon, and an icon-less node stays icon-less"
  );
}

/// A same-workspace clone carries the icons the same way: renamed root, and
/// every descendant page/folder.
#[tokio::test]
async fn clone_preserves_icons_of_root_page_and_folder() {
  let Some(db) = pool().await else { return };
  let user = seed_user(&db).await;
  let ws = seed_workspace(&db, user).await;

  let folder = seed_folder(&db, ws, user, None, "Icons").await;
  let (page, _doc) = seed_document(
    &db,
    ws,
    user,
    Some(folder),
    "Emoji page",
    json!([{"id": "b1", "type": "paragraph", "text": "hello", "data": {}}]),
  )
  .await;
  let sub = seed_folder(&db, ws, user, Some(folder), "Emoji folder").await;
  set_icon(&db, folder, "📁").await;
  set_icon(&db, page, "📄").await;
  set_icon(&db, sub, "🗂").await;

  let request: CloneRequest = serde_json::from_value(json!({ "name": "Icons copy" })).unwrap();
  let Json(response) = clone_view(
    State(test_state(&db)),
    bearer_headers(user),
    Path((ws, folder)),
    Json(request),
  )
  .await
  .expect("clone should succeed");
  let response = serde_json::to_value(&response).unwrap();
  let new_root: Uuid = response["new_root_view_id"]
    .as_str()
    .unwrap()
    .parse()
    .unwrap();

  // The source rows are looked up by ID, not by name: the copy's descendants
  // keep the source's names on purpose.
  let original: Vec<(String, Option<String>)> = sqlx::query_as(
    "SELECT name, icon FROM views WHERE id = ANY($1) AND is_deleted = false ORDER BY name",
  )
  .bind(&[folder, page, sub][..])
  .fetch_all(&db)
  .await
  .unwrap();
  assert_eq!(
    original,
    vec![
      ("Emoji folder".to_string(), Some("🗂".to_string())),
      ("Emoji page".to_string(), Some("📄".to_string())),
      ("Icons".to_string(), Some("📁".to_string())),
    ],
    "the source keeps its own icons"
  );
  let copied: Vec<(String, Option<String>)> = sqlx::query_as(
    "SELECT name, icon FROM views WHERE workspace_id = $1 AND is_deleted = false \
       AND id = $2 OR (workspace_id = $1 AND is_deleted = false AND parent_view_id = $2) \
     ORDER BY name",
  )
  .bind(ws)
  .bind(new_root)
  .fetch_all(&db)
  .await
  .unwrap();
  assert_eq!(
    copied,
    vec![
      ("Emoji folder".to_string(), Some("🗂".to_string())),
      ("Emoji page".to_string(), Some("📄".to_string())),
      ("Icons copy".to_string(), Some("📁".to_string())),
    ],
    "the copy keeps every icon, including the renamed root's"
  );
}

// ── reorder (gh#48) ──────────────────────────────────────────────────────────
// #48 replaced reorder's per-id validation (three queries each) and per-id
// writes with set operations, kept inside the gh#46 tree lock. These pin the
// BEHAVIOUR — reparenting, spacing, and each refusal — whatever the query count.

use super::transfer::reorder_views;

/// `POST /api/workspaces/{ws}/views/reorder` re-parents every listed view and
/// hands out 10-spaced, zero-padded positions in the caller's order.
#[tokio::test]
async fn reorder_views_reparents_in_one_call_with_spaced_positions() {
  let Some(db) = pool().await else { return };
  let user = seed_user(&db).await;
  let ws = seed_workspace(&db, user).await;
  let folder = seed_folder(&db, ws, user, None, "F").await;
  let (a, _) = seed_document(&db, ws, user, None, "A", json!([])).await;
  let (b, _) = seed_document(&db, ws, user, None, "B", json!([])).await;
  let (c, _) = seed_document(&db, ws, user, None, "C", json!([])).await;

  let request: super::ReorderRequest = serde_json::from_value(json!({
    "parent_view_id": folder,
    "ordered_view_ids": [c, a, b],
  }))
  .unwrap();
  let Json(response) = reorder_views(
    State(test_state(&db)),
    bearer_headers(user),
    Path(ws),
    Json(request),
  )
  .await
  .expect("reorder should succeed");
  let response = serde_json::to_value(&response).unwrap();
  assert_eq!(response["reordered"], json!(3));

  // c first, then a, then b — spaced exactly as the old per-id writes spaced
  // them, and all three now under the target folder.
  let rows: Vec<(String, Option<Uuid>)> = sqlx::query_as(
    "SELECT position, parent_view_id FROM views \
       WHERE id = ANY($1) AND is_deleted = false ORDER BY position",
  )
  .bind(&[c, a, b][..])
  .fetch_all(&db)
  .await
  .unwrap();
  assert_eq!(
    rows,
    vec![
      ("0000000010".to_string(), Some(folder)),
      ("0000000020".to_string(), Some(folder)),
      ("0000000030".to_string(), Some(folder)),
    ],
    "caller order IS the sibling order, 10-spaced, under the new parent"
  );
}

/// A reorder naming a view that is not live in the workspace is a 404 — the
/// same error the old per-id membership check gave, now out of ONE set query.
#[tokio::test]
async fn reorder_views_rejects_a_view_not_live_in_the_workspace() {
  let Some(db) = pool().await else { return };
  let user = seed_user(&db).await;
  let ws = seed_workspace(&db, user).await;
  let (a, _) = seed_document(&db, ws, user, None, "A", json!([])).await;

  let request: super::ReorderRequest = serde_json::from_value(json!({
    "ordered_view_ids": [a, Uuid::new_v4()],
  }))
  .unwrap();
  let err = reorder_views(
    State(test_state(&db)),
    bearer_headers(user),
    Path(ws),
    Json(request),
  )
  .await
  .expect_err("a reorder into thin air must be refused");
  assert!(
    matches!(err, ApiError::NotFound),
    "expected 404 for a non-live id, got {err:?}"
  );

  // And because validation precedes every write, the LIVE id kept its old spot.
  let (parent, position): (Option<Uuid>, String) = sqlx::query_as(
    "SELECT parent_view_id, position FROM views WHERE id = $1 AND is_deleted = false",
  )
  .bind(a)
  .fetch_one(&db)
  .await
  .unwrap();
  assert_eq!((parent, position), (None, "0".to_string()));
}

/// Re-parenting folder F1 under its own descendant folder F2 is a 400 from the
/// cycle walk seeded with ALL ids at once — the same refusal the per-id walk
/// gave (and still gives via `ensure_valid_parent_view` elsewhere).
#[tokio::test]
async fn reorder_views_rejects_a_descendant_parent() {
  let Some(db) = pool().await else { return };
  let user = seed_user(&db).await;
  let ws = seed_workspace(&db, user).await;
  let f1 = seed_folder(&db, ws, user, None, "F1").await;
  let f2 = seed_folder(&db, ws, user, Some(f1), "F2").await;

  let request: super::ReorderRequest = serde_json::from_value(json!({
    "parent_view_id": f2,
    "ordered_view_ids": [f1],
  }))
  .unwrap();
  let err = reorder_views(
    State(test_state(&db)),
    bearer_headers(user),
    Path(ws),
    Json(request),
  )
  .await
  .expect_err("a folder under its own descendant must be refused");
  assert!(
    matches!(err, ApiError::BadRequest(_)),
    "expected 400 for a descendant parent, got {err:?}"
  );
  assert!(
    !is_deleted(&db, f1).await && !is_deleted(&db, f2).await,
    "a refused reorder writes nothing"
  );
}

/// The batch ceiling the route never had (#48): past MAX_BATCH_VIEWS ids the
/// request is refused before validation touches the database at all.
#[tokio::test]
async fn reorder_views_rejects_a_list_past_the_batch_ceiling() {
  let Some(db) = pool().await else { return };
  let user = seed_user(&db).await;
  let ws = seed_workspace(&db, user).await;
  let ids: Vec<Uuid> = (0..1001).map(|_| Uuid::new_v4()).collect();

  let request: super::ReorderRequest = serde_json::from_value(json!({
    "ordered_view_ids": ids,
  }))
  .unwrap();
  let err = reorder_views(
    State(test_state(&db)),
    bearer_headers(user),
    Path(ws),
    Json(request),
  )
  .await
  .expect_err("an oversized reorder must be refused");
  match err {
    ApiError::BadRequest(message) => assert!(
      message.contains("at most"),
      "the refusal should name the ceiling, got: {message}"
    ),
    other => panic!("expected 400 for an oversized list, got {other:?}"),
  }
}
