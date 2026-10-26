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

fn assert_round_trip(input: &str) {
  let exported = export_markdown(&import_markdown(input, "root")).unwrap();
  assert_eq!(
    shape(&exported),
    shape(input),
    "input {input:?}, export {exported:?}"
  );
}

#[test]
fn literal_entities_decode_exactly_once_per_import() {
  for input in ["&amp;amp;", "&amp;copy;", "plain &amp; text", "&amp;#35;"] {
    assert_round_trip(input);
  }
}

#[test]
fn inline_math_with_dollars_uses_an_unambiguous_delimiter() {
  assert_round_trip(r"\(a$b\)");
  assert_round_trip(r"x \(a$$b\) y");
}

#[test]
fn strike_delimiters_inside_code_do_not_close_the_strike() {
  let input = "~~`a~~b`~~";
  let first = shape(input);
  assert_eq!(first[0].1, "a~~b");
  assert_eq!(
    first[0].2["marks"],
    serde_json::json!([
      {"start": 0, "end": 4, "type": "code"},
      {"start": 0, "end": 4, "type": "strike"}
    ])
  );
  assert_round_trip(input);
}

#[test]
fn empty_links_keep_their_position_and_destination() {
  for input in [
    "a[](/u)b",
    "[](/u)",
    "a[](/u)",
    "[](/u)a",
    "**a[](/u)b**",
    "a[](/u)[](/v)b",
  ] {
    assert_round_trip(input);
  }
}

#[test]
fn html_empty_link_behavior_stays_unchanged() {
  let snapshot = import_markdown("a[](/u)b", "root");
  assert_eq!(
    mica_markdown::export_html(&snapshot).unwrap().trim(),
    "<p>ab</p>"
  );
}

#[test]
fn link_destinations_preserve_decoded_punctuation_and_entities() {
  for input in [
    r"[x](a\)b)",
    r"[x](a\(b)",
    r"[x](a\\b)",
    "[x](a&amp;amp;b)",
    "[x](<a b>)",
    "[x](a%20b)",
  ] {
    assert_round_trip(input);
  }
}

#[test]
fn link_titles_preserve_backslashes_quotes_and_entities() {
  for input in [
    r#"[x](u "a\\")"#,
    r#"[x](u "a\"b")"#,
    r#"[x](u "&amp;amp;")"#,
  ] {
    assert_round_trip(input);
  }
}

#[test]
fn image_blocks_preserve_destinations_and_titles() {
  for input in [
    "![x](<a b.png>)",
    r#"![x](a\)b.png "a\\")"#,
    "![x](a&amp;amp;b.png)",
    r#"![x](u "&amp;amp;")"#,
  ] {
    assert_round_trip(input);
  }
}

#[test]
fn zero_width_link_at_a_shorthand_start_survives_export() {
  assert_round_trip("[](x)www.example.com");
  let first = shape("[](x)<https://e.com>");
  let link_count = |blocks: &[(String, String, serde_json::Value)]| {
    blocks[0]
      .2
      .get("marks")
      .and_then(|m| m.as_array())
      .map(|ms| ms.iter().filter(|m| m["type"] == "link").count())
      .unwrap_or(0)
  };
  assert_eq!(link_count(&first), 2, "{first:?}");
  let exported = export_markdown(&import_markdown("[](x)<https://e.com>", "root")).unwrap();
  let again = shape(&exported);
  assert_eq!(link_count(&again), 2, "{exported:?}");
  assert!(again[0].1.contains("https://e.com"), "{exported:?}");
}
