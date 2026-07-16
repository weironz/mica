use std::env;

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// Configuration for an S3-compatible object store (AWS S3, MinIO, etc.).
///
/// Built from the environment; absent configuration disables the file
/// endpoints rather than failing startup.
#[derive(Debug, Clone)]
pub struct S3Config {
  pub endpoint: String,
  pub region: String,
  pub bucket: String,
  pub access_key: String,
  pub secret_key: String,
  pub presign_ttl_seconds: u64,
  pub max_upload_bytes: i64,
  pub public_base_url: Option<String>,
  /// MinIO and most self-hosted setups require path-style addressing
  /// (`endpoint/bucket/key`) rather than virtual-hosted (`bucket.endpoint/key`).
  pub force_path_style: bool,
}

/// A presigned upload target returned to clients.
#[derive(Debug, Clone)]
pub struct PresignedUpload {
  pub url: String,
  pub method: &'static str,
  pub expires_in: u64,
}

impl S3Config {
  /// Load from `S3_*` environment variables. Returns `None` when the required
  /// variables are missing, leaving file features disabled.
  pub fn from_env() -> Option<Self> {
    let endpoint = env::var("S3_ENDPOINT").ok()?;
    let bucket = env::var("S3_BUCKET").ok()?;
    let access_key = env::var("S3_ACCESS_KEY").ok()?;
    let secret_key = env::var("S3_SECRET_KEY").ok()?;

    let region = env::var("S3_REGION").unwrap_or_else(|_| "us-east-1".to_string());
    let presign_ttl_seconds = env::var("S3_PRESIGN_TTL_SECONDS")
      .ok()
      .and_then(|value| value.parse().ok())
      .unwrap_or(900);
    let max_upload_bytes = env::var("S3_MAX_UPLOAD_BYTES")
      .ok()
      .and_then(|value| value.parse().ok())
      .unwrap_or(25 * 1024 * 1024);
    let public_base_url = env::var("S3_PUBLIC_BASE_URL")
      .ok()
      .filter(|v| !v.is_empty());
    let force_path_style = env::var("S3_FORCE_PATH_STYLE")
      .map(|value| matches!(value.as_str(), "1" | "true" | "yes"))
      .unwrap_or(true);

    Some(Self {
      endpoint,
      region,
      bucket,
      access_key,
      secret_key,
      presign_ttl_seconds,
      max_upload_bytes,
      public_base_url,
      force_path_style,
    })
  }

  /// Presigned `PUT` URL a client uses to upload an object directly.
  pub fn presign_put(&self, key: &str) -> PresignedUpload {
    PresignedUpload {
      url: self.presign("PUT", key, Utc::now()),
      method: "PUT",
      expires_in: self.presign_ttl_seconds,
    }
  }

  /// Presigned `DELETE` URL for an object. Server-side only — this is the blob
  /// GC's hand, and is never issued to a client. Not `public_base_url`: that is
  /// the read path and may be a CDN with no write access at all.
  pub fn presign_delete(&self, key: &str) -> String {
    self.presign("DELETE", key, Utc::now())
  }

  /// URL a client uses to read an object: the public base URL when configured,
  /// otherwise a presigned `GET`.
  pub fn download_url(&self, key: &str) -> String {
    match &self.public_base_url {
      Some(base) => format!("{}/{}", base.trim_end_matches('/'), uri_encode(key, false)),
      None => self.presign("GET", key, Utc::now()),
    }
  }

  fn presign(&self, method: &str, key: &str, now: DateTime<Utc>) -> String {
    let (base_url, host, canonical_uri) = self.object_location(key);
    let request = PresignRequest {
      method,
      base_url: &base_url,
      host: &host,
      canonical_uri: &canonical_uri,
      region: &self.region,
      access_key: &self.access_key,
      secret_key: &self.secret_key,
      expires_in: self.presign_ttl_seconds,
    };
    sign_presigned(&request, now)
  }

  fn object_location(&self, key: &str) -> (String, String, String) {
    let endpoint = self.endpoint.trim_end_matches('/');
    let (scheme, host_port) = split_scheme(endpoint);
    let encoded_key = uri_encode(key, false);

    if self.force_path_style {
      let base_url = format!("{endpoint}/{}/{encoded_key}", self.bucket);
      let canonical_uri = format!("/{}/{encoded_key}", self.bucket);
      (base_url, host_port.to_string(), canonical_uri)
    } else {
      let host = format!("{}.{host_port}", self.bucket);
      let base_url = format!("{scheme}://{host}/{encoded_key}");
      let canonical_uri = format!("/{encoded_key}");
      (base_url, host, canonical_uri)
    }
  }
}

/// Inputs for a single AWS SigV4 query-string presign computation.
struct PresignRequest<'a> {
  method: &'a str,
  base_url: &'a str,
  host: &'a str,
  canonical_uri: &'a str,
  region: &'a str,
  access_key: &'a str,
  secret_key: &'a str,
  expires_in: u64,
}

