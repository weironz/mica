//! Document operations over the block model (insert/update/delete/move).
//! The Markdown engine itself — the model types, parsing and rendering —
//! lives in `mica-markdown` and is re-exported here for compatibility.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use mica_markdown::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DocumentOperation {
  InsertBlock {
    block: Block,
    parent_id: String,
    index: Option<usize>,
  },
  UpdateBlock {
    block_id: String,
    kind: Option<String>,
    text: Option<String>,
    #[serde(default)]
    data: Option<Value>,
  },
  DeleteBlock {
    block_id: String,
  },
  MoveBlock {
    block_id: String,
    parent_id: String,
    index: Option<usize>,
  },
}


pub fn apply_operations(
  mut snapshot: DocumentSnapshotPayload,
  operations: &[DocumentOperation],
) -> DocumentOperationResult<DocumentSnapshotPayload> {
  if snapshot.schema_version != 1 {
    return Err(DocumentOperationError::UnsupportedSchemaVersion(
      snapshot.schema_version,
    ));
  }

  for operation in operations {
    apply_operation(&mut snapshot, operation)?;
  }

  Ok(snapshot)
}

fn apply_operation(
  snapshot: &mut DocumentSnapshotPayload,
  operation: &DocumentOperation,
) -> DocumentOperationResult<()> {
  match operation {
    DocumentOperation::InsertBlock {
      block,
      parent_id,
      index,
    } => insert_block(snapshot, block.clone(), parent_id, *index),
    DocumentOperation::UpdateBlock {
      block_id,
      kind,
      text,
      data,
    } => update_block(
      snapshot,
      block_id,
      kind.as_deref(),
      text.as_deref(),
      data.as_ref(),
    ),
    DocumentOperation::DeleteBlock { block_id } => delete_block(snapshot, block_id),
    DocumentOperation::MoveBlock {
      block_id,
      parent_id,
      index,
    } => move_block(snapshot, block_id, parent_id, *index),
  }
}


fn insert_block(
  snapshot: &mut DocumentSnapshotPayload,
  block: Block,
  parent_id: &str,
  index: Option<usize>,
) -> DocumentOperationResult<()> {
  validate_block(&block)?;

  // A newly inserted block is ALWAYS a leaf — both clients send `children: []`
  // (see controller.dart `_insertOp` and workspace_migration.dart). Accepting
  // caller-supplied children is how a crafted op forged a cycle (`{id:X,
  // children:[root]}` → delete_block infinite-loops) or a second parent (a child
  // already owned elsewhere → its subtree silently vanishes on read). Children
  // are attached by their own inserts/moves; refuse them here.
  // (docs/code-review-2026-07-20.md P0-2 / P0-3.)
  if !block.children.is_empty() {
    return Err(DocumentOperationError::InsertBlockWithChildren);
  }

  if block_index(snapshot, &block.id).is_some() {
    return Err(DocumentOperationError::BlockAlreadyExists(block.id));
  }

  let parent_index = block_index(snapshot, parent_id)
    .ok_or_else(|| DocumentOperationError::BlockNotFound(parent_id.to_string()))?;
  let block_id = block.id.clone();
  snapshot.blocks.push(block);
  insert_child(&mut snapshot.blocks[parent_index].children, block_id, index);
  Ok(())
}

fn update_block(
  snapshot: &mut DocumentSnapshotPayload,
  block_id: &str,
  kind: Option<&str>,
  text: Option<&str>,
  data: Option<&Value>,
) -> DocumentOperationResult<()> {
  let index = block_index(snapshot, block_id)
    .ok_or_else(|| DocumentOperationError::BlockNotFound(block_id.to_string()))?;

  if let Some(kind) = kind {
    let kind = kind.trim();
    if kind.is_empty() {
      return Err(DocumentOperationError::EmptyBlockType);
    }
    snapshot.blocks[index].kind = kind.to_string();
  }

  if let Some(text) = text {
    snapshot.blocks[index].text = text.to_string();
  }

  if let Some(data) = data {
    snapshot.blocks[index].data = data.clone();
  }

  Ok(())
}

