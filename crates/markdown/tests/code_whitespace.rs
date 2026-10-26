use mica_markdown::{export_markdown, import_markdown};

fn shape(markdown: &str) -> Vec<(String, String, serde_json::Value)> {
  let snapshot = import_markdown(markdown, "root");
  snapshot
    .blocks
    .into_iter()
    .filter(|b| b.id != snapshot.root_block_id)
    .map(|b| (b.kind, b.text, b.data))
    .collect()
}

#[test]
fn whitespace_only_code_paragraph_survives_export() {
  let input = "`   `";
  let first = import_markdown(input, "root");
  let exported = export_markdown(&first).unwrap();
  assert!(!exported.is_empty(), "code whitespace is content");
  assert_eq!(shape(&exported), shape(input), "{exported:?}");
}

#[test]
fn code_padding_preserves_tabs_spaces_and_unicode_whitespace() {
  for input in [
    "`  \t  `",
    "x `  \t  ` y",
    "`\t `",
    "` \t`",
    "`  \u{2003}  `",
    "`\u{2003}`",
    "`   `",
  ] {
    let exported = export_markdown(&import_markdown(input, "root")).unwrap();
    assert_eq!(
      shape(&exported),
      shape(input),
      "input {input:?}, export {exported:?}"
    );
  }
  assert_eq!(
    export_markdown(&import_markdown(" \t ", "root")).unwrap(),
    ""
  );
}
