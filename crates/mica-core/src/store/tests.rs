//! Unit tests for the local store (`store.rs`), kept beside it so the module
//! itself stays a readable size.

use super::*;
use crate::Block;

fn sample() -> (String, Vec<Block>) {
  (
    "r".to_string(),
    vec![
      Block::new("r", "page").with_children(vec!["a".into()]),
      Block::new("a", "paragraph").with_text("Hello"),
    ],
  )
}

fn temp_path() -> std::path::PathBuf {
  std::env::temp_dir().join(format!("mica_test_{}.db", uuid::Uuid::new_v4()))
}

#[test]
fn schema_version_stamped_and_downgrade_guarded() {
  let path = temp_path();
  let p = path.to_str().unwrap();
  {
    let store = LocalStore::open(p).unwrap();
    let v: String = store
      .conn
      .query_row(
        "SELECT value FROM local_meta WHERE key='schema_version'",
        [],
        |r| r.get(0),
      )
      .unwrap();
    assert_eq!(v, SCHEMA_VERSION.to_string(), "current version stamped");
  }
  // A store written by a future app version must be refused, not corrupted.
  {
    let conn = Connection::open(p).unwrap();
    conn
      .execute(
        "UPDATE local_meta SET value='99' WHERE key='schema_version'",
        [],
      )
      .unwrap();
  }
  assert!(matches!(
    LocalStore::open(p),
    Err(StoreError::SchemaTooNew { found: 99, .. })
  ));
  std::fs::remove_file(p).ok();
}

#[test]
fn too_new_store_is_refused_before_any_destructive_migration() {
  // The gate must run BEFORE the probe-driven migrations: a future schema
  // whose PK shape differs would otherwise trip the v4 rebuild, which DROPs
  // and recreates from a hard-coded column list — silently stripping the
  // newer app's columns before the trailing gate could reject. Craft a fake
  // future-versioned store (3-column PK + a v5-only column) and assert open() refuses it
  // with the structure untouched.
  let path = temp_path();
  let p = path.to_str().unwrap();
  {
    let conn = Connection::open(p).unwrap();
    conn
      .execute_batch(
        "CREATE TABLE local_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
              INSERT INTO local_meta VALUES('schema_version','8');
               CREATE TABLE local_view(
                   origin TEXT NOT NULL, device TEXT NOT NULL, id TEXT NOT NULL,
                   workspace_id TEXT NOT NULL DEFAULT 'local', parent_id TEXT,
                   object_id TEXT NOT NULL, name TEXT NOT NULL, position TEXT NOT NULL,
                   trashed INTEGER NOT NULL DEFAULT 0,
                   v6_only TEXT,
                   PRIMARY KEY(origin, device, id));
               CREATE TABLE local_workspace(
                   origin TEXT NOT NULL, device TEXT NOT NULL, id TEXT NOT NULL,
                   name TEXT NOT NULL, position TEXT NOT NULL,
                   role TEXT NOT NULL DEFAULT 'viewer',
                   v6_only TEXT,
                   PRIMARY KEY(origin, device, id));
               INSERT INTO local_view(origin,device,id,object_id,name,position,v6_only)
                   VALUES('local','dev1','v1','d1','Future Page','0000000010','keep-me');",
      )
      .unwrap();
  }
  assert!(matches!(
    LocalStore::open(p),
    Err(StoreError::SchemaTooNew { found: 8, .. })
  ));
  // Structure untouched: the v6-only column and 3-column PK are intact, and
  // no rebuild artifacts (backup / _v4 temp tables) were created.
  {
    let conn = Connection::open(p).unwrap();
    let v6_kept: String = conn
      .query_row("SELECT v6_only FROM local_view WHERE id='v1'", [], |r| {
        r.get(0)
      })
      .unwrap();
    assert_eq!(v6_kept, "keep-me");
    let pk: i64 = conn
      .query_row(
        "SELECT COUNT(*) FROM pragma_table_info('local_view') WHERE pk>0",
        [],
        |r| r.get(0),
      )
      .unwrap();
    assert_eq!(pk, 3, "future PK shape untouched");
    let artifacts: i64 = conn
      .query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE '%_v3_backup' OR name LIKE '%_v4'",
        [],
        |r| r.get(0),
      )
      .unwrap();
    assert_eq!(artifacts, 0, "no migration artifacts on a refused store");
  }
  std::fs::remove_file(p).ok();
}

#[test]
fn migrates_legacy_schema_without_data_loss() {
  let path = temp_path();
  let p = path.to_str().unwrap();
  // A pre-multi-workspace store: local_view has no workspace_id, no
  // local_workspace table, no schema_version.
  {
    let conn = Connection::open(p).unwrap();
    conn.execute_batch(
              "CREATE TABLE doc_snapshot(doc_id TEXT PRIMARY KEY, state BLOB NOT NULL, updated_at INTEGER NOT NULL);
               CREATE TABLE local_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
               CREATE TABLE local_view(id TEXT PRIMARY KEY, parent_id TEXT, object_id TEXT NOT NULL, name TEXT NOT NULL, position TEXT NOT NULL, trashed INTEGER NOT NULL DEFAULT 0);
               INSERT INTO local_view(id,object_id,name,position) VALUES('v1','doc1','Old Page','0000000010');",
          )
          .unwrap();
  }
  let store = LocalStore::open(p).unwrap();
  let views = store.list_views("local").unwrap();
  let v = views
    .iter()
    .find(|x| x.id == "v1")
    .expect("legacy view kept");
  assert_eq!(v.name, "Old Page", "legacy data preserved");
  assert_eq!(v.workspace_id, "local", "back-filled to default workspace");
  assert_eq!(v.origin, "local", "legacy rows back-filled to local origin");
  assert_eq!(v.object_type, "document", "object_type (v6) back-filled");
  let local_ws = store.list_workspaces("local").unwrap();
  let default_ws = local_ws.iter().find(|w| w.id == "local");
  assert!(default_ws.is_some(), "default workspace created");
  assert_eq!(
    default_ws.unwrap().role,
    "viewer",
    "role column back-filled"
  );
  let sv: String = store
    .conn
    .query_row(
      "SELECT value FROM local_meta WHERE key='schema_version'",
      [],
      |r| r.get(0),
    )
    .unwrap();
  assert_eq!(sv, SCHEMA_VERSION.to_string());
  drop(store);
  std::fs::remove_file(p).ok();
}

#[test]
fn checkpoint_and_rollback() {
  let store = LocalStore::open_in_memory().unwrap();
  let cid = store.identity().unwrap().client_id;
  let (root, blocks) = sample();
  store
    .save_doc(
      "local",
      "d",
      &MicaDoc::from_blocks_with_client_id(&root, &blocks, Some(cid)),
    )
    .unwrap();
  store.checkpoint_doc("local", "d").unwrap();

  // Overwrite the live base with an edit.
  let mut working = store.load_doc("local", "d", cid).unwrap().unwrap();
  working.text_insert("a", 5, " EDIT");
  store.save_doc("local", "d", &working).unwrap();
  let has_edit = |d: &MicaDoc| d.to_blocks().iter().any(|b| b.text.contains("EDIT"));
  assert!(has_edit(
    &store.load_doc("local", "d", cid).unwrap().unwrap()
  ));

  // Rollback restores the checkpoint (edit gone) and clears the update log.
  let restored = store.rollback_doc("local", "d", cid).unwrap().unwrap();
  assert!(!has_edit(&restored));
  assert!(!has_edit(
    &store.load_doc("local", "d", cid).unwrap().unwrap()
  ));
  assert!(store
    .rollback_doc("local", "missing", cid)
    .unwrap()
    .is_none());
}

#[test]
fn local_version_history_captures_names_and_restores() {
  let store = LocalStore::open_in_memory().unwrap();
  let cid = store.identity().unwrap().client_id;
  let (root, blocks) = sample();

  // Several saves in a burst → one auto version (10-min convergence).
  let mut d = MicaDoc::from_blocks_with_client_id(&root, &blocks, Some(cid));
  for i in 0..4 {
    d.text_insert("a", 5, &format!("{i}"));
    store.save_doc("local", "d", &d).unwrap();
  }
  let autos = store.list_local_versions("local", "d").unwrap();
  assert_eq!(autos.len(), 1, "burst converges to one auto version");
  assert!(autos[0].label.is_none());

  // Pin a NAMED checkpoint at the current state.
  let named = store
    .create_local_version("local", "d", "milestone")
    .unwrap()
    .unwrap();
  assert_eq!(named.label.as_deref(), Some("milestone"));
  assert_eq!(store.list_local_versions("local", "d").unwrap().len(), 2);

  // Edit past the checkpoint, then restore to it.
  let mut working = store.load_doc("local", "d", cid).unwrap().unwrap();
  working.text_insert("a", 5, "LATER");
  store.save_doc("local", "d", &working).unwrap();
  let restored = store
    .restore_local_version("local", "d", &named.id, cid)
    .unwrap()
    .unwrap();
  let has_later = |x: &MicaDoc| x.to_blocks().iter().any(|b| b.text.contains("LATER"));
  assert!(!has_later(&restored), "restore dropped the later edit");
  assert!(!has_later(
    &store.load_doc("local", "d", cid).unwrap().unwrap()
  ));

  // A missing version id is a clean None, not an error.
  assert!(store
    .restore_local_version("local", "d", "nope", cid)
    .unwrap()
    .is_none());
  // No snapshot yet → naming returns None rather than an empty version.
  assert!(store
    .create_local_version("local", "ghost", "x")
    .unwrap()
    .is_none());
}

#[test]
fn load_doc_rejects_corrupt_snapshot() {
  let store = LocalStore::open_in_memory().unwrap();
  let cid = store.identity().unwrap().client_id;
  // §10: a corrupt base must surface as an error, never a silent bad state.
  store
    .conn
    .execute(
      "INSERT INTO doc_snapshot(origin,doc_id,state,updated_at) VALUES('local','bad',?1,0)",
      params![vec![9u8, 9, 9, 9]],
    )
    .unwrap();
  assert!(store.load_doc("local", "bad", cid).is_err());
}

#[test]
fn save_and_load_round_trip() {
  let store = LocalStore::open_in_memory().unwrap();
  let id = store.identity().unwrap();
  let (root, blocks) = sample();
  let doc = MicaDoc::from_blocks_with_client_id(&root, &blocks, Some(id.client_id));
  store.save_doc("local", "doc1", &doc).unwrap();

  let loaded = store
    .load_doc("local", "doc1", id.client_id)
    .unwrap()
    .unwrap();
  assert_eq!(loaded.to_blocks(), doc.to_blocks());
  assert_eq!(loaded.client_id(), id.client_id);
}

#[test]
fn edits_persist_after_resave() {
  let store = LocalStore::open_in_memory().unwrap();
  let cid = store.identity().unwrap().client_id;
  let (root, blocks) = sample();
  let mut doc = MicaDoc::from_blocks_with_client_id(&root, &blocks, Some(cid));
  store.save_doc("local", "d", &doc).unwrap();

  doc.text_insert("a", 5, " world");
  store.save_doc("local", "d", &doc).unwrap();

  let loaded = store.load_doc("local", "d", cid).unwrap().unwrap();
  let a = loaded
    .to_blocks()
    .into_iter()
    .find(|b| b.id == "a")
    .unwrap();
  assert_eq!(a.text, "Hello world");
}

#[test]
fn list_and_delete() {
  let store = LocalStore::open_in_memory().unwrap();
  let cid = store.identity().unwrap().client_id;
  let (root, blocks) = sample();
  let doc = MicaDoc::from_blocks_with_client_id(&root, &blocks, Some(cid));
  store.save_doc("local", "a", &doc).unwrap();
  store.save_doc("local", "b", &doc).unwrap();
  assert_eq!(store.list_docs("local").unwrap(), vec!["a", "b"]);
  store.delete_doc("local", "a").unwrap();
  assert_eq!(store.list_docs("local").unwrap(), vec!["b"]);
  assert!(store.load_doc("local", "a", cid).unwrap().is_none());
}

fn view(id: &str, parent: Option<&str>, name: &str, pos: &str) -> LocalView {
  LocalView {
    id: id.into(),
    workspace_id: "local".into(),
    parent_id: parent.map(|s| s.into()),
    object_id: format!("doc-{id}"),
    name: name.into(),
    position: pos.into(),
    trashed: false,
    origin: "local".into(),
    object_type: "document".into(),
  }
}

#[test]
fn views_crud_and_tree_fields() {
  let store = LocalStore::open_in_memory().unwrap();
  // v1 is a folder: only folders hold children (ensure_valid_parent).
  store
    .save_view(&LocalView {
      object_type: "folder".into(),
      ..view("v1", None, "Page 1", "0000000010")
    })
    .unwrap();
  store
    .save_view(&view("v2", None, "Page 2", "0000000020"))
    .unwrap();
  store
    .save_view(&view("v3", Some("v1"), "Child", "0000000010"))
    .unwrap();

  let all = store.list_views("local").unwrap();
  assert_eq!(all.len(), 3);
  // ordered by position; v3 (child, pos 10) and v1 (pos 10) share pos but
  // the child carries its parent.
  let v3 = all.iter().find(|v| v.id == "v3").unwrap();
  assert_eq!(v3.parent_id.as_deref(), Some("v1"));
  assert_eq!(v3.object_id, "doc-v3");

  // Rename + move (upsert same id).
  store
    .save_view(&LocalView {
      name: "Renamed".into(),
      ..view("v2", None, "x", "0000000005")
    })
    .unwrap();
  let v2 = store
    .list_views("local")
    .unwrap()
    .into_iter()
    .find(|v| v.id == "v2")
    .unwrap();
  assert_eq!(v2.name, "Renamed");
  assert_eq!(v2.position, "0000000005");

  // Trash then purge.
  store
    .save_view(&LocalView {
      trashed: true,
      ..view("v1", None, "Page 1", "0000000010")
    })
    .unwrap();
  assert!(
    store
      .list_views("local")
      .unwrap()
      .iter()
      .find(|v| v.id == "v1")
      .unwrap()
      .trashed
  );
  store.purge_view("local", "v1").unwrap();
  assert!(store
    .list_views("local")
    .unwrap()
    .iter()
    .all(|v| v.id != "v1"));
}

#[test]
fn view_object_type_round_trips_folder_and_document() {
  let store = LocalStore::open_in_memory().unwrap();
  // A folder (pure container) and a document, mirrored offline.
  store
    .save_view(&LocalView {
      object_type: "folder".into(),
      ..view("f1", None, "Chapter", "0000000010")
    })
    .unwrap();
  store
    .save_view(&view("d1", Some("f1"), "Intro", "0000000010"))
    .unwrap();

  let all = store.list_views("local").unwrap();
  let f = all.iter().find(|v| v.id == "f1").unwrap();
  let d = all.iter().find(|v| v.id == "d1").unwrap();
  assert_eq!(f.object_type, "folder", "folder type persists");
  assert_eq!(d.object_type, "document", "document is the default");

  // Upsert preserves the type across a rename (edited via save_view).
  store
    .save_view(&LocalView {
      object_type: "folder".into(),
      name: "Renamed Chapter".into(),
      ..view("f1", None, "x", "0000000010")
    })
    .unwrap();
  let f = store
    .list_views("local")
    .unwrap()
    .into_iter()
    .find(|v| v.id == "f1")
    .unwrap();
  assert_eq!(f.object_type, "folder");
  assert_eq!(f.name, "Renamed Chapter");
}

