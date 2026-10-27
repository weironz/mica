//! I5: injectable block ids. `Sequential` is deterministic, and every parse
//! agrees with the `Uuid` mode once ids are masked.

use std::path::Path;

use mica_md::{BlockIds, DocumentSnapshotPayload, ParseMode, ParseOptions, parse};

fn options(ids: BlockIds) -> ParseOptions {
  ParseOptions {
    mode: ParseMode::Document,
    root_id: "root".to_string(),
    ids,
  }
}

fn sequential() -> BlockIds {
  BlockIds::Sequential {
    prefix: "b".to_string(),
  }
}

/// Replace each id by `b<n>` in order of first appearance, rewriting children.
#[cfg_attr(not(feature = "uuid"), allow(dead_code))]
fn masked(doc: &DocumentSnapshotPayload) -> serde_json::Value {
  let mut map: Vec<(String, String)> = vec![(doc.root_block_id.clone(), doc.root_block_id.clone())];
  for block in &doc.blocks {
    if !map.iter().any(|(from, _)| *from == block.id) {
      let next = format!("b{}", map.len());
      map.push((block.id.clone(), next));
    }
  }
  let remap = |id: &str| -> String {
    map
      .iter()
      .find(|(from, _)| from == id)
      .map(|(_, to)| to.clone())
      .unwrap_or_else(|| id.to_string())
  };
  let blocks: Vec<serde_json::Value> = doc
    .blocks
    .iter()
    .map(|block| {
      let mut value = serde_json::to_value(block).unwrap();
      value["id"] = serde_json::Value::String(remap(&block.id));
      let children: Vec<serde_json::Value> = block
        .children
        .iter()
        .map(|c| serde_json::Value::String(remap(c)))
        .collect();
      value["children"] = serde_json::Value::Array(children);
      value
    })
    .collect();
  serde_json::json!({
    "schema_version": doc.schema_version,
    "root_block_id": remap(&doc.root_block_id),
    "blocks": blocks,
  })
}

fn fixtures() -> Vec<(String, String)> {
  let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../markdown/tests/fixtures/conformance");
  let mut paths: Vec<_> = std::fs::read_dir(dir)
    .expect("conformance fixtures")
    .filter_map(|e| e.ok())
    .map(|e| e.path())
    .filter(|p| p.extension().is_some_and(|x| x == "md"))
    .collect();
  paths.sort();
  paths
    .into_iter()
    .map(|p| {
      (
        p.file_name().unwrap().to_string_lossy().to_string(),
        std::fs::read_to_string(&p).unwrap(),
      )
    })
    .collect()
}

#[test]
fn sequential_ids_are_deterministic() {
  for (name, markdown) in fixtures() {
    let a = parse(&markdown, &options(sequential()));
    let b = parse(&markdown, &options(sequential()));
    assert_eq!(a, b, "parse is not deterministic for {name}");
  }
}

#[test]
fn no_temporary_id_survives_a_parse() {
  for (name, markdown) in fixtures() {
    let doc = parse(&markdown, &options(sequential()));
    for block in &doc.blocks {
      assert!(
        !block.id.contains('\u{1}'),
        "temporary id left in {name}: {:?}",
        block.id
      );
      for child in &block.children {
        assert!(!child.contains('\u{1}'), "temporary child id in {name}");
      }
    }
  }
}

#[cfg(feature = "uuid")]
#[test]
fn uuid_and_sequential_agree_once_ids_are_masked() {
  for (name, markdown) in fixtures() {
    let uuid = parse(&markdown, &options(BlockIds::Uuid));
    let seq = parse(&markdown, &options(sequential()));
    assert_eq!(
      masked(&uuid),
      masked(&seq),
      "id mode changed parse semantics for {name}"
    );
  }
}

#[test]
fn a_root_id_in_the_sequential_namespace_never_collides() {
  let doc = parse(
    "one\n\ntwo",
    &ParseOptions {
      mode: ParseMode::Document,
      root_id: "b0".to_string(),
      ids: sequential(),
    },
  );
  assert_eq!(doc.root_block_id, "b0");
  let ids: std::collections::HashSet<&str> = doc.blocks.iter().map(|b| b.id.as_str()).collect();
  assert_eq!(
    ids.len(),
    doc.blocks.len(),
    "ids must be unique: {:?}",
    doc.blocks
  );
  // Contract exception: an effective prefix keeps n from 0 and avoids the
  // caller's root id deterministically.
  let body_ids: Vec<&str> = doc.blocks.iter().map(|b| b.id.as_str()).collect();
  assert!(body_ids.contains(&"b_0"), "{body_ids:?}");
  assert!(body_ids.contains(&"b_1"), "{body_ids:?}");
  let root = doc.blocks.iter().find(|b| b.id == "b0").unwrap();
  assert_eq!(root.children.len(), 2);
  for child in &root.children {
    assert!(
      doc.blocks.iter().any(|b| &b.id == child),
      "child {child} must exist"
    );
  }
  let markdown = mica_md::serialize(&doc, &mica_md::SerializeOptions::default()).unwrap();
  assert!(
    markdown.contains("one") && markdown.contains("two"),
    "{markdown}"
  );
}

#[test]
fn normal_sequential_ids_keep_the_callers_prefix() {
  let doc = parse(
    "one\n\ntwo",
    &ParseOptions {
      mode: ParseMode::Document,
      root_id: "root".to_string(),
      ids: sequential(),
    },
  );
  let body_ids: Vec<&str> = doc
    .blocks
    .iter()
    .filter(|b| b.id != "root")
    .map(|b| b.id.as_str())
    .collect();
  assert_eq!(body_ids, vec!["b0", "b1"]);
}
