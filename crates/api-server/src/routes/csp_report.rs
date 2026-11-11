//! `POST /api/csp-report` — where browsers send violations of the web app's
//! `Content-Security-Policy-Report-Only` header (deploy/nginx.conf points the
//! policy's `report-uri` here).
//!
//! The policy is in observation mode: before it is ever enforced, someone has to
//! see what it would have blocked. This endpoint only LOGS a short summary of
//! each report; nothing is stored, and the answer is always 204 so a browser has
//! nothing to retry.
//!
//! It is public and unauthenticated by nature (the browser sends the report, not
//! the app), so it is throttled per IP, its body is capped well below anything a
//! real report needs, and every logged field is truncated and stripped of its
//! query string and fragment — a page URL can carry a share token or a reset
//! link's token, and those must not end up in the logs.
//!
//! Both report shapes are accepted: the legacy `application/csp-report`
//! (`{"csp-report": {...}}`, kebab-case) and the Reporting API's
//! `application/reports+json` (an array of `{type, body: {...}}`, camelCase).
use axum::body::Bytes;
use axum::http::StatusCode;
use serde_json::Value;

/// A real report is well under 2 KiB; the cap only has to stop abuse.
pub const MAX_BODY_BYTES: usize = 16 * 1024;

/// At most this many reports are logged from one request (the Reporting API
/// batches them).
const MAX_REPORTS: usize = 8;

/// Longest value logged per field.
const MAX_FIELD: usize = 200;

pub async fn csp_report(body: Bytes) -> StatusCode {
  for r in summarize(&body) {
    tracing::warn!(
      target: "mica::csp",
      directive = %r.directive,
      blocked = %r.blocked,
      document = %r.document,
      source = %r.source,
      disposition = %r.disposition,
      "csp report-only violation"
    );
  }
  StatusCode::NO_CONTENT
}

#[derive(Debug, PartialEq, Eq)]
pub struct Summary {
  pub directive: String,
  pub blocked: String,
  pub document: String,
  pub source: String,
  pub disposition: String,
}

/// The loggable part of every report in `body`; anything unparseable is
/// dropped silently (a broken report is not worth an error line).
pub fn summarize(body: &[u8]) -> Vec<Summary> {
  let Ok(v) = serde_json::from_slice::<Value>(body) else {
    return Vec::new();
  };
  let bodies: Vec<&Value> = match &v {
    Value::Object(o) => o.get("csp-report").into_iter().collect(),
    Value::Array(items) => items
      .iter()
      .filter(|i| i.get("type").and_then(Value::as_str) == Some("csp-violation"))
      .filter_map(|i| i.get("body"))
      .collect(),
    _ => Vec::new(),
  };
  bodies
    .into_iter()
    .filter(|b| b.is_object())
    .take(MAX_REPORTS)
    .map(|b| {
      let field = |legacy: &str, modern: &str| {
        b.get(legacy)
          .or_else(|| b.get(modern))
          .map(|v| match v {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
          })
          .unwrap_or_default()
      };
      let line = field("line-number", "lineNumber");
      let mut source = scrub_url(&field("source-file", "sourceFile"));
      if !source.is_empty() && !line.is_empty() {
        source = clip(&format!("{source}:{line}"));
      }
      Summary {
        directive: clip(&{
          let d = field("effective-directive", "effectiveDirective");
          if d.is_empty() {
            field("violated-directive", "violatedDirective")
          } else {
            d
          }
        }),
        blocked: scrub_url(&field("blocked-uri", "blockedURL")),
        document: scrub_url(&field("document-uri", "documentURL")),
        source,
        disposition: clip(&field("disposition", "disposition")),
      }
    })
    .collect()
}

/// A URL without its query string or fragment (a share or reset token can sit
/// in either), clipped. Keywords such as `inline` / `eval` pass unchanged.
fn scrub_url(s: &str) -> String {
  let end = s.find(['?', '#']).unwrap_or(s.len());
  clip(&s[..end])
}

/// At most [`MAX_FIELD`] characters with control characters removed, so a log
/// line stays one line.
fn clip(s: &str) -> String {
  s.chars()
    .filter(|c| !c.is_control())
    .take(MAX_FIELD)
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn legacy_report_is_summarized_without_tokens() {
    let body = br#"{"csp-report":{
      "document-uri":"https://mica.example/s/abc?token=SECRET#frag",
      "violated-directive":"script-src-elem",
      "effective-directive":"script-src-elem",
      "blocked-uri":"https://evil.example/x.js?k=SECRET",
      "source-file":"https://mica.example/main.dart.js","line-number":12,
      "disposition":"report"}}"#;
    let s = summarize(body);
    assert_eq!(
      s,
      vec![Summary {
        directive: "script-src-elem".into(),
        blocked: "https://evil.example/x.js".into(),
        document: "https://mica.example/s/abc".into(),
        source: "https://mica.example/main.dart.js:12".into(),
        disposition: "report".into(),
      }]
    );
  }

  #[test]
  fn reporting_api_batches_keep_only_csp_entries() {
    let body = br#"[
      {"type":"csp-violation","body":{"documentURL":"https://m/x?y",
        "effectiveDirective":"img-src","blockedURL":"inline","disposition":"report"}},
      {"type":"deprecation","body":{"id":"x"}}
    ]"#;
    let s = summarize(body);
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].directive, "img-src");
    assert_eq!(s[0].blocked, "inline");
    assert_eq!(s[0].document, "https://m/x");
  }

  #[test]
  fn garbage_and_floods_are_bounded() {
    assert!(summarize(b"not json").is_empty());
    assert!(summarize(b"{\"csp-report\":3}").is_empty());
    let one = r#"{"type":"csp-violation","body":{"effectiveDirective":"a"}}"#;
    let many = format!("[{}]", vec![one; 50].join(","));
    assert_eq!(summarize(many.as_bytes()).len(), MAX_REPORTS);
    let long = format!(
      r#"{{"csp-report":{{"blocked-uri":"{}\n"}}}}"#,
      "a".repeat(5000)
    );
    let s = summarize(long.as_bytes());
    assert_eq!(s[0].blocked.chars().count(), MAX_FIELD);
  }
}