#[test]
fn update_log_replays_on_load_and_squash_collapses() {
  let store = LocalStore::open_in_memory().unwrap();
  let cid = store.identity().unwrap().client_id;
  let (root, blocks) = sample();
  let doc = MicaDoc::from_blocks_with_client_id(&root, &blocks, Some(cid));
  store.save_doc("local", "d", &doc).unwrap();

  // Make two edits as incremental updates (capture each as a diff).
  let mut working = store.load_doc("local", "d", cid).unwrap().unwrap();
  let sv0 = working.state_vector();
  working.text_insert("a", 5, " world");
  let u1 = working.encode_diff(&sv0).unwrap();
  store.append_update("local", "d", &u1).unwrap();
  let sv1 = working.state_vector();
  working.insert_block("r", 1, &Block::new("b", "paragraph").with_text("two"));
  let u2 = working.encode_diff(&sv1).unwrap();
  let clock2 = store.append_update("local", "d", &u2).unwrap();
  assert_eq!(clock2, 2);

  // Loading replays base + updates.
  let loaded = store.load_doc("local", "d", cid).unwrap().unwrap();
  let a = loaded
    .to_blocks()
    .into_iter()
    .find(|b| b.id == "a")
    .unwrap();
  assert_eq!(a.text, "Hello world");
  assert_eq!(
    loaded
      .to_blocks()
      .iter()
      .find(|b| b.id == "r")
      .unwrap()
      .children,
    vec!["a", "b"]
  );
  assert_eq!(store.doc_updates("local", "d").unwrap().len(), 2);

  // Squash re-baselines: it folds the log into the base and drops the acked
  // prefix. With both edits acked, it clears the log; state is preserved.
  store
    .set_sync_cursor(
      "local",
      "d",
      SyncCursor {
        last_synced_rid: 0,
        pushed_clock: 2,
      },
    )
    .unwrap();
  store.squash("local", "d", cid).unwrap();
  assert_eq!(store.doc_updates("local", "d").unwrap().len(), 0);
  let after = store.load_doc("local", "d", cid).unwrap().unwrap();
  assert_eq!(after.to_blocks(), loaded.to_blocks());
}

#[test]
fn squash_keeps_unpushed_tail_and_clock_monotonic() {
  // squash must NOT drop the un-pushed outbox (edits the cloud never got)
  // nor reset the clock — else those edits never sync and a later clock
  // collides. Mirror of trim_bounds_log_and_clock_stays_monotonic, for the
  // squash path (which deletes by a different criterion).
  let store = LocalStore::open_in_memory().unwrap();
  let cid = store.identity().unwrap().client_id;
  let (root, blocks) = sample();
  store
    .save_doc(
      "local",
      "d",
      &MicaDoc::from_blocks_with_client_id(&root, &blocks, Some(cid)),
    )
    .unwrap();

  let mut working = store.load_doc("local", "d", cid).unwrap().unwrap();
  let sv0 = working.state_vector();
  working.text_insert("a", 5, " one");
  store
    .append_update("local", "d", &working.encode_diff(&sv0).unwrap())
    .unwrap(); // clock 1
  let sv1 = working.state_vector();
  working.text_insert("a", 9, " two");
  store
    .append_update("local", "d", &working.encode_diff(&sv1).unwrap())
    .unwrap(); // clock 2

  // Clock 1 acked; clock 2 still un-pushed (the offline-edit norm).
  store
    .set_sync_cursor(
      "local",
      "d",
      SyncCursor {
        last_synced_rid: 5,
        pushed_clock: 1,
      },
    )
    .unwrap();
  store.squash("local", "d", cid).unwrap();

  // The un-pushed tail (clock 2) survives so it can still be pushed; the
  // base folds both edits, so content is intact either way.
  let clocks = |s: &LocalStore| {
    s.doc_updates("local", "d")
      .unwrap()
      .into_iter()
      .map(|(c, _)| c)
      .collect::<Vec<_>>()
  };
  assert_eq!(
    clocks(&store),
    vec![2],
    "un-pushed clock 2 kept through squash"
  );
  assert_eq!(
    store
      .load_doc("local", "d", cid)
      .unwrap()
      .unwrap()
      .to_blocks()
      .iter()
      .find(|b| b.id == "a")
      .unwrap()
      .text,
    "Hello one two",
  );

  // No clock reset: a new edit continues past what was issued.
  let sv2 = working.state_vector();
  working.text_insert("a", 13, "!");
  assert_eq!(
    store
      .append_update("local", "d", &working.encode_diff(&sv2).unwrap())
      .unwrap(),
    3,
    "no reset after squash",
  );
  assert_eq!(
    store
      .updates_after("local", "d", 1)
      .unwrap()
      .into_iter()
      .map(|(c, _)| c)
      .collect::<Vec<_>>(),
    vec![2, 3],
    "outbox still holds the un-pushed edits",
  );
}

/// A local-only document has no server, so `pushed_clock` stays 0 forever —
/// which means `squash` (keeps `clock > pushed_clock`) and
/// `trim_updates_through` (clamps to it) can BOTH never delete a single row
/// of its log. This is the test that makes that concrete: without
/// `compact_local`, switching the offline backend to an append-log would
/// have produced an unbounded log replayed on every open.
#[test]
fn a_local_only_doc_cannot_be_bounded_by_squash_or_trim() {
  let store = LocalStore::open_in_memory().unwrap();
  let cid = store.identity().unwrap().client_id;
  let (root, blocks) = sample();
  store
    .save_doc(
      "local",
      "d",
      &MicaDoc::from_blocks_with_client_id(&root, &blocks, Some(cid)),
    )
    .unwrap();
  let mut working = store.load_doc("local", "d", cid).unwrap().unwrap();
  for i in 0..3 {
    let sv = working.state_vector();
    working.text_insert("a", 5, &format!(" {i}"));
    store
      .append_update("local", "d", &working.encode_diff(&sv).unwrap())
      .unwrap();
  }
  assert_eq!(store.doc_updates("local", "d").unwrap().len(), 3);

  store.squash("local", "d", cid).unwrap();
  store.trim_updates_through("local", "d", 999).unwrap();
  assert_eq!(
    store.doc_updates("local", "d").unwrap().len(),
    3,
    "both cloud-shaped trims are no-ops when pushed_clock is 0",
  );

  assert!(store.compact_local("local", "d", cid).unwrap());
  assert_eq!(
    store.doc_updates("local", "d").unwrap().len(),
    0,
    "log cleared"
  );
  assert_eq!(
    store
      .load_doc("local", "d", cid)
      .unwrap()
      .unwrap()
      .to_blocks()
      .iter()
      .find(|b| b.id == "a")
      .unwrap()
      .text,
    "Hello 2 1 0",
    "every edit survives in the folded base",
  );
}

/// The guard, and the reason it is in the store rather than in the caller:
/// clearing the log of a doc that still owes the server is silent
/// server-side loss. Caller discipline is not a guarantee — this is.
#[test]
fn compact_local_refuses_anything_that_smells_of_cloud() {
  let store = LocalStore::open_in_memory().unwrap();
  let cid = store.identity().unwrap().client_id;
  let (root, blocks) = sample();

  // ① a doc with an un-pushed outbox and a live sync cursor
  store
    .save_doc(
      "local",
      "cloud",
      &MicaDoc::from_blocks_with_client_id(&root, &blocks, Some(cid)),
    )
    .unwrap();
  let mut working = store.load_doc("local", "cloud", cid).unwrap().unwrap();
  let sv = working.state_vector();
  working.text_insert("a", 5, " edit");
  store
    .append_update("local", "cloud", &working.encode_diff(&sv).unwrap())
    .unwrap();
  store
    .set_sync_cursor(
      "local",
      "cloud",
      SyncCursor {
        last_synced_rid: 7,
        pushed_clock: 0,
      },
    )
    .unwrap();
  assert!(
    !store.compact_local("local", "cloud", cid).unwrap(),
    "synced cursor → refuse"
  );
  assert_eq!(
    store.doc_updates("local", "cloud").unwrap().len(),
    1,
    "outbox untouched"
  );

  // ② a doc that has received remote updates (cursor still zero)
  store
    .save_doc(
      "local",
      "remote",
      &MicaDoc::from_blocks_with_client_id(&root, &blocks, Some(cid)),
    )
    .unwrap();
  let mut w2 = store.load_doc("local", "remote", cid).unwrap().unwrap();
  let sv2 = w2.state_vector();
  w2.text_insert("a", 5, " r");
  let diff = w2.encode_diff(&sv2).unwrap();
  store
    .append_remote_update("local", "remote", 1, &diff)
    .unwrap();
  store.append_update("local", "remote", &diff).unwrap();
  assert!(
    !store.compact_local("local", "remote", cid).unwrap(),
    "remote log → refuse"
  );
  assert_eq!(
    store.doc_updates("local", "remote").unwrap().len(),
    1,
    "outbox untouched"
  );

  // ③ a doc that does not exist at all
  assert!(!store.compact_local("local", "nope", cid).unwrap());
}

/// Local version history rode on the debounced full `save_doc`. Once the
/// backend stops calling it, the only thing that can keep history alive is
/// this — and a missing auto-version is exactly the kind of loss that says
/// nothing at the time.
#[test]
fn compact_local_still_captures_a_version() {
  let store = LocalStore::open_in_memory().unwrap();
  let cid = store.identity().unwrap().client_id;
  let (root, blocks) = sample();
  store
    .save_doc(
      "local",
      "d",
      &MicaDoc::from_blocks_with_client_id(&root, &blocks, Some(cid)),
    )
    .unwrap();
  let before = store.list_local_versions("local", "d").unwrap().len();

  let mut working = store.load_doc("local", "d", cid).unwrap().unwrap();
  let sv = working.state_vector();
  working.text_insert("a", 5, " more");
  store
    .append_update("local", "d", &working.encode_diff(&sv).unwrap())
    .unwrap();
  assert!(store.compact_local("local", "d", cid).unwrap());

  // The cadence is min-interval, so this is "history did not go dead",
  // not "one version per compaction".
  assert!(
    store.list_local_versions("local", "d").unwrap().len() >= before,
    "compaction must go through the same version capture save_doc uses",
  );
}

#[test]
fn remote_log_replays_advances_cursor_and_squashes() {
  // P4-1: remote updates persist in their own log (never the outbox),
  // advance last_synced_rid transactionally, replay on load, and squash
  // folds BOTH logs while clearing the remote one entirely.
  let store = LocalStore::open_in_memory().unwrap();
  let cid = store.identity().unwrap().client_id;
  let (root, blocks) = sample();
  let doc = MicaDoc::from_blocks_with_client_id(&root, &blocks, Some(cid));
  store.save_doc("local", "d", &doc).unwrap();

  // A "remote" edit: a diff minted by another actor.
  let mut remote = MicaDoc::from_update_with_client_id(&doc.encode_state(), Some(999)).unwrap();
  let sv = remote.state_vector();
  remote.text_insert("a", 5, " remote");
  let r_diff = remote.encode_diff(&sv).unwrap();

  store
    .append_remote_update("local", "d", 7, &r_diff)
    .unwrap();
  // Idempotent per rid (a re-pull replays the same row).
  store
    .append_remote_update("local", "d", 7, &r_diff)
    .unwrap();
  assert_eq!(store.doc_remote_updates("local", "d").unwrap().len(), 1);
  assert_eq!(
    store.sync_cursor("local", "d").unwrap().last_synced_rid,
    7,
    "cursor moved with the append",
  );
  // A lower rid never regresses the cursor.
  store
    .append_remote_update("local", "d", 3, &r_diff)
    .unwrap();
  assert_eq!(store.sync_cursor("local", "d").unwrap().last_synced_rid, 7);

  // The outbox stays pure-local: remote rows never enter doc_update.
  assert!(store.doc_updates("local", "d").unwrap().is_empty());

  // A local edit alongside; load replays base + remote + local.
  let mut working = store.load_doc("local", "d", cid).unwrap().unwrap();
  assert!(
    working
      .to_blocks()
      .iter()
      .any(|b| b.text.contains("remote")),
    "remote log replayed",
  );
  let sv2 = working.state_vector();
  working.text_insert("a", 0, "local ");
  store
    .append_update("local", "d", &working.encode_diff(&sv2).unwrap())
    .unwrap();
  let loaded = store.load_doc("local", "d", cid).unwrap().unwrap();
  let a = loaded
    .to_blocks()
    .into_iter()
    .find(|b| b.id == "a")
    .unwrap();
  assert_eq!(a.text, "local Hello remote");

  // Squash folds both logs into the base; remote log cleared, un-pushed
  // local tail kept (pushed_clock=0 so nothing trims from the outbox).
  store.squash("local", "d", cid).unwrap();
  assert!(
    store.doc_remote_updates("local", "d").unwrap().is_empty(),
    "remote log cleared"
  );
  assert_eq!(
    store.doc_updates("local", "d").unwrap().len(),
    1,
    "un-pushed outbox kept"
  );
  assert_eq!(
    store
      .load_doc("local", "d", cid)
      .unwrap()
      .unwrap()
      .to_blocks()
      .iter()
      .find(|b| b.id == "a")
      .unwrap()
      .text,
    "local Hello remote",
    "content intact after squash",
  );
  let (local, remote_rows) = store.log_sizes("local", "d").unwrap();
  assert_eq!((local, remote_rows), (1, 0));
}

#[test]
fn batch_remote_append_and_delete_doc_cover_remote_log() {
  let store = LocalStore::open_in_memory().unwrap();
  let cid = store.identity().unwrap().client_id;
  let (root, blocks) = sample();
  let doc = MicaDoc::from_blocks_with_client_id(&root, &blocks, Some(cid));
  store.save_doc("local", "d", &doc).unwrap();
  let mut remote = MicaDoc::from_update_with_client_id(&doc.encode_state(), Some(42)).unwrap();
  let mut items = Vec::new();
  for i in 0..5 {
    let sv = remote.state_vector();
    remote.text_insert("a", 5, &format!(" r{i}"));
    items.push((10 + i as i64, remote.encode_diff(&sv).unwrap()));
  }
  // One transaction: all rows land, cursor at the max rid.
  store.append_remote_updates("local", "d", &items).unwrap();
  assert_eq!(store.doc_remote_updates("local", "d").unwrap().len(), 5);
  assert_eq!(store.sync_cursor("local", "d").unwrap().last_synced_rid, 14);
  // Batch is idempotent per rid too.
  store.append_remote_updates("local", "d", &items).unwrap();
  assert_eq!(store.doc_remote_updates("local", "d").unwrap().len(), 5);
  // delete_doc removes the remote log with everything else.
  store.delete_doc("local", "d").unwrap();
  assert!(store.doc_remote_updates("local", "d").unwrap().is_empty());
  assert!(store.load_doc("local", "d", cid).unwrap().is_none());
}

