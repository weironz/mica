//! The two pages a running instance has to be able to show a user: what it does
//! with their data, and on what terms it runs.
//!
//! Server-rendered and mounted OUTSIDE `/api`, like the share and email pages —
//! they must be reachable without logging in, and by a plain link someone can
//! send. **Both need an nginx rule** (`deploy/nginx.conf`, `nginx.dev.conf`):
//! without an exact-match `location` proxied ahead of the SPA fallback,
//! `try_files` answers with `index.html` and the page silently becomes the app.
//!
//! The text lives in Markdown next to this file and is rendered with the same
//! engine the product uses, so editing a policy is editing prose — not HTML,
//! and not a Rust string literal. Compiled in with `include_str!` so the binary
//! carries its own terms: a deploy can never leave the pages behind.
use axum::{
  http::header,
  response::{Html, IntoResponse, Response},
  routing::get,
  Router,
};

use crate::AppState;

const PRIVACY_MD: &str = include_str!("../../legal/privacy.md");
const TERMS_MD: &str = include_str!("../../legal/terms.md");

pub fn router() -> Router<AppState> {
  Router::new()
    .route("/privacy", get(privacy))
    .route("/terms", get(terms))
}

async fn privacy() -> Response {
  page("隐私声明", PRIVACY_MD)
}

async fn terms() -> Response {
  page("服务条款", TERMS_MD)
}

/// Render Markdown to a minimal, self-contained page.
///
/// No stylesheet link and no script: a legal page that depends on the app's
/// bundle would break exactly when the app does, which is one of the moments
/// someone might go looking for it.
fn page(title: &str, markdown: &str) -> Response {
  let body = mica_markdown::export_html(&mica_markdown::import_markdown(markdown, "root"))
    .unwrap_or_else(|_| "<p>This page could not be rendered.</p>".to_string());
  let html = format!(
    "<!doctype html><html lang=\"zh\"><head><meta charset=\"utf-8\">\
     <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
     <title>{title} · Mica</title><style>\
     body{{max-width:42rem;margin:3rem auto;padding:0 1.25rem;line-height:1.7;\
     font-family:system-ui,-apple-system,'Segoe UI',Roboto,'Helvetica Neue',sans-serif;\
     color:#1f2937}}\
     h1{{font-size:1.6rem}}h2{{font-size:1.15rem;margin-top:2rem}}\
     a{{color:#2563eb}}code{{background:#f3f4f6;padding:.1em .3em;border-radius:3px}}\
     @media(prefers-color-scheme:dark){{body{{background:#111827;color:#e5e7eb}}\
     a{{color:#93c5fd}}code{{background:#1f2937}}}}\
     </style></head><body>{body}</body></html>"
  );
  (
    [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
    Html(html),
  )
    .into_response()
}

#[cfg(test)]
mod tests {
  use super::*;

  /// The pages must RENDER, not just exist. A Markdown file that fails to parse
  /// would otherwise ship as "This page could not be rendered" and nobody would
  /// look until someone asked for the policy.
  #[test]
  fn both_documents_render_to_html() {
    for (name, md) in [("privacy", PRIVACY_MD), ("terms", TERMS_MD)] {
      let html = mica_markdown::export_html(&mica_markdown::import_markdown(md, "root"))
        .unwrap_or_default();
      assert!(html.contains("<h1"), "{name} lost its heading: {html:.120}");
      assert!(html.len() > 500, "{name} rendered suspiciously short");
    }
  }

  /// The privacy page makes concrete claims that stop being true if the instance
  /// is reconfigured. Pinning the sentences means a reviewer who changes the
  /// posture has to come here — the page cannot quietly keep saying "we don't".
  #[test]
  fn privacy_states_the_claims_the_instance_relies_on() {
    for claim in ["telemetry", "AI 服务商", "opt-in", "阿里云"] {
      assert!(PRIVACY_MD.contains(claim), "privacy.md no longer mentions {claim}");
    }
  }
}
