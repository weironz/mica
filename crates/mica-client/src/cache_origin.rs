//! Account-scoped cloud cache origin keys (`#111`).
//!
//! Matches `main:clients/mica_flutter/lib/local/cache_stats.dart` byte-for-byte
//! so desktop upgrades from Flutter 0.14.11 read and replay the exact same
//! `store.db` outbox & cached rows, two accounts on the same server never share
//! cache rows, and pre-#111 bare-URL rows remain safely quarantined.

use thiserror::Error;

/// Errors returned when constructing a cloud cache origin key.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CacheOriginError {
  #[error("a cloud cache key needs an absolute http(s) server URL: {0}")]
  InvalidServerUrl(String),
  #[error("cloud cache is account-scoped; refusing an empty account id")]
  EmptyAccountId,
  #[error("cloud cache is unavailable while signed out")]
  SignedOut,
}

/// Canonical form of a server base URL, used as the first element of
/// [`cloud_cache_origin`] and as the pre-#111 quarantine key.
///
/// Rules match `main`'s Dart `normalizeServerUrl` byte-for-byte:
/// - Requires absolute `http://` or `https://` scheme and non-empty host.
/// - Lowercases scheme and host.
/// - Strips default ports (`http:80`, `https:443`); preserves non-default ports.
/// - Strips all trailing `/` characters from the path; preserves remaining path.
/// - Drops query/fragment if present (matching `uri.path` in Dart).
pub fn normalize_server_url(server_url: &str) -> Result<String, CacheOriginError> {
  let trimmed = server_url.trim();
  let Some(scheme_end) = trimmed.find("://") else {
    return Err(CacheOriginError::InvalidServerUrl(server_url.to_string()));
  };
  let scheme = trimmed[..scheme_end].to_ascii_lowercase();
  if scheme != "http" && scheme != "https" {
    return Err(CacheOriginError::InvalidServerUrl(server_url.to_string()));
  }

  let after_scheme = &trimmed[scheme_end + 3..];
  let authority_end = after_scheme
    .find(['/', '?', '#'])
    .unwrap_or(after_scheme.len());
  let authority = &after_scheme[..authority_end];
  if authority.is_empty() || authority.contains('@') {
    return Err(CacheOriginError::InvalidServerUrl(server_url.to_string()));
  }

  let (host_raw, port_opt) = if let Some(rest) = authority.strip_prefix('[') {
    let Some(close) = rest.find(']') else {
      return Err(CacheOriginError::InvalidServerUrl(server_url.to_string()));
    };
    let ipv6 = &rest[..close];
    let after_bracket = &rest[close + 1..];
    let port = if after_bracket.is_empty() {
      None
    } else if let Some(p) = after_bracket.strip_prefix(':') {
      Some(parse_port(p, server_url)?)
    } else {
      return Err(CacheOriginError::InvalidServerUrl(server_url.to_string()));
    };
    (format!("[{}]", ipv6.to_ascii_lowercase()), port)
  } else if let Some((h, p)) = authority.rsplit_once(':') {
    (h.to_ascii_lowercase(), Some(parse_port(p, server_url)?))
  } else {
    (authority.to_ascii_lowercase(), None)
  };

  if host_raw.is_empty()
    || host_raw == "[]"
    || host_raw
      .bytes()
      .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
  {
    return Err(CacheOriginError::InvalidServerUrl(server_url.to_string()));
  }

  let port_suffix = match port_opt {
    Some(80) if scheme == "http" => String::new(),
    Some(443) if scheme == "https" => String::new(),
    Some(p) => format!(":{p}"),
    None => String::new(),
  };

  let remainder = &after_scheme[authority_end..];
  let path_end = remainder.find(['?', '#']).unwrap_or(remainder.len());
  let mut path = remainder[..path_end].to_string();
  while path.ends_with('/') {
    path.pop();
  }

  Ok(format!("{scheme}://{host_raw}{port_suffix}{path}"))
}

fn parse_port(raw: &str, full_url: &str) -> Result<u16, CacheOriginError> {
  if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
    return Err(CacheOriginError::InvalidServerUrl(full_url.to_string()));
  }
  raw
    .parse::<u16>()
    .ok()
    .filter(|&p| p > 0)
    .ok_or_else(|| CacheOriginError::InvalidServerUrl(full_url.to_string()))
}