#[test]
fn sync_cursor_round_trip() {
  let store = LocalStore::open_in_memory().unwrap();
  assert_eq!(
    store.sync_cursor("local", "d").unwrap(),
    SyncCursor::default()
  );
  store
    .set_sync_cursor(
      "local",
      "d",
      SyncCursor {
        last_synced_rid: 42,
        pushed_clock: 7,
      },
    )
    .unwrap();
  let c = store.sync_cursor("local", "d").unwrap();
  assert_eq!(c.last_synced_rid, 42);
  assert_eq!(c.pushed_clock, 7);
  // updates_after honours the pushed cursor.
  let cid = store.identity().unwrap().client_id;
  let doc = MicaDoc::from_blocks_with_client_id("r", &[Block::new("r", "page")], Some(cid));
  store.save_doc("local", "d", &doc).unwrap();
  // append_update is monotonic PAST pushed_clock — a fresh clock always
  // lands in the outbox (updates_after(pushed_clock)), never ≤ it.
  let c1 = store.append_update("local", "d", &[1, 2, 3]).unwrap();
  let c2 = store.append_update("local", "d", &[4, 5, 6]).unwrap();
  assert_eq!((c1, c2), (8, 9), "clock resumes past pushed_clock=7");
  assert_eq!(
    store.updates_after("local", "d", 7).unwrap().len(),
    2,
    "both are the outbox"
  );
  assert_eq!(store.updates_after("local", "d", 8).unwrap().len(), 1);
}

#[test]
fn trim_bounds_log_and_clock_stays_monotonic() {
  // The P2 outbox invariant: trimming acked entries must never let a later
  // clock be reused at/below pushed_clock (it would fall outside the outbox
  // query `updates_after(pushed_clock)` and never be pushed = divergence).
  let store = LocalStore::open_in_memory().unwrap();
  let cid = store.identity().unwrap().client_id;
  let doc = MicaDoc::from_blocks_with_client_id("r", &[Block::new("r", "page")], Some(cid));
  store.save_doc("local", "d", &doc).unwrap();
  assert_eq!(store.append_update("local", "d", &[1]).unwrap(), 1);
  assert_eq!(store.append_update("local", "d", &[2]).unwrap(), 2);
  assert_eq!(store.append_update("local", "d", &[3]).unwrap(), 3);

  // Ack through clock 2, then trim the acked prefix.
  store
    .set_sync_cursor(
      "local",
      "d",
      SyncCursor {
        last_synced_rid: 5,
        pushed_clock: 2,
      },
    )
    .unwrap();
  store.trim_updates_through("local", "d", 2).unwrap();
  let clocks = |s: &LocalStore| {
    s.doc_updates("local", "d")
      .unwrap()
      .into_iter()
      .map(|(c, _)| c)
      .collect::<Vec<_>>()
  };
  assert_eq!(clocks(&store), vec![3], "only the un-pushed tail survives");

  // A new edit continues past the trimmed range, not back at 1.
  assert_eq!(
    store.append_update("local", "d", &[4]).unwrap(),
    4,
    "monotonic past trim"
  );
  assert_eq!(
    store
      .updates_after("local", "d", 2)
      .unwrap()
      .into_iter()
      .map(|(c, _)| c)
      .collect::<Vec<_>>(),
    vec![3, 4],
    "outbox = unpushed tail"
  );

  // Full trim (everything acked) then append still doesn't reset to 1.
  store
    .set_sync_cursor(
      "local",
      "d",
      SyncCursor {
        last_synced_rid: 6,
        pushed_clock: 4,
      },
    )
    .unwrap();
  store.trim_updates_through("local", "d", 4).unwrap();
  assert!(clocks(&store).is_empty(), "empty after full trim");
  assert_eq!(
    store.append_update("local", "d", &[5]).unwrap(),
    5,
    "no reset after full trim"
  );
}

#[test]
fn workspaces_crud_and_default() {
  let store = LocalStore::open_in_memory().unwrap();
  // A default workspace always exists.
  let all = store.list_workspaces("local").unwrap();
  assert_eq!(all.len(), 1);
  assert_eq!(all[0].id, "local");
  assert_eq!(all[0].origin, "local", "default workspace is local-origin");

  store
    .save_workspace(&LocalWorkspace {
      id: "w2".into(),
      name: "Work".into(),
      position: "0000000020".into(),
      origin: "local".into(),
      role: "owner".into(),
    })
    .unwrap();
  assert_eq!(store.list_workspaces("local").unwrap().len(), 2);
  assert_eq!(
    store
      .list_workspaces("local")
      .unwrap()
      .iter()
      .find(|w| w.id == "w2")
      .unwrap()
      .role,
    "owner",
    "role round-trips",
  );

  // Views are scoped; deleting a workspace removes only its views.
  store
    .save_view(&LocalView {
      workspace_id: "w2".into(),
      ..view("a", None, "A", "0000000010")
    })
    .unwrap();
  store
    .save_view(&LocalView {
      workspace_id: "local".into(),
      ..view("b", None, "B", "0000000010")
    })
    .unwrap();
  store.delete_workspace("local", "w2").unwrap();
  let views = store.list_views("local").unwrap();
  assert!(views.iter().any(|v| v.id == "b"));
  assert!(views.iter().all(|v| v.id != "a"));
  assert!(store
    .list_workspaces("local")
    .unwrap()
    .iter()
    .all(|w| w.id != "w2"));
}

#[test]
fn origin_scopes_views_and_workspaces() {
  // One store holds both on-device data (origin "local") and a cloud
  // workspace mirrored for offline nav (origin = a server URL). Listing by
  // origin must never leak one provenance into the other.
  let store = LocalStore::open_in_memory().unwrap();
  let cloud = "https://mica.example.com";

  store
    .save_workspace(&LocalWorkspace {
      id: "cw".into(),
      name: "Cloud WS".into(),
      position: "0000000010".into(),
      origin: cloud.into(),
      role: "editor".into(),
    })
    .unwrap();
  store
    .save_view(&LocalView {
      origin: cloud.into(),
      workspace_id: "cw".into(),
      ..view("cv", None, "Cloud Page", "0000000010")
    })
    .unwrap();
  // A local view/workspace alongside it.
  store
    .save_view(&view("lv", None, "Local Page", "0000000010"))
    .unwrap();

  let local_views = store.list_views("local").unwrap();
  assert!(local_views.iter().any(|v| v.id == "lv"));
  assert!(
    local_views.iter().all(|v| v.id != "cv"),
    "cloud view hidden from local"
  );

  let cloud_views = store.list_views(cloud).unwrap();
  assert_eq!(cloud_views.len(), 1);
  assert_eq!(cloud_views[0].id, "cv");
  assert_eq!(cloud_views[0].origin, cloud);

  // Default "local" workspace exists; cloud origin sees only its own.
  assert!(store
    .list_workspaces("local")
    .unwrap()
    .iter()
    .any(|w| w.id == "local"));
  let cloud_ws = store.list_workspaces(cloud).unwrap();
  assert_eq!(cloud_ws.len(), 1);
  assert_eq!(cloud_ws[0].id, "cw");
  assert_eq!(
    cloud_ws[0].role, "editor",
    "mirrored role available offline"
  );
}

#[test]
fn composite_pk_isolates_same_id_across_origins() {
  // P3a: the SAME id may exist under 'local' AND a server URL (the detach
  // scenario). The (origin,id) PK must let both rows coexist, upserts stay
  // in-namespace, and purge/delete touch only their own origin.
  let store = LocalStore::open_in_memory().unwrap();
  let cloud = "https://mica.example.com";

  // Same view id in both namespaces.
  store
    .save_view(&view("x", None, "Local X", "0000000010"))
    .unwrap();
  store
    .save_view(&LocalView {
      origin: cloud.into(),
      name: "Cloud X".into(),
      ..view("x", None, "ignored", "0000000020")
    })
    .unwrap();
  assert_eq!(
    store
      .list_views("local")
      .unwrap()
      .iter()
      .find(|v| v.id == "x")
      .unwrap()
      .name,
    "Local X"
  );
  assert_eq!(
    store
      .list_views(cloud)
      .unwrap()
      .iter()
      .find(|v| v.id == "x")
      .unwrap()
      .name,
    "Cloud X"
  );

  // Upsert stays in its namespace (would have clobbered cross-origin
  // under the old bare-id PK).
  store
    .save_view(&view("x", None, "Local X2", "0000000010"))
    .unwrap();
  assert_eq!(
    store
      .list_views(cloud)
      .unwrap()
      .iter()
      .find(|v| v.id == "x")
      .unwrap()
      .name,
    "Cloud X"
  );

  // Purge one origin's row only.
  store.purge_view("local", "x").unwrap();
  assert!(store
    .list_views("local")
    .unwrap()
    .iter()
    .all(|v| v.id != "x"));
  assert!(
    store.list_views(cloud).unwrap().iter().any(|v| v.id == "x"),
    "cloud row untouched"
  );

  // Same workspace id in both namespaces; delete is origin-scoped and its
  // view cascade never crosses origins.
  for (origin, role) in [("local", "owner"), (cloud, "editor")] {
    store
      .save_workspace(&LocalWorkspace {
        id: "w".into(),
        name: format!("{origin} W"),
        position: "0000000010".into(),
        origin: origin.into(),
        role: role.into(),
      })
      .unwrap();
    store
      .save_view(&LocalView {
        origin: origin.into(),
        workspace_id: "w".into(),
        ..view(&format!("v-{}", role), None, "p", "0000000010")
      })
      .unwrap();
  }
  store.delete_workspace(cloud, "w").unwrap();
  assert!(
    store
      .list_workspaces("local")
      .unwrap()
      .iter()
      .any(|w| w.id == "w"),
    "local ws kept"
  );
  assert!(
    store
      .list_views("local")
      .unwrap()
      .iter()
      .any(|v| v.id == "v-owner"),
    "local view kept"
  );
  assert!(store
    .list_workspaces(cloud)
    .unwrap()
    .iter()
    .all(|w| w.id != "w"));
  assert!(store
    .list_views(cloud)
    .unwrap()
    .iter()
    .all(|v| v.id != "v-editor"));
}

#[test]
fn v3_bare_pk_store_rebuilds_to_composite_without_data_loss() {
  // A real v3-era store: bare-id PKs, origin/role columns present, data in
  // both namespaces. Opening must rebuild both tables to the (origin,id)
  // PK, keep every row, and leave the one-time pre-migration backups.
  let path = temp_path();
  let p = path.to_str().unwrap();
  {
    let conn = Connection::open(p).unwrap();
    conn.execute_batch(
              "CREATE TABLE doc_snapshot(doc_id TEXT PRIMARY KEY, state BLOB NOT NULL, updated_at INTEGER NOT NULL);
               CREATE TABLE local_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
               INSERT INTO local_meta VALUES('schema_version','3');
               CREATE TABLE local_view(
                   id TEXT PRIMARY KEY, parent_id TEXT, object_id TEXT NOT NULL,
                   name TEXT NOT NULL, position TEXT NOT NULL,
                   trashed INTEGER NOT NULL DEFAULT 0,
                   origin TEXT NOT NULL DEFAULT 'local',
                   workspace_id TEXT NOT NULL DEFAULT 'local');
               CREATE TABLE local_workspace(
                   id TEXT PRIMARY KEY, name TEXT NOT NULL, position TEXT NOT NULL,
                   origin TEXT NOT NULL DEFAULT 'local',
                   role TEXT NOT NULL DEFAULT 'viewer');
               INSERT INTO local_workspace(id,name,position) VALUES('local','本地工作区','0000000010');
               INSERT INTO local_workspace(id,name,position,origin,role) VALUES('cw','Cloud','0000000020','https://s','editor');
               INSERT INTO local_view(id,object_id,name,position) VALUES('v1','d1','Local Page','0000000010');
               INSERT INTO local_view(id,object_id,name,position,origin,workspace_id) VALUES('v2','d2','Cloud Page','0000000010','https://s','cw');",
          )
          .unwrap();
  }
  let store = LocalStore::open(p).unwrap();
  // PKs are composite now.
  for table in ["local_view", "local_workspace"] {
    let pk: i64 = store
      .conn
      .query_row(
        &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE pk>0"),
        [],
        |r| r.get(0),
      )
      .unwrap();
    assert_eq!(pk, 2, "{table} has the (origin,id) composite PK");
  }
  // Every row survived, per namespace.
  assert_eq!(
    store
      .list_views("local")
      .unwrap()
      .iter()
      .find(|v| v.id == "v1")
      .unwrap()
      .name,
    "Local Page"
  );
  let cv = store.list_views("https://s").unwrap();
  assert_eq!(cv.len(), 1);
  assert_eq!(
    (cv[0].id.as_str(), cv[0].workspace_id.as_str()),
    ("v2", "cw")
  );
  assert_eq!(
    store.list_workspaces("https://s").unwrap()[0].role,
    "editor",
    "cloud role survived the rebuild",
  );
  // The one-time pre-migration snapshots exist (recovery hatch).
  for backup in ["local_view_v3_backup", "local_workspace_v3_backup"] {
    let n: i64 = store
      .conn
      .query_row(&format!("SELECT COUNT(*) FROM {backup}"), [], |r| r.get(0))
      .unwrap();
    assert!(n >= 1, "{backup} holds the pre-v4 rows");
  }
  // Reopen is a no-op (idempotent — the PK probe skips the rebuild).
  drop(store);
  let again = LocalStore::open(p).unwrap();
  assert_eq!(again.list_views("local").unwrap().len(), 1);
  drop(again);
  std::fs::remove_file(p).ok();
}

