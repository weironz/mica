//! P0 cost-attribution spike: the S1 engine behind a wasm-bindgen edge, with
//! exports chosen so each stage of the production `parseBody` path can be
//! measured in isolation.
//!
//! No caching, no structured JS objects, no shortcut around the wire format:
//! `parseBody` is exactly `mica_md::wire::parse_body_json` (§3.3).

mod stage3;

use mica_md::{ParseMode, wire};
use wasm_bindgen::prelude::*;

/// Production path: Markdown → `[BlockJson]` JSON string (02 §3.3).
#[wasm_bindgen(js_name = parseBody)]
pub fn parse_body(markdown: &str, fragment: bool) -> Result<String, JsError> {
    wire::parse_body_json(markdown, fragment).map_err(|e| JsError::new(&e.to_string()))
}

/// Stage 2 only: parse, consume the blocks into a cheap digest, build no JSON.
/// The digest reads every block's kind/text and every `data.marks` entry, so
/// the parser output cannot be optimised away.
fn digest(blocks: &[mica_md::Block]) -> f64 {
    let mut sum = blocks.len() as u64;
    for block in blocks {
        sum = sum
            .wrapping_mul(31)
            .wrapping_add(block.kind.len() as u64)
            .wrapping_add(block.text.len() as u64);
        if let Some(marks) = block.data.get("marks").and_then(|m| m.as_array()) {
            sum = sum.wrapping_add(marks.len() as u64);
            for mark in marks {
                sum = sum
                    .wrapping_add(mark.get("start").and_then(|v| v.as_u64()).unwrap_or(0))
                    .wrapping_add(mark.get("end").and_then(|v| v.as_u64()).unwrap_or(0));
            }
        }
    }
    sum as f64
}

#[wasm_bindgen(js_name = parseOnlyDigest)]
pub fn parse_only_digest(markdown: &str) -> f64 {
    digest(&mica_md::parse_body(markdown, ParseMode::Document))
}

/// Matched ID probes: both use `parse`, the same root/mode, and the same
/// digest including the root. The ONLY variable is BlockIds. This measures
/// the ID strategy including uniqueness bookkeeping, not random calls alone.
#[cfg(feature = "uuid")]
#[wasm_bindgen(js_name = parseIdDigest)]
pub fn parse_id_digest(markdown: &str, uuid: bool) -> f64 {
    let doc = mica_md::parse(
        markdown,
        &mica_md::ParseOptions {
            mode: ParseMode::Document,
            root_id: "root".to_string(),
            ids: if uuid {
                mica_md::BlockIds::Uuid
            } else {
                mica_md::BlockIds::Sequential { prefix: "b".to_string() }
            },
        },
    );
    digest(&doc.blocks)
}

#[cfg(all(test, feature = "uuid"))]
mod tests {
    #[test]
    fn matched_id_probes_have_equal_consumed_values() {
        for input in [
            include_str!("../../inputs/p0_87.md"),
            include_str!("../../inputs/p0_10k.md"),
            include_str!("../../inputs/p0_100k.md"),
        ] {
            assert_eq!(super::parse_id_digest(input, false), super::parse_id_digest(input, true));
        }
    }
}

/// Boundary probe: same codec as a real string export (TextEncoder/Decoder,
/// malloc/free), no parsing. Measures the round-trip cost of a payload.
#[wasm_bindgen(js_name = echo)]
pub fn echo(payload: &str) -> String {
    payload.to_string()
}

/// Call-overhead probe: no arguments, no body.
#[wasm_bindgen(js_name = noop)]
pub fn noop() {}

#[wasm_bindgen(js_name = engineInfo)]
pub fn engine_info() -> String {
    wire::engine_info_json()
}
