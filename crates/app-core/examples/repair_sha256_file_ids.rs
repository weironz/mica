//! One-off repair: rewrite image blocks whose `data.file_id` holds a 64-hex
//! local CAS SHA-256 placeholder back to the matching `files.id` UUID in the
//! same workspace.
//!
//! ```text
//! DATABASE_URL=postgres://... cargo run -p mica-app-core --example repair_sha256_file_ids
//! DATABASE_URL=postgres://... cargo run -p mica-app-core --example repair_sha256_file_ids -- --apply
//! ```
//!
//! Goes through [`mica_app_core::store::apply_derived_operations`] so the fix
//! runs inside the document's `FOR UPDATE` lock, preserves full Yrs CRDT
//! history (clients holding older replicas converge cleanly on reconnect),
//! updates `document_yrs_base.file_ids`, and advances `documents.current_seq`.

use mica_app_core::{documents::DocumentOperation, store, sync::SyncTuning};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

fn is_sha256_hex(s: &str) -> bool {
  s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
  let apply = std::env::args().any(|a| a == "--apply");
  let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
  let pool = PgPoolOptions::new()
    .max_connections(4)
    .connect(&url)
    .await?;

  let candidates: Vec<(Uuid, Uuid, Uuid, String)> = sqlx::query_as(
    r#"
      SELECT DISTINCT b.document_id, d.workspace_id, d.created_by, fid
      FROM document_yrs_base b
      JOIN documents d ON d.id = b.document_id
      CROSS JOIN LATERAL unnest(b.file_ids) AS fid
      WHERE fid ~ '^[0-9a-f]{64}$'
      ORDER BY b.document_id, fid
    "#,
  )
  .fetch_all(&pool)
  .await?;

  if candidates.is_empty() {
    println!("0 documents with 64-hex sha256 file_ids.");
    return Ok(());
  }

  let mut by_doc: std::collections::BTreeMap<(Uuid, Uuid, Uuid), Vec<(String, Uuid)>> =
    std::collections::BTreeMap::new();

  for (doc_id, workspace_id, actor_id, sha) in candidates {
    let prefix = format!("workspaces/{workspace_id}/{sha}.%");
    let exact = format!("workspaces/{workspace_id}/{sha}");
    let matched: Vec<(Uuid, String)> = sqlx::query_as(
      "SELECT id, object_key FROM files \
       WHERE workspace_id = $1 AND (object_key LIKE $2 OR object_key = $3)",
    )
    .bind(workspace_id)
    .bind(&prefix)
    .bind(&exact)
    .fetch_all(&pool)
    .await?;

    if matched.len() != 1 {
      eprintln!(
        "SKIP doc={doc_id} ws={workspace_id} sha={sha}: expected 1 files row, found {} ({matched:?})",
        matched.len()
      );
      continue;
    }
    let (file_uuid, object_key) = &matched[0];
    println!("MATCH doc={doc_id} ws={workspace_id} sha={sha} -> uuid={file_uuid} ({object_key})");
    by_doc
      .entry((doc_id, workspace_id, actor_id))
      .or_default()
      .push((sha, *file_uuid));
  }

  let tuning = SyncTuning::default();
  let mut docs_fixed = 0usize;
  let mut blocks_fixed = 0usize;

  for ((doc_id, workspace_id, actor_id), rewrites) in by_doc {
    let map: std::collections::HashMap<String, String> = rewrites
      .into_iter()
      .map(|(sha, uuid)| (sha, uuid.to_string()))
      .collect();

    if !apply {
      let Some(payload) = store::current_payload(&pool, doc_id).await? else {
        eprintln!("SKIP doc={doc_id}: no current payload");
        continue;
      };
      let hits: Vec<String> = payload
        .blocks
        .iter()
        .filter(|b| {
          b.kind == "image"
            && b
              .data
              .get("file_id")
              .and_then(|v| v.as_str())
              .is_some_and(|fid| is_sha256_hex(fid) && map.contains_key(fid))
        })
        .map(|b| format!("block={} data={}", b.id, b.data))
        .collect();
      println!(
        "DRY-RUN doc={doc_id}: {} image block(s) would rewrite:",
        hits.len()
      );
      for h in &hits {
        println!("  {h}");
      }
      docs_fixed += usize::from(!hits.is_empty());
      blocks_fixed += hits.len();
      continue;
    }

    let mut doc_blocks = 0usize;
    let applied = store::apply_derived_operations(
      &pool,
      workspace_id,
      doc_id,
      actor_id,
      None,
      &tuning,
      |payload| {
        let mut ops = Vec::new();
        for block in &payload.blocks {
          if block.kind != "image" {
            continue;
          }
          let Some(fid) = block.data.get("file_id").and_then(|v| v.as_str()) else {
            continue;
          };
          let Some(uuid) = map.get(fid) else {
            continue;
          };
          let mut next_data = block.data.clone();
          if let Some(obj) = next_data.as_object_mut() {
            obj.insert(
              "file_id".to_string(),
              serde_json::Value::String(uuid.clone()),
            );
          }
          ops.push(DocumentOperation::UpdateBlock {
            block_id: block.id.clone(),
            kind: None,
            text: None,
            data: Some(next_data),
          });
        }
        doc_blocks = ops.len();
        Ok(ops)
      },
    )
    .await?;

    println!(
      "APPLIED doc={doc_id}: rewrote {doc_blocks} block(s), new seq={}",
      applied.document.current_seq
    );
    docs_fixed += usize::from(doc_blocks > 0);
    blocks_fixed += doc_blocks;
  }

  println!(
    "\nSummary: {docs_fixed} document(s), {blocks_fixed} block(s) {}",
    if apply {
      "REPAIRED"
    } else {
      "would change (dry run; pass --apply)"
    }
  );
  Ok(())
}
