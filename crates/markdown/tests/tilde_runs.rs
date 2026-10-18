//! A tilde run that does not open strikethrough is literal AS A WHOLE (GFM).
//!
//! The engine used to emit ONE tilde and re-scan the rest as a shorter run, so
//! `x ~~~a~~` came out as `x ~<del>a</del>` and `x ~~a~` struck `a` too —
//! while the Dart mirror, which already wrote the whole run literally, kept
//! them as text. Two engines disagreeing on the same source is exactly what
//! CLAUDE.md principle 2 (Rust authoritative, Dart mirror) exists to prevent.

fn html(md: &str) -> String {
  mica_markdown::export_html(&mica_markdown::import_markdown(md, "root")).unwrap()
}

#[test]
fn an_unopened_tilde_run_stays_literal() {
  assert_eq!(html("x ~~~a~~"), "<p>x ~~~a~~</p>");
  assert_eq!(html("x ~~a~"), "<p>x ~~a~</p>");
  assert_eq!(html("a ~~~b~~~ c"), "<p>a ~~~b~~~ c</p>");
}

#[test]
fn matched_runs_still_strike() {
  assert_eq!(html("~~a~~"), "<p><del>a</del></p>");
  assert_eq!(html("~a~"), "<p><del>a</del></p>");
  assert_eq!(html("1~2 and 3~4"), "<p>1~2 and 3~4</p>", "intraword single tildes stay literal");
}