/// Pre-release migration smoke: point MICA_REAL_STORE at a COPY of a real
/// on-device store.db and run with `cargo test --features store -- --ignored`.
/// Opens it through the full migration chain and asserts the terminal shape
/// (v4 composite PK) plus that existing local rows are still listed.
#[test]
#[ignore]
fn upgrade_real_store_smoke() {
  let p = match std::env::var("MICA_REAL_STORE") {
    Ok(p) if !p.is_empty() => p,
    _ => {
      eprintln!("MICA_REAL_STORE not set — skipping");
      return;
    }
  };
  // Per ORIGIN, not a single total. The original compared
  // `COUNT(*) FROM local_view` (every origin) against `list_views("local")`
  // (one origin) — fine when a real store held only local pages, and wrong
  // from the moment the app began mirroring the cloud page tree into the
  // same table under the server URL as origin. On this machine that is 153
  // mirrored rows against 9 local ones, so the gate failed for everyone who
  // had ever signed in — and failed saying «view rows lost in the upgrade»,
  // which sends whoever hits it hunting a corruption bug that is not there.
  //
  // Comparing every origin to itself is also strictly stronger than the
  // total ever was: it catches rows moving BETWEEN origins, which a sum
  // would hide.
  let counts_by_origin = |conn: &Connection| -> Vec<(String, i64)> {
    let mut stmt = conn
      .prepare("SELECT origin, COUNT(*) FROM local_view GROUP BY origin ORDER BY origin")
      .unwrap();
    let rows = stmt
      .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
      .unwrap();
    rows.map(|r| r.unwrap()).collect()
  };
  let before_by_origin = {
    let conn = Connection::open(&p).unwrap();
    counts_by_origin(&conn)
  };
  let store = LocalStore::open(&p).unwrap();
  for table in ["local_view", "local_workspace"] {
    let pk: i64 = store
      .conn
      .query_row(
        &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE pk>0"),
        [],
        |r| r.get(0),
      )
      .unwrap();
    assert_eq!(pk, 2, "{table} migrated to the (origin,id) composite PK");
  }
  assert_eq!(
    counts_by_origin(&store.conn),
    before_by_origin,
    "no view rows lost in the upgrade (compared per origin)"
  );
  let after = store.list_views("local").unwrap();
  eprintln!(
    "real store upgraded OK: {} local views, origins={:?}, workspaces={:?}",
    after.len(),
    before_by_origin,
    store
      .list_workspaces("local")
      .unwrap()
      .iter()
      .map(|w| w.id.clone())
      .collect::<Vec<_>>(),
  );
}

/// The `contain_yrs_panic` guard, in ISOLATION from the checksum.
///
/// Reported from a real install: `yrs-0.27.0/src/block_store.rs:60 index out
/// of bounds: the len is 732 but the index is 715827364`, raised from
/// `load_doc` on the cloud-seed path. The desktop FFI holds
/// `Mutex<LocalStore>` across this call, so the panic poisoned that lock and
/// every later `list_views` failed: the page tree rendered blank on every
/// launch.
///
/// Byte 9 flipped survives `decode_v1` and blows up later during INTEGRATION
/// (the class `?` never saw and `catch_unwind` does) — a truncation would be
/// rejected by `decode_v1` and prove nothing. Crucially, `state_crc` is set
/// to NULL first: this is a LEGACY row with no checksum, so `verify_blob_crc`
/// passes it through and the bytes actually reach yrs. Without that, the new
/// checksum would catch the flip before the guard and this test would no
/// longer exercise `contain_yrs_panic` at all.
#[test]
fn a_corrupt_snapshot_is_an_error_not_a_panic() {
  let store = LocalStore::open_in_memory().unwrap();
  let doc = MicaDoc::from_blocks(
    "r",
    &[
      crate::Block::new("r", "page").with_children(vec!["a".into()]),
      crate::Block::new("a", "paragraph").with_text("hello world".to_string()),
    ],
  );
  store.save_doc("local", "d", &doc).unwrap();
  let good: Vec<u8> = store
    .conn
    .query_row("SELECT state FROM doc_snapshot WHERE doc_id='d'", [], |r| {
      r.get(0)
    })
    .unwrap();

  let mut bad = good.clone();
  bad[0] ^= 0xff;
  // Corrupt the bytes AND null the checksum → the crc can't mask the flip,
  // so the corruption actually reaches yrs.
  store
    .conn
    .execute(
      "UPDATE doc_snapshot SET state=?1, state_crc=NULL WHERE doc_id='d'",
      params![bad],
    )
    .unwrap();

  // The contract that matters: `load_doc` RETURNS instead of unwinding out
  // of the caller's mutex. We deliberately do NOT assert a specific error
  // variant: which yrs failure mode a corrupted blob hits (decode_v1
  // rejecting it vs. an integration panic the guard converts) depends on
  // the exact bytes, and asserting `CorruptDoc` is what made this test
  // FLAKY — `from_blocks` mints a RANDOM client id, so the encoding (and
  // thus the failure mode) differed per run. It passed locally and on one
  // CI run, then failed on another with "got a different error: failed to
  // decode yrs update". The guard mechanism itself is pinned deterministically
  // by `contain_yrs_panic_converts_a_panic_into_corrupt_doc` below.
  assert!(
    store.load_doc("local", "d", 1).is_err(),
    "a corrupt snapshot must come back as an error, never unwind"
  );
}

/// The panic-containment contract, deterministically — no dependence on
/// coaxing yrs into panicking (which is not portable, see above). Remove the
/// `catch_unwind` in `contain_yrs_panic` and this test unwinds instead of
/// returning, so it still kills that mutation.
#[test]
fn contain_yrs_panic_converts_a_panic_into_corrupt_doc() {
  let caught: Result<(), StoreError> =
    contain_yrs_panic("d", || panic!("simulated yrs panic while integrating"));
  match caught {
    Err(StoreError::CorruptDoc { doc_id }) => assert_eq!(doc_id, "d"),
    other => panic!("a panic must become CorruptDoc, got {other:?}"),
  }
  // The non-panicking path passes values through untouched.
  let passed: Result<u8, StoreError> = contain_yrs_panic("d", || Ok(7));
  assert!(matches!(passed, Ok(7)));
}

/// A legacy snapshot has no checksum: invalid UTF-8 must still be rejected
/// as a normal decode error by yrs 0.28.0, not abort the desktop process.
#[test]
fn invalid_utf8_in_legacy_snapshot_is_a_decode_error() {
  let store = LocalStore::open_in_memory().unwrap();
  let doc = MicaDoc::from_blocks(
    "r",
    &[
      crate::Block::new("r", "page").with_children(vec!["a".into()]),
      crate::Block::new("a", "paragraph").with_text("hello world".to_string()),
    ],
  );
  store.save_doc("local", "d", &doc).unwrap();
  let good: Vec<u8> = store
    .conn
    .query_row("SELECT state FROM doc_snapshot WHERE doc_id='d'", [], |r| {
      r.get(0)
    })
    .unwrap();

  let mut bad = good;
  let text = b"hello world";
  let at = bad
    .windows(text.len())
    .position(|w| w == text)
    .expect("encoded text");
  bad[at] = 0xff;
  store
    .conn
    .execute(
      "UPDATE doc_snapshot SET state=?1, state_crc=NULL WHERE doc_id='d'",
      params![bad],
    )
    .unwrap();
  assert!(matches!(
    store.load_doc("local", "d", 1),
    Err(StoreError::Doc(DocError::Decode(_)))
  ));
}

// ── local blob checksum (docs/reviews/code-review-2026-07-20.md P0-0) ────────────

/// Locks the polynomial/params so a silent CRC drift can't slip through.
/// Values are the standard CRC-32/ISO-HDLC check vectors.
#[test]
fn crc32_known_vectors() {
  assert_eq!(crc32(b""), 0x0000_0000);
  assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
  assert_eq!(
    crc32(b"The quick brown fox jumps over the lazy dog"),
    0x414F_A339
  );
}

fn stored_doc() -> (LocalStore, Vec<u8>) {
  let store = LocalStore::open_in_memory().unwrap();
  let (root, blocks) = sample();
  let doc = MicaDoc::from_blocks(&root, &blocks);
  store.save_doc("local", "d", &doc).unwrap();
  let state: Vec<u8> = store
    .conn
    .query_row("SELECT state FROM doc_snapshot WHERE doc_id='d'", [], |r| {
      r.get(0)
    })
    .unwrap();
  (store, state)
}

