//! T2.9: shared golden vectors for native/wasm binding contract tests.
//!
//! Generate: `GEN_GOLD=1 cargo test -p mica-md --test wire_gold`.
//! Verify: `cargo test -p mica-md --test wire_gold`.
//!
//! Each file has `function` and `cases`; each case has `id`, named `args` and
//! the exact returned `output` string (JSON outputs are deliberately not parsed
//! or reordered). Binding tests can replay args without knowing Rust's model.
//! Gold is generated without rich rendering. Verification replays the stored
//! args byte-for-byte in both builds, including Clipboard HTML. Export HTML
//! keeps its established feature-dependent rendering and is verified against
//! this default gold only without `render`; it remains an explicit contract
//! exception rather than being normalized away.

use std::path::{Path, PathBuf};

use mica_md::wire;
use serde_json::{Value, json};

fn gold_dir() -> PathBuf {
  Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wire")
}

fn corpus() -> Vec<(String, String)> {
  let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../markdown/tests/fixtures/conformance");
  let mut paths: Vec<_> = std::fs::read_dir(dir)
    .expect("conformance fixtures")
    .map(|entry| entry.expect("fixture entry").path())
    .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
    .collect();
  paths.sort();
  assert!(!paths.is_empty(), "conformance corpus must not be empty");
  let mut cases: Vec<_> = paths
    .into_iter()
    .map(|path| {
      (
        path.file_name().unwrap().to_string_lossy().into_owned(),
        std::fs::read_to_string(path).expect("read fixture"),
      )
    })
    .collect();
  cases.push(("empty".into(), String::new()));
  cases.push(("unicode".into(), "中文😀𠀀 **粗体**\n\n$x$".into()));
  cases.push(("front-matter".into(), "---\ntags: [a]\n---\nbody\n".into()));
  cases
}

fn inline_sources() -> Vec<&'static str> {
  vec![
    "",
    "plain 中文 😀 𠀀",
    "**bold** and *italic* and ~~strike~~ and `code`",
    "**a `b`**",
    "[link](https://example.com \"title\") and [](/empty)",
    "![alt](image.png \"title\")",
    r"literal \*star\* &amp; &#128512;",
    "line  \nbreak",
    "https://example.com www.example.com a@b.co",
    "中文😀 $x^2$ and \\(a+b\\) 𠀀",
    "price $5 and $10; $$display$$",
    "`$HOME` and $x$",
    "text[^1] <em>html</em>",
  ]
}

fn inline_values() -> Vec<Value> {
  vec![
    json!({"text": "", "marks": []}),
    json!({"text": "literal *x* | \\ [x] $", "marks": []}),
    json!({"text": "https://example.com", "marks": []}),
    json!({"text": "x ", "marks": []}),
    json!({"text": "a\nb", "marks": []}),
    json!({"text": "😀𠀀bold", "marks": [{"start":4,"end":8,"type":"bold"}]}),
    json!({"text": "abcdef", "marks": [
      {"start":4,"end":6,"type":"bold"},
      {"start":0,"end":1,"type":"italic"},
      {"start":2,"end":3,"type":"link","href":"https://x.y","title":"t"}
    ]}),
    json!({"text": "linked code", "marks": [
      {"start":0,"end":6,"type":"link","href":"https://x.y"},
      {"start":0,"end":6,"type":"code"}
    ]}),
    json!({"text": "a b c", "marks": [
      {"start":0,"end":1,"type":"strike"},
      {"start":2,"end":3,"type":"math"}
    ]}),
  ]
}

/// Sort only fixture container objects. In verification mode, stored args are
/// replayed verbatim rather than regenerated using feature-dependent json!
/// insertion order. Output strings are never parsed or normalized.
fn canonical(value: &Value) -> Value {
  match value {
    Value::Object(map) => {
      let mut keys: Vec<&String> = map.keys().collect();
      keys.sort();
      let entries: Vec<(String, Value)> = keys
        .into_iter()
        .map(|key| (key.clone(), canonical(&map[key])))
        .collect();
      // Building through a BTreeMap keeps an ordinary Map BTree-backed under
      // preserve_order and preserves our explicit sort order under every build.
      let mut out = serde_json::Map::new();
      for (key, value) in entries {
        out.insert(key.clone(), value);
      }
      Value::Object(out)
    }
    Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
    other => other.clone(),
  }
}

