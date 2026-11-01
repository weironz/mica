//! States the web client wrote with yjs must keep decoding.
//!
//! Until 2026-11-01 the browser ran yjs while desktop and server ran yrs, and
//! this test shelled out to `node` to run the shipped yjs bundle against a
//! yrs-written base on every run (P2-M4, W2). The web client now runs this same
//! Rust core as wasm (`crates/doc-wasm`), so there is no second engine to
//! check against — but every browser that used the old client still holds
//! yjs-encoded documents in IndexedDB, and those are read by yrs from now on.
//!
//! `fixtures/yjs_written_state.b64` is the last output of that harness,
//! captured before the bundle was deleted: a yrs-written base (root → seed,
//! seed with an italic mark and an int prop) that yjs then edited the way the
//! old web client did — set_text_and_marks on `seed` (bold replaces italic),
//! and a new block `w2new` at index 0 with a link mark (href + title) and
//! field-level Y.Map props (a string and an int). The assertions are the ones
//! the live harness made. Never regenerate this file: there is no yjs left to
//! regenerate it with, and its value is that it was written by the real one.

use base64::{engine::general_purpose::STANDARD, Engine};
use mica_core::MicaDoc;

#[test]
fn a_state_written_by_the_retired_yjs_web_client_still_decodes() {
  let bytes = STANDARD
    .decode(include_str!("fixtures/yjs_written_state.b64").trim())
    .expect("fixture is base64");

  let doc = MicaDoc::from_update(&bytes).expect("yjs-written state decodes in yrs");
  let blocks = doc.to_blocks();
  assert_eq!(doc.root_block_id(), "root");

  // The block the yjs side added with a link mark (href + title).
  let w2new = blocks
    .iter()
    .find(|b| b.id == "w2new")
    .expect("yjs-inserted block present");
  assert_eq!(w2new.text, "hello link");
  let link = w2new.data["marks"]
    .as_array()
    .and_then(|a| a.iter().find(|m| m["type"] == "link"))
    .expect("link mark present");
  assert_eq!(link["start"], 0);
  assert_eq!(link["end"], 5);
  assert_eq!(link["href"], "http://x");
  assert_eq!(link["title"], "T");
  // Field-level props written by yjs (P2-M4.7 Y.Map) read back in Rust,
  // ints preserved.
  assert_eq!(
    w2new.data["role"], "note",
    "yjs-written string prop present"
  );
  assert_eq!(w2new.data["level"], 2, "yjs-written int prop preserved");

  // seed after the yjs rewrite: same text, marks REPLACED (bold in, the
  // original italic gone — the update was a full set_text_and_marks), and
  // the int prop survived the yjs read→JSON→rewrite cycle as an int.
  let seed = blocks
    .iter()
    .find(|b| b.id == "seed")
    .expect("seed present");
  assert_eq!(seed.text, "seed text here");
  let seed_marks = seed.data["marks"].as_array().expect("seed has marks");
  assert_eq!(
    seed_marks.len(),
    1,
    "exactly the bold mark, italic replaced: {seed_marks:?}"
  );
  assert_eq!(seed_marks[0]["type"], "bold");
  assert_eq!(seed_marks[0]["start"], 0);
  assert_eq!(seed_marks[0]["end"], 5);
  assert_eq!(seed.data["indent"], 1, "int prop survives yjs round-trip");

  // Tree edit made by yjs (insert at index 0) is what yrs reads back.
  let root = blocks
    .iter()
    .find(|b| b.id == "root")
    .expect("root present");
  assert_eq!(root.children, vec!["w2new", "seed"]);
}
