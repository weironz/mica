//! YAML front matter handling: a leading `---` fence is lifted off the block
//! stream verbatim (never parsed as YAML, never degraded into thematic breaks
//! or paragraphs), stashed on the root, restored on Markdown export, and
//! ignored on HTML export. Boundary cases pin "read the full spec / write a
//! normalized subset": only a true opening + closing fence counts.

use mica_markdown::{export_html, export_markdown, import_markdown, import_markdown_fragment};

fn root_front_matter(md: &str) -> Option<String> {
  let snap = import_markdown(md, "root");
  let root = snap
    .blocks
    .iter()
    .find(|b| b.id == snap.root_block_id)
    .unwrap();
  root
    .data
    .get("front_matter")
    .and_then(|v| v.as_str())
    .map(str::to_string)
}

#[test]
fn front_matter_is_lifted_off_the_block_stream() {
  let md = "---\ntitle: Hello\ntags: [a, b]\n---\n# Heading\n\nBody.";
  let snap = import_markdown(md, "root");

  // Raw inner text is captured verbatim, YAML left unparsed.
  assert_eq!(
    root_front_matter(md).as_deref(),
    Some("title: Hello\ntags: [a, b]")
  );

  // The fences did NOT become thematic-break / setext-heading blocks: the
  // only children are the heading and the paragraph.
  let kinds: Vec<&str> = snap
    .blocks
    .iter()
    .filter(|b| b.id != snap.root_block_id)
    .map(|b| b.kind.as_str())
    .collect();
  assert_eq!(kinds, vec!["heading", "paragraph"]);
}

#[test]
fn round_trip_is_identity() {
  let md = "---\ntitle: Hello\ncount: 3\n---\n# Heading\n\nBody text.";
  let back = export_markdown(&import_markdown(md, "root")).unwrap();
  assert_eq!(back, md);
}

#[test]
fn round_trip_front_matter_only_document() {
  let md = "---\ntitle: Hello\n---";
  let back = export_markdown(&import_markdown(md, "root")).unwrap();
  assert_eq!(back, md);
}

#[test]
fn empty_front_matter_round_trips() {
  let md = "---\n---\nBody.";
  assert_eq!(root_front_matter(md).as_deref(), Some(""));
  let back = export_markdown(&import_markdown(md, "root")).unwrap();
  assert_eq!(back, md);
}

#[test]
fn dot_close_fence_is_recognized_and_normalized_to_dashes() {
  // `...` is a valid YAML close; export normalizes it to `---` (write a
  // normalized subset). The inner text still round-trips byte-for-byte.
  let md = "---\ntitle: x\n...\nBody.";
  assert_eq!(root_front_matter(md).as_deref(), Some("title: x"));
  let back = export_markdown(&import_markdown(md, "root")).unwrap();
  assert_eq!(back, "---\ntitle: x\n---\nBody.");
}

#[test]
fn dashes_inside_front_matter_value_do_not_close_early() {
  // A `---` that is NOT a whole line (here it's a quoted value) is part of
  // the YAML, not the close fence.
  let md = "---\nsep: \"---\"\nrule: value\n---\nBody.";
  assert_eq!(
    root_front_matter(md).as_deref(),
    Some("sep: \"---\"\nrule: value")
  );
  let back = export_markdown(&import_markdown(md, "root")).unwrap();
  assert_eq!(back, md);
}

#[test]
fn first_line_not_a_fence_is_not_front_matter() {
  // A leading `---` that is NOT the first line, or a non-`---` first line,
  // parses normally. Here `# Heading` first, then a thematic break.
  let md = "# Heading\n\n---\n\nBody.";
  assert_eq!(root_front_matter(md), None);
  let kinds: Vec<String> = import_markdown(md, "root")
    .blocks
    .into_iter()
    .filter(|b| b.id != "root")
    .map(|b| b.kind)
    .collect();
  assert!(kinds.contains(&"divider".to_string()), "kinds: {kinds:?}");
}

#[test]
fn setext_heading_first_line_is_not_front_matter() {
  // `Title\n---` is a setext H2, not an (un-opened) front matter fence.
  let md = "Title\n---\n";
  assert_eq!(root_front_matter(md), None);
  let html = export_html(&import_markdown(md, "root")).unwrap();
  assert!(html.contains("<h2>"), "html: {html}");
}