fn case(id: impl Into<String>, args: Value, output: String) -> Value {
  json!({"id": id.into(), "args": args, "output": output})
}

fn verify(function: &str, cases: Vec<Value>) {
  assert!(!cases.is_empty(), "{function}: no vectors");
  let value = json!({"function": function, "cases": cases});
  let text = format!(
    "{}\n",
    serde_json::to_string_pretty(&canonical(&value)).unwrap()
  );
  let name = function.strip_suffix("_json").unwrap_or(function);
  let path = gold_dir().join(format!("{name}.json"));
  if std::env::var("GEN_GOLD").as_deref() == Ok("1") {
    if cfg!(feature = "render") {
      panic!("generate shared gold without --features render");
    }
    std::fs::create_dir_all(gold_dir()).expect("create wire fixture directory");
    std::fs::write(&path, text).expect("write wire gold");
    println!(
      "generated {function}: {} cases",
      value["cases"].as_array().unwrap().len()
    );
  } else {
    let expected = std::fs::read_to_string(&path)
      .unwrap_or_else(|error| panic!("read {}: {error}; generate with GEN_GOLD=1", path.display()));
    let expected: Value = serde_json::from_str(&expected).unwrap();
    assert_eq!(expected["function"], function);
    let stored = expected["cases"].as_array().unwrap();
    assert_eq!(stored.len(), value["cases"].as_array().unwrap().len());
    for (case, current) in stored.iter().zip(value["cases"].as_array().unwrap()) {
      assert_eq!(case["id"], current["id"], "fixture inventory drift");
      for key in ["markdown", "fragment", "source", "profile", "request_json"] {
        assert_eq!(
          case["args"][key], current["args"][key],
          "fixture input drift: {} {key}",
          case["id"]
        );
      }
      #[cfg(feature = "render")]
      if function == "render_html_body_json" && case["args"]["profile"] == "export" {
        // §2 Export intentionally uses rich math/Mermaid in this feature.
        // Do not compare it to a no-render gold or claim cross-feature parity.
        continue;
      }
      let args = &case["args"];
      let s = |key: &str| args[key].as_str().unwrap();
      let output = match function {
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
        "properties_json" => wire::properties_json(s("request_json")).unwrap(),
        _ => unreachable!(),
      };
      assert_eq!(
        output.as_bytes(),
        case["output"].as_str().unwrap().as_bytes(),
        "wire byte drift {function}: {}",
        case["id"]
      );
    }
  }
}

#[test]
fn parse_body_json() {
  let mut cases = Vec::new();
  for (id, markdown) in corpus() {
    for fragment in [false, true] {
      cases.push(case(
        format!("{id}/{}", if fragment { "fragment" } else { "document" }),
        json!({"markdown": markdown, "fragment": fragment}),
        wire::parse_body_json(&markdown, fragment).unwrap(),
      ));
    }
  }
  verify("parse_body_json", cases);
}

#[test]
fn serialize_body_json() {
  let mut cases = Vec::new();
  for (id, markdown) in corpus() {
    for fragment in [false, true] {
      let blocks_json = wire::parse_body_json(&markdown, fragment).unwrap();
      cases.push(case(
        format!("{id}/{}", if fragment { "fragment" } else { "document" }),
        json!({"blocks_json": blocks_json, "assets_json": "{}"}),
        wire::serialize_body_json(&blocks_json, "{}").unwrap(),
      ));
    }
  }
  let blocks = r#"[{"type":"image","text":"alt","data":{"file_id":"asset","url":"old.png"}}]"#;
  let assets = r#"{"asset":"assets/photo.png"}"#;
  cases.push(case(
    "asset-rewrite",
    json!({"blocks_json":blocks,"assets_json":assets}),
    wire::serialize_body_json(blocks, assets).unwrap(),
  ));
  verify("serialize_body_json", cases);
}