fn delete_block(
  snapshot: &mut DocumentSnapshotPayload,
  block_id: &str,
) -> DocumentOperationResult<()> {
  if block_id == snapshot.root_block_id {
    return Err(DocumentOperationError::CannotMoveRoot);
  }

  block_index(snapshot, block_id)
    .ok_or_else(|| DocumentOperationError::BlockNotFound(block_id.to_string()))?;

  // A `seen` set bounds the walk. Without it, a block graph that ever became
  // cyclic (see the insert_block guard above) makes this loop grow delete_ids
  // without end — 100% CPU + OOM while holding the document's FOR UPDATE lock,
  // no timeout, no log. (docs/code-review-2026-07-20.md P0-3.)
  let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
  seen.insert(block_id.to_string());
  let mut delete_ids = vec![block_id.to_string()];
  let mut cursor = 0;
  while cursor < delete_ids.len() {
    let current_id = delete_ids[cursor].clone();
    if let Some(index) = block_index(snapshot, &current_id) {
      for child in snapshot.blocks[index].children.clone() {
        if seen.insert(child.clone()) {
          delete_ids.push(child);
        }
      }
    }
    cursor += 1;
  }

  for block in &mut snapshot.blocks {
    block
      .children
      .retain(|child_id| !delete_ids.contains(child_id));
  }

  snapshot
    .blocks
    .retain(|block| !delete_ids.contains(&block.id));

  Ok(())
}

fn move_block(
  snapshot: &mut DocumentSnapshotPayload,
  block_id: &str,
  parent_id: &str,
  index: Option<usize>,
) -> DocumentOperationResult<()> {
  if block_id == snapshot.root_block_id {
    return Err(DocumentOperationError::CannotMoveRoot);
  }

  block_index(snapshot, block_id)
    .ok_or_else(|| DocumentOperationError::BlockNotFound(block_id.to_string()))?;
  let parent_index = block_index(snapshot, parent_id)
    .ok_or_else(|| DocumentOperationError::BlockNotFound(parent_id.to_string()))?;

  if is_descendant(snapshot, parent_id, block_id) {
    return Err(DocumentOperationError::ParentIsDescendant);
  }

  for block in &mut snapshot.blocks {
    block.children.retain(|child_id| child_id != block_id);
  }

  insert_child(
    &mut snapshot.blocks[parent_index].children,
    block_id.to_string(),
    index,
  );

  Ok(())
}

fn validate_block(block: &Block) -> DocumentOperationResult<()> {
  // An empty id is the block-level twin of the 2026-07-19 root-erasure incident:
  // `block_index(snapshot, "")` never matches, so BlockAlreadyExists never fires,
  // and an id-`""` block lands in the CRDT with a `""` child reference that every
  // later read reports as `block not found:` — undeletable, self-perpetuating.
  // (docs/code-review-2026-07-20.md P0-1.)
  if block.id.trim().is_empty() {
    return Err(DocumentOperationError::EmptyBlockId);
  }
  if block.kind.trim().is_empty() {
    return Err(DocumentOperationError::EmptyBlockType);
  }

  Ok(())
}


fn insert_child(children: &mut Vec<String>, block_id: String, index: Option<usize>) {
  let index = index.unwrap_or(children.len()).min(children.len());
  children.insert(index, block_id);
}

#[cfg(test)]
mod tests {
  use serde_json::json;

  use super::*;

  #[test]
  fn inline_marks_round_trip() {
    let md = "This is **bold**, *italic*, `code`, ~~gone~~ and a [link](https://x.io).";
    let snapshot = import_markdown(md, "root");
    let para = snapshot
      .blocks
      .iter()
      .find(|b| b.kind == "paragraph" && b.id != "root")
      .expect("paragraph block");
    assert_eq!(para.text, "This is bold, italic, code, gone and a link.");
    let marks = para.data.get("marks").and_then(Value::as_array).unwrap();
    let kinds: Vec<&str> = marks
      .iter()
      .filter_map(|m| m.get("type").and_then(Value::as_str))
      .collect();
    assert!(kinds.contains(&"bold"));
    assert!(kinds.contains(&"italic"));
    assert!(kinds.contains(&"code"));
    assert!(kinds.contains(&"strike"));
    assert!(kinds.contains(&"link"));

    let exported = export_markdown(&snapshot).expect("export");
    assert!(exported.contains("**bold**"));
    assert!(exported.contains("*italic*"));
    assert!(exported.contains("`code`"));
    assert!(exported.contains("~~gone~~"));
    assert!(exported.contains("[link](https://x.io)"));
  }

