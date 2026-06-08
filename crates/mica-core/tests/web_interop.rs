//! P2-M4 (web→yjs) W2: confirm the Rust `yrs` core reads a document written by
//! the JS `yjs` web client — including inline marks (bold + link with metadata).
//! This is the reverse direction of the in-browser W1 check, so together they
//! prove full bidirectional wire-compat over Mica's exact doc layout.
//!
//! Gated on `MICA_WEB_STATE_B64` (the base64 yjs state captured from the browser
//! W2 self-test); skips (passes) when unset.
//!
//!   $env:MICA_WEB_STATE_B64="<captured>"; cargo test -p mica-core --test web_interop

use base64::{engine::general_purpose::STANDARD, Engine};
use mica_core::MicaDoc;

#[test]
fn rust_reads_yjs_written_marks() {
    let Ok(b64) = std::env::var("MICA_WEB_STATE_B64") else {
        eprintln!("skipping rust_reads_yjs_written_marks: no MICA_WEB_STATE_B64");
        return;
    };
    let bytes = STANDARD.decode(b64.trim()).expect("valid base64");
    let doc = MicaDoc::from_update(&bytes).expect("yjs-written state decodes in yrs");
    let blocks = doc.to_blocks();

    // The block the web client added with a link mark (href + title).
    let w2new = blocks
        .iter()
        .find(|b| b.id == "w2new")
        .expect("web-inserted block present");
    assert_eq!(w2new.text, "hello link");
    let link = w2new.data["marks"]
        .as_array()
        .and_then(|a| a.iter().find(|m| m["type"] == "link"))
        .expect("link mark present");
    assert_eq!(link["start"], 0);
    assert_eq!(link["end"], 5);
    assert_eq!(link["href"], "http://x");
    assert_eq!(link["title"], "T");
    // Field-level props written by the web (P2-M4.7 Y.Map) read back in Rust,
    // ints preserved.
    assert_eq!(w2new.data["role"], "note", "web-written string prop present");
    assert_eq!(w2new.data["level"], 2, "web-written int prop preserved");

    // Some existing block now carries a bold mark over [0,5) written by the web.
    let has_bold = blocks.iter().any(|b| {
        b.data
            .get("marks")
            .and_then(|m| m.as_array())
            .map(|a| a.iter().any(|m| m["type"] == "bold" && m["start"] == 0 && m["end"] == 5))
            .unwrap_or(false)
    });
    assert!(has_bold, "bold mark written by the web client is present");
}
