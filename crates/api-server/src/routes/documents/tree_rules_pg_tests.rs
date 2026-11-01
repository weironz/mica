//! Replays `contracts/tree-rules.json` against the SERVER (Postgres), through
//! the same functions the batch endpoints call. The local store replays the
//! same file against SQLite (`crates/mica-core/tests/tree_rules.rs`), so a
//! page-tree rule changed on one side only fails the other side's replay.

use std::collections::{BTreeMap, HashMap};

use mica_core::tree_rules::ParentRefusal;
use mica_infra::ApiError;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use super::transfer::move_views_tx;
use super::trash::{restore_views_batch, trash_views_batch};

fn scenarios() -> Vec<Value> {
  let path = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../contracts/tree-rules.json"
  );
  let file: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
  file["scenarios"].as_array().unwrap().clone()
}

fn refusal_code(err: &ApiError) -> Option<&'static str> {
  match err {
    ApiError::NotFound => Some(ParentRefusal::NotFound.code()),
    ApiError::BadRequest(msg) => ParentRefusal::ALL
      .iter()
      .find(|r| *msg == format!("parent_view_id: {}", r.reason()))
      .map(|r| r.code()),
    _ => None,
  }
}

async fn seed_workspace(db: &PgPool) -> (Uuid, Uuid) {
  let user = Uuid::new_v4();
  let ws = Uuid::new_v4();
  sqlx::query("INSERT INTO users(id,email,display_name,password_hash) VALUES($1,$2,'T','x')")
    .bind(user)
    .bind(format!("{user}@tree-rules.test"))
    .execute(db)
    .await
    .unwrap();
  sqlx::query("INSERT INTO workspaces(id,name,owner_id) VALUES($1,'W',$2)")
    .bind(ws)
    .bind(user)
    .execute(db)
    .await
    .unwrap();
  (ws, user)
}

#[tokio::test]
async fn the_server_follows_the_shared_tree_rules() {
  let Some(url) = mica_infra::test_database_url() else {
    return;
  };
  let db = PgPool::connect(&url)
    .await
    .expect("DATABASE_URL is set but the connection failed");
  let all = scenarios();
  assert!(
    all.len() >= 7,
    "the scenario file lost cases: {}",
    all.len()
  );

  for sc in all {
    let name = sc["name"].as_str().unwrap();
    let (ws, user) = seed_workspace(&db).await;
    let mut ids: HashMap<String, Uuid> = HashMap::new();
    for (i, v) in sc["tree"].as_array().unwrap().iter().enumerate() {
      let key = v["id"].as_str().unwrap().to_string();
      let id = Uuid::new_v4();
      let parent = v["parent"].as_str().map(|p| ids[p]);
      let kind = if v["kind"] == "folder" {
        "folder"
      } else {
        "document"
      };
      sqlx::query(
        "INSERT INTO views(id,workspace_id,parent_view_id,object_id,object_type,name,position,created_by) \
         VALUES($1,$2,$3,$4,$5::object_type,$6,$7,$8)",
      )
      .bind(id)
      .bind(ws)
      .bind(parent)
      .bind(Uuid::new_v4())
      .bind(kind)
      .bind(&key)
      .bind(format!("{:04}", (i + 1) * 10))
      .bind(user)
      .execute(&db)
      .await
      .unwrap_or_else(|e| panic!("{name}: seeding {key}: {e}"));
      ids.insert(key, id);
    }

    for op in sc["ops"].as_array().unwrap() {
      let id = ids[op["id"].as_str().unwrap()];
      match op["op"].as_str().unwrap() {
        "trash" => {
          trash_views_batch(&db, ws, &[id]).await.unwrap();
        }
        "restore" => {
          restore_views_batch(&db, ws, &[id]).await.unwrap();
        }
        "move" => {
          let parent = op["parent"].as_str().map(|p| ids[p]);
          let got = move_views_tx(&db, ws, &[id], parent).await;
          match op["refused"].as_str() {
            Some(code) => {
              let err = got.expect_err(&format!("{name}: move must be refused"));
              assert_eq!(refusal_code(&err), Some(code), "{name}: {err:?}");
            }
            None => got.unwrap_or_else(|e| panic!("{name}: move: {e:?}")),
          }
        }
        other => panic!("{name}: unknown op {other}"),
      }
    }

    let by_id: HashMap<Uuid, String> = ids.iter().map(|(k, v)| (*v, k.clone())).collect();
    let rows: Vec<(Uuid, Option<Uuid>, bool)> =
      sqlx::query_as("SELECT id, parent_view_id, is_deleted FROM views WHERE workspace_id = $1")
        .bind(ws)
        .fetch_all(&db)
        .await
        .unwrap();
    let live: BTreeMap<String, Option<String>> = rows
      .iter()
      .filter(|r| !r.2)
      .map(|r| (by_id[&r.0].clone(), r.1.map(|p| by_id[&p].clone())))
      .collect();
    let mut trashed: Vec<String> = rows
      .iter()
      .filter(|r| r.2)
      .map(|r| by_id[&r.0].clone())
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
  }
}
