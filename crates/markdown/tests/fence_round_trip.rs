use mica_markdown::{export_markdown, import_markdown};

fn assert_code_round_trip(snapshot: mica_markdown::DocumentSnapshotPayload) {
  let exported = export_markdown(&snapshot).unwrap();
  let first = import_markdown(&exported, "root");
  let one = first
    .blocks
    .iter()
    .filter(|b| b.kind == "code_block")
    .collect::<Vec<_>>();
  let original = snapshot
    .blocks
    .iter()
    .filter(|b| b.kind == "code_block")
    .collect::<Vec<_>>();
  assert_eq!(one.len(), 1, "{exported:?}");
  assert_eq!(one[0].text, original[0].text, "{exported}");
  assert_eq!(
    one[0].data["language"], original[0].data["language"],
    "{exported}"
  );
}
#[test]
fn quoted_fences_share_top_level_delimiter_rules() {
  for (input, expected) in [
    ("> ````\n> ```\n> ````", "```"),
    ("> ~~~\n> x\n> ~~~", "x"),
    ("> intro\n> ~~~rust\n> x\n> ~~~", "x"),
    ("> ```\n> x  \n> ```", "x  "),
  ] {
    let snapshot = import_markdown(input, "root");
    let code: Vec<_> = snapshot
      .blocks
      .iter()
      .filter(|b| b.kind == "code_block")
      .collect();
    assert_eq!(code.len(), 1, "{input}");
    assert_eq!(code[0].text, expected, "{input}");
    assert_eq!(code[0].data["quote"], 1);
  }
}

#[test]
fn every_code_block_exports_a_nonconflicting_fence() {
  for text in [
    "```",
    "````\n```\n````",
    "~~~",
    "no fence",
    "```\n````",
    "`````",
  ] {
    let mut snapshot = import_markdown("placeholder", "root");
    let block = snapshot
      .blocks
      .iter_mut()
      .find(|b| b.text == "placeholder")
      .unwrap();
    block.kind = "code_block".into();
    block.text = text.into();
    assert_code_round_trip(snapshot);
  }
  for language in ["", "rust"] {
    let mut snapshot = import_markdown("placeholder", "root");
    let block = snapshot
      .blocks
      .iter_mut()
      .find(|b| b.text == "placeholder")
      .unwrap();
    block.kind = "code_block".into();
    block.text = "```".into();
    block.data = if language.is_empty() {
      serde_json::Value::Null
    } else {
      serde_json::json!({ "language": language })
    };
    assert_code_round_trip(snapshot);
  }
}
