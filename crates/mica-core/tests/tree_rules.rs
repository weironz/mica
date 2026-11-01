//! Replays `contracts/tree-rules.json` against the LOCAL store (SQLite). The
//! server replays the same file against Postgres
//! (`crates/api-server/src/routes/documents/tree_rules_pg_tests.rs`), so a
//! page-tree rule changed on one side only fails the other side's replay.
#![cfg(feature = "store")]

use std::collections::BTreeMap;

use mica_core::tree_rules::ParentRefusal;
use mica_core::{LocalStore, LocalView, StoreError};
use serde_json::Value;

const ORIGIN: &str = "local";

fn scenarios() -> Vec<Value> {
  let path = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../contracts/tree-rules.json"
  );
  let file: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
  file["scenarios"].as_array().unwrap().clone()
}

fn refusal_code(err: &StoreError) -> Option<&'static str> {
  let StoreError::InvalidParent { reason, .. } = err else {
    return None;
  };
  ParentRefusal::ALL
    .iter()
    .find(|r| r.reason() == *reason)
    .map(|r| r.code())
}

#[test]
fn the_local_store_follows_the_shared_tree_rules() {
  let all = scenarios();
  assert!(
    all.len() >= 7,
    "the scenario file lost cases: {}",
    all.len()
  );
  for sc in all {
    let name = sc["name"].as_str().unwrap();
    let dir = std::env::temp_dir().join(format!("mica_tree_rules_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = LocalStore::open(dir.join("s.db").to_str().unwrap()).unwrap();

    for (i, v) in sc["tree"].as_array().unwrap().iter().enumerate() {
      let id = v["id"].as_str().unwrap();
      store
        .save_view(&LocalView {
          id: id.into(),
          workspace_id: "ws".into(),
          parent_id: v["parent"].as_str().map(str::to_string),
          object_id: format!("doc_{id}"),
          name: id.into(),
          position: format!("{:04}", (i + 1) * 10),
          trashed: false,
          origin: ORIGIN.into(),
          object_type: if v["kind"] == "folder" {
            "folder"
          } else {
            "document"
          }
          .into(),
        })
        .unwrap_or_else(|e| panic!("{name}: seeding {id}: {e}"));
    }

    for op in sc["ops"].as_array().unwrap() {
      let id = op["id"].as_str().unwrap();
      match op["op"].as_str().unwrap() {
        "trash" => {
          store.trash_view_subtree(ORIGIN, id).unwrap();
        }
        "restore" => {
          store.restore_view_subtree(ORIGIN, id).unwrap();
        }
        "move" => {
          let parent = op["parent"].as_str();
          let got = store.reorder_views(ORIGIN, parent, &[id.to_string()]);
          match op["refused"].as_str() {
            Some(code) => {
              let err = got.expect_err(&format!("{name}: move {id} must be refused"));
              assert_eq!(refusal_code(&err), Some(code), "{name}: {err}");
            }
            None => got.unwrap_or_else(|e| panic!("{name}: move {id}: {e}")),
          }
        }
        other => panic!("{name}: unknown op {other}"),
      }
    }

    let views = store.list_views(ORIGIN).unwrap();
    let live: BTreeMap<String, Option<String>> = views
      .iter()
      .filter(|v| !v.trashed)
      .map(|v| (v.id.clone(), v.parent_id.clone()))
      .collect();
    let mut trashed: Vec<String> = views
      .iter()
      .filter(|v| v.trashed)
      .map(|v| v.id.clone())
      .collect();
    trashed.sort();
    let want_live: BTreeMap<String, Option<String>> = sc["expect"]["live"]
      .as_object()
      .unwrap()
      .iter()
      .map(|(k, v)| (k.clone(), v.as_str().map(str::to_string)))
      .collect();
    let mut want_trashed: Vec<String> = sc["expect"]["trashed"]
      .as_array()
      .unwrap()
      .iter()
      .map(|v| v.as_str().unwrap().to_string())
      .collect();
    want_trashed.sort();
    assert_eq!(live, want_live, "{name}: live views and parents");
    assert_eq!(trashed, want_trashed, "{name}: trashed views");
    let _ = std::fs::remove_dir_all(&dir);
  }
}