/// The write path stamps a correct checksum, and — critically — re-stamps it
/// on the SECOND save (the UPSERT). A missing `state_crc=excluded.state_crc`
/// in the SET clause would leave the crc stale here and false-flag the doc.
#[test]
fn save_records_and_updates_crc() {
  let (store, state) = stored_doc();
  let crc: Option<i64> = store
    .conn
    .query_row(
      "SELECT state_crc FROM doc_snapshot WHERE doc_id='d'",
      [],
      |r| r.get(0),
    )
    .unwrap();
  assert_eq!(crc, Some(blob_crc(&state)), "first save records the crc");

  // Second save with changed content must advance the crc, not leave it stale.
  let (root, mut blocks) = sample();
  blocks.push(Block::new("b", "paragraph").with_text("more"));
  blocks[0] = Block::new("r", "page").with_children(vec!["a".into(), "b".into()]);
  store
    .save_doc("local", "d", &MicaDoc::from_blocks(&root, &blocks))
    .unwrap();
  let (state2, crc2): (Vec<u8>, Option<i64>) = store
    .conn
    .query_row(
      "SELECT state, state_crc FROM doc_snapshot WHERE doc_id='d'",
      [],
      |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap();
  assert_eq!(
    crc2,
    Some(blob_crc(&state2)),
    "second save re-stamps the crc"
  );
  assert!(
    store.load_doc("local", "d", 1).unwrap().is_some(),
    "still loads after re-save"
  );
}

/// Primary mutation test, isolated from yrs: corrupt ONLY the checksum, leave
/// `state` a VALID yrs update. yrs would decode it without panicking, so the
/// ONLY thing that can produce an error is `verify_blob_crc`. Remove that call
/// and `load_doc` returns `Ok(Some(_))` → this test fails. That is the
/// mutation-kills-test guarantee, independent of how yrs reacts to garbage.
#[test]
fn corrupt_state_crc_is_caught_before_yrs() {
  let (store, _state) = stored_doc();
  store
    .conn
    .execute(
      "UPDATE doc_snapshot SET state_crc = state_crc + 1 WHERE doc_id='d'",
      [],
    )
    .unwrap();
  assert!(
    matches!(
      store.load_doc("local", "d", 1),
      Err(StoreError::CorruptDoc { .. })
    ),
    "a stale checksum over valid bytes must be caught by the crc alone"
  );
}

/// Real bit-rot: flip a byte of `state`, keep the (now-mismatched) crc → the
/// crc catches it BEFORE the bytes reach yrs, independently of whether
/// the altered bytes would still decode.
#[test]
fn bit_flip_in_state_is_caught_by_crc() {
  let (store, mut state) = stored_doc();
  state[5] ^= 0x01;
  store
    .conn
    .execute(
      "UPDATE doc_snapshot SET state=?1 WHERE doc_id='d'",
      params![state],
    )
    .unwrap();
  assert!(matches!(
    store.load_doc("local", "d", 1),
    Err(StoreError::CorruptDoc { .. })
  ));
}

/// The two previously-UNPROTECTED recovery decodes now verify their blob.
#[test]
fn restore_and_rollback_verify_crc() {
  // restore_local_version: corrupt a version's crc → CorruptDoc, no panic.
  let (store, _s) = stored_doc();
  let v = store
    .create_local_version("local", "d", "v1")
    .unwrap()
    .unwrap();
  store
    .conn
    .execute(
      "UPDATE doc_version SET state_crc = state_crc + 1 WHERE id=?1",
      params![v.id],
    )
    .unwrap();
  assert!(matches!(
    store.restore_local_version("local", "d", &v.id, 1),
    Err(StoreError::CorruptDoc { .. })
  ));

  // rollback_doc: checkpoint, corrupt the backup's crc → CorruptDoc.
  let (store2, _s2) = stored_doc();
  store2.checkpoint_doc("local", "d").unwrap();
  store2
    .conn
    .execute(
      "UPDATE doc_snapshot_backup SET state_crc = state_crc + 1 WHERE doc_id='d'",
      [],
    )
    .unwrap();
  assert!(matches!(
    store2.rollback_doc("local", "d", 1),
    Err(StoreError::CorruptDoc { .. })
  ));
}

/// A rotted local update log row is caught (via the log helpers) and never
/// replayed into yrs.
#[test]
fn corrupt_update_log_crc_is_caught() {
  let (store, _s) = stored_doc();
  store.append_update("local", "d", b"\x00\x01\x02").unwrap();
  store
    .conn
    .execute(
      "UPDATE doc_update SET payload_crc = payload_crc + 1 WHERE doc_id='d'",
      [],
    )
    .unwrap();
  assert!(matches!(
    store.load_doc("local", "d", 1),
    Err(StoreError::CorruptDoc { .. })
  ));
}

/// `local_version_state` verifies the preview blob it hands out (its decode
/// happens outside store.rs).
#[test]
fn local_version_state_verifies_crc() {
  let (store, _s) = stored_doc();
  let v = store
    .create_local_version("local", "d", "v1")
    .unwrap()
    .unwrap();
  assert!(
    store
      .local_version_state("local", "d", &v.id)
      .unwrap()
      .is_some(),
    "healthy preview loads"
  );
  store
    .conn
    .execute(
      "UPDATE doc_version SET state_crc = state_crc + 1 WHERE id=?1",
      params![v.id],
    )
    .unwrap();
  assert!(matches!(
    store.local_version_state("local", "d", &v.id),
    Err(StoreError::CorruptDoc { .. })
  ));
}

/// A legacy row (crc NULL) must NOT error on load — the None branch passes
/// through to exactly today's behavior. And with the crc NULL, integration-
/// phase corruption still degrades via the guard (not the crc), proving the
/// fall-through is intact. (Companion to `a_corrupt_snapshot_is_an_error_not_a_panic`.)
#[test]
fn legacy_null_crc_still_loads() {
  let (store, _state) = stored_doc();
  store
    .conn
    .execute(
      "UPDATE doc_snapshot SET state_crc=NULL WHERE doc_id='d'",
      [],
    )
    .unwrap();
  assert!(
    store.load_doc("local", "d", 1).unwrap().is_some(),
    "NULL crc = no checksum recorded → must load, not error"
  );
}

/// A pre-checksum v6 store (built with a RAW connection using the OLD DDL, so
/// the probe-ALTER — not the base DDL — is what adds the column) upgrades
/// without data loss: column added, existing row's crc NULL, blob byte-
/// identical, loads fine, version unchanged.
#[test]
fn v6_store_without_crc_column_migrates() {
  let path = temp_path();
  let p = path.to_str().unwrap();
  let (root, blocks) = sample();
  let state = MicaDoc::from_blocks(&root, &blocks).encode_state();
  {
    // OLD shape: no state_crc column.
    let conn = Connection::open(p).unwrap();
    conn.execute_batch(
              "CREATE TABLE doc_snapshot(doc_id TEXT PRIMARY KEY, state BLOB NOT NULL, updated_at INTEGER NOT NULL);
               CREATE TABLE local_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);",
          )
          .unwrap();
    conn
      .execute(
        "INSERT INTO doc_snapshot(doc_id,state,updated_at) VALUES('d',?1,0)",
        params![state],
      )
      .unwrap();
    conn
      .execute(
        "INSERT INTO local_meta(key,value) VALUES('schema_version','6')",
        [],
      )
      .unwrap();
  }
  let store = LocalStore::open(p).unwrap();
  let has: i64 = store
    .conn
    .query_row(
      "SELECT COUNT(*) FROM pragma_table_info('doc_snapshot') WHERE name='state_crc'",
      [],
      |r| r.get(0),
    )
    .unwrap();
  assert_eq!(has, 1, "probe-ALTER added state_crc");
  let (blob, crc): (Vec<u8>, Option<i64>) = store
    .conn
    .query_row(
      "SELECT state, state_crc FROM doc_snapshot WHERE doc_id='d'",
      [],
      |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap();
  assert_eq!(blob, state, "existing blob byte-identical (no data loss)");
  assert!(
    crc.is_none(),
    "legacy row's crc stays NULL (not fabricated)"
  );
  assert!(
    store.load_doc("local", "d", 1).unwrap().is_some(),
    "legacy row still loads"
  );
  let v: String = store
    .conn
    .query_row(
      "SELECT value FROM local_meta WHERE key='schema_version'",
      [],
      |r| r.get(0),
    )
    .unwrap();
  assert_eq!(v, SCHEMA_VERSION.to_string(), "no version bump");
  let _ = std::fs::remove_file(p);
}

/// Local backlinks exist at all. The local (offline) world used to hide the
/// panel entirely — `onLoadBacklinks` was null there — so a page that WAS
/// linked from three others showed nothing, indistinguishable from a page
/// nobody links to.
///
/// These pin BEHAVIOUR: what the panel would list. Asserting that a
/// `link_targets` column exists would pass with a query that never matches.
mod local_backlinks {
  use super::super::*;
  use crate::block::Block;
  use crate::marks::Mark;

  /// A document whose single block links to `targets`.
  ///
  /// Marks go on through [`MicaDoc::set_block_text`], NOT by handing
  /// `from_blocks` a `data.marks` — inline marks live in the CRDT as yrs
  /// text formatting, and `to_blocks()` is what rebuilds `data.marks` from
  /// it. A fixture that set `data` directly stored a block with `data:
  /// Null`, so every backlink assertion here came back empty and would have
  /// read as "the query is broken".
  fn linking_doc(targets: &[&str]) -> MicaDoc {
    let text = "see";
    let marks: Vec<Mark> = targets
      .iter()
      .map(|t| Mark {
        start: 0,
        end: text.chars().count() as u32,
        ty: "link".to_string(),
        href: Some(format!("mica://page/{t}")),
        title: None,
      })
      .collect();
    let mut doc = MicaDoc::from_blocks("root", &[Block::new("root", "page")]);
    doc.set_block_text("root", text, &marks);
    doc
  }

  fn view(id: &str, doc: &str, name: &str) -> LocalView {
    LocalView {
      id: id.into(),
      workspace_id: "local".into(),
      parent_id: None,
      object_id: doc.into(),
      name: name.into(),
      position: "0000000010".into(),
      trashed: false,
      origin: "local".into(),
      object_type: "document".into(),
    }
  }

  /// v1 and v2 both link to v3; v4 links nowhere.
  fn seeded() -> LocalStore {
    let store = LocalStore::open_in_memory().unwrap();
    for (v, d, name, targets) in [
      ("v1", "d1", "甲", vec!["v3"]),
      ("v2", "d2", "乙", vec!["v3"]),
      ("v3", "d3", "丙", vec![]),
      ("v4", "d4", "丁", vec![]),
    ] {
      store.save_doc("local", d, &linking_doc(&targets)).unwrap();
      store.save_view(&view(v, d, name)).unwrap();
    }
    store
  }

  /// Order is the store's own collation, not a locale one: SQLite compares
  /// TEXT by code point, so 乙 (U+4E59) precedes 甲 (U+7532). Pinned as-is
  /// rather than "fixed" — the panel needs a STABLE order, and giving the
  /// local world an ICU collation to match whatever Postgres happens to use
  /// would be a dependency bought for the sort order of a sidebar list.
  #[test]
  fn lists_every_page_that_links_here() {
    let store = seeded();
    let names: Vec<String> = store
      .backlinks_local("local", "v3")
      .unwrap()
      .into_iter()
      .map(|b| b.name)
      .collect();
    assert_eq!(names, vec!["乙".to_string(), "甲".to_string()]);
  }

  #[test]
  fn a_page_nobody_links_to_has_none() {
    assert!(seeded().backlinks_local("local", "v4").unwrap().is_empty());
  }

  /// The half that rots silently. An index that only ever GAINS entries
  /// passes every "the backlink shows up" test while the panel keeps
  /// listing a page whose link was deleted — and on the local side the old
  /// code could not regress this way at all, because there was no index.
  #[test]
  fn deleting_the_link_removes_the_backlink() {
    let store = seeded();
    assert_eq!(store.backlinks_local("local", "v3").unwrap().len(), 2);

    store.save_doc("local", "d1", &linking_doc(&[])).unwrap();
    let names: Vec<String> = store
      .backlinks_local("local", "v3")
      .unwrap()
      .into_iter()
      .map(|b| b.name)
      .collect();
    assert_eq!(names, vec!["乙".to_string()], "甲 must leave the index");
  }

  /// The graph view's data. `丁` links nowhere and nothing links to it, so
  /// it must be COUNTED, not drawn — a canvas of disconnected dots hides
  /// the structure the view exists to show, and every "the edge is there"
  /// assertion would still pass while it happened.
  #[test]
  fn the_graph_carries_edges_and_counts_the_rest() {
    let store = seeded();
    let g = store.graph_local("local").unwrap();

    assert_eq!(g.edges.len(), 2, "{:?}", g.edges);
    assert!(g.edges.iter().all(|e| e.target == "v3"));
    let sources: std::collections::HashSet<&str> =
      g.edges.iter().map(|e| e.source.as_str()).collect();
    assert_eq!(sources, ["v1", "v2"].into_iter().collect());

    let ids: Vec<&str> = g.nodes.iter().map(|n| n.view_id.as_str()).collect();
    assert_eq!(ids.len(), 3, "v1 v2 v3 participate: {ids:?}");
    assert!(!ids.contains(&"v4"), "丁 must not be a node: {ids:?}");
    assert_eq!(g.unlinked, 1, "丁 is counted");
    // v3 is the hub: two edges in.
    assert_eq!(g.nodes[0].view_id, "v3");
    assert_eq!(g.nodes[0].degree, 2);
  }

  /// A link whose target is in the trash is a DANGLING reference, not an
  /// edge — drawing it would invent a node that is not in the workspace.
  #[test]
  fn a_link_to_a_trashed_page_is_not_an_edge() {
    let store = seeded();
    let mut v3 = view("v3", "d3", "丙");
    v3.trashed = true;
    store.save_view(&v3).unwrap();

    let g = store.graph_local("local").unwrap();
    assert!(g.edges.is_empty(), "dangling edges survived: {:?}", g.edges);
    assert!(g.nodes.is_empty(), "{:?}", g.nodes);
  }

  #[test]
  fn a_self_link_is_not_a_backlink() {
    let store = LocalStore::open_in_memory().unwrap();
    store
      .save_doc("local", "d1", &linking_doc(&["v1"]))
      .unwrap();
    store.save_view(&view("v1", "d1", "自引")).unwrap();
    assert!(store.backlinks_local("local", "v1").unwrap().is_empty());
  }

  #[test]
  fn a_trashed_source_stops_counting() {
    let store = seeded();
    let mut v = view("v1", "d1", "甲");
    v.trashed = true;
    store.save_view(&v).unwrap();
    let names: Vec<String> = store
      .backlinks_local("local", "v3")
      .unwrap()
      .into_iter()
      .map(|b| b.name)
      .collect();
    assert_eq!(names, vec!["乙".to_string()]);
  }

  /// A row written before the column existed contributes nothing rather
  /// than erroring — `json_each` over NULL would otherwise blow up the
  /// whole panel for one legacy row.
  #[test]
  fn a_row_predating_the_column_is_skipped_not_fatal() {
    let store = seeded();
    store
      .conn
      .execute(
        "UPDATE doc_snapshot SET link_targets=NULL WHERE doc_id='d1'",
        [],
      )
      .unwrap();
    let names: Vec<String> = store
      .backlinks_local("local", "v3")
      .unwrap()
      .into_iter()
      .map(|b| b.name)
      .collect();
    assert_eq!(names, vec!["乙".to_string()]);

    // …and the backfill picks it up.
    assert!(store.backfill_content_text(100).unwrap() >= 1);
    assert_eq!(store.backlinks_local("local", "v3").unwrap().len(), 2);
  }
}

/// Local search exists at all — a local workspace used to answer every query
/// with "nothing matches", stating as fact that the page was not there.
///
/// These pin BEHAVIOUR, not the column: what a user typed, and what came
/// back. A test that asserted `content_text` exists would pass with a search
/// that never returns anything.
mod local_search {
  use super::super::*;
  use crate::block::Block;

  fn seeded() -> LocalStore {
    let store = LocalStore::open_in_memory().unwrap();
    for (view, doc, name, body) in [
      ("v1", "d1", "部署流程", "先装 Docker,再起 compose"),
      ("v2", "d2", "会议记录", "讨论了部署节奏与回滚"),
      ("v3", "d3", "读书笔记", "与运维无关的内容"),
    ] {
      let mut d = MicaDoc::from_blocks("root", &[Block::new("root", "page")]);
      d.text_insert("root", 0, body);
      store.save_doc("local", doc, &d).unwrap();
      store
        .save_view(&LocalView {
          id: view.into(),
          workspace_id: "local".into(),
          parent_id: None,
          object_id: doc.into(),
          name: name.into(),
          position: "0000000010".into(),
          trashed: false,
          origin: "local".into(),
          object_type: "document".into(),
        })
        .unwrap();
    }
    store
  }

  #[test]
  fn finds_pages_by_title_and_by_body() {
    let store = seeded();
    let hits = store.search_local("local", "部署", 20).unwrap();
    let names: Vec<&str> = hits.iter().map(|h| h.name.as_str()).collect();

    assert!(names.contains(&"部署流程"), "title hit missing: {names:?}");
    assert!(names.contains(&"会议记录"), "body hit missing: {names:?}");
    assert!(
      !names.contains(&"读书笔记"),
      "matched something it should not"
    );
  }

  /// A title hit sorts first and says so, so a hit with no quotable body
  /// does not read as an empty row.
  #[test]
  fn a_title_hit_comes_first_and_is_labelled() {
    let store = seeded();
    let hits = store.search_local("local", "部署", 20).unwrap();

    assert_eq!(hits[0].name, "部署流程");
    assert!(hits[0].title_match);
    assert!(
      !hits
        .iter()
        .find(|h| h.name == "会议记录")
        .unwrap()
        .title_match
    );
  }

  #[test]
  fn the_snippet_is_a_window_not_the_whole_document() {
    let store = seeded();
    let hit = store
      .search_local("local", "Docker", 20)
      .unwrap()
      .into_iter()
      .find(|h| h.name == "部署流程")
      .unwrap();

    assert!(hit.snippet.contains("Docker"), "{:?}", hit.snippet);
    assert!(hit.snippet.chars().count() <= 95, "{:?}", hit.snippet);
  }

  /// Second-round review #3: the local snippet sliced the ORIGINAL body at a
  /// byte offset found in a lowercased copy. `ẞ`→`ß` (3 bytes → 2) before
  /// the hit put that offset off a char boundary — a panic inside the
  /// store's mutex, poisoning it for every later call.
  #[test]
  fn a_body_whose_lowercase_is_shorter_neither_panics_nor_loses_the_hit() {
    let store = LocalStore::open_in_memory().unwrap();
    let mut d = MicaDoc::from_blocks("root", &[Block::new("root", "page")]);
    d.text_insert("root", 0, &format!("{}needle here", "ẞ".repeat(50)));
    store.save_doc("local", "dx", &d).unwrap();
    store
      .save_view(&LocalView {
        id: "vx".into(),
        workspace_id: "local".into(),
        parent_id: None,
        object_id: "dx".into(),
        name: "页".into(),
        position: "0000000010".into(),
        trashed: false,
        origin: "local".into(),
        object_type: "document".into(),
      })
      .unwrap();
    let hits = store.search_local("local", "needle", 20).unwrap();
    assert_eq!(hits.len(), 1);
    assert!(
      hits[0].snippet.contains("needle here"),
      "{:?}",
      hits[0].snippet
    );
  }

  /// `%` is a LIKE wildcard. Unescaped, searching for it returns the whole
  /// workspace — a "search" that matches everything is worse than one that
  /// finds nothing, because it looks like it worked.
  #[test]
  fn wildcards_in_the_query_are_literal() {
    let store = seeded();
    assert!(store.search_local("local", "%", 20).unwrap().is_empty());
    assert!(store.search_local("local", "_", 20).unwrap().is_empty());
  }

  #[test]
  fn trashed_pages_do_not_come_back() {
    let store = seeded();
    store
      .save_view(&LocalView {
        id: "v1".into(),
        workspace_id: "local".into(),
        parent_id: None,
        object_id: "d1".into(),
        name: "部署流程".into(),
        position: "0000000010".into(),
        trashed: true,
        origin: "local".into(),
        object_type: "document".into(),
      })
      .unwrap();

    let names: Vec<String> = store
      .search_local("local", "部署", 20)
      .unwrap()
      .into_iter()
      .map(|h| h.name)
      .collect();
    assert!(!names.contains(&"部署流程".to_string()), "{names:?}");
  }

  #[test]
  fn an_empty_query_returns_nothing_rather_than_everything() {
    let store = seeded();
    assert!(store.search_local("local", "   ", 20).unwrap().is_empty());
  }

  /// A row written before the column existed still matches on its name, and
  /// the backfill makes its body searchable without a schema migration.
  #[test]
  fn rows_from_an_older_build_backfill() {
    let store = seeded();
    store
      .conn
      .execute("UPDATE doc_snapshot SET content_text=NULL", [])
      .unwrap();

    assert_eq!(
      store.search_local("local", "部署", 20).unwrap().len(),
      1,
      "the title hit must survive an un-indexed store"
    );

    assert_eq!(store.backfill_content_text(100).unwrap(), 3);
    assert_eq!(store.search_local("local", "部署", 20).unwrap().len(), 2);
    assert_eq!(
      store.backfill_content_text(100).unwrap(),
      0,
      "a second pass has nothing left to do"
    );
  }
}

/// docs/reviews/code-review-2026-10-21.md C3/C4/C7 + 10-19 P2-11 — the local
/// store's data-correctness group. Each test is phrased as what the user
/// would observe, not as which statement runs.
mod local_data_correctness {
  use super::super::*;
  use crate::block::Block;
  use crate::marks::Mark;

  fn page(text: &str, link_targets: &[&str]) -> MicaDoc {
    let marks: Vec<Mark> = link_targets
      .iter()
      .map(|t| Mark {
        start: 0,
        end: text.chars().count() as u32,
        ty: "link".to_string(),
        href: Some(format!("mica://page/{t}")),
        title: None,
      })
      .collect();
    let mut doc = MicaDoc::from_blocks("root", &[Block::new("root", "page")]);
    doc.set_block_text("root", text, &marks);
    doc
  }

  fn put(store: &LocalStore, at: (&str, &str), id: &str, doc_id: &str, name: &str, doc: &MicaDoc) {
    store.save_doc(at.0, doc_id, doc).unwrap();
    store
      .save_view(&LocalView {
        id: id.into(),
        workspace_id: at.1.into(),
        parent_id: None,
        object_id: doc_id.into(),
        name: name.into(),
        position: "0000000010".into(),
        trashed: false,
        origin: at.0.into(),
        object_type: "document".into(),
      })
      .unwrap();
  }

  const CLOUD: &str = "https://mica.example";

  /// One store, three worlds that must not see each other: local workspace
  /// `w1`, local workspace `w2`, and a cloud mirror whose workspace id is
  /// ALSO `w1` (ids are only unique per origin) and which even reuses the
  /// view id `v3`. Every source page mentions 部署 and links to `v3`.
  fn three_worlds() -> LocalStore {
    let store = LocalStore::open_in_memory().unwrap();
    put(
      &store,
      ("local", "w1"),
      "v1",
      "d1",
      "部署 甲",
      &page("部署 see", &["v3"]),
    );
    put(
      &store,
      ("local", "w1"),
      "v3",
      "d3",
      "目标",
      &page("target", &[]),
    );
    put(
      &store,
      ("local", "w2"),
      "v2",
      "d2",
      "部署 乙",
      &page("部署 see", &["v3"]),
    );
    put(
      &store,
      (CLOUD, "w1"),
      "c1",
      "dc1",
      "部署 云",
      &page("部署 see", &["v3"]),
    );
    put(
      &store,
      (CLOUD, "w1"),
      "v3",
      "dc3",
      "云目标",
      &page("cloud", &[]),
    );
    store
  }

  /// C4: local search used to read every `local_view` row — the other
  /// local workspace and the cloud mirror's pages came back as hits.
  #[test]
  fn search_stays_inside_the_open_local_workspace() {
    let store = three_worlds();
    let ids: Vec<String> = store
      .search_local("w1", "部署", 20)
      .unwrap()
      .into_iter()
      .map(|h| h.view_id)
      .collect();
    assert_eq!(ids, vec!["v1".to_string()]);
  }

  #[test]
  fn backlinks_stay_inside_the_open_local_workspace() {
    let store = three_worlds();
    let ids: Vec<String> = store
      .backlinks_local("w1", "v3")
      .unwrap()
      .into_iter()
      .map(|b| b.view_id)
      .collect();
    assert_eq!(ids, vec!["v1".to_string()]);
  }

  /// The graph had two leaks: sources from every world, and the target
  /// join by bare id — a cloud page with the same id became a second edge.
  #[test]
  fn the_graph_stays_inside_the_open_local_workspace() {
    let store = three_worlds();
    let g = store.graph_local("w1").unwrap();
    let edges: Vec<(String, String)> = g.edges.into_iter().map(|e| (e.source, e.target)).collect();
    assert_eq!(edges, vec![("v1".to_string(), "v3".to_string())]);
    let mut nodes: Vec<String> = g.nodes.into_iter().map(|n| n.view_id).collect();
    nodes.sort();
    assert_eq!(nodes, vec!["v1".to_string(), "v3".to_string()]);
    assert_eq!(g.unlinked, 0, "nothing from w2 or the cloud is counted");
  }

  /// C3: 「永久删除」 must leave no copy of the document's content behind.
  /// Checked against EVERY table with a `doc_id` column, so a content
  /// table added later without joining `delete_doc` fails here too.
  #[test]
  fn delete_doc_leaves_nothing_keyed_by_the_doc() {
    let store = LocalStore::open_in_memory().unwrap();
    let doc = page("secret body", &[]);
    store.save_doc("local", "d", &doc).unwrap(); // also captures an auto version
    store
      .create_local_version("local", "d", "named")
      .unwrap()
      .unwrap();
    store.checkpoint_doc("local", "d").unwrap();
    store
      .append_update("local", "d", &doc.encode_state())
      .unwrap();
    store
      .append_remote_update("local", "d", 1, &doc.encode_state())
      .unwrap();

    store.delete_doc("local", "d").unwrap();

    let tables: Vec<String> = store
      .conn
      .prepare(
        "SELECT m.name FROM sqlite_master m
                   WHERE m.type='table'
                     AND EXISTS (SELECT 1 FROM pragma_table_info(m.name) WHERE name='doc_id')",
      )
      .unwrap()
      .query_map([], |r| r.get(0))
      .unwrap()
      .collect::<Result<_, _>>()
      .unwrap();
    assert!(tables.contains(&"doc_version".to_string()), "{tables:?}");
    for t in tables {
      let n: i64 = store
        .conn
        .query_row(
          &format!("SELECT COUNT(*) FROM {t} WHERE doc_id='d'"),
          [],
          |r| r.get(0),
        )
        .unwrap();
      assert_eq!(n, 0, "{t} still holds the deleted document");
    }
    assert!(store.list_local_versions("local", "d").unwrap().is_empty());
  }

  /// P2-11: the outbox reader handed rotted rows to the network unchecked,
  /// while its sibling `doc_updates` verified them.
  #[test]
  fn updates_after_refuses_a_rotted_row() {
    let store = LocalStore::open_in_memory().unwrap();
    store.save_doc("local", "d", &page("x", &[])).unwrap();
    store.append_update("local", "d", b"\x00\x01\x02").unwrap();
    store
      .conn
      .execute(
        "UPDATE doc_update SET payload_crc = payload_crc + 1 WHERE doc_id='d'",
        [],
      )
      .unwrap();
    assert!(matches!(
      store.updates_after("local", "d", 0),
      Err(StoreError::CorruptDoc { .. })
    ));
  }

  fn base_doc(cid: u64) -> MicaDoc {
    MicaDoc::from_blocks_with_client_id(
      "r",
      &[
        Block::new("r", "page").with_children(vec!["a".into()]),
        Block::new("a", "paragraph").with_text("base"),
      ],
      Some(cid),
    )
  }

  /// Append " logged" to `doc` through the LOG (not a full save) — the
  /// shape every local page has between two compactions.
  fn log_edit(store: &LocalStore, doc: &mut MicaDoc) {
    let sv = doc.state_vector();
    doc.text_insert("a", 4, " logged");
    store
      .append_update("local", "d", &doc.encode_diff(&sv).unwrap())
      .unwrap();
  }

  fn text_of(doc: &MicaDoc) -> String {
    doc
      .to_blocks()
      .into_iter()
      .find(|b| b.id == "a")
      .map(|b| b.text)
      .unwrap_or_default()
  }

  fn raw_base(store: &LocalStore) -> Vec<u8> {
    store
      .conn
      .query_row("SELECT state FROM doc_snapshot WHERE doc_id='d'", [], |r| {
        r.get(0)
      })
      .unwrap()
  }

  fn count(store: &LocalStore, sql: &str) -> i64 {
    store.conn.query_row(sql, [], |r| r.get(0)).unwrap()
  }

  /// Bytes that pass the checksum (it was computed over them) but are not
  /// a yrs update — the shape a buggy writer leaves, which a crc cannot see.
  const GARBAGE: [u8; 4] = [9, 9, 9, 9];

  /// C7: restore wrote the version as the live base and dropped the log
  /// BEFORE decoding it — a bad version replaced a good page with an
  /// unopenable one. A failed restore must change nothing.
  #[test]
  fn a_version_that_will_not_decode_leaves_the_page_untouched() {
    let store = LocalStore::open_in_memory().unwrap();
    let cid = store.identity().unwrap().client_id;
    let mut doc = base_doc(cid);
    store.save_doc("local", "d", &doc).unwrap();
    log_edit(&store, &mut doc);
    store
      .conn
      .execute(
        "INSERT INTO doc_version(origin,id,doc_id,label,created_at,state,state_crc)
                  VALUES('local','bad','d','bad',0,?1,?2)",
        params![GARBAGE.to_vec(), blob_crc(&GARBAGE)],
      )
      .unwrap();
    let base_before = raw_base(&store);
    let versions_before = count(&store, "SELECT COUNT(*) FROM doc_version");

    assert!(store
      .restore_local_version("local", "d", "bad", cid)
      .is_err());

    assert_eq!(
      raw_base(&store),
      base_before,
      "the bad version became the live base"
    );
    assert_eq!(
      count(&store, "SELECT COUNT(*) FROM doc_update"),
      1,
      "the log was dropped"
    );
    assert_eq!(
      count(&store, "SELECT COUNT(*) FROM doc_version"),
      versions_before
    );
    assert_eq!(
      text_of(&store.load_doc("local", "d", cid).unwrap().unwrap()),
      "base logged"
    );
  }

  #[test]
  fn a_checkpoint_that_will_not_decode_leaves_the_page_untouched() {
    let store = LocalStore::open_in_memory().unwrap();
    let cid = store.identity().unwrap().client_id;
    let mut doc = base_doc(cid);
    store.save_doc("local", "d", &doc).unwrap();
    log_edit(&store, &mut doc);
    store
      .conn
      .execute(
        "INSERT INTO doc_snapshot_backup(origin,doc_id,state,updated_at,state_crc) VALUES('local','d',?1,0,?2)",
        params![GARBAGE.to_vec(), blob_crc(&GARBAGE)],
      )
      .unwrap();
    let base_before = raw_base(&store);

    assert!(store.rollback_doc("local", "d", cid).is_err());

    assert_eq!(
      raw_base(&store),
      base_before,
      "the bad checkpoint became the live base"
    );
    assert_eq!(
      count(&store, "SELECT COUNT(*) FROM doc_update"),
      1,
      "the log was dropped"
    );
    assert_eq!(
      text_of(&store.load_doc("local", "d", cid).unwrap().unwrap()),
      "base logged"
    );
  }

  /// C7 / 10-19 P2-12: archive + base swap + log drop were separate
  /// statements. Any of them failing part-way (power loss, disk full —
  /// injected here with a trigger) left a half-restore on disk; with the
  /// base swapped but the log kept, the discarded edits replay on the next
  /// open and silently come back. All or nothing.
  #[test]
  fn a_restore_that_fails_mid_write_commits_nothing() {
    let store = LocalStore::open_in_memory().unwrap();
    let cid = store.identity().unwrap().client_id;
    let mut doc = base_doc(cid);
    store.save_doc("local", "d", &doc).unwrap();
    let v = store
      .create_local_version("local", "d", "v")
      .unwrap()
      .unwrap();
    store.checkpoint_doc("local", "d").unwrap();
    log_edit(&store, &mut doc);
    store.save_doc("local", "d", &doc).unwrap(); // base now differs from v / the checkpoint
    store
      .conn
      .execute_batch(
        "CREATE TRIGGER inject_failure BEFORE DELETE ON doc_update
                   BEGIN SELECT RAISE(ABORT, 'injected'); END;",
      )
      .unwrap();
    let base_before = raw_base(&store);
    let versions_before = count(&store, "SELECT COUNT(*) FROM doc_version");

    assert!(store
      .restore_local_version("local", "d", &v.id, cid)
      .is_err());
    assert_eq!(
      raw_base(&store),
      base_before,
      "restore: base swapped without the log drop"
    );
    assert_eq!(
      count(&store, "SELECT COUNT(*) FROM doc_version"),
      versions_before,
      "restore: the archive row outlived the failed restore"
    );

    assert!(store.rollback_doc("local", "d", cid).is_err());
    assert_eq!(
      raw_base(&store),
      base_before,
      "rollback: base swapped without the log drop"
    );
  }

  /// C7: the pre-restore archive (what makes a restore undoable) copied
  /// only the BASE row. Edits still in the log were in neither the archive
  /// nor the restored page — undo could not bring them back.
  #[test]
  fn the_pre_restore_archive_includes_logged_edits() {
    let store = LocalStore::open_in_memory().unwrap();
    let cid = store.identity().unwrap().client_id;
    let mut doc = base_doc(cid);
    store.save_doc("local", "d", &doc).unwrap();
    let v = store
      .create_local_version("local", "d", "v")
      .unwrap()
      .unwrap();
    log_edit(&store, &mut doc);

    let restored = store
      .restore_local_version("local", "d", &v.id, cid)
      .unwrap()
      .unwrap();
    assert_eq!(text_of(&restored), "base");

    let archived: Vec<String> = store
      .list_local_versions("local", "d")
      .unwrap()
      .into_iter()
      .filter_map(|ver| store.local_version_state("local", "d", &ver.id).unwrap())
      .map(|bytes| text_of(&MicaDoc::from_update(&bytes).unwrap()))
      .collect();
    assert!(
      archived.iter().any(|t| t == "base logged"),
      "the logged edit is in no version: {archived:?}"
    );
  }
}

/// Pins the on-disk version and confirms a fresh store carries all five crc
/// columns. The version is bumped for a LAYOUT change (v7: `origin` in every
/// document-level key, issue #102); the crc columns themselves are additive
/// and cost no bump — the comment on `SCHEMA_VERSION` is the rule.
#[test]
fn schema_version_pinned_and_crc_columns_present() {
  assert_eq!(SCHEMA_VERSION, 7, "v7 = origin in every doc-level key");
  let store = LocalStore::open_in_memory().unwrap();
  for (table, col) in [
    ("doc_snapshot", "state_crc"),
    ("doc_version", "state_crc"),
    ("doc_snapshot_backup", "state_crc"),
    ("doc_update", "payload_crc"),
    ("doc_remote_update", "payload_crc"),
  ] {
    let has: i64 = store
      .conn
      .query_row(
        &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name='{col}'"),
        [],
        |r| r.get(0),
      )
      .unwrap();
    assert_eq!(has, 1, "{table}.{col} present in a fresh store");
  }
}

#[test]
fn views_survive_reopen() {
  let dir = std::env::temp_dir();
  let path = dir.join(format!("mica_views_test_{}.db", std::process::id()));
  let p = path.to_string_lossy().to_string();
  let _ = std::fs::remove_file(&p);
  {
    let s = LocalStore::open(&p).unwrap();
    s.save_view(&view("v1", None, "Persisted", "0000000010"))
      .unwrap();
  }
  {
    let s = LocalStore::open(&p).unwrap();
    let all = s.list_views("local").unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].name, "Persisted");
  }
  let _ = std::fs::remove_file(&p);
}

