//! P2-M4 (web→yjs): the cross-engine gate. yjs↔yrs byte-compat over Mica's
//! exact doc layout is the premise CLAUDE.md dependency exemption #7 rests on
//! (AFFiNE's y-octo hit production bugs on this same compat surface), so it is
//! enforced here on every `cargo test -p mica-core` run, not assumed.
//!
//! Full round-trip, both directions, no browser:
//!   1. Rust (`yrs`) writes a base doc — marks + int props included.
//!   2. `node` loads the COMMITTED web bundle (`web/yjs_bundle.js`, the exact
//!      yjs bytes the web client ships) via tool/yjs/w2_headless.cjs, asserts
//!      it can read the yrs-written base (direction 1), applies the same edits
//!      the browser W2 self-test makes, and prints the re-encoded state.
//!   3. Rust decodes that state and asserts the yjs-written marks/props/tree
//!      read back exactly (direction 2).
//!
//! History: this used to be `#[ignore]`, gated on a hand-captured
//! MICA_WEB_STATE_B64 from the browser W2 self-test, because no CI job
//! produced the state — the one honest option at the time
//! (docs/code-review-2026-07-20.md, P2-3). The headless harness closes that
//! gap; a browser capture is no longer needed (the in-browser probe,
//! yjs_probe_web.dart, still exists for e2e work). Requires `node` on PATH —
//! preinstalled on all GitHub-hosted runners; the harness fails loudly (not
//! skips) without it, per the sync_pg.rs discipline.

use std::path::PathBuf;
use std::process::Command;

use base64::{engine::general_purpose::STANDARD, Engine};
use mica_core::{Block, MicaDoc};
use serde_json::json;

/// The yrs-written base: root → [seed], where seed carries an inline italic
/// mark and an integer prop — the two encodings (Y.Text formatting attrs,
/// `Any::BigInt`) that must survive the engine boundary.
fn yrs_base() -> MicaDoc {
    let root = Block::new("root", "paragraph").with_children(vec!["seed".into()]);
    let seed = Block::new("seed", "paragraph")
        .with_text("seed text here")
        .with_data(json!({
            "indent": 1,
            "marks": [{"start": 0, "end": 4, "type": "italic"}],
        }));
    MicaDoc::from_blocks("root", &[root, seed])
}

/// Run the headless yjs harness on `base_b64`, returning the yjs-re-encoded
/// state. Any yjs-side assertion failure surfaces here as a panic with the
/// harness's stderr.
fn yjs_w2_roundtrip(base_b64: &str) -> Vec<u8> {
    // CARGO_MANIFEST_DIR = crates/mica-core → repo root is two levels up.
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root")
        .join("clients/mica_flutter/tool/yjs/w2_headless.cjs");
    assert!(script.is_file(), "harness missing: {}", script.display());

    let out = Command::new("node")
        .arg(&script)
        .arg(base_b64)
        .output()
        .expect(
            "spawn `node` — the cross-engine gate needs Node on PATH \
             (preinstalled on CI runners)",
        );
    assert!(
        out.status.success(),
        "yjs side rejected the yrs-written base or failed to re-encode:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    STANDARD
        .decode(String::from_utf8_lossy(&out.stdout).trim())
        .expect("harness stdout is base64")
}

#[test]
fn yjs_yrs_cross_engine_roundtrip() {
    let base_b64 = STANDARD.encode(yrs_base().encode_state());
    let bytes = yjs_w2_roundtrip(&base_b64);

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
    assert_eq!(w2new.data["role"], "note", "yjs-written string prop present");
    assert_eq!(w2new.data["level"], 2, "yjs-written int prop preserved");

    // seed after the yjs rewrite: same text, marks REPLACED (bold in, the
    // original italic gone — the update was a full set_text_and_marks), and
    // the int prop survived the yjs read→JSON→rewrite cycle as an int.
    let seed = blocks.iter().find(|b| b.id == "seed").expect("seed present");
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
    let root = blocks.iter().find(|b| b.id == "root").expect("root present");
    assert_eq!(root.children, vec!["w2new", "seed"]);
}
