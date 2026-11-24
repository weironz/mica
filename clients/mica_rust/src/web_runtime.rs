//! Browser platform helpers for the pure-Rust WebAssembly (`wasm32`) target.
//!
//! Isolates `web_sys` calls behind `#[cfg(target_arch = "wasm32")]` so the rest
//! of `mica_rust` compiles cleanly on native desktop targets with zero browser
//! dependencies while Web runs as a cloud-only, same-origin client backed by
//! `window.localStorage` and `XMLHttpRequest`.

/// Returns `true` when compiled for WebAssembly (`wasm32`), where Mica runs as
/// a pure cloud client with no on-device local workspace.
pub const fn is_web() -> bool {
  cfg!(target_arch = "wasm32")
}

/// Returns the browser page's canonical origin (e.g. `https://mica.example.com`
/// or `http://127.0.0.1:8090`) on `wasm32`, or `None` on native desktop.
pub fn web_origin() -> Option<String> {
  #[cfg(target_arch = "wasm32")]
  {
    let origin = web_sys::window()?.location().origin().ok()?;
    let trimmed = origin.trim().trim_end_matches('/');
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("null") {
      None
    } else {
      Some(trimmed.to_string())
    }
  }
  #[cfg(not(target_arch = "wasm32"))]
  {
    None
  }
}

/// Read a key from `window.localStorage` on `wasm32` (always `None` on desktop).
pub fn local_storage_get(key: &str) -> Option<String> {
  #[cfg(target_arch = "wasm32")]
  {
    let storage = web_sys::window()?.local_storage().ok()??;
    storage.get_item(key).ok()?
  }
  #[cfg(not(target_arch = "wasm32"))]
  {
    let _ = key;
    None
  }
}

/// Write a key to `window.localStorage` on `wasm32` (no-op on desktop).
pub fn local_storage_set(key: &str, value: &str) -> Result<(), String> {
  #[cfg(target_arch = "wasm32")]
  {
    let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) else {
      return Ok(());
    };
    storage
      .set_item(key, value)
      .map_err(|e| format!("failed to write localStorage[{key}]: {e:?}"))
  }
  #[cfg(not(target_arch = "wasm32"))]
  {
    let _ = (key, value);
    Ok(())
  }
}

/// Execute a same-origin / CORS HTTP request via `XMLHttpRequest` on `wasm32`.
///
/// Uses synchronous XHR (`async = false`) so the existing synchronous
/// `CloudConnector` API works identically across Desktop (`ureq`) and Web
/// without duplicating endpoint codecs. Deliberately avoids setting `timeout`
/// or `responseType` on synchronous XHR per the browser XHR specification.
#[cfg(target_arch = "wasm32")]
pub fn xhr_request(
  method: &str,
  url: &str,
  bearer_token: Option<&str>,
  json_body: Option<&str>,
) -> Result<String, String> {
  let xhr = web_sys::XmlHttpRequest::new()
    .map_err(|e| format!("failed to construct XMLHttpRequest: {e:?}"))?;
  xhr
    .open_with_async(method, url, false)
    .map_err(|e| format!("{method} {url} open failed: {e:?}"))?;
  let _ = xhr.set_request_header("Accept", "application/json");
  if let Some(tok) = bearer_token.map(str::trim).filter(|t| !t.is_empty()) {
    let _ = xhr.set_request_header("Authorization", &format!("Bearer {tok}"));
  }
  if json_body.is_some() {
    let _ = xhr.set_request_header("Content-Type", "application/json");
  }
  xhr
    .send_with_opt_str(json_body)
    .map_err(|e| format!("{method} {url} network error: {e:?}"))?;

  let status = xhr.status().unwrap_or(0);
  let text = xhr.response_text().ok().flatten().unwrap_or_default();
  if (200..300).contains(&status) {
    return Ok(text);
  }

  let detail = extract_server_error_message(&text);
  if detail.is_empty() {
    Err(format!("HTTP {status}"))
  } else {
    Err(format!("HTTP {status}: {detail}"))
  }
}

#[cfg(any(target_arch = "wasm32", test))]
fn extract_server_error_message(raw_body: &str) -> String {
  let trimmed = raw_body.trim();
  if trimmed.is_empty() {
    return String::new();
  }
  if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
    for key in ["error", "message", "detail"] {
      if let Some(msg) = val.get(key).and_then(serde_json::Value::as_str) {
        let m = msg.trim();
        if !m.is_empty() {
          return m.to_string();
        }
      }
    }
  }
  trimmed.chars().take(240).collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn extract_server_error_reads_json_error_or_plain_text() {
    assert_eq!(
      extract_server_error_message(r#"{"error":"Invalid email or password"}"#),
      "Invalid email or password"
    );
    assert_eq!(
      extract_server_error_message(r#"{"message":"Unauthorized"}"#),
      "Unauthorized"
    );
    assert_eq!(extract_server_error_message("bad gateway"), "bad gateway");
    assert_eq!(extract_server_error_message("   "), "");
  }

  #[test]
  fn desktop_reports_non_web_runtime() {
    assert!(!is_web());
    assert!(web_origin().is_none());
  }
}
