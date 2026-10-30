//! One-off repair: pages whose body still opens with a hand-written `## 页名`.
//!
//! Since 0.13.30 the page title lives in the document and every export writes
//! `# 页名` above the body. `with_page_title` skips that only when the first
//! line is exactly `# 页名`, so a page whose author had typed the title as a
//! level-2 heading exports as `# 页名` + `## 页名`. New pages do not need a
//! hand-written title; this removes it from existing ones.
//!
//! ```text
//! DATABASE_URL=postgres://... cargo run -p mica-app-core --example repair_duplicate_title
//! DATABASE_URL=postgres://... cargo run -p mica-app-core --example repair_duplicate_title -- --apply
//! ```
//!
//! Dry run by default: it prints how many pages would change and a sample.
//! Running `--apply` is a content rewrite and needs the owner's go-ahead.
//!
//! **What counts, and nothing wider.** The FIRST child of the page root is a
//! level-2 heading whose text equals the page title exactly (after trimming),
//! with no inline marks and no children. A `## 页名` further down, a
//! `### 页名`, or one with a link/bold in it is the author's content and is
//! left alone, the same rule `a_same_named_heading_at_another_level_is_not_treated_as_the_title`
//! pins for the export. A heading that a comment thread is anchored to is
//! skipped and listed, because deleting it would orphan the thread.
//!
//! **How it writes.** Not by overwriting the stored state: the deletion is a
//! yrs diff on the current base, pushed through `sync::push_update` like any
//! server-side edit. That takes the document lock, folds into the base and
//! appends to the update stream, so editors converge on it when they next
//! catch up (an editor open during the run sees it after it reconnects).
//!
//! An example, not a bin: it runs once and has no place in a shipped binary.

use mica_app_core::sync::{bootstrap_base, guarded_from_update, push_update};
use mica_core::{Block, marks_from_data};
use mica_infra::SyncTuning;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

/// The block to delete, if this document opens with a duplicate of `title`.
fn duplicate_title_block<'a>(blocks: &'a [Block], root_id: &str, title: &str) -> Option<&'a Block> {
  let title = title.trim();
  if title.is_empty() {
    return None;
  }
  let root = blocks.iter().find(|b| b.id == root_id)?;
  let first_id = root.children.first()?;
  let first = blocks.iter().find(|b| &b.id == first_id)?;
  let level = first.data.get("level").and_then(|v| v.as_u64());
  (first.kind == "heading"
    && level == Some(2)
    && first.text.trim() == title
    && first.children.is_empty()
    && marks_from_data(&first.data).is_empty())
  .then_some(first)
}