#[test]
fn identity_is_stable_across_reopen() {
  // A real file so identity must survive a fresh connection.
  let dir = std::env::temp_dir();
  let path = dir.join(format!("mica_store_test_{}.db", std::process::id()));
  let p = path.to_string_lossy().to_string();
  let _ = std::fs::remove_file(&p);

  let first = {
    let s = LocalStore::open(&p).unwrap();
    s.identity().unwrap()
  };
  let second = {
    let s = LocalStore::open(&p).unwrap();
    s.identity().unwrap()
  };
  assert_eq!(first, second, "device id + client id persist across reopen");
  assert_ne!(first.client_id, 0);

  let _ = std::fs::remove_file(&p);
}

fn body_doc(cid: u64, text: &str) -> MicaDoc {
  let (root, _) = sample();
  let blocks = vec![
    Block::new("r", "page").with_children(vec!["a".into()]),
    Block::new("a", "paragraph").with_text(text),
  ];
  MicaDoc::from_blocks_with_client_id(&root, &blocks, Some(cid))
}

fn origin_view(origin: &str, id: &str, doc: &str, name: &str) -> LocalView {
  LocalView {
    id: id.into(),
    workspace_id: "workspace-same".into(),
    parent_id: None,
    object_id: doc.into(),
    name: name.into(),
    position: "0000000010".into(),
    trashed: false,
    origin: origin.into(),
    object_type: "document".into(),
  }
}