/// Produce a presigned URL using AWS Signature Version 4 (query parameters,
/// `UNSIGNED-PAYLOAD`, `host` as the only signed header).
fn sign_presigned(request: &PresignRequest, now: DateTime<Utc>) -> String {
  let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
  let date_stamp = now.format("%Y%m%d").to_string();
  let scope = format!("{date_stamp}/{}/s3/aws4_request", request.region);
  let credential = format!("{}/{scope}", request.access_key);

  let mut params = [
    (
      "X-Amz-Algorithm".to_string(),
      "AWS4-HMAC-SHA256".to_string(),
    ),
    ("X-Amz-Credential".to_string(), credential),
    ("X-Amz-Date".to_string(), amz_date.clone()),
    ("X-Amz-Expires".to_string(), request.expires_in.to_string()),
    ("X-Amz-SignedHeaders".to_string(), "host".to_string()),
  ];
  params.sort_by(|a, b| a.0.cmp(&b.0));

  let canonical_query = params
    .iter()
    .map(|(key, value)| format!("{}={}", uri_encode(key, true), uri_encode(value, true)))
    .collect::<Vec<_>>()
    .join("&");

  let canonical_headers = format!("host:{}\n", request.host);
  let canonical_request = format!(
    "{}\n{}\n{}\n{}\nhost\nUNSIGNED-PAYLOAD",
    request.method, request.canonical_uri, canonical_query, canonical_headers
  );

  let string_to_sign = format!(
    "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
    sha256_hex(canonical_request.as_bytes())
  );

  let signing_key = signature_key(request.secret_key, &date_stamp, request.region, "s3");
  let signature = hex_lower(&hmac_sha256(&signing_key, string_to_sign.as_bytes()));

  format!(
    "{}?{canonical_query}&X-Amz-Signature={signature}",
    request.base_url
  )
}

fn signature_key(secret: &str, date_stamp: &str, region: &str, service: &str) -> Vec<u8> {
  let k_date = hmac_sha256(format!("AWS4{secret}").as_bytes(), date_stamp.as_bytes());
  let k_region = hmac_sha256(&k_date, region.as_bytes());
  let k_service = hmac_sha256(&k_region, service.as_bytes());
  hmac_sha256(&k_service, b"aws4_request")
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
  let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts keys of any length");
  mac.update(data);
  mac.finalize().into_bytes().to_vec()
}

fn sha256_hex(data: &[u8]) -> String {
  hex_lower(&Sha256::digest(data))
}

fn hex_lower(bytes: &[u8]) -> String {
  let mut out = String::with_capacity(bytes.len() * 2);
  for byte in bytes {
    out.push_str(&format!("{byte:02x}"));
  }
  out
}

/// RFC 3986 percent-encoding as required by SigV4. `'/'` is preserved in path
/// position and encoded in query position.
fn uri_encode(input: &str, encode_slash: bool) -> String {
  let mut out = String::with_capacity(input.len());
  for byte in input.bytes() {
    match byte {
      b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
      b'/' if !encode_slash => out.push('/'),
      _ => out.push_str(&format!("%{byte:02X}")),
    }
  }
  out
}

fn split_scheme(endpoint: &str) -> (&str, &str) {
  match endpoint.split_once("://") {
    Some((scheme, rest)) => (scheme, rest),
    None => ("https", endpoint),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use chrono::TimeZone;

  // AWS SigV4 documented example: presigned GET for examplebucket/test.txt.
  // https://docs.aws.amazon.com/AmazonS3/latest/API/sigv4-query-string-auth.html
  #[test]
  fn presign_matches_aws_reference_vector() {
    let now = Utc.with_ymd_and_hms(2013, 5, 24, 0, 0, 0).unwrap();
    let request = PresignRequest {
      method: "GET",
      base_url: "https://examplebucket.s3.amazonaws.com/test.txt",
      host: "examplebucket.s3.amazonaws.com",
      canonical_uri: "/test.txt",
      region: "us-east-1",
      access_key: "AKIAIOSFODNN7EXAMPLE",
      secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
      expires_in: 86400,
    };

    let url = sign_presigned(&request, now);
    assert!(url.contains(
      "X-Amz-Signature=aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404"
    ));
    assert!(url.contains(
      "X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20130524%2Fus-east-1%2Fs3%2Faws4_request"
    ));
  }

  #[test]
  fn path_style_location_includes_bucket_in_path() {
    let config = test_config(true, None);
    let (base_url, host, canonical_uri) = config.object_location("workspaces/a/b.png");
    assert_eq!(base_url, "http://localhost:9000/mica/workspaces/a/b.png");
    assert_eq!(host, "localhost:9000");
    assert_eq!(canonical_uri, "/mica/workspaces/a/b.png");
  }

  #[test]
  fn virtual_hosted_location_uses_bucket_subdomain() {
    let mut config = test_config(false, None);
    config.endpoint = "https://s3.amazonaws.com".to_string();
    let (base_url, host, canonical_uri) = config.object_location("k.png");
    assert_eq!(base_url, "https://mica.s3.amazonaws.com/k.png");
    assert_eq!(host, "mica.s3.amazonaws.com");
    assert_eq!(canonical_uri, "/k.png");
  }

  #[test]
  fn download_url_prefers_public_base() {
    let config = test_config(true, Some("https://cdn.example.com".to_string()));
    assert_eq!(
      config.download_url("workspaces/a/b.png"),
      "https://cdn.example.com/workspaces/a/b.png"
    );
  }

  fn test_config(force_path_style: bool, public_base_url: Option<String>) -> S3Config {
    S3Config {
      endpoint: "http://localhost:9000".to_string(),
      region: "us-east-1".to_string(),
      bucket: "mica".to_string(),
      access_key: "key".to_string(),
      secret_key: "secret".to_string(),
      presign_ttl_seconds: 900,
      max_upload_bytes: 25 * 1024 * 1024,
      public_base_url,
      force_path_style,
    }
  }
}
