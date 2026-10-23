//! Shared machinery for the DTO gold-file contract tests (#85).
//!
//! The server, the CLI, the MCP server and the Flutter client each decode the
//! same REST payloads with their own hand-written types — no shared crate
//! exists, deliberately (the web client needs a Dart implementation, so a Rust
//! crate could only ever cover three of the four). The drift guard is this
//! directory's gold files: the server types SERIALISE into them, and every
//! client's tests DESERIALISE out of them, so a renamed field breaks all four
//! test suites at once instead of breaking one client in production.
//!
//! Regenerating after an intentional shape change:
//!
//! ```text
//! MICA_UPDATE_GOLDEN=1 cargo test -p mica-api-server contract_gold
//! ```
//!
//! …and then updating the cli / mcp / Dart tests to match, in the same commit.

/// Compare a serialised DTO against `contracts/{name}.json`, or write that
/// file when `MICA_UPDATE_GOLDEN` is set.
///
/// `name` is also the human handle in the failure message — keep it stable;
/// it is how a red CI run finds the file to regenerate.
pub(crate) fn check(name: &str, value: &serde_json::Value) {
  let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
    .join("../../contracts")
    .join(format!("{name}.json"));
  let rendered = serde_json::to_string_pretty(value).expect("serialising DTO");
  if std::env::var("MICA_UPDATE_GOLDEN").is_ok_and(|v| !v.is_empty()) {
    if let Some(dir) = path.parent() {
      std::fs::create_dir_all(dir).expect("creating contracts dir");
    }
    std::fs::write(&path, format!("{rendered}\n")).expect("writing gold file");
    return;
  }
  let gold = std::fs::read_to_string(&path).unwrap_or_else(|error| {
    panic!(
      "reading {}: {error} — the gold files are committed, not generated on \
       the fly. Run `MICA_UPDATE_GOLDEN=1 cargo test -p mica-api-server \
       contract_gold` to create or refresh them",
      path.display()
    )
  });
  assert_eq!(
    rendered,
    gold.trim_end(),
    "{name}: the server DTO drifted from contracts/{name}.json. If the change \
     is intentional, regenerate the gold AND update the cli / mcp / Dart \
     contract tests in the same commit — each of them fails on this file too"
  );
}