/// Issue #102: two origins that ended up holding the same document UUID (a
/// restored backup, a migrated or cloned server) must not share one
/// document. Covered through the public store API on all four paths the
/// issue names — read, write, delete, sync cursor — plus the outbox and the
/// local version history, which ride the same rows.
#[test]
fn origins_do_not_share_document_state() {
  let store = LocalStore::open_in_memory().unwrap();
  let cid = store.identity().unwrap().client_id;
  let a = "https://server-a.example.invalid";
  let b = "https://server-b.example.invalid";
  let doc_id = "doc-same";
  store
    .save_view(&origin_view(a, "v-a", doc_id, "A title"))
    .unwrap();
  store
    .save_view(&origin_view(b, "v-b", doc_id, "B title"))
    .unwrap();

  // A: body, sync cursor, outbox edit, named version.
  let mut a_doc = body_doc(cid, "A private body");
  store.save_doc(a, doc_id, &a_doc).unwrap();
  store
    .set_sync_cursor(
      a,
      doc_id,
      SyncCursor {
        last_synced_rid: 120,
        pushed_clock: 8,
      },
    )
    .unwrap();
  let sv = a_doc.state_vector();
  a_doc.text_insert("a", "A private body".len() as u32, " edit");
  store
    .append_update(a, doc_id, &a_doc.encode_diff(&sv).unwrap())
    .unwrap();
  store
    .create_local_version(a, doc_id, "A checkpoint")
    .unwrap()
    .unwrap();

  // Nothing of A is visible through B.
  assert!(
    store.load_doc(b, doc_id, cid).unwrap().is_none(),
    "B must not read A's body"
  );
  assert_eq!(
    store.sync_cursor(b, doc_id).unwrap(),
    SyncCursor::default(),
    "B must not inherit A's sync cursor"
  );
  assert!(
    store.doc_updates(b, doc_id).unwrap().is_empty(),
    "B must not inherit A's offline outbox"
  );
  assert!(
    store.list_local_versions(b, doc_id).unwrap().is_empty(),
    "B must not inherit A's version history"
  );

  // B saves its own divergent body; A is untouched.
  store
    .save_doc(b, doc_id, &body_doc(cid, "B private body"))
    .unwrap();
  let paragraph = |origin: &str| {
    store
      .load_doc(origin, doc_id, cid)
      .unwrap()
      .unwrap()
      .to_blocks()
      .into_iter()
      .find(|x| x.id == "a")
      .unwrap()
      .text
  };
  assert_eq!(paragraph(a), "A private body edit", "A keeps its own text");
  assert_eq!(paragraph(b), "B private body", "B keeps its own text");
  assert_eq!(store.sync_cursor(a, doc_id).unwrap().last_synced_rid, 120);
  assert_eq!(store.sync_cursor(b, doc_id).unwrap(), SyncCursor::default());
  assert_eq!(store.doc_updates(a, doc_id).unwrap().len(), 1);
  assert!(store.doc_updates(b, doc_id).unwrap().is_empty());
  assert!(
    store
      .list_local_versions(a, doc_id)
      .unwrap()
      .iter()
      .any(|v| v.label.as_deref() == Some("A checkpoint")),
    "A's named version stays in A"
  );
  assert!(
    store
      .list_local_versions(b, doc_id)
      .unwrap()
      .iter()
      .all(|v| v.label.is_none()),
    "B has only the auto version its own save captured"
  );
  assert_eq!(
    store.list_docs(a).unwrap(),
    vec![doc_id.to_string()],
    "both origins list the id in their own namespace"
  );
  assert_eq!(store.list_docs(b).unwrap(), vec![doc_id.to_string()]);

  // Clearing A's mirror (the forgetOrigin path: views then documents) must
  // leave B whole.
  store.delete_doc(a, doc_id).unwrap();
  assert!(store.load_doc(a, doc_id, cid).unwrap().is_none());
  assert_eq!(store.sync_cursor(a, doc_id).unwrap(), SyncCursor::default());
  assert_eq!(
    paragraph(b),
    "B private body",
    "purging origin A must not delete B's document"
  );
  assert!(
    !store.list_local_versions(b, doc_id).unwrap().is_empty(),
    "nor B's history"
  );
  assert!(store.list_docs(a).unwrap().is_empty());
  assert_eq!(store.list_docs(b).unwrap(), vec![doc_id.to_string()]);
}

/// Issue #102, upgrade path: a pre-v7 store (bare-`doc_id` keys) opens with
/// every row readable, filed under the origin of the view that referenced
/// it — and an orphan document, which no view mentions, stays under
/// `'local'`, the only origin such a store could have had.
#[test]
fn legacy_documents_upgrade_to_their_views_origin() {
  let path = temp_path();
  let p = path.to_str().unwrap();
  let cloud = "https://mica.example";
  let cid = 7u64;
  let state = body_doc(cid, "legacy body").encode_state();
  let update = body_doc(cid, "x").encode_state();
  {
    let conn = Connection::open(p).unwrap();
    conn
      .execute_batch(
        "CREATE TABLE local_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
       INSERT INTO local_meta VALUES('schema_version','6');
       CREATE TABLE doc_snapshot(
           doc_id TEXT PRIMARY KEY, state BLOB NOT NULL, updated_at INTEGER NOT NULL);
       CREATE TABLE doc_update(
           doc_id TEXT NOT NULL, clock INTEGER NOT NULL, payload BLOB NOT NULL,
           PRIMARY KEY(doc_id, clock));
       CREATE TABLE doc_remote_update(
           doc_id TEXT NOT NULL, rid INTEGER NOT NULL, payload BLOB NOT NULL,
           PRIMARY KEY(doc_id, rid));
       CREATE TABLE doc_snapshot_backup(
           doc_id TEXT PRIMARY KEY, state BLOB NOT NULL, updated_at INTEGER NOT NULL);
       CREATE TABLE sync_cursor(
           doc_id TEXT PRIMARY KEY,
           last_synced_rid INTEGER NOT NULL DEFAULT 0,
           pushed_clock INTEGER NOT NULL DEFAULT 0);
       CREATE TABLE doc_version(
           id TEXT PRIMARY KEY, doc_id TEXT NOT NULL, label TEXT,
           created_at INTEGER NOT NULL, state BLOB NOT NULL);
       CREATE TABLE local_view(
           origin TEXT NOT NULL DEFAULT 'local', id TEXT NOT NULL,
           workspace_id TEXT NOT NULL DEFAULT 'local', parent_id TEXT,
           object_id TEXT NOT NULL, name TEXT NOT NULL, position TEXT NOT NULL,
           trashed INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(origin, id));
       CREATE TABLE local_workspace(
           origin TEXT NOT NULL DEFAULT 'local', id TEXT NOT NULL,
           name TEXT NOT NULL, position TEXT NOT NULL,
           role TEXT NOT NULL DEFAULT 'viewer', PRIMARY KEY(origin, id));",
      )
      .unwrap();
    for doc_id in ["d-local", "d-cloud", "d-orphan"] {
      conn
        .execute(
          "INSERT INTO doc_snapshot(doc_id,state,updated_at) VALUES(?1,?2,0)",
          params![doc_id, &state],
        )
        .unwrap();
    }
    conn
      .execute(
        "INSERT INTO local_view(id,object_id,name,position) VALUES('v1','d-local','Local page','0000000010')",
        [],
      )
      .unwrap();
    conn
      .execute(
        "INSERT INTO local_view(origin,id,object_id,name,position) VALUES(?1,'v2','d-cloud','Cloud page','0000000010')",
        params![cloud],
      )
      .unwrap();
    conn
      .execute(
        "INSERT INTO doc_update(doc_id,clock,payload) VALUES('d-cloud',1,?1)",
        params![&update],
      )
      .unwrap();
    conn
      .execute(
        "INSERT INTO sync_cursor(doc_id,last_synced_rid,pushed_clock) VALUES('d-cloud',42,1)",
        [],
      )
      .unwrap();
    conn
      .execute(
        "INSERT INTO doc_version(id,doc_id,label,created_at,state) VALUES('ver','d-cloud','named',0,?1)",
        params![&state],
      )
      .unwrap();
    conn
      .execute(
        "INSERT INTO doc_remote_update(doc_id,rid,payload) VALUES('d-cloud',9,?1)",
        params![&update],
      )
      .unwrap();
    conn
      .execute(
        "INSERT INTO doc_snapshot_backup(doc_id,state,updated_at) VALUES('d-cloud',?1,0)",
        params![&state],
      )
      .unwrap();
  }

  let store = LocalStore::open(p).unwrap();
  let text = |origin: &str, doc_id: &str| {
    store
      .load_doc(origin, doc_id, cid)
      .unwrap()
      .expect("the legacy document is still readable")
      .to_blocks()
      .into_iter()
      .find(|b| b.id == "a")
      .unwrap()
      .text
  };
  assert_eq!(text("local", "d-local"), "legacy body");
  assert_eq!(
    text(cloud, "d-cloud"),
    "legacy body",
    "filed under the origin of the view that referenced it"
  );
  assert_eq!(
    text("local", "d-orphan"),
    "legacy body",
    "an orphan document stays under 'local'"
  );
  // Logs, cursor and history moved with their document.
  assert_eq!(store.doc_updates(cloud, "d-cloud").unwrap().len(), 1);
  assert_eq!(
    store.sync_cursor(cloud, "d-cloud").unwrap(),
    SyncCursor {
      last_synced_rid: 42,
      pushed_clock: 1,
    }
  );
  assert_eq!(
    store
      .list_local_versions(cloud, "d-cloud")
      .unwrap()
      .iter()
      .map(|v| v.label.clone())
      .collect::<Vec<_>>(),
    vec![Some("named".to_string())]
  );
  assert!(store.doc_updates("local", "d-cloud").unwrap().is_empty());
  assert_eq!(
    store.sync_cursor("local", "d-cloud").unwrap(),
    SyncCursor::default()
  );
  assert!(store
    .list_local_versions("local", "d-cloud")
    .unwrap()
    .is_empty());
  // The two logs that do not have a reader yet must be carried over with
  // their payloads intact — a rebuild that loses them would silently break
  // the next sync (issue #102 review).
  assert_eq!(store.doc_remote_updates(cloud, "d-cloud").unwrap().len(), 1);
  assert_eq!(
    store.doc_remote_updates(cloud, "d-cloud").unwrap()[0].1,
    update,
    "remote payload byte-identical after the rebuild"
  );
  assert!(store
    .doc_remote_updates("local", "d-cloud")
    .unwrap()
    .is_empty());
  assert!(
    store.rollback_doc(cloud, "d-cloud", cid).unwrap().is_some(),
    "the checkpoint row moved with its document"
  );
  assert!(store
    .rollback_doc("local", "d-cloud", cid)
    .unwrap()
    .is_none());
  // Reopen is a no-op: the composite-key probe skips every rebuild.
  drop(store);
  let again = LocalStore::open(p).unwrap();
  assert_eq!(
    again.list_docs(cloud).unwrap(),
    vec!["d-cloud".to_string()],
    "the migrated row is stable across reopen"
  );
  drop(again);
  std::fs::remove_file(p).ok();
}

/// Issue #102, upgrade path with a document UUID shared by MORE THAN ONE
/// origin (a restored backup or a cloned server keeps ids). The pre-v7 store
/// has one row per id and cannot say which copy was "the" one, so the
/// migration must COPY it — body, backup, outbox, remote log, cursor and
/// version history — to EVERY origin whose page tree references it. Losing
/// one copy, or assigning the single copy to only one origin, would silently
/// drop another account's offline edits.
#[test]
fn legacy_same_uuid_across_origins_is_copied_to_each() {
  let path = temp_path();
  let p = path.to_str().unwrap();
  let cloud_a = "https://server-a.example";
  let cloud_b = "https://server-b.example";
  let shared = "doc-shared-uuid";
  let cid = 9u64;
  let state = body_doc(cid, "shared legacy body").encode_state();
  let update = body_doc(cid, "pending edit").encode_state();
  {
    let conn = Connection::open(p).unwrap();
    conn
      .execute_batch(
        "CREATE TABLE local_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
       INSERT INTO local_meta VALUES('schema_version','6');
       CREATE TABLE doc_snapshot(
           doc_id TEXT PRIMARY KEY, state BLOB NOT NULL, updated_at INTEGER NOT NULL);
       CREATE TABLE doc_update(
           doc_id TEXT NOT NULL, clock INTEGER NOT NULL, payload BLOB NOT NULL,
           PRIMARY KEY(doc_id, clock));
       CREATE TABLE doc_remote_update(
           doc_id TEXT NOT NULL, rid INTEGER NOT NULL, payload BLOB NOT NULL,
           PRIMARY KEY(doc_id, rid));
       CREATE TABLE doc_snapshot_backup(
           doc_id TEXT PRIMARY KEY, state BLOB NOT NULL, updated_at INTEGER NOT NULL);
       CREATE TABLE sync_cursor(
           doc_id TEXT PRIMARY KEY,
           last_synced_rid INTEGER NOT NULL DEFAULT 0,
           pushed_clock INTEGER NOT NULL DEFAULT 0);
       CREATE TABLE doc_version(
           id TEXT PRIMARY KEY, doc_id TEXT NOT NULL, label TEXT,
           created_at INTEGER NOT NULL, state BLOB NOT NULL);
       CREATE TABLE local_view(
           origin TEXT NOT NULL DEFAULT 'local', id TEXT NOT NULL,
           workspace_id TEXT NOT NULL DEFAULT 'local', parent_id TEXT,
           object_id TEXT NOT NULL, name TEXT NOT NULL, position TEXT NOT NULL,
           trashed INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(origin, id));
       CREATE TABLE local_workspace(
           origin TEXT NOT NULL DEFAULT 'local', id TEXT NOT NULL,
           name TEXT NOT NULL, position TEXT NOT NULL,
           role TEXT NOT NULL DEFAULT 'viewer', PRIMARY KEY(origin, id));",
      )
      .unwrap();
    // One legacy document row, referenced by a page in EACH origin.
    conn
      .execute(
        "INSERT INTO doc_snapshot(doc_id,state,updated_at) VALUES(?1,?2,0)",
        params![shared, &state],
      )
      .unwrap();
    conn
      .execute(
        "INSERT INTO doc_snapshot_backup(doc_id,state,updated_at) VALUES(?1,?2,0)",
        params![shared, &state],
      )
      .unwrap();
    conn
      .execute(
        "INSERT INTO doc_update(doc_id,clock,payload) VALUES(?1,1,?2)",
        params![shared, &update],
      )
      .unwrap();
    conn
      .execute(
        "INSERT INTO doc_remote_update(doc_id,rid,payload) VALUES(?1,77,?2)",
        params![shared, &update],
      )
      .unwrap();
    conn
      .execute(
        "INSERT INTO sync_cursor(doc_id,last_synced_rid,pushed_clock) VALUES(?1,77,1)",
        params![shared],
      )
      .unwrap();
    conn
      .execute(
        "INSERT INTO doc_version(id,doc_id,label,created_at,state) VALUES('v-shared',?1,'named',0,?2)",
        params![shared, &state],
      )
      .unwrap();
    for (origin, id) in [("local", "v-local"), (cloud_a, "v-a"), (cloud_b, "v-b")] {
      conn
        .execute(
          "INSERT INTO local_view(origin,id,object_id,name,position) VALUES(?1,?2,?3,'Shared','0000000010')",
          params![origin, id, shared],
        )
        .unwrap();
    }
  }

  let store = LocalStore::open(p).unwrap();
  let body = |origin: &str| {
    store
      .load_doc(origin, shared, cid)
      .unwrap()
      .expect("every origin keeps the shared document")
      .to_blocks()
      .into_iter()
      .find(|b| b.id == "a")
      .unwrap()
      .text
  };
  for origin in ["local", cloud_a, cloud_b] {
    assert_eq!(
      body(origin),
      "shared legacy body",
      "body copied to {origin}"
    );
    assert_eq!(
      store.doc_updates(origin, shared).unwrap().len(),
      1,
      "outbox copied to {origin}"
    );
    assert_eq!(
      store.doc_remote_updates(origin, shared).unwrap().len(),
      1,
      "remote log copied to {origin}"
    );
    assert_eq!(
      store.sync_cursor(origin, shared).unwrap(),
      SyncCursor {
        last_synced_rid: 77,
        pushed_clock: 1,
      },
      "cursor copied to {origin}"
    );
    assert_eq!(
      store
        .list_local_versions(origin, shared)
        .unwrap()
        .iter()
        .map(|v| v.label.clone())
        .collect::<Vec<_>>(),
      vec![Some("named".to_string())],
      "version history copied to {origin}"
    );
    assert!(
      store.rollback_doc(origin, shared, cid).unwrap().is_some(),
      "recovery checkpoint copied to {origin}"
    );
  }
  // Deleting one origin's mirror must leave the other copies untouched.
  store.delete_doc(cloud_a, shared).unwrap();
  assert!(store.load_doc(cloud_a, shared, cid).unwrap().is_none());
  assert_eq!(body("local"), "shared legacy body");
  assert_eq!(body(cloud_b), "shared legacy body");
  drop(store);
  std::fs::remove_file(p).ok();
}

