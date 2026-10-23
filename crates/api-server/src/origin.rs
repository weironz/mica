//! Same-ORIGIN enforcement for credentials a browser attaches on its own.
//!
//! The session cookie is `SameSite=Strict`, and that was treated as the CSRF
//! defence. SameSite is per SITE, not per origin: the object store answers on
//! the same site (`s3.example.com` beside `example.com`, or `IP:9000` beside
//! `IP`), so a hostile HTML or SVG file served from it is same-site — the
//! browser attaches the cookie to whatever that page sends to the app. And a
//! WebSocket is not covered by CORS at all: such a page could open a document
//! socket as the victim and read every edit (cross-site WebSocket hijacking).
//!
//! So a request carrying an `Origin` header must come from the app's own origin
//! (`MICA_APP_BASE_URL`) or one the operator listed in `CORS_ALLOWED_ORIGINS`.
//! No `Origin` means no browser (the desktop app, the CLI, MCP) — allowed, they
//! hold bearer tokens a hostile page cannot read. Enforced in production only,
//! like CORS: in development the web app runs on a `flutter run` port.

use axum::http::{HeaderMap, header};
use mica_infra::{AppConfig, Environment};

/// `scheme://host[:port]` of a URL, lowercased — the browser's origin form.
fn origin_of(url: &str) -> String {
  let url = url.trim();
  let after_scheme = url.find("://").map_or(0, |i| i + 3);
  let end = url[after_scheme..]
    .find('/')
    .map_or(url.len(), |i| after_scheme + i);
  url[..end].to_ascii_lowercase()
}

/// Whether `headers` come from a page the app trusts (see the module doc).
pub fn origin_allowed(config: &AppConfig, headers: &HeaderMap) -> bool {
  allowed(
    config.environment,
    &config.app_base_url,
    &config.cors_allowed_origins,
    headers,
  )
}

fn allowed(env: Environment, app_base_url: &str, extra: &[String], headers: &HeaderMap) -> bool {
  let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
    return true;
  };
  if env != Environment::Production {
    return true;
  }
  let origin = origin_of(origin);
  let ok = origin == origin_of(app_base_url) || extra.iter().any(|o| origin_of(o) == origin);
  if !ok {
    tracing::warn!(
      %origin,
      app = %origin_of(app_base_url),
      "refused a browser request from a foreign origin; if this is a legitimate \
       address for the app, add it to CORS_ALLOWED_ORIGINS"
    );
  }
  ok
}

#[cfg(test)]
mod tests {
  use super::*;
  use axum::http::HeaderValue;

  fn check(env: Environment, h: &HeaderMap) -> bool {
    allowed(
      env,
      "https://mica.example.com",
      &["https://alias.example.com".to_string()],
      h,
    )
  }

  fn with_origin(o: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(header::ORIGIN, HeaderValue::from_str(o).unwrap());
    h
  }

  /// THE BUG (B4): the storage origin is same-SITE, so its pages carry the
  /// cookie. They must be refused by ORIGIN.
  #[test]
  fn the_storage_origin_is_refused_and_the_app_origin_is_not() {
    let p = Environment::Production;
    assert!(check(p, &with_origin("https://mica.example.com")));
    assert!(check(p, &with_origin("https://MICA.example.com")));
    assert!(check(p, &with_origin("https://alias.example.com")));
    assert!(!check(p, &with_origin("https://s3.mica.example.com")));
    assert!(!check(p, &with_origin("https://mica.example.com:9000")));
    assert!(!check(p, &with_origin("null")));
  }

  #[test]
  fn non_browser_clients_and_development_are_not_affected() {
    assert!(check(Environment::Production, &HeaderMap::new()));
    assert!(check(
      Environment::Development,
      &with_origin("http://localhost:5555")
    ));
  }

  #[test]
  fn origin_of_strips_paths() {
    assert_eq!(
      origin_of("https://Mica.example.com/app/"),
      "https://mica.example.com"
    );
    assert_eq!(origin_of("http://10.0.0.1"), "http://10.0.0.1");
  }
}