  #[test]
  fn divider_round_trips() {
    let md = "before\n\n---\n\nafter";
    let snapshot = import_markdown(md, "root");
    let kinds: Vec<&str> = snapshot
      .blocks
      .iter()
      .filter(|b| b.id != "root")
      .map(|b| b.kind.as_str())
      .collect();
    assert_eq!(kinds, vec!["paragraph", "divider", "paragraph"]);

    let exported = export_markdown(&snapshot).expect("export");
    assert!(exported.contains("\n---\n"), "exported: {exported:?}");
  }

  #[test]
  fn insert_block_adds_child_at_index() {
    let snapshot = sample_snapshot();
    let updated = apply_operations(
      snapshot,
      &[DocumentOperation::InsertBlock {
        parent_id: "root".to_string(),
        index: Some(0),
        block: Block {
          id: "new".to_string(),
          kind: "paragraph".to_string(),
          text: "hello".to_string(),
          data: Value::Null,
          children: vec![],
        },
      }],
    )
    .expect("operation should apply");

    let root = updated
      .blocks
      .iter()
      .find(|block| block.id == "root")
      .unwrap();
    assert_eq!(root.children, vec!["new", "a"]);
    assert!(updated.blocks.iter().any(|block| block.id == "new"));
  }

  #[test]
  fn update_block_changes_text_and_kind() {
    let snapshot = sample_snapshot();
    let updated = apply_operations(
      snapshot,
      &[DocumentOperation::UpdateBlock {
        block_id: "a".to_string(),
        kind: Some("heading".to_string()),
        text: Some("Title".to_string()),
        data: None,
      }],
    )
    .expect("operation should apply");

    let block = updated.blocks.iter().find(|block| block.id == "a").unwrap();
    assert_eq!(block.kind, "heading");
    assert_eq!(block.text, "Title");
  }

  #[test]
  fn delete_block_removes_descendants() {
    let snapshot = DocumentSnapshotPayload {
      schema_version: 1,
      root_block_id: "root".to_string(),
      blocks: vec![
        block("root", vec!["a"]),
        block("a", vec!["b"]),
        block("b", vec![]),
      ],
    };

    let updated = apply_operations(
      snapshot,
      &[DocumentOperation::DeleteBlock {
        block_id: "a".to_string(),
      }],
    )
    .expect("operation should apply");

    assert_eq!(updated.blocks.len(), 1);
    assert_eq!(updated.blocks[0].children, Vec::<String>::new());
  }

  #[test]
  fn move_block_rejects_cycle() {
    let snapshot = DocumentSnapshotPayload {
      schema_version: 1,
      root_block_id: "root".to_string(),
      blocks: vec![
        block("root", vec!["a"]),
        block("a", vec!["b"]),
        block("b", vec![]),
      ],
    };

    let error = apply_operations(
      snapshot,
      &[DocumentOperation::MoveBlock {
        block_id: "a".to_string(),
        parent_id: "b".to_string(),
        index: None,
      }],
    )
    .expect_err("cycle should be rejected");

    assert!(matches!(error, DocumentOperationError::ParentIsDescendant));
  }

  // ── P0-1/2/3: block-level invariants (docs/code-review-2026-07-20.md) ──────

  #[test]
  fn insert_block_rejects_empty_id() {
    let err = apply_operations(
      sample_snapshot(),
      &[DocumentOperation::InsertBlock {
        parent_id: "root".to_string(),
        index: None,
        block: Block {
          id: String::new(),
          kind: "paragraph".to_string(),
          text: "x".to_string(),
          data: Value::Null,
          children: vec![],
        },
      }],
    )
    .expect_err("empty id must be rejected");
    assert!(matches!(err, DocumentOperationError::EmptyBlockId));
  }

