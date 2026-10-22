// Postgres-backed integration tests for the documents routes. Kept in one
// module because they share seed fixtures (pool/seed_workspace/seed_view).
// The page-tree invariant guard runs entirely in SQL — `object_type` lives in
// Postgres, and the backstop is a DB trigger — so a mock would assert nothing.
// Gated on `DATABASE_URL`, hardened the same way as auth.rs::refresh_pg and
// app-core/tests/sync_pg.rs: skipped (green) without a database locally, but a
// set-but-unreachable URL — or a missing one in CI — is a hard failure, never
// a silent pass.
//
//   $env:DATABASE_URL="postgres://mica:mica@127.0.0.1:5432/mica"
//   cargo test -p mica-api-server parent_guard_pg
#[cfg(test)]
use super::*;
use super::search::scan_backlinks;
use mica_app_core::store;
use mica_app_core::sync::page_link_targets;
use sqlx::PgPool;
use uuid::Uuid;

  mod parent_guard_pg {
    use super::super::*;
    use mica_app_core::AppState;
    use super::super::export::{
      build_export_manifest, fetch_export_images, gathered_tree_bytes, parse_export_skip_assets,
      stream_tree_bytes,
    };
    use super::super::search::{scan_backlinks, search_views, workspace_graph};
    use super::super::transfer::{
      ancestor_pairs_of_roots, independent_roots, live_subtree, remove_moved_source,
    };
    use super::super::trash::{purge_one, purge_views_batch, restore_views_batch, trash_views_batch};
    use mica_app_core::search::BodyIndex;
    use mica_infra::ApiError;
    use sqlx::PgPool;
    use uuid::Uuid;

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

    #[tokio::test]
    async fn incremental_export_detects_body_writes_without_tree_timestamps() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;
      let (view, doc) = seed_document(&db, ws, user, "Page", serde_json::json!([])).await;
      let initial = build_export_manifest(&db, ws).await.unwrap();
      assert_eq!(initial.pages.len(), 1);
      assert_eq!(initial.pages[0].view_id, view);
      let (_, actual_revision) = store::current_payload_versioned(&db, doc)
        .await
        .unwrap()
        .unwrap();
      assert_eq!(initial.pages[0].revision, actual_revision);
      let tree_updated_at: DateTime<Utc> =
        sqlx::query_scalar("SELECT updated_at FROM views WHERE id = $1")
          .bind(view)
          .fetch_one(&db)
          .await
          .unwrap();

      // Deliberately bypass all timestamp/revision-maintaining application code.
      // Any state writer must invalidate the export, even one added in future.
      let replacement = mica_core::MicaDoc::from_blocks(
        "root",
        &[mica_core::Block {
          id: "root".into(),
          kind: "paragraph".into(),
          text: "new body".into(),
          data: serde_json::Value::Null,
          children: vec![],
        }],
      );
      sqlx::query("UPDATE document_yrs_base SET state = $1 WHERE document_id = $2")
        .bind(replacement.encode_state())
        .bind(doc)
        .execute(&db)
        .await
        .unwrap();
      let changed = build_export_manifest(&db, ws).await.unwrap();
      assert_ne!(initial.pages[0].revision, changed.pages[0].revision);
      assert_eq!(initial.structure_revision, changed.structure_revision);
      let (payload, actual_revision) = store::current_payload_versioned(&db, doc)
        .await
        .unwrap()
        .unwrap();
      assert_eq!(changed.pages[0].revision, actual_revision);
      assert!(payload.blocks.iter().any(|b| b.text == "new body"));
      assert_eq!(
        tree_updated_at,
        sqlx::query_scalar::<_, DateTime<Utc>>("SELECT updated_at FROM views WHERE id = $1")
          .bind(view)
          .fetch_one(&db)
          .await
          .unwrap()
      );

      sqlx::query("UPDATE views SET name = 'Renamed' WHERE id = $1")
        .bind(view)
        .execute(&db)
        .await
        .unwrap();
      let renamed = build_export_manifest(&db, ws).await.unwrap();
      assert_ne!(changed.structure_revision, renamed.structure_revision);
      assert_eq!(changed.pages[0].revision, renamed.pages[0].revision);
      sqlx::query("UPDATE views SET is_deleted = true WHERE id = $1")
        .bind(view)
        .execute(&db)
        .await
        .unwrap();
      let deleted = build_export_manifest(&db, ws).await.unwrap();
      assert!(deleted.pages.is_empty());
      assert_eq!(deleted.manifest["pages"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn incremental_export_tracks_asset_revisions_without_invalidating_the_tree() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;
      seed_document(&db, ws, user, "Page", serde_json::json!([])).await;
      let before = build_export_manifest(&db, ws).await.unwrap();
      assert!(before.files.is_empty());
      let file_id = Uuid::new_v4();
      sqlx::query(
        "INSERT INTO files(id, workspace_id, uploaded_by, object_key, original_name, mime_type, byte_size)
         VALUES($1,$2,$3,$4,'photo.png','image/png',123)",
      ).bind(file_id).bind(ws).bind(user).bind(format!("test/{file_id}"))
        .execute(&db).await.unwrap();
      let uploaded = build_export_manifest(&db, ws).await.unwrap();
      assert_eq!(before.tree_revision, uploaded.tree_revision);
      assert_ne!(before.structure_revision, uploaded.structure_revision);
      assert_eq!(before.pages[0].revision, uploaded.pages[0].revision);
      assert_eq!(uploaded.files.len(), 1);
      assert_eq!(uploaded.files[0].file_id, file_id);

      sqlx::query("UPDATE files SET object_key = $1 WHERE id = $2")
        .bind(format!("replacement/{file_id}"))
        .bind(file_id)
        .execute(&db)
        .await
        .unwrap();
      let replaced = build_export_manifest(&db, ws).await.unwrap();
      assert_eq!(uploaded.tree_revision, replaced.tree_revision);
      assert_ne!(uploaded.structure_revision, replaced.structure_revision);
      assert_ne!(uploaded.files[0].revision, replaced.files[0].revision);
      // A second, unrelated upload leaves the existing file version intact.
      let other = Uuid::new_v4();
      sqlx::query(
        "INSERT INTO files(id, workspace_id, uploaded_by, object_key, original_name, mime_type, byte_size)
         VALUES($1,$2,$3,$4,'another.png','image/png',456)",
      ).bind(other).bind(ws).bind(user).bind(format!("test/{other}"))
        .execute(&db).await.unwrap();
      let unrelated = build_export_manifest(&db, ws).await.unwrap();
      assert_eq!(replaced.tree_revision, unrelated.tree_revision);
      assert_eq!(
        replaced.files[0].revision,
        unrelated
          .files
          .iter()
          .find(|f| f.file_id == file_id)
          .unwrap()
          .revision
      );
      assert_eq!(unrelated.files.len(), 2);
    }

    #[test]
    fn incremental_export_skip_assets_validates_ids_and_bounds_the_list() {
      assert!(parse_export_skip_assets(None).unwrap().is_empty());
      assert!(parse_export_skip_assets(Some("")).unwrap().is_empty());
      assert!(parse_export_skip_assets(Some("not-a-uuid")).is_err());
      let ids: Vec<_> = (0..64).map(|_| Uuid::new_v4().to_string()).collect();
      assert_eq!(
        parse_export_skip_assets(Some(&ids.join(",")))
          .unwrap()
          .len(),
        64
      );
      let too_many = format!("{},{}", ids.join(","), Uuid::new_v4());
      assert!(parse_export_skip_assets(Some(&too_many)).is_err());
      assert!(parse_export_skip_assets(Some(&format!("{},", ids[0]))).is_err());
      let uppercase = ids[0].to_uppercase();
      assert!(
        parse_export_skip_assets(Some(&uppercase))
          .unwrap()
          .contains(&ids[0])
      );
    }

    #[tokio::test]
    async fn incremental_export_reuses_asset_metadata_without_accessing_object_storage() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;
      let file_id = Uuid::new_v4();
      let (_, doc) = seed_document(
        &db,
        ws,
        user,
        "Page",
        serde_json::json!([{
          "id":"image", "type":"image", "data":{"file_id":file_id,"name":"photo.png"}
        }]),
      )
      .await;
      let mut config = mica_infra::AppConfig::from_env().unwrap();
      config.jwt_secret = "incremental-export-test-secret".into();
      config.environment = mica_infra::Environment::Test;
      let mut state = AppState::new(
        config,
        db.clone(),
        std::sync::Arc::new(mica_infra::LogMailer),
      );
      // No endpoint is configured: a path that attempted a download cannot pass.
      state.storage = None;
      let record = store::FileRecord {
        id: file_id,
        workspace_id: ws,
        uploaded_by: user,
        object_key: format!("test/{file_id}"),
        original_name: "photo.png".into(),
        mime_type: "image/png".into(),
        byte_size: 3,
        created_at: Utc::now(),
      };
      let payload = store::current_payload(&db, doc).await.unwrap().unwrap();
      let payloads = std::collections::HashMap::from([(doc.to_string(), payload)]);
      let skipped = std::collections::HashSet::from([file_id.to_string()]);
      let images = fetch_export_images(
        &state,
        ws,
        &payloads,
        Some(std::slice::from_ref(&record)),
        &skipped,
      )
      .await
      .unwrap();
      let image = images.get(&file_id.to_string()).unwrap();
      assert_eq!(image.name, "photo.png");
      assert!(image.bytes.is_empty());
      assert!(
        fetch_export_images(
          &state,
          ws,
          &payloads,
          Some(&[record]),
          &std::collections::HashSet::new()
        )
        .await
        .is_err()
      );
    }

    /// D3: the tree exports stream now — one page and one image at a time
    /// instead of every payload and image of the subtree held at once. The
    /// archive must be the very bytes the gathered build produced, for the
    /// whole workspace and for a folder's subtree (folders, a page inside one,
    /// a page linking to it, a page with no body).
    #[tokio::test]
    async fn the_streamed_tree_export_is_byte_identical_to_the_gathered_one() {
      let Some(db) = pool().await else { return };
      let (ws, folder, inner, _) = seed_folder_with_page(&db).await;
      let user = ws_owner(&db, ws).await;
      seed_document(
        &db,
        ws,
        user,
        "Top",
        serde_json::json!([{
          "id": "p", "type": "paragraph", "text": "see inner",
          "data": {"marks": [{"type": "link", "start": 4, "end": 9,
                              "href": format!("mica://page/{inner}")}]}
        }]),
      )
      .await;
      let (empty_view, empty_doc) = seed_document(&db, ws, user, "Hollow", serde_json::json!([])).await;
      sqlx::query("DELETE FROM document_yrs_base WHERE document_id = $1")
        .bind(empty_doc)
        .execute(&db)
        .await
        .unwrap();
      let _ = empty_view;
      let mut config = mica_infra::AppConfig::from_env().unwrap();
      config.jwt_secret = "streamed-export-test-secret".into();
      config.environment = mica_infra::Environment::Test;
      let mut state = AppState::new(config, db.clone(), std::sync::Arc::new(mica_infra::LogMailer));
      state.storage = None;

      let streamed = stream_tree_bytes(&state, ws, None).await;
      assert_eq!(streamed, gathered_tree_bytes(&state, ws, None).await);
      let names: Vec<String> =
        mica_interchange::read_zip(&streamed).into_iter().map(|e| e.name).collect();
      assert!(names.contains(&"manifest.json".to_string()), "{names:?}");
      assert!(names.iter().any(|n| n.ends_with("里面的页.md")), "{names:?}");
      assert_eq!(
        stream_tree_bytes(&state, ws, Some(folder)).await,
        gathered_tree_bytes(&state, ws, Some(folder)).await
      );
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

    /// Insert a LIVE document view straight into the table under `parent`
    /// (in `ws`) — the shape of the import executor's per-page INSERT, with
    /// no API guard in front of it.
    async fn insert_child(db: &PgPool, ws: Uuid, user: Uuid, parent: Uuid) -> Result<(), sqlx::Error> {
      sqlx::query(
        "INSERT INTO views(workspace_id,parent_view_id,object_id,object_type,name,position,created_by) \
         VALUES($1,$2,$3,'document'::object_type,'V','0',$4)",
      )
      .bind(ws)
      .bind(parent)
      .bind(Uuid::new_v4())
      .bind(user)
      .execute(db)
      .await
      .map(|_| ())
    }

    async fn status_of(error: sqlx::Error) -> (axum::http::StatusCode, String) {
      use axum::response::IntoResponse;
      let response = ApiError::from(error).into_response();
      let status = response.status();
      let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
      (status, String::from_utf8(body.to_vec()).unwrap())
    }

    /// C2 (review 2026-10-21): `ensure_parent_accepts_children` runs once, at
    /// the START of a request — and an import runs for minutes after it. A
    /// folder trashed in that window kept receiving pages: live rows under a
    /// deleted parent, invisible in the tree, then deleted for real by
    /// `parent_view_id ON DELETE CASCADE` when the bin was emptied, leaving
    /// their `documents`/share rows behind. The trigger is the only check that
    /// runs at the moment of the write, so it has to know about the bin.
    #[tokio::test]
    async fn the_trigger_refuses_a_live_child_under_a_trashed_folder() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;
      let folder = seed_view(&db, ws, user, "folder").await;
      // The request-time check passed …
      ensure_parent_accepts_children(&db, ws, folder).await.unwrap();
      // … then the folder went to the bin while the import kept running.
      trash_views_batch(&db, ws, &[folder]).await.unwrap();

      let error = insert_child(&db, ws, user, folder)
        .await
        .expect_err("a live page must not land under a trashed folder");
      // A conflict the caller can act on (refresh, pick another folder), not
      // a 500 — and not the raw Postgres text either.
      let (status, body) = status_of(error).await;
      assert_eq!(status, axum::http::StatusCode::CONFLICT, "{body}");
      assert!(body.contains("folder"), "{body}");
      assert!(!body.contains(&folder.to_string()), "no raw database text: {body}");
    }

    /// The window the trigger itself could leave open: a trash of the folder
    /// that has written the row but not yet committed. A plain read inside the
    /// trigger sees the old, live row and lets the child in; `FOR SHARE` makes
    /// the insert wait for the trash to commit and then see the bin.
    #[tokio::test]
    async fn an_insert_racing_an_uncommitted_trash_waits_and_is_refused() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;
      let folder = seed_view(&db, ws, user, "folder").await;

      let mut trash = db.begin().await.unwrap();
      sqlx::query("UPDATE views SET is_deleted = true WHERE id = $1")
        .bind(folder)
        .execute(&mut *trash)
        .await
        .unwrap();
      let db2 = db.clone();
      let insert = tokio::spawn(async move { insert_child(&db2, ws, user, folder).await });
      tokio::time::sleep(std::time::Duration::from_millis(300)).await;
      assert!(!insert.is_finished(), "the insert must wait on the trash, not read past it");
      trash.commit().await.unwrap();

      assert!(
        insert.await.unwrap().is_err(),
        "after the trash commits, the waiting insert sees the bin and is refused"
      );
    }

    #[tokio::test]
    async fn the_trigger_refuses_moving_a_live_page_under_a_trashed_folder() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;
      let folder = seed_view(&db, ws, user, "folder").await;
      let page = seed_view(&db, ws, user, "document").await;
      trash_views_batch(&db, ws, &[folder]).await.unwrap();

      let moved = sqlx::query("UPDATE views SET parent_view_id = $1 WHERE id = $2")
        .bind(folder)
        .bind(page)
        .execute(&db)
        .await;
      assert!(moved.is_err(), "a move is a write of parent_view_id too");
    }

    /// The same rule's other half: nothing checks at write time that a parent
    /// lives in the CHILD's workspace — a cross-workspace edge makes a subtree
    /// that one workspace's queries walk into the other's rows.
    #[tokio::test]
    async fn the_trigger_refuses_a_parent_in_another_workspace() {
      let Some(db) = pool().await else { return };
      let (ws_a, user) = seed_workspace(&db).await;
      let (ws_b, _) = seed_workspace(&db).await;
      let folder_in_a = seed_view(&db, ws_a, user, "folder").await;

      let error = insert_child(&db, ws_b, user, folder_in_a)
        .await
        .expect_err("a parent from another workspace must be refused");
      assert_eq!(status_of(error).await.0, axum::http::StatusCode::CONFLICT);
      // The ordinary path is untouched.
      insert_child(&db, ws_a, user, folder_in_a).await.unwrap();
    }

    /// Issue #47: restoring a folder brings back what was deleted WITH it, not
    /// a child the user had trashed on its own before. Rows trashed before the
    /// `deleted_at` stamp existed (NULL) still restore together, as before.
    #[tokio::test]
    async fn restore_brings_back_only_what_was_deleted_together() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;
      let folder = seed_view(&db, ws, user, "folder").await;
      let child = |db: PgPool| async move {
        let id: Uuid = sqlx::query_scalar(
          "INSERT INTO views(workspace_id,parent_view_id,object_id,object_type,name,position,created_by) \
           VALUES($1,$2,$3,'document'::object_type,'V','0',$4) RETURNING id",
        )
        .bind(ws)
        .bind(folder)
        .bind(Uuid::new_v4())
        .bind(user)
        .fetch_one(&db)
        .await
        .unwrap();
        id
      };
      let early = child(db.clone()).await;
      let kept = child(db.clone()).await;
      let deleted = |id: Uuid| {
        let db = db.clone();
        async move {
          sqlx::query_scalar::<_, bool>("SELECT is_deleted FROM views WHERE id = $1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap()
        }
      };

      trash_views_batch(&db, ws, &[early]).await.unwrap();
      // A distinct action, so a distinct stamp.
      tokio::time::sleep(std::time::Duration::from_millis(5)).await;
      trash_views_batch(&db, ws, &[folder]).await.unwrap();

      let restored = restore_views_batch(&db, ws, &[folder]).await.unwrap();
      assert_eq!(restored.len(), 2, "{restored:?}");
      assert!(!deleted(folder).await && !deleted(kept).await);
      assert!(deleted(early).await, "trashed on its own earlier: it stays in the bin");

      // Legacy rows (no stamp) behave as one batch.
      trash_views_batch(&db, ws, &[folder]).await.unwrap();
      sqlx::query("UPDATE views SET deleted_at = NULL WHERE workspace_id = $1")
        .bind(ws)
        .execute(&db)
        .await
        .unwrap();
      restore_views_batch(&db, ws, &[folder]).await.unwrap();
      assert!(!deleted(early).await && !deleted(kept).await);
    }

    /// What must keep working under the stricter trigger: the bin itself. A
    /// trashed child may sit under a trashed folder (that is what trashing a
    /// folder produces), and restoring a page whose folder is STILL in the bin
    /// lifts it to the top level instead of being refused — the trigger fires
    /// on writes of `parent_view_id`, never on `is_deleted` flips.
    #[tokio::test]
    async fn the_bin_round_trip_survives_the_stricter_trigger() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;
      let folder = seed_view(&db, ws, user, "folder").await;
      let child = Uuid::new_v4();
      sqlx::query(
        "INSERT INTO views(id,workspace_id,parent_view_id,object_id,object_type,name,position,created_by) \
         VALUES($1,$2,$3,$4,'document'::object_type,'V','0',$5)",
      )
      .bind(child)
      .bind(ws)
      .bind(folder)
      .bind(Uuid::new_v4())
      .bind(user)
      .execute(&db)
      .await
      .unwrap();

      let trashed = trash_views_batch(&db, ws, &[folder]).await.unwrap();
      assert_eq!(trashed.len(), 2, "folder and child go to the bin together");
      // A trashed row may still be written under a trashed parent.
      sqlx::query("UPDATE views SET parent_view_id = $1 WHERE id = $2")
        .bind(folder)
        .bind(child)
        .execute(&db)
        .await
        .expect("trashed under trashed is the bin's own shape");

      // Restore just the child: its folder is still in the bin → lifted.
      restore_views_batch(&db, ws, &[child]).await.unwrap();
      let (deleted, parent): (bool, Option<Uuid>) =
        sqlx::query_as("SELECT is_deleted, parent_view_id FROM views WHERE id = $1")
          .bind(child)
          .fetch_one(&db)
          .await
          .unwrap();
      assert!(!deleted);
      assert_eq!(parent, None, "lifted to the top level, not refused");

      // And the folder restores too.
      restore_views_batch(&db, ws, &[folder]).await.unwrap();
      let live: bool = sqlx::query_scalar("SELECT NOT is_deleted FROM views WHERE id = $1")
        .bind(folder)
        .fetch_one(&db)
        .await
        .unwrap();
      assert!(live);
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

    /// Within the name tier, how WELL the name matches: exact, then prefix, then
    /// contains — each against a fresher rival. It was a yes/no, so the page
    /// NAMED "部署" sat below "旧版部署记录" whenever that was edited later.
    #[tokio::test]
    async fn an_exact_name_outranks_a_prefix_which_outranks_a_mention() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;
      let (contains, _) = seed_document(&db, ws, user, "旧版部署记录", serde_json::json!([])).await;
      let (prefix, _) = seed_document(&db, ws, user, "部署手册", serde_json::json!([])).await;
      let (exact, _) = seed_document(&db, ws, user, "部署", serde_json::json!([])).await;
      for (view, age) in [(exact, "30 days"), (prefix, "10 days"), (contains, "0 days")] {
        sqlx::query("UPDATE views SET updated_at = now() - $2::interval WHERE id = $1")
          .bind(view)
          .bind(age)
          .execute(&db)
          .await
          .unwrap();
      }
      let hits = search_views(&db, &BodyIndex::default(), &[ws], "部署", false, None).await.unwrap();
      let order: Vec<Uuid> = hits.iter().map(|h| h.view_id).collect();
      assert_eq!(order, vec![exact, prefix, contains], "{hits:?}");
    }

    /// The snippet is the one-line window the client can show: the match near
    /// its start, inside the matched block, no raw newlines.
    #[tokio::test]
    async fn a_body_hit_snippet_shows_the_match_on_one_line() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;
      let (view, doc) = seed_document(&db, ws, user, "笔记", serde_json::json!([])).await;
      let body = format!("{}\n第二段落：上线前检查部署流程。\n结尾", "很长的第一段".repeat(30));
      set_content_text(&db, doc, &body).await;
      let hits = search_views(&db, &BodyIndex::default(), &[ws], "部署流程", false, None).await.unwrap();
      assert_eq!(hits[0].view_id, view);
      let snippet = &hits[0].snippet;
      assert!(snippet.starts_with("…第二段落"), "{snippet}");
      assert!(!snippet.contains('\n'), "{snippet}");
      assert!(snippet.chars().take(25).collect::<String>().contains("部署流程"), "{snippet}");
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

    /// Permanent delete (`purge_one`, behind `purge_view`) must erase the document AND every
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

      // Only the bin can be purged, so it goes there first — the way the UI does it.
      trash_views_batch(&db, ws, &[view]).await.unwrap();
      purge_one(&db, ws, view).await.unwrap();

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

    /// The single purge must refuse a page that is not in the bin.
    ///
    /// It used to have its own statement with no `is_deleted` filter: one
    /// DELETE from any editor permanently erased a LIVE page and its subtree,
    /// with no recoverable step (2026-10-21 review P1-8). It is now the batch
    /// purge of one id, so this pins the call the handler actually makes.
    #[tokio::test]
    async fn a_single_purge_leaves_a_live_page_alone() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;
      let (view, doc) = seed_document(&db, ws, user, "活页", serde_json::json!([])).await;

      assert!(
        matches!(purge_one(&db, ws, view).await, Err(ApiError::NotFound)),
        "purging a page that is not in the bin must 404"
      );
      let alive: i64 = sqlx::query_scalar("SELECT count(*) FROM documents WHERE id=$1")
        .bind(doc)
        .fetch_one(&db)
        .await
        .unwrap();
      assert_eq!(alive, 1, "the live page's document was erased");
    }

    /// The bulk purge must refuse to touch a page that was never trashed.
    ///
    /// Batching a purge that skipped the bin check would turn one call into
    /// "destroy 300 pages nobody deleted, with no recoverable step". So the
    /// untrashed id has to survive, and come back as skipped.
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

    /// THE BUG: restoring only the PAGE out of a trashed folder left it live
    /// under a deleted parent — invisible in the tree — and emptying the trash
    /// then deleted it for real through `parent_view_id ON DELETE CASCADE`.
    /// Single-item restore lifted it to the top level; the batch twin did not.
    #[tokio::test]
    async fn a_bulk_restore_lifts_a_page_out_of_a_still_trashed_folder() {
      let Some(db) = pool().await else { return };
      let (ws, folder, page, _) = seed_folder_with_page(&db).await;
      trash_views_batch(&db, ws, &[folder]).await.unwrap();

      let restored = restore_views_batch(&db, ws, &[page]).await.unwrap();

      assert_eq!(restored, vec![page]);
      assert!(is_deleted(&db, folder).await, "the folder stays in the trash");
      let parent: Option<Uuid> = sqlx::query_scalar("SELECT parent_view_id FROM views WHERE id = $1")
        .bind(page)
        .fetch_one(&db)
        .await
        .unwrap();
      assert_eq!(parent, None, "lifted to the top level, like single restore");

      // Purging the folder (what emptying the trash does) must not take it.
      purge_views_batch(&db, ws, &[folder]).await.unwrap();
      let alive: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM views WHERE id = $1)")
        .bind(page)
        .fetch_one(&db)
        .await
        .unwrap();
      assert!(alive, "the restored page survives the purge");
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

    /// Issue #46: two concurrent opposite moves (A under B, B under A) must not
    /// both pass the cycle check. Racing them repeatedly; at most one wins.
    #[tokio::test]
    async fn concurrent_opposite_moves_never_form_a_cycle() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;
      for _ in 0..40 {
        let a = seed_view(&db, ws, user, "folder").await;
        let b = seed_view(&db, ws, user, "folder").await;
        let (ab, ba) = ([a], [b]);
        let (r1, r2) = tokio::join!(
          move_views_tx(&db, ws, &ab, Some(b)),
          move_views_tx(&db, ws, &ba, Some(a)),
        );
        assert!(r1.is_err() || r2.is_err(), "both moves succeeded: a cycle");
        let parents: Vec<Option<Uuid>> =
          sqlx::query_scalar("SELECT parent_view_id FROM views WHERE id = ANY($1)")
            .bind([a, b])
            .fetch_all(&db)
            .await
            .unwrap();
        assert!(parents.iter().any(Option::is_none), "{parents:?}");
      }
    }

    /// A cycle already in the data (written before the lock existed) must not
    /// send the recursive walks into an endless loop.
    #[tokio::test]
    async fn tree_walks_terminate_on_an_existing_cycle() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;
      let a = seed_view(&db, ws, user, "folder").await;
      let b = seed_view(&db, ws, user, "folder").await;
      for (child, parent) in [(a, b), (b, a)] {
        sqlx::query("UPDATE views SET parent_view_id = $1 WHERE id = $2")
          .bind(parent)
          .bind(child)
          .execute(&db)
          .await
          .unwrap();
      }
      let walk = async {
        let c = seed_view(&db, ws, user, "folder").await;
        assert!(move_views_tx(&db, ws, &[c], Some(a)).await.is_ok());
        assert_eq!(trash_views_batch(&db, ws, &[a]).await.unwrap().len(), 3);
        assert_eq!(restore_views_batch(&db, ws, &[a]).await.unwrap().len(), 3);
      };
      tokio::time::timeout(std::time::Duration::from_secs(10), walk)
        .await
        .expect("a recursive walk looped on the cycle");
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

    /// Seed `folder` (top level) with one live child page; returns (folder, child).
    async fn seed_folder_with_child(db: &PgPool, ws: Uuid, user: Uuid) -> (Uuid, Uuid) {
      let folder = seed_view(db, ws, user, "folder").await;
      let child = Uuid::new_v4();
      sqlx::query(
        "INSERT INTO views(id,workspace_id,parent_view_id,object_id,object_type,name,position,created_by) \
         VALUES($1,$2,$3,$4,'document'::object_type,'旧子页','0',$5)",
      )
      .bind(child)
      .bind(ws)
      .bind(folder)
      .bind(Uuid::new_v4())
      .bind(user)
      .execute(db)
      .await
      .unwrap();
      (folder, child)
    }

    /// C1 (review 2026-10-21): a cross-workspace MOVE enumerates the subtree,
    /// copies blobs (slow — network), builds the copy, and only then
    /// soft-deletes the source — by walking the subtree AGAIN. A page a
    /// collaborator created in that window was in the second walk but not the
    /// first: soft-deleted at the source, never copied, while the user was
    /// told "moved". The second walk now has to agree with the first, or the
    /// whole move (copy included) rolls back with a 409.
    #[tokio::test]
    async fn a_page_created_during_a_move_fails_it_instead_of_being_binned() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;
      let (folder, child) = seed_folder_with_child(&db, ws, user).await;

      // Phase 1 of the move: what gets copied.
      let copied: std::collections::HashSet<Uuid> =
        live_subtree(&db, &[folder], ws).await.unwrap().iter().map(|r| r.id).collect();
      assert_eq!(copied, [folder, child].into_iter().collect());

      // A collaborator adds a page while the blobs are copying …
      let (late, _late_doc) = seed_document(&db, ws, user, "移动中新建", serde_json::json!([])).await;
      sqlx::query("UPDATE views SET parent_view_id = $1 WHERE id = $2")
        .bind(folder)
        .bind(late)
        .execute(&db)
        .await
        .unwrap();

      // … then the move's transaction removes the source.
      let mut tx = db.begin().await.unwrap();
      let result = remove_moved_source(&mut tx, ws, &[folder], &copied).await;
      let err = result.expect_err("a subtree that changed since the copy must not be removed");
      assert!(matches!(err, ApiError::Conflict(_)), "{err:?}");
      drop(tx); // the handler's `?` drops it: rollback, the copy goes too

      assert!(!is_deleted(&db, late).await, "the uncopied page must stay live");
      assert!(!is_deleted(&db, folder).await, "nothing of the source was removed");
      assert!(!is_deleted(&db, child).await);
    }

    /// The unchanged case still moves: exactly the copied rows go to the bin.
    #[tokio::test]
    async fn an_unchanged_subtree_is_removed_after_the_copy() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;
      let (folder, child) = seed_folder_with_child(&db, ws, user).await;
      let copied: std::collections::HashSet<Uuid> =
        live_subtree(&db, &[folder], ws).await.unwrap().iter().map(|r| r.id).collect();

      let mut tx = db.begin().await.unwrap();
      remove_moved_source(&mut tx, ws, &[folder], &copied).await.unwrap();
      tx.commit().await.unwrap();

      assert!(is_deleted(&db, folder).await);
      assert!(is_deleted(&db, child).await);
    }

    /// Once the move holds its locks, a page created under the subtree has to
    /// wait for it — and then finds the folder in the bin and is refused
    /// (migration 0033), rather than landing live under a source that was just
    /// moved away.
    #[tokio::test]
    async fn a_page_created_after_the_check_waits_for_the_move() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;
      let (folder, _child) = seed_folder_with_child(&db, ws, user).await;
      let copied: std::collections::HashSet<Uuid> =
        live_subtree(&db, &[folder], ws).await.unwrap().iter().map(|r| r.id).collect();

      let mut tx = db.begin().await.unwrap();
      remove_moved_source(&mut tx, ws, &[folder], &copied).await.unwrap();
      let db2 = db.clone();
      let late = tokio::spawn(async move { insert_child(&db2, ws, user, folder).await });
      tokio::time::sleep(std::time::Duration::from_millis(300)).await;
      assert!(!late.is_finished(), "the insert must wait for the move to commit");
      tx.commit().await.unwrap();
      assert!(late.await.unwrap().is_err(), "and then be refused: its folder is in the bin");
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
    /// with them — seeded from a set, which is the part that would silently
    /// strand documents if the recursive arm were written wrong.
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
