//! Unified client-side search & backlink graph helpers built over `mica-core`
//! and `mica-board` (zero duplication of snippet folding or board text extraction).

use std::collections::{HashMap, HashSet};

use mica_board::extract_board_text_json;
use mica_core::{Block, page_link_targets, search_text::snippet};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchHit {
  pub document_id: Uuid,
  pub title: String,
  pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BacklinkEdge {
  pub from_document_id: Uuid,
  pub to_document_id: Uuid,
}

/// Extract full searchable plain text from a block list, including `mica-board`
/// JSON code blocks via `mica_board::extract_board_text_json`.
pub fn extract_document_search_text(blocks: &[Block]) -> String {
  let mut parts = Vec::with_capacity(blocks.len());
  for b in blocks {
    if b.kind == "code"
      && b
        .data
        .get("language")
        .and_then(|v| v.as_str())
        .is_some_and(|l| l.eq_ignore_ascii_case("mica-board"))
    {
      let board_text = extract_board_text_json(&b.text);
      if !board_text.is_empty() {
        parts.push(board_text);
      }
      continue;
    }
    if !b.text.is_empty() {
      parts.push(b.text.clone());
    }
  }
  parts.join("\n")
}

/// Search an in-memory map of `(document_id, title, blocks)` using the exact
/// same `mica_core::search_text::snippet` algorithm as the server and local SQLite store.
pub fn search_cached_documents(
  docs: &[(Uuid, String, Vec<Block>)],
  query: &str,
  limit: usize,
) -> Vec<SearchHit> {
  let needle = query.trim();
  if needle.is_empty() || limit == 0 {
    return Vec::new();
  }
  let mut hits = Vec::new();
  for (doc_id, title, blocks) in docs {
    let body = extract_document_search_text(blocks);
    let matched = snippet(title, needle).or_else(|| snippet(&body, needle));
    if let Some(snip) = matched {
      hits.push(SearchHit {
        document_id: *doc_id,
        title: title.clone(),
        snippet: snip,
      });
      if hits.len() >= limit {
        break;
      }
    }
  }
  hits
}

/// Compute deduplicated page-link graph edges from cached documents.
pub fn build_backlink_graph(docs: &[(Uuid, Vec<Block>)]) -> Vec<BacklinkEdge> {
  let mut seen: HashMap<Uuid, HashSet<Uuid>> = HashMap::new();
  let mut edges = Vec::new();
  for (from_id, blocks) in docs {
    for block in blocks {
      for target in page_link_targets(&block.data) {
        if let Ok(to_id) = Uuid::parse_str(&target)
          && to_id != *from_id
          && seen.entry(*from_id).or_default().insert(to_id)
        {
          edges.push(BacklinkEdge {
            from_document_id: *from_id,
            to_document_id: to_id,
          });
        }
      }
    }
  }
  edges
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn searches_plain_text_and_embedded_mica_board() {
    let board_json = mica_board::starter_board_json("mindmap");
    let doc_id = Uuid::new_v4();
    let blocks = vec![
      Block {
        id: "b1".into(),
        kind: "paragraph".into(),
        text: "Hello Rust architecture".into(),
        data: json!({}),
        children: vec![],
      },
      Block {
        id: "b2".into(),
        kind: "code".into(),
        text: board_json,
        data: json!({ "language": "mica-board" }),
        children: vec![],
      },
    ];
    let text = extract_document_search_text(&blocks);
    assert!(text.contains("Hello Rust architecture"));
    let hits = search_cached_documents(&[(doc_id, "Doc".into(), blocks)], "rust", 10);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].document_id, doc_id);
  }
}