  #[test]
  fn insert_block_rejects_children() {
    // The crafted-cycle vector: an inserted block carrying `children: [root]`.
    let err = apply_operations(
      sample_snapshot(),
      &[DocumentOperation::InsertBlock {
        parent_id: "root".to_string(),
        index: None,
        block: Block {
          id: "x".to_string(),
          kind: "paragraph".to_string(),
          text: String::new(),
          data: Value::Null,
          children: vec!["root".to_string()],
        },
      }],
    )
    .expect_err("insert with children must be rejected");
    assert!(matches!(err, DocumentOperationError::InsertBlockWithChildren));
  }

  #[test]
  fn delete_block_terminates_on_a_cyclic_graph() {
    // A pre-existing cycle a<->b (built directly, as if forged before the
    // insert guard existed). Without the `seen` set delete_block loops forever;
    // with it, this returns. NOTE: absent the fix this test HANGS (a timeout in
    // CI), which is the honest failure signature of an unbounded loop.
    let snapshot = DocumentSnapshotPayload {
      schema_version: 1,
      root_block_id: "root".to_string(),
      blocks: vec![
        block("root", vec!["a"]),
        block("a", vec!["b"]),
        block("b", vec!["a"]),
      ],
    };
    let updated = apply_operations(
      snapshot,
      &[DocumentOperation::DeleteBlock {
        block_id: "a".to_string(),
      }],
    )
    .expect("delete must terminate and succeed");
    // a and b both removed; root left childless.
    assert_eq!(updated.blocks.len(), 1);
    assert_eq!(updated.blocks[0].id, "root");
    assert_eq!(updated.blocks[0].children, Vec::<String>::new());
  }

  #[test]
  fn move_block_reorders_siblings() {
    let snapshot = DocumentSnapshotPayload {
      schema_version: 1,
      root_block_id: "root".to_string(),
      blocks: vec![
        block("root", vec!["a", "b", "c"]),
        block("a", vec![]),
        block("b", vec![]),
        block("c", vec![]),
      ],
    };

    let updated = apply_operations(
      snapshot,
      &[DocumentOperation::MoveBlock {
        block_id: "c".to_string(),
        parent_id: "root".to_string(),
        index: Some(0),
      }],
    )
    .expect("operation should apply");

    let root = updated
      .blocks
      .iter()
      .find(|block| block.id == "root")
      .unwrap();
    assert_eq!(root.children, vec!["c", "a", "b"]);
  }

  #[test]
  fn export_markdown_renders_basic_blocks() {
    let snapshot = DocumentSnapshotPayload {
      schema_version: 1,
      root_block_id: "root".to_string(),
      blocks: vec![
        block("root", vec!["heading", "paragraph", "list"]),
        Block {
          id: "heading".to_string(),
          kind: "heading".to_string(),
          text: "Title".to_string(),
          data: Value::Null,
          children: vec![],
        },
        Block {
          id: "paragraph".to_string(),
          kind: "paragraph".to_string(),
          text: "Body".to_string(),
          data: Value::Null,
          children: vec![],
        },
        Block {
          id: "list".to_string(),
          kind: "bulleted_list".to_string(),
          text: "Item".to_string(),
          data: Value::Null,
          children: vec![],
        },
      ],
    };

    let markdown = export_markdown(&snapshot).expect("markdown should export");

    assert_eq!(markdown, "# Title\n\nBody\n\n- Item");
  }

  #[test]
  fn export_markdown_includes_root_text() {
    let snapshot = DocumentSnapshotPayload {
      schema_version: 1,
      root_block_id: "root".to_string(),
      blocks: vec![Block {
        id: "root".to_string(),
        kind: "paragraph".to_string(),
        text: "Root text".to_string(),
        data: Value::Null,
        children: vec![],
      }],
    };

    let markdown = export_markdown(&snapshot).expect("markdown should export");

    assert_eq!(markdown, "Root text");
  }

  fn sample_snapshot() -> DocumentSnapshotPayload {
    DocumentSnapshotPayload {
      schema_version: 1,
      root_block_id: "root".to_string(),
      blocks: vec![block("root", vec!["a"]), block("a", vec![])],
    }
  }

  fn block(id: &str, children: Vec<&str>) -> Block {
    Block {
      id: id.to_string(),
      kind: "paragraph".to_string(),
      text: String::new(),
      data: Value::Null,
      children: children.into_iter().map(str::to_string).collect(),
    }
  }

