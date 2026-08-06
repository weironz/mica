//! What the `<details>` fold in the editor is allowed to assume, pinned on the
//! side that decides it: the parser.
//!
//! The editor renders a fold for `<details>` and stores the open/closed state in
//! `data.collapsed` on the OPENING block. That is only safe while two things
//! hold, and neither is visible from the Dart side:
//!
//! 1. The blank-line form GitHub recommends parses into a `code_block` opener,
//!    the body as ordinary Markdown blocks, and a `code_block` holding nothing
//!    but `</details>`. The renderer pairs opener with closer by exactly that
//!    shape.
//! 2. `data.collapsed` is not Markdown. Setting it must not move a single byte
//!    of the exported document — folding a section is a view action, and the
//!    round-trip fixed point is the invariant (CLAUDE.md principle 4).

use mica_markdown::{export_markdown, import_markdown};
use serde_json::json;

const GITHUB_FORM: &str =
  "<details>\n<summary>Tips</summary>\n\nBody **markdown** here.\n\n- one\n- two\n\n</details>";

/// (kind, text) of every block below the root, in order.
fn shape(md: &str) -> Vec<(String, String)> {
  let payload = import_markdown(md, "root");
  payload
    .blocks
    .iter()
    .filter(|b| b.id != payload.root_block_id)
    .map(|b| (b.kind.clone(), b.text.clone()))
    .collect()
}

#[test]
fn the_blank_line_form_is_an_opener_a_body_and_a_bare_closer() {
  assert_eq!(
    shape(GITHUB_FORM),
    vec![
      ("code_block".into(), "<details>\n<summary>Tips</summary>".into()),
      ("paragraph".into(), "Body markdown here.".into()),
      ("bulleted_list".into(), "one".into()),
      ("bulleted_list".into(), "two".into()),
      ("code_block".into(), "</details>".into()),
    ],
  );
}

#[test]
fn the_opener_and_closer_are_raw_html_blocks() {
  let payload = import_markdown(GITHUB_FORM, "root");
  for text in ["<details>\n<summary>Tips</summary>", "</details>"] {
    let block = payload.blocks.iter().find(|b| b.text == text).unwrap();
    assert_eq!(block.data.get("raw"), Some(&json!(true)), "{text:?}");
    assert_eq!(block.data.get("language"), Some(&json!("html")), "{text:?}");
  }
}

#[test]
fn a_summary_less_opener_still_splits_off_its_closer() {
  assert_eq!(
    shape("<details>\n\nbody\n\n</details>"),
    vec![
      ("code_block".into(), "<details>".into()),
      ("paragraph".into(), "body".into()),
      ("code_block".into(), "</details>".into()),
    ],
  );
}

#[test]
fn open_is_carried_on_the_opener_verbatim() {
  let blocks = shape("<details open>\n<summary>S</summary>\n\nbody\n\n</details>");
  assert_eq!(blocks[0].1, "<details open>\n<summary>S</summary>");
}

/// The round-trip red line. Folding writes `data.collapsed` and nothing else, so
/// the exported bytes must be indistinguishable from the source — including the
/// `<details>` having no `open` attribute after the user expanded it on screen.
#[test]
fn collapsing_writes_data_but_never_bytes() {
  for source in [
    GITHUB_FORM,
    "<details open>\n<summary>S</summary>\n\nbody\n\n</details>",
    // The tight one-block form the first step shipped, checked here too so one
    // test guards both shapes.
    "<details>\n<summary>S</summary>\nbody\n</details>",
  ] {
    let baseline = export_markdown(&import_markdown(source, "root")).unwrap();
    assert_eq!(baseline, source, "import→export is not a fixed point");

    for collapsed in [true, false] {
      let mut payload = import_markdown(source, "root");
      let opener = payload
        .blocks
        .iter_mut()
        .find(|b| b.text.to_lowercase().starts_with("<details"))
        .expect("an opening block");
      opener
        .data
        .as_object_mut()
        .expect("raw blocks carry an object data")
        .insert("collapsed".into(), json!(collapsed));

      assert_eq!(
        export_markdown(&payload).unwrap(),
        source,
        "collapsed={collapsed} changed the document for {source:?}",
      );
    }
  }
}