/// The page title as the export sees it: the document's own title, else the
/// view name (pages from before the title moved into the document).
fn page_title(blocks: &[Block], root_id: &str, view_name: &str) -> String {
  blocks
    .iter()
    .find(|b| b.id == root_id)
    .and_then(|root| root.data.get("title"))
    .and_then(|t| t.as_str())
    .map(str::trim)
    .filter(|t| !t.is_empty())
    .unwrap_or(view_name)
    .to_string()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
  let apply = std::env::args().any(|a| a == "--apply");
  let url = std::env::var("DATABASE_URL").map_err(|_| "set DATABASE_URL")?;
  let pool = PgPoolOptions::new()
    .max_connections(4)
    .connect(&url)
    .await?;

  // Prefilter in SQL: `content_text` starts with the block texts in tree
  // order (the root's title is not in it), so a page whose first line is not
  // its own name cannot qualify. The Rust check below is the real test.
  let rows: Vec<(Uuid, Uuid, String, Uuid)> = sqlx::query_as(
    "SELECT v.object_id, v.workspace_id, v.name, w.owner_id
       FROM views v
       JOIN workspaces w ON w.id = v.workspace_id
       JOIN document_yrs_base b ON b.document_id = v.object_id
      WHERE v.object_type = 'document' AND NOT v.is_deleted
        AND btrim(split_part(b.content_text, E'\\n', 1)) = btrim(v.name)
      ORDER BY v.workspace_id, v.object_id",
  )
  .fetch_all(&pool)
  .await?;
  println!("candidate pages: {}", rows.len());

  let tuning = SyncTuning::default();
  let mut changed = 0usize;
  let mut samples: Vec<String> = Vec::new();
  let mut skipped: Vec<String> = Vec::new();

  for (document_id, workspace_id, view_name, owner_id) in rows {
    let base = bootstrap_base(&pool, document_id).await?;
    let mut doc = match guarded_from_update(&base.state) {
      Ok(doc) => doc,
      Err(e) => {
        skipped.push(format!("{document_id}: undecodable base ({e})"));
        continue;
      }
    };
    let root_id = doc.root_block_id();
    let blocks = doc.to_blocks();
    let title = page_title(&blocks, &root_id, &view_name);
    let Some(block) = duplicate_title_block(&blocks, &root_id, &title) else {
      continue;
    };
    let block_id = block.id.clone();

    let anchored: i64 = sqlx::query_scalar(
      "SELECT count(*) FROM comment_threads
        WHERE document_id = $1 AND (anchor_start_block = $2 OR anchor_end_block = $2)",
    )
    .bind(document_id)
    .bind(&block_id)
    .fetch_one(&pool)
    .await?;
    if anchored > 0 {
      skipped.push(format!(
        "{document_id} ({title}): {anchored} comment thread(s) anchored to the heading"
      ));
      continue;
    }

    changed += 1;
    if samples.len() < 10 {
      samples.push(format!("  {workspace_id} / {document_id}: ## {title}"));
    }
    if !apply {
      continue;
    }
    let sv = doc.state_vector();
    doc.delete_block(&block_id, false);
    let update = doc
      .encode_diff(&sv)
      .map_err(|e| format!("{document_id}: diff failed ({e})"))?;
    if update.is_empty() {
      continue;
    }
    // Attributed to the workspace owner: the stream needs an actor, and the
    // owner is the one who approved the sweep.
    push_update(&pool, workspace_id, document_id, owner_id, &update, &tuning).await?;
  }

  if !samples.is_empty() {
    println!("\nsamples:");
    for s in &samples {
      println!("{s}");
    }
  }
  if !skipped.is_empty() {
    println!("\nskipped ({}):", skipped.len());
    for s in skipped.iter().take(25) {
      println!("  {s}");
    }
  }
  println!(
    "\n{changed} pages {}",
    if apply {
      "REPAIRED"
    } else {
      "would change (dry run; pass --apply)"
    }
  );
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn block(id: &str, kind: &str, text: &str, data: serde_json::Value, children: &[&str]) -> Block {
    Block {
      id: id.to_string(),
      kind: kind.to_string(),
      text: text.to_string(),
      data,
      children: children.iter().map(|c| c.to_string()).collect(),
    }
  }

  fn page(first: Block) -> Vec<Block> {
    vec![
      block(
        "r",
        "page",
        "",
        json!({ "title": "我的页" }),
        &[&first.id.clone(), "p"],
      ),
      first,
      block("p", "paragraph", "正文。", json!(null), &[]),
    ]
  }

  #[test]
  fn an_exact_level_two_title_at_the_top_is_found() {
    let blocks = page(block("h", "heading", "我的页", json!({ "level": 2 }), &[]));
    let title = page_title(&blocks, "r", "旧名");
    assert_eq!(title, "我的页");
    assert_eq!(
      duplicate_title_block(&blocks, "r", &title).map(|b| b.id.as_str()),
      Some("h")
    );
  }

  #[test]
  fn anything_else_is_the_authors_content() {
    for first in [
      block("h", "heading", "我的页", json!({ "level": 3 }), &[]),
      block("h", "heading", "我的页 v2", json!({ "level": 2 }), &[]),
      block("h", "paragraph", "我的页", json!(null), &[]),
      block(
        "h",
        "heading",
        "我的页",
        json!({ "level": 2, "marks": [{ "type": "bold", "start": 0, "end": 3 }] }),
        &[],
      ),
    ] {
      let blocks = page(first);
      assert!(
        duplicate_title_block(&blocks, "r", "我的页").is_none(),
        "{:?}",
        blocks[1]
      );
    }
  }

  #[test]
  fn a_matching_heading_further_down_is_left_alone() {
    let blocks = vec![
      block("r", "page", "", json!({ "title": "我的页" }), &["p", "h"]),
      block("p", "paragraph", "开头。", json!(null), &[]),
      block("h", "heading", "我的页", json!({ "level": 2 }), &[]),
    ];
    assert!(duplicate_title_block(&blocks, "r", "我的页").is_none());
  }

  #[test]
  fn without_a_document_title_the_view_name_is_the_title() {
    let mut blocks = page(block("h", "heading", "旧名", json!({ "level": 2 }), &[]));
    blocks[0].data = json!(null);
    let title = page_title(&blocks, "r", "旧名");
    assert_eq!(title, "旧名");
    assert!(duplicate_title_block(&blocks, "r", &title).is_some());
  }
}