  #[test]
  fn import_markdown_parses_basic_blocks() {
    let snapshot = import_markdown("# Title\n\nBody\n\n- one\n- two", "root");

    let root = snapshot
      .blocks
      .iter()
      .find(|block| block.id == "root")
      .unwrap();
    assert_eq!(root.children.len(), 4);

    let kinds: Vec<&str> = root
      .children
      .iter()
      .map(|id| {
        snapshot
          .blocks
          .iter()
          .find(|block| &block.id == id)
          .unwrap()
          .kind
          .as_str()
      })
      .collect();
    assert_eq!(
      kinds,
      vec!["heading", "paragraph", "bulleted_list", "bulleted_list"]
    );
  }

  #[test]
  fn import_export_markdown_is_idempotent() {
    let source = "# Title\n\nBody\n\n- Item\n\n1. First\n\n> Quote\n\n- [x] Done";
    let once = export_markdown(&import_markdown(source, "root")).expect("first export");
    let twice = export_markdown(&import_markdown(&once, "root")).expect("second export");
    assert_eq!(once, twice);
    // A re-imported export preserves the original block kinds in order.
    let kinds: Vec<String> = {
      let snapshot = import_markdown(&once, "root");
      let root = snapshot
        .blocks
        .iter()
        .find(|block| block.id == "root")
        .unwrap();
      root
        .children
        .iter()
        .map(|id| {
          snapshot
            .blocks
            .iter()
            .find(|block| &block.id == id)
            .unwrap()
            .kind
            .clone()
        })
        .collect()
    };
    assert_eq!(
      kinds,
      vec![
        "heading",
        "paragraph",
        "bulleted_list",
        "numbered_list",
        "quote",
        "todo"
      ]
    );
  }

  #[test]
  fn import_markdown_parses_image() {
    let snapshot = import_markdown("![alt text](https://example.com/a.png)", "root");
    let image = snapshot
      .blocks
      .iter()
      .find(|block| block.kind == "image")
      .expect("image block exists");
    assert_eq!(image.text, "alt text");
    assert_eq!(image.data["url"], "https://example.com/a.png");
  }

  #[test]
  fn export_markdown_renders_image() {
    let snapshot = DocumentSnapshotPayload {
      schema_version: 1,
      root_block_id: "root".to_string(),
      blocks: vec![
        block("root", vec!["img"]),
        Block {
          id: "img".to_string(),
          kind: "image".to_string(),
          text: "cat".to_string(),
          data: json!({ "url": "https://example.com/cat.png" }),
          children: vec![],
        },
      ],
    };

    let markdown = export_markdown(&snapshot).expect("markdown should export");
    assert_eq!(markdown, "![cat](https://example.com/cat.png)");
  }

  #[test]
  fn export_html_renders_headings_and_lists() {
    let snapshot = DocumentSnapshotPayload {
      schema_version: 1,
      root_block_id: "root".to_string(),
      blocks: vec![
        block("root", vec!["h", "a", "b"]),
        Block {
          id: "h".to_string(),
          kind: "heading".to_string(),
          text: "Title".to_string(),
          data: json!({ "level": 2 }),
          children: vec![],
        },
        Block {
          id: "a".to_string(),
          kind: "bulleted_list".to_string(),
          text: "one".to_string(),
          data: Value::Null,
          children: vec![],
        },
        Block {
          id: "b".to_string(),
          kind: "bulleted_list".to_string(),
          text: "two".to_string(),
          data: Value::Null,
          children: vec![],
        },
      ],
    };

    let html = export_html(&snapshot).expect("html should export");
    assert_eq!(
      html,
      "<h2>Title</h2>\n<ul>\n<li>one</li>\n<li>two</li>\n</ul>"
    );
  }

  #[test]
  fn export_html_escapes_text() {
    let snapshot = DocumentSnapshotPayload {
      schema_version: 1,
      root_block_id: "root".to_string(),
      blocks: vec![
        block("root", vec!["p"]),
        Block {
          id: "p".to_string(),
          kind: "paragraph".to_string(),
          text: "a < b & c".to_string(),
          data: Value::Null,
          children: vec![],
        },
      ],
    };

    let html = export_html(&snapshot).expect("html should export");
    assert_eq!(html, "<p>a &lt; b &amp; c</p>");
  }
}