#[test]
fn unterminated_fence_is_treated_as_body() {
  // First line is `---` but nothing closes it: not front matter. The opener
  // becomes an ordinary thematic break and the rest is body.
  let md = "---\ntitle: Hello\nbody line\n";
  assert_eq!(root_front_matter(md), None);
  let kinds: Vec<String> = import_markdown(md, "root")
    .blocks
    .into_iter()
    .filter(|b| b.id != "root")
    .map(|b| b.kind)
    .collect();
  // The leading `---` survives as a divider; the rest are paragraphs.
  assert_eq!(kinds.first().map(String::as_str), Some("divider"));
}

#[test]
fn html_export_ignores_front_matter() {
  let md = "---\ntitle: Hello\n---\n# Heading";
  let html = export_html(&import_markdown(md, "root")).unwrap();
  assert!(!html.contains("title"), "html leaked front matter: {html}");
  assert!(html.contains("<h1>"), "html: {html}");
}

// ---- Fragments -------------------------------------------------------------
//
// Front matter exists exactly once, at the very top of a file. Content grafted
// INTO an existing document is never at that position, so reading a leading
// `---` as front matter there swallows real text. Reported 2026-08-07: an MCP
// append whose body sat between two `---` lines arrived with only the tail, no
// error — the caller concluded "avoid `---` in Mica", a rule the format has no
// reason to have.

#[test]
fn a_fragment_keeps_the_body_between_two_dividers() {
  // The exact reported shape. Before the fix this yielded ONE block ("tail").
  let md = "---\nbody\n---\ntail";
  let snap = import_markdown_fragment(md, "root");
  let root = snap.blocks.iter().find(|b| b.id == "root").unwrap();
  assert!(
    root.data.get("front_matter").is_none(),
    "a fragment must not produce front matter: {:?}",
    root.data
  );

  // Note the SECOND `---`: pressed straight against the line above it, that is
  // a setext underline, so `body` is an h2 and not paragraph-then-divider. That
  // is CommonMark, not a Mica quirk, and it is why this test asserts the TEXT
  // rather than a pretty block sequence — the bug being fixed is "the text
  // disappeared", and pinning the exact kinds here would pin the wrong thing.
  let kinds: Vec<String> = snap
    .blocks
    .iter()
    .filter(|b| b.id != "root")
    .map(|b| b.kind.clone())
    .collect();
  assert_eq!(kinds, vec!["divider", "heading", "paragraph"]);

  let texts: Vec<String> = snap
    .blocks
    .iter()
    .filter(|b| b.id != "root" && !b.text.is_empty())
    .map(|b| b.text.clone())
    .collect();
  assert_eq!(
    texts,
    vec!["body", "tail"],
    "the body between the fences must survive"
  );
}

#[test]
fn a_fragment_with_blank_lines_gives_real_dividers() {
  // The shape a caller usually means when they open with `---`: blank lines on
  // both sides, so neither `---` can be a setext underline.
  let snap = import_markdown_fragment("---\n\nbody\n\n---\n\ntail", "root");
  let kinds: Vec<String> = snap
    .blocks
    .iter()
    .filter(|b| b.id != "root")
    .map(|b| b.kind.clone())
    .collect();
  assert_eq!(kinds, vec!["divider", "paragraph", "divider", "paragraph"]);
}

#[test]
fn a_whole_document_still_reads_the_same_text_as_front_matter() {
  // The two entry points must genuinely disagree here — otherwise the fix is
  // just a rename and the document path lost its front matter too.
  let md = "---\nbody\n---\ntail";
  assert_eq!(root_front_matter(md).as_deref(), Some("body"));
}

#[test]
fn a_fragment_that_only_looks_like_front_matter_is_not_swallowed() {
  // No close fence: both entry points agree, and neither eats anything.
  let md = "---\njust a line\n";
  for snap in [
    import_markdown_fragment(md, "root"),
    import_markdown(md, "root"),
  ] {
    let root = snap.blocks.iter().find(|b| b.id == "root").unwrap();
    assert!(root.data.get("front_matter").is_none());
    assert_eq!(root.children.len(), 2, "divider + paragraph");
  }
}