#[test]
fn render_html_body_json() {
  let mut cases = Vec::new();
  for (id, markdown) in corpus() {
    for fragment in [false, true] {
      let blocks_json = wire::parse_body_json(&markdown, fragment).unwrap();
      for profile in ["export", "clipboard"] {
        cases.push(case(
          format!(
            "{id}/{}/{profile}",
            if fragment { "fragment" } else { "document" }
          ),
          json!({"blocks_json":blocks_json,"image_srcs_json":"{}","profile":profile}),
          wire::render_html_body_json(&blocks_json, "{}", profile).unwrap(),
        ));
      }
    }
  }
  let blocks = r#"[{"type":"image","text":"alt","data":{"file_id":"asset","url":"old.png"}},{"type":"paragraph","text":"","data":null}]"#;
  let srcs = r#"{"asset":"https://example.com/photo.png"}"#;
  for profile in ["export", "clipboard"] {
    cases.push(case(
      format!("image-src/{profile}"),
      json!({"blocks_json":blocks,"image_srcs_json":srcs,"profile":profile}),
      wire::render_html_body_json(blocks, srcs, profile).unwrap(),
    ));
  }
  verify("render_html_body_json", cases);
}

#[test]
fn parse_inline_batch_json() {
  let mut cases = Vec::new();
  for (i, source) in inline_sources().iter().enumerate() {
    let sources_json = json!([source]).to_string();
    cases.push(case(
      format!("inline-{i}"),
      json!({"sources_json":sources_json}),
      wire::parse_inline_batch_json(&sources_json).unwrap(),
    ));
  }
  for (id, sources_json) in [
    ("empty-batch", "[]".to_string()),
    ("all", json!(inline_sources()).to_string()),
  ] {
    cases.push(case(
      id,
      json!({"sources_json":sources_json}),
      wire::parse_inline_batch_json(&sources_json).unwrap(),
    ));
  }
  verify("parse_inline_batch_json", cases);
}

#[test]
fn serialize_inline_json() {
  let cases = inline_values()
    .into_iter()
    .enumerate()
    .map(|(i, inline)| {
      let inline_json = inline.to_string();
      case(
        format!("inline-{i}"),
        json!({"inline_json":inline_json}),
        wire::serialize_inline_json(&inline_json).unwrap(),
      )
    })
    .collect();
  verify("serialize_inline_json", cases);
}

#[test]
fn decode_cells_json() {
  let mut cells: Vec<Value> = inline_sources().into_iter().map(|src| json!(src)).collect();
  cells.extend(inline_values());
  cells.extend([
    Value::Null,
    json!(17),
    json!({}),
    json!({"text":"legacy","marks":[]}),
  ]);
  let mut cases = Vec::new();
  for (i, cell) in cells.iter().enumerate() {
    let cells_json = json!([cell]).to_string();
    cases.push(case(
      format!("cell-{i}"),
      json!({"cells_json":cells_json}),
      wire::decode_cells_json(&cells_json).unwrap(),
    ));
  }
  for (id, cells_json) in [
    ("empty-batch", "[]".to_string()),
    ("all", json!(cells).to_string()),
  ] {
    cases.push(case(
      id,
      json!({"cells_json":cells_json}),
      wire::decode_cells_json(&cells_json).unwrap(),
    ));
  }
  verify("decode_cells_json", cases);
}

#[test]
fn encode_cells_json() {
  let mut cases = Vec::new();
  for (i, inline) in inline_values().iter().enumerate() {
    let inlines_json = json!([inline]).to_string();
    cases.push(case(
      format!("inline-{i}"),
      json!({"inlines_json":inlines_json}),
      wire::encode_cells_json(&inlines_json).unwrap(),
    ));
  }
  for (id, inlines_json) in [
    ("empty-batch", "[]".to_string()),
    ("all", json!(inline_values()).to_string()),
  ] {
    cases.push(case(
      id,
      json!({"inlines_json":inlines_json}),
      wire::encode_cells_json(&inlines_json).unwrap(),
    ));
  }
  verify("encode_cells_json", cases);
}

#[test]
fn math_run_spans_json() {
  verify(
    "math_run_spans_json",
    inline_sources()
      .into_iter()
      .enumerate()
      .map(|(i, source)| {
        case(
          format!("math-{i}"),
          json!({"source":source}),
          wire::math_run_spans_json(source),
        )
      })
      .collect(),
  );
}