/// Account-scoped local cache origin key: compact JSON tuple
/// `["<normalized_server_url>","<account_id>"]`.
///
/// Identical to Dart's `jsonEncode([normalizeServerUrl(serverUrl), account])`
/// so Flutter 0.14.11 offline edits and pure-Rust replicas share the exact same
/// SQLite `origin` namespace.
pub fn cloud_cache_origin(server_url: &str, account_id: &str) -> Result<String, CacheOriginError> {
  let account = account_id.trim();
  if account.is_empty() {
    return Err(CacheOriginError::EmptyAccountId);
  }
  let normalized = normalize_server_url(server_url)?;
  serde_json::to_string(&[normalized.as_str(), account])
    .map_err(|_| CacheOriginError::InvalidServerUrl(server_url.to_string()))
}

/// Require an authenticated `account_id` before producing a cloud cache key
/// (mirrors `requireCloudCacheOrigin` in `main`).
pub fn require_cloud_cache_origin(
  server_url: &str,
  account_id: Option<&str>,
) -> Result<String, CacheOriginError> {
  match account_id.map(str::trim).filter(|s| !s.is_empty()) {
    Some(id) => cloud_cache_origin(server_url, id),
    None => Err(CacheOriginError::SignedOut),
  }
}

/// Pre-#111 bare server URL cache key (for quarantine/migration inspection
/// ONLY — live cloud cache reads/writes must NEVER use this key).
pub fn legacy_cloud_cache_origin(server_url: &str) -> Result<String, CacheOriginError> {
  normalize_server_url(server_url)
}

/// Return `true` iff `origin` is a valid `#111` account-scoped JSON tuple key
/// (`["http(s)://...","<non-empty-account>"]`).
pub fn is_account_scoped_cloud_origin(origin: &str) -> bool {
  let Ok(tuple) = serde_json::from_str::<[String; 2]>(origin) else {
    return false;
  };
  !tuple[1].trim().is_empty() && normalize_server_url(&tuple[0]).is_ok()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn normalize_server_url_matches_flutter_dart_byte_for_byte() {
    assert_eq!(
      normalize_server_url("https://Mica.Example.com:443/mica/").unwrap(),
      "https://mica.example.com/mica"
    );
    assert_eq!(
      normalize_server_url("  HTTP://LOCALHOST:80///  ").unwrap(),
      "http://localhost"
    );
    assert_eq!(
      normalize_server_url("http://127.0.0.1:8080/").unwrap(),
      "http://127.0.0.1:8080"
    );
    assert_eq!(
      normalize_server_url("https://notes.example.com:8443/sub/path///").unwrap(),
      "https://notes.example.com:8443/sub/path"
    );
    assert_eq!(
      normalize_server_url("https://notes.example.com/app?foo=bar#hash").unwrap(),
      "https://notes.example.com/app"
    );
    assert_eq!(
      normalize_server_url("http://[::1]:8080/").unwrap(),
      "http://[::1]:8080"
    );

    // Reject malformed, non-http(s), or empty inputs
    assert!(normalize_server_url("").is_err());
    assert!(normalize_server_url("local").is_err());
    assert!(normalize_server_url("ftp://mica.example.com").is_err());
    assert!(normalize_server_url("mica.example.com").is_err());
    assert!(normalize_server_url("https://").is_err());
    assert!(normalize_server_url("https://user@mica.example.com").is_err());
    assert!(normalize_server_url("https://mica.example.com:0").is_err());
    assert!(normalize_server_url("https://mica.example.com:99999").is_err());
  }

  #[test]
  fn cloud_cache_origin_matches_dart_json_encode_and_isolates_accounts() {
    let key_a = cloud_cache_origin("https://Mica.Example.com:443/", "user-a").unwrap();
    let key_b = cloud_cache_origin("https://mica.example.com", "user-b").unwrap();
    let key_a2 = cloud_cache_origin("https://mica.example.com///", "  user-a  ").unwrap();

    // Byte-for-byte match with Dart `jsonEncode(['https://mica.example.com', 'user-a'])`
    assert_eq!(key_a, r#"["https://mica.example.com","user-a"]"#);
    assert_eq!(key_a, key_a2);
    assert_ne!(key_a, key_b);
    assert_ne!(
      key_a,
      legacy_cloud_cache_origin("https://mica.example.com").unwrap()
    );

    assert!(is_account_scoped_cloud_origin(&key_a));
    assert!(!is_account_scoped_cloud_origin("https://mica.example.com"));
    assert!(!is_account_scoped_cloud_origin("local"));

    assert_eq!(
      cloud_cache_origin("https://mica.example.com", "   "),
      Err(CacheOriginError::EmptyAccountId)
    );
    assert_eq!(
      require_cloud_cache_origin("https://mica.example.com", None),
      Err(CacheOriginError::SignedOut)
    );
  }
}
