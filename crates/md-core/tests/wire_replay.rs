//! Replay identical committed args in every feature build. Output strings are
//! compared as bytes, never parsed/canonicalized to hide a feature difference.
use std::path::Path;

use mica_md::wire;
use serde_json::Value;

fn replay(function: &str, include: impl Fn(&Value) -> bool) {
  let name = function.strip_suffix("_json").unwrap();
  let path = Path::new(env!("CARGO_MANIFEST_DIR"))
    .join("tests/fixtures/wire")
    .join(format!("{name}.json"));
  let gold: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
  let mut checked = 0;
  for case in gold["cases"]
    .as_array()
    .unwrap()
    .iter()
    .filter(|c| include(c))
  {
    let args = &case["args"];
    let s = |key: &str| args[key].as_str().unwrap();
    let got = match function {
      "parse_body_json" => {
        wire::parse_body_json(s("markdown"), args["fragment"].as_bool().unwrap()).unwrap()
      }
      "serialize_body_json" => {
        wire::serialize_body_json(s("blocks_json"), s("assets_json")).unwrap()
      }
      "render_html_body_json" => {
        wire::render_html_body_json(s("blocks_json"), s("image_srcs_json"), s("profile")).unwrap()
      }
      "parse_inline_batch_json" => wire::parse_inline_batch_json(s("sources_json")).unwrap(),
      "serialize_inline_json" => wire::serialize_inline_json(s("inline_json")).unwrap(),
      "decode_cells_json" => wire::decode_cells_json(s("cells_json")).unwrap(),
      "encode_cells_json" => wire::encode_cells_json(s("inlines_json")).unwrap(),
      "math_run_spans_json" => wire::math_run_spans_json(s("source")),
      "parse_inline_math_json" => wire::parse_inline_math_json(s("source")),
      _ => unreachable!(),
    };
    assert_eq!(
      got.as_bytes(),
      case["output"].as_str().unwrap().as_bytes(),
      "{function}: {}",
      case["id"]
    );
    checked += 1;
  }
  assert!(checked > 0, "{function}: no replay cases");
  println!("{function}: {checked} fixed-args byte comparisons passed");
}

#[test]
fn json_and_markdown_bytes_match_fixed_default_gold() {
  for function in [
    "parse_body_json",
    "serialize_body_json",
    "parse_inline_batch_json",
    "serialize_inline_json",
    "decode_cells_json",
    "encode_cells_json",
    "math_run_spans_json",
    "parse_inline_math_json",
  ] {
    replay(function, |_| true);
  }
}

#[test]
fn clipboard_bytes_match_fixed_default_gold() {
  replay("render_html_body_json", |c| {
    c["args"]["profile"] == "clipboard"
  });
}

// Export intentionally retains feature-dependent rich rendering. It is not a
// cross-feature byte contract: the open Export/ten-function contract conflict
// is recorded separately, rather than normalizing SVG away or claiming parity.
#[cfg(not(feature = "render"))]
#[test]
fn export_bytes_match_default_gold() {
  replay("render_html_body_json", |c| {
    c["args"]["profile"] == "export"
  });
}