#[test]
fn parse_inline_math_json() {
  verify(
    "parse_inline_math_json",
    inline_sources()
      .into_iter()
      .enumerate()
      .map(|(i, source)| {
        case(
          format!("math-{i}"),
          json!({"source":source}),
          wire::parse_inline_math_json(source),
        )
      })
      .collect(),
  );
}

/// Front-matter properties (02 §3.3). The inputs are the ones the retired Dart
/// mirror's tests pinned (`properties_test.dart`), so behaviour both sides
/// agreed on stays pinned at the contract.
#[test]
fn properties_json() {
  let front_matters = [
    (
      "typed-subset",
      "title: My Page\ntags: [work, urgent]\ncount: 3\nratio: 1.5\ndone: true\ndue: 2026-07-22\nauthors:\n  - Alice\n  - Bob",
    ),
    (
      "skips-unknown",
      "# a comment\ntitle: Hi\n\nnested:\n  child: 1\nafter: ok",
    ),
    ("ambiguous-007", "a: 007"),
    ("ambiguous-1.0", "a: 1.0"),
    ("quoted-number", "a: \"3\""),
    ("tags", "tags: [rust, crdt]"),
    ("empty", ""),
    ("unicode", "标题: 中文😀\n标签: [甲, 乙]"),
  ];
  let mut cases = Vec::new();
  let mut push = |id: String, request: Value| {
    let request_json = request.to_string();
    let output = wire::properties_json(&request_json).unwrap();
    cases.push(case(id, json!({ "request_json": request_json }), output));
  };
  for (id, fm) in front_matters {
    push(
      format!("parse/{id}"),
      json!({"op": "parse", "front_matter": fm}),
    );
  }
  let edits = [
    (
      "keeps-others",
      "# keep me\ntitle: Old  # trailing comment kept\ntags: [a, b]\nnote: 'single quoted'",
      "tags",
      json!({"type": "list", "value": ["a", "b", "c"]}),
    ),
    (
      "block-list",
      "authors:\n  - Alice\ntitle: X",
      "authors",
      json!({"type": "list", "value": ["Alice", "Bob"]}),
    ),
    (
      "append-checkbox",
      "title: X",
      "done",
      json!({"type": "checkbox", "value": true}),
    ),
    (
      "append-to-empty",
      "",
      "title",
      json!({"type": "text", "value": "First"}),
    ),
    (
      "append-number",
      "a: 1\n",
      "b",
      json!({"type": "number", "value": 2.0}),
    ),
    (
      "needs-quoting",
      "",
      "s",
      json!({"type": "text", "value": "needs: quoting"}),
    ),
    (
      "text-true",
      "",
      "s",
      json!({"type": "text", "value": "true"}),
    ),
    (
      "negative",
      "",
      "n",
      json!({"type": "number", "value": -2.5}),
    ),
    (
      "date",
      "",
      "d",
      json!({"type": "date", "value": "2026-01-09"}),
    ),
    (
      "list-quoting",
      "",
      "l",
      json!({"type": "list", "value": ["x", "y, z", "true"]}),
    ),
    ("empty-list", "", "l", json!({"type": "list", "value": []})),
  ];
  for (id, fm, key, value) in edits {
    push(
      format!("upsert/{id}"),
      json!({"op": "upsert", "front_matter": fm, "key": key, "value": value}),
    );
  }
  for (id, key) in [("block", "authors"), ("missing", "missing")] {
    push(
      format!("remove/{id}"),
      json!({"op": "remove", "front_matter": "title: X\nauthors:\n  - Alice\n  - Bob\ndone: true", "key": key}),
    );
  }
  for raw in [
    "",
    "  ",
    "hello",
    "42",
    "007",
    "1.5",
    "true",
    "False",
    "2026-07-22",
    "2026-13-01",
    "[a, b]",
    "\"quoted\"",
  ] {
    push(format!("infer/{raw:?}"), json!({"op": "infer", "raw": raw}));
  }
  verify("properties_json", cases);
}
