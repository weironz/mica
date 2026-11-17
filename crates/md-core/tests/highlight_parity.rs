//! Parity with the Dart highlighter this module replaced (gh#178): the fixture
//! was recorded from `lib/editor/highlight.dart` over its whole test corpus —
//! every language on every sample, plus detection — before the port.

use mica_md::highlight::{
  Role, canonical_code_language, detect_code_language, highlight_code, resolve_code_language,
  strong_language_signature,
};
use serde_json::Value;

const ROLES: [Role; 6] = [
  Role::Keyword,
  Role::String,
  Role::Comment,
  Role::Number,
  Role::Function,
  Role::Key,
];

fn fixture() -> Value {
  serde_json::from_str(include_str!("fixtures/highlight_parity.json")).unwrap()
}

#[test]
fn tokens_match_the_dart_highlighter() {
  let f = fixture();
  let cases = f["highlight"].as_array().unwrap();
  assert!(cases.len() > 3000);
  let mut failures = Vec::new();
  for case in cases {
    let code = case[0].as_str().unwrap();
    let lang = case[1].as_str().unwrap();
    let want: Vec<(u32, u32, Role)> = case[2]
      .as_array()
      .unwrap()
      .iter()
      .map(|t| {
        (
          t[0].as_u64().unwrap() as u32,
          t[1].as_u64().unwrap() as u32,
          ROLES[t[2].as_u64().unwrap() as usize],
        )
      })
      .collect();
    let got: Vec<(u32, u32, Role)> = highlight_code(code, lang)
      .into_iter()
      .map(|t| (t.start, t.end, t.role))
      .collect();
    if got != want {
      failures.push(format!("{lang} {code:?}\n  want {want:?}\n  got  {got:?}"));
    }
  }
  assert!(
    failures.is_empty(),
    "{} mismatches:\n{}",
    failures.len(),
    failures
      .iter()
      .take(8)
      .cloned()
      .collect::<Vec<_>>()
      .join("\n")
  );
}

#[test]
fn detection_matches_the_dart_highlighter() {
  let f = fixture();
  let mut failures = Vec::new();
  for case in f["detect"].as_array().unwrap() {
    let code = case[0].as_str().unwrap();
    let got = (
      detect_code_language(code),
      strong_language_signature(code).map(str::to_string),
      resolve_code_language(code, None),
      resolve_code_language(code, Some("py")),
    );
    let want = (
      case[1].as_str().unwrap().to_string(),
      case[2].as_str().map(str::to_string),
      case[3].as_str().unwrap().to_string(),
      case[4].as_str().unwrap().to_string(),
    );
    if got != want {
      failures.push(format!("{code:?}\n  want {want:?}\n  got  {got:?}"));
    }
  }
  assert!(
    failures.is_empty(),
    "{} mismatches:\n{}",
    failures.len(),
    failures.join("\n")
  );
  for (name, canonical) in f["canonical"].as_object().unwrap() {
    assert_eq!(
      canonical_code_language(name),
      canonical.as_str().unwrap(),
      "{name}"
    );
  }
}