/// Issue #104: an edit that only reached the incremental log changes
/// `load_doc` at once, but the search/link projections kept describing the
/// pre-edit page — so searching straight after typing found nothing until a
/// save or a compaction happened to rewrite them. Both readers must answer
/// from the CURRENT document, and compaction must not change the answer.
#[test]
fn an_appended_edit_is_visible_to_search_and_links_straight_away() {
  let store = LocalStore::open_in_memory().unwrap();
  let cid = store.identity().unwrap().client_id;
  for (doc_id, text) in [("doc-same", "baseline"), ("doc-target", "target page")] {
    store
      .save_doc("local", doc_id, &body_doc(cid, text))
      .unwrap();
  }
  for (view_id, doc_id, name) in [
    ("v1", "doc-same", "Search page"),
    ("v2", "doc-target", "Target page"),
  ] {
    store
      .save_view(&origin_view("local", view_id, doc_id, name))
      .unwrap();
  }
  let ws = "workspace-same";
  assert!(store
    .search_local(ws, "UNIQUE_TOKEN", 10)
    .unwrap()
    .is_empty());

  // The edit goes through the LOG only — the shape every local page has
  // between two compactions. It adds both the token and a link to v2.
  let mut working = store.load_doc("local", "doc-same", cid).unwrap().unwrap();
  let sv = working.state_vector();
  working.set_block_text(
    "a",
    "baseline UNIQUE_TOKEN",
    &[crate::marks::Mark {
      start: 9,
      end: 21,
      ty: "link".to_string(),
      href: Some("mica://page/v2".to_string()),
      title: None,
    }],
  );
  store
    .append_update("local", "doc-same", &working.encode_diff(&sv).unwrap())
    .unwrap();

  let hits = store.search_local(ws, "UNIQUE_TOKEN", 10).unwrap();
  assert_eq!(
    hits.len(),
    1,
    "the appended edit must be searchable at once"
  );
  assert_eq!(hits[0].view_id, "v1");
  assert!(
    store
      .backlinks_local(ws, "v2")
      .unwrap()
      .iter()
      .any(|b| b.view_id == "v1"),
    "the link added by the appended edit must reach the backlink index"
  );
  assert!(
    store
      .graph_local(ws)
      .unwrap()
      .edges
      .iter()
      .any(|e| e.source == "v1" && e.target == "v2"),
    "and the graph"
  );

  // Compaction is not a second source of truth: same answers afterwards.
  assert!(store.compact_local("local", "doc-same", cid).unwrap());
  assert_eq!(store.search_local(ws, "UNIQUE_TOKEN", 10).unwrap().len(), 1);
  assert!(store
    .backlinks_local(ws, "v2")
    .unwrap()
    .iter()
    .any(|b| b.view_id == "v1"));
  assert!(store
    .graph_local(ws)
    .unwrap()
    .edges
    .iter()
    .any(|e| e.source == "v1" && e.target == "v2"));
}

/// Review finding (#102/#104): the v7 rebuild must DERIVE `projection_stale`
/// from the legacy logs, not hard-write 0. A pre-v7 base whose log was never
/// folded describes yesterday's text; claiming it is current would leave the
/// appended edits invisible forever — exactly the bug #104 fixed.
#[test]
fn legacy_log_edits_are_searchable_right_after_the_upgrade() {
  let path = temp_path();
  let p = path.to_str().unwrap();
  let cid = 11u64;
  let base = body_doc(cid, "baseline");
  let mut edited = MicaDoc::from_update_with_client_id(&base.encode_state(), Some(cid)).unwrap();
  let sv = base.state_vector();
  edited.text_insert("a", 8, " UNIQUE_TOKEN");
  let diff = edited.encode_diff(&sv).unwrap();
  {
    let conn = Connection::open(p).unwrap();
    conn
      .execute_batch(
        "CREATE TABLE local_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
         INSERT INTO local_meta VALUES('schema_version','6');
         CREATE TABLE doc_snapshot(
             doc_id TEXT PRIMARY KEY, state BLOB NOT NULL,
             updated_at INTEGER NOT NULL, content_text TEXT);
         CREATE TABLE doc_update(
             doc_id TEXT NOT NULL, clock INTEGER NOT NULL, payload BLOB NOT NULL,
             PRIMARY KEY(doc_id, clock));
         CREATE TABLE doc_remote_update(
             doc_id TEXT NOT NULL, rid INTEGER NOT NULL, payload BLOB NOT NULL,
             PRIMARY KEY(doc_id, rid));
         CREATE TABLE doc_snapshot_backup(
             doc_id TEXT PRIMARY KEY, state BLOB NOT NULL, updated_at INTEGER NOT NULL);
         CREATE TABLE local_view(
             origin TEXT NOT NULL DEFAULT 'local', id TEXT NOT NULL,
             workspace_id TEXT NOT NULL DEFAULT 'local', parent_id TEXT,
             object_id TEXT NOT NULL, name TEXT NOT NULL, position TEXT NOT NULL,
             trashed INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(origin, id));
         CREATE TABLE local_workspace(
             origin TEXT NOT NULL DEFAULT 'local', id TEXT NOT NULL,
             name TEXT NOT NULL, position TEXT NOT NULL,
             role TEXT NOT NULL DEFAULT 'viewer', PRIMARY KEY(origin, id));",
      )
      .unwrap();
    conn
      .execute(
        "INSERT INTO doc_snapshot(doc_id,state,updated_at,content_text)
         VALUES('doc-log',?1,0,'baseline')",
        params![base.encode_state()],
      )
      .unwrap();
    conn
      .execute(
        "INSERT INTO doc_update(doc_id,clock,payload) VALUES('doc-log',1,?1)",
        params![diff],
      )
      .unwrap();
    conn
      .execute(
        "INSERT INTO local_view(id,workspace_id,object_id,name,position)
         VALUES('v1','workspace-same','doc-log','Log page','0000000010')",
        [],
      )
      .unwrap();
  }
  let store = LocalStore::open(p).unwrap();
  assert_eq!(
    store
      .search_local("workspace-same", "UNIQUE_TOKEN", 10)
      .unwrap()
      .len(),
    1,
    "an un-folded legacy log must be visible after the upgrade, not next save"
  );
  std::fs::remove_file(p).ok();
}

/// Review finding (#104): the backlink panel is opened on its own. It must
/// refresh stale projections too, not rely on a search having run first.
#[test]
fn a_logged_link_reaches_the_backlink_panel_without_a_search_first() {
  let store = LocalStore::open_in_memory().unwrap();
  let cid = store.identity().unwrap().client_id;
  for (doc_id, text) in [("doc-same", "baseline"), ("doc-target", "target")] {
    store
      .save_doc("local", doc_id, &body_doc(cid, text))
      .unwrap();
  }
  store
    .save_view(&origin_view("local", "v1", "doc-same", "Source"))
    .unwrap();
  store
    .save_view(&origin_view("local", "v2", "doc-target", "Target"))
    .unwrap();
  let mut working = store.load_doc("local", "doc-same", cid).unwrap().unwrap();
  let sv = working.state_vector();
  working.set_block_text(
    "a",
    "baseline see",
    &[crate::marks::Mark {
      start: 9,
      end: 12,
      ty: "link".to_string(),
      href: Some("mica://page/v2".to_string()),
      title: None,
    }],
  );
  store
    .append_update("local", "doc-same", &working.encode_diff(&sv).unwrap())
    .unwrap();

  // Deliberately NO search_local call before this one.
  assert!(
    store
      .backlinks_local("workspace-same", "v2")
      .unwrap()
      .iter()
      .any(|b| b.view_id == "v1"),
    "the panel must see a link that only exists in the log"
  );
}

/// Review finding (#104): a REMOTE append changes the document just as much
/// as a local one, and its consumer is the local world — it must mark the
/// projections stale too.
#[test]
fn a_remote_append_is_searchable_without_a_fold() {
  let store = LocalStore::open_in_memory().unwrap();
  let cid = store.identity().unwrap().client_id;
  let (root, blocks) = sample();
  let doc = MicaDoc::from_blocks_with_client_id(&root, &blocks, Some(cid));
  store.save_doc("local", "d", &doc).unwrap();
  store
    .save_view(&origin_view("local", "v1", "d", "Remote page"))
    .unwrap();
  let mut remote = MicaDoc::from_update_with_client_id(&doc.encode_state(), Some(999)).unwrap();
  let sv = remote.state_vector();
  remote.text_insert("a", 5, " UNIQUE_TOKEN");
  store
    .append_remote_updates("local", "d", &[(1, remote.encode_diff(&sv).unwrap())])
    .unwrap();
  assert_eq!(
    store
      .search_local("workspace-same", "UNIQUE_TOKEN", 10)
      .unwrap()
      .len(),
    1,
    "a remote update must be searchable before anything folds it"
  );
}

/// Review finding (#104): restore/rollback keep the REMOTE log (the server
/// owns those updates) but write projections from the restored blob alone.
/// `load_doc` replays the remote rows, so the index must not disagree.
#[test]
fn a_restore_keeps_the_index_in_step_with_the_remaining_remote_log() {
  let store = LocalStore::open_in_memory().unwrap();
  let cid = store.identity().unwrap().client_id;
  let (root, blocks) = sample();
  let doc = MicaDoc::from_blocks_with_client_id(&root, &blocks, Some(cid));
  store.save_doc("local", "d", &doc).unwrap();
  store
    .save_view(&origin_view("local", "v1", "d", "Restored page"))
    .unwrap();
  let v = store
    .create_local_version("local", "d", "pin")
    .unwrap()
    .unwrap();
  let mut remote = MicaDoc::from_update_with_client_id(&doc.encode_state(), Some(999)).unwrap();
  let sv = remote.state_vector();
  remote.text_insert("a", 5, " UNIQUE_TOKEN");
  store
    .append_remote_update("local", "d", 1, &remote.encode_diff(&sv).unwrap())
    .unwrap();

  store
    .restore_local_version("local", "d", &v.id, cid)
    .unwrap()
    .expect("the version exists");
  // The restore RETURNS the version's own state; `load_doc` is the reader
  // that replays the remote log which survived it — and the index must agree
  // with that reader, not with the blob the restore happened to write.
  let loaded = store.load_doc("local", "d", cid).unwrap().unwrap();
  assert!(
    loaded
      .to_blocks()
      .iter()
      .any(|b| b.text.contains("UNIQUE_TOKEN")),
    "load replays the remote log that survived the restore"
  );
  assert_eq!(
    store
      .search_local("workspace-same", "UNIQUE_TOKEN", 10)
      .unwrap()
      .len(),
    1,
    "and the index agrees with what load returns"
  );
}

/// Review finding (#102): the downgrade gate must run BEFORE any DDL or
/// PRAGMA. `PRAGMA journal_mode=WAL` alone rewrites the file, so refusing a
/// newer store afterwards still mutated it. Assert the schema AND the journal
/// mode are byte-for-byte what the newer app left behind.
#[test]
fn a_newer_store_is_refused_before_any_ddl_or_journal_change() {
  let path = temp_path();
  let p = path.to_str().unwrap();
  {
    let conn = Connection::open(p).unwrap();
    conn
      .execute_batch(
        "CREATE TABLE local_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
         INSERT INTO local_meta VALUES('schema_version','8');
         CREATE TABLE future_only(id TEXT PRIMARY KEY, payload BLOB);",
      )
      .unwrap();
  }
  let observe = || {
    let conn = Connection::open(p).unwrap();
    let journal: String = conn
      .query_row("PRAGMA journal_mode", [], |r| r.get(0))
      .unwrap();
    let names: Vec<String> = conn
      .prepare("SELECT name FROM sqlite_master ORDER BY name")
      .unwrap()
      .query_map([], |r| r.get::<_, String>(0))
      .unwrap()
      .collect::<Result<_, _>>()
      .unwrap();
    (journal, names)
  };
  let before = observe();
  assert!(matches!(
    LocalStore::open(p),
    Err(StoreError::SchemaTooNew { found: 8, .. })
  ));
  let after = observe();
  assert_eq!(after.0, before.0, "journal mode must not be switched");
  assert_eq!(
    after.1, before.1,
    "a refused store must gain no tables or indexes"
  );
  std::fs::remove_file(p).ok();
}
