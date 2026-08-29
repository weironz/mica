use std::env;

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// The `S3_SECRET_ACCESS_KEY` both compose files fall back to when the operator sets
/// none (`${S3_SECRET_ACCESS_KEY:-…}`).
///
/// It exists so the quickstart is one command with nothing to fill in. It is NOT
/// a secret: it is written in this file and in a public repository, and unlike
/// the postgres password — which is safe to default because that service
/// publishes no port — the object store IS published, on :9000, because browsers
/// presign against it directly. Anyone who knows this constant can read and
/// write the bucket of any install that kept it.
///
/// [`default_s3_secret_in_use`] is what makes that visible at runtime instead of
/// only in documentation.
pub const DEFAULT_S3_SECRET_KEY: &str = "mica-default-not-a-secret";

/// Is this instance signing S3 requests with the published default?
///
/// Separate from [`S3Config::from_env`] so the caller can decide what to do
/// about it — in practice, warn once at startup in production.
pub fn default_s3_secret_in_use() -> bool {
  env::var("S3_SECRET_ACCESS_KEY")
    .map(|value| value == DEFAULT_S3_SECRET_KEY)
    .unwrap_or(false)
}

/// Configuration for an S3-compatible object store (AWS S3, MinIO, etc.).
///
/// Built from the environment; absent configuration disables the file
/// endpoints rather than failing startup.
#[derive(Debug, Clone)]
pub struct S3Config {
  /// The endpoint **browsers** use. Presigned URLs embed it, so it has to be
  /// reachable from the client, and that is the only thing it has to be.
  pub endpoint: String,
  /// The endpoint the **server** uses to reach the same store, when that is a
  /// different address (`S3_INTERNAL_ENDPOINT`). `None` means "same as
  /// `endpoint`".
  ///
  /// These are separate because [`endpoint`](Self::endpoint) is chosen for the
  /// browser: it is a public IP or hostname. Whether the api container can
  /// reach that address is a different question with a different answer —
  /// inside compose the store is one hop away as `http://rustfs:9000`, while
  /// the public address may route out through NAT and back, or not resolve at
  /// all. The server-side callers (bucket provisioning, blob GC) would then
  /// fail while every browser upload worked, which is a hard failure to see.
  pub internal_endpoint: Option<String>,
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

/// The per-FILE upload ceiling from the raw `S3_MAX_UPLOAD_BYTES`.
///
/// Takes the raw string rather than reading the environment itself, for the same
/// reason `config::workspace_quota` does: the interesting cases are blank and
/// garbage, and those are untestable through a process-global env var.
///
/// **Blank means unset.** Compose resolves an unset `${VAR:-}` to the EMPTY
/// STRING, so `env::var` hands back `Ok("")` and not `Err` — the shape that
/// crash-looped the JWT secret. A non-positive or unparseable value falls back
/// too: a `0` here would otherwise mean "no file may ever be uploaded", which no
/// operator types on purpose, and the quota's `0 = unlimited` convention does not
/// transfer (a per-file cap of "unlimited" is what a huge number is for).
pub fn max_upload_bytes(raw: Option<&str>) -> i64 {
  const DEFAULT: i64 = 25 * 1024 * 1024;
  match raw.map(str::trim) {
    None | Some("") => DEFAULT,
    Some(v) => match v.parse::<i64>() {
      Ok(n) if n > 0 => n,
      _ => DEFAULT,
    },
  }
}

impl S3Config {
  /// Load from `S3_*` environment variables. Returns `None` when the required
  /// variables are missing, leaving file features disabled.
  pub fn from_env() -> Option<Self> {
    // `.ok()` alone would accept an EMPTY string: `env::var` returns `Ok("")`
    // for `FOO=`, and compose resolves an unset `${FOO:-}` to exactly that. An
    // empty key would configure S3 and then fail every upload with a signature
    // error instead of leaving file features cleanly disabled. Same shape as the
    // JWT_SECRET bug — see `config::resolve_jwt_secret`.
    let non_blank = |name: &str| env::var(name).ok().filter(|v| !v.trim().is_empty());

    let endpoint = non_blank("S3_ENDPOINT")?;
    let internal_endpoint = non_blank("S3_INTERNAL_ENDPOINT");
    let bucket = non_blank("S3_BUCKET")?;
    let access_key = non_blank("S3_ACCESS_KEY_ID")?;
    let secret_key = non_blank("S3_SECRET_ACCESS_KEY")?;

    let region = env::var("S3_REGION").unwrap_or_else(|_| "us-east-1".to_string());
    let presign_ttl_seconds = env::var("S3_PRESIGN_TTL_SECONDS")
      .ok()
      .and_then(|value| value.parse().ok())
      .unwrap_or(900);
    let max_upload_bytes = max_upload_bytes(env::var("S3_MAX_UPLOAD_BYTES").ok().as_deref());
    let public_base_url = env::var("S3_PUBLIC_BASE_URL")
      .ok()
      .filter(|v| !v.is_empty());
    let force_path_style = env::var("S3_FORCE_PATH_STYLE")
      .map(|value| matches!(value.as_str(), "1" | "true" | "yes"))
      .unwrap_or(true);

    Some(Self {
      endpoint,
      internal_endpoint,
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

  /// Presigned `PUT` URL a **client** uses to upload an object directly.
  ///
  /// Signed against the browser-facing endpoint. If this process is going to
  /// send the request itself, use [`presign_put_server`](Self::presign_put_server).
  pub fn presign_put(&self, key: &str) -> PresignedUpload {
    PresignedUpload {
      url: self.presign("PUT", key, Utc::now()),
      method: "PUT",
      expires_in: self.presign_ttl_seconds,
    }
  }

  /// Presigned `PUT` for an upload **this process** performs: avatars, imported
  /// images, anything re-hosted server-side.
  ///
  /// The distinction is not cosmetic. Handing these paths the browser-facing
  /// URL is why "the API in a container cannot do server-side uploads" is a
  /// documented dev-stack limitation (`docs/lessons.md`): `127.0.0.1:9000`
  /// means the api container itself, so the PUT fails with
  /// `error sending request`. In production the public address usually does
  /// resolve from inside, which makes this the worst kind of coupling — it
  /// works until DNS, TLS or the proxy in front of the store hiccups.
  pub fn presign_put_server(&self, key: &str) -> PresignedUpload {
    let (base_url, host, canonical_uri) =
      self.location(self.server_endpoint(), &uri_encode(key, false));
    PresignedUpload {
      url: self.sign_at("PUT", &base_url, &host, &canonical_uri, Utc::now()),
      method: "PUT",
      expires_in: self.presign_ttl_seconds,
    }
  }

  /// Presigned `DELETE` URL for an object. Server-side only — this is the blob
  /// GC's hand, and is never issued to a client. Not `public_base_url`: that is
  /// the read path and may be a CDN with no write access at all.
  ///
  /// Signed against [`server_endpoint`](Self::server_endpoint), because the
  /// process that will send it is this one, not a browser.
  pub fn presign_delete(&self, key: &str) -> String {
    let (base_url, host, canonical_uri) =
      self.location(self.server_endpoint(), &uri_encode(key, false));
    self.sign_at("DELETE", &base_url, &host, &canonical_uri, Utc::now())
  }

  /// The address THIS PROCESS should use to reach the store.
  ///
  /// `S3_INTERNAL_ENDPOINT` when set, otherwise the browser-facing one — which
  /// is the right fallback for a single-host deployment where both routes are
  /// the same address, and the only sane guess when nobody said otherwise.
  pub fn server_endpoint(&self) -> &str {
    self.internal_endpoint.as_deref().unwrap_or(&self.endpoint)
  }

  /// URL a client uses to read an object: the public base URL when configured,
  /// otherwise a presigned `GET`.
  pub fn download_url(&self, key: &str) -> String {
    match &self.public_base_url {
      Some(base) => format!("{}/{}", base.trim_end_matches('/'), uri_encode(key, false)),
      None => self.presign("GET", key, Utc::now()),
    }
  }

  /// Presigned `HEAD` on the bucket itself — "does this bucket exist, and may I
  /// see it?". Server-side only.
  pub fn presign_head_bucket(&self) -> String {
    self.presign_bucket("HEAD", Utc::now())
  }

  /// Presigned `PUT` on the bucket itself — S3 `CreateBucket`. Server-side only.
  ///
  /// Pair it with [`create_bucket_body`](Self::create_bucket_body): the region
  /// travels in the body, not the URL.
  pub fn presign_create_bucket(&self) -> String {
    self.presign_bucket("PUT", Utc::now())
  }

  /// The `CreateBucket` request body, or `None` when none is needed.
  ///
  /// `us-east-1` is the odd one out and must send NO `LocationConstraint` —
  /// it is the implicit default, and sending it explicitly is rejected. Every
  /// other region must state itself or the bucket lands in the wrong place (or
  /// the request fails outright). Self-hosted stores ignore the whole thing.
  ///
  /// Signing is unaffected either way: [`sign_presigned`] signs
  /// `UNSIGNED-PAYLOAD`, so a body may be attached to a presigned URL without
  /// entering the signature.
  pub fn create_bucket_body(&self) -> Option<String> {
    if self.region.eq_ignore_ascii_case("us-east-1") {
      return None;
    }
    Some(format!(
      "<CreateBucketConfiguration xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\
       <LocationConstraint>{}</LocationConstraint>\
       </CreateBucketConfiguration>",
      self.region
    ))
  }

  fn presign_bucket(&self, method: &str, now: DateTime<Utc>) -> String {
    let (base_url, host, canonical_uri) = self.bucket_location();
    self.sign_at(method, &base_url, &host, &canonical_uri, now)
  }

  /// Same split as [`object_location`](Self::object_location) but for the bucket
  /// itself: path style addresses it as `/{bucket}`, virtual-hosted style as `/`
  /// on the bucket's own host.
  fn bucket_location(&self) -> (String, String, String) {
    self.location(self.server_endpoint(), "")
  }

  /// Address one thing in the store, against a given endpoint.
  ///
  /// `path` is the ALREADY-ENCODED object key, or empty for the bucket itself.
  /// The two addressing styles differ in where the bucket goes, which is why
  /// this is one function rather than two: getting the canonical URI to match
  /// the URL actually sent is what the signature depends on.
  fn location(&self, endpoint: &str, path: &str) -> (String, String, String) {
    let endpoint = endpoint.trim_end_matches('/');
    let (scheme, host_port) = split_scheme(endpoint);

    if self.force_path_style {
      let bucket = &self.bucket;
      if path.is_empty() {
        (
          format!("{endpoint}/{bucket}"),
          host_port.to_string(),
          format!("/{bucket}"),
        )
      } else {
        (
          format!("{endpoint}/{bucket}/{path}"),
          host_port.to_string(),
          format!("/{bucket}/{path}"),
        )
      }
    } else {
      let host = format!("{}.{host_port}", self.bucket);
      (
        format!("{scheme}://{host}/{path}"),
        host,
        format!("/{path}"),
      )
    }
  }

  fn sign_at(
    &self,
    method: &str,
    base_url: &str,
    host: &str,
    canonical_uri: &str,
    now: DateTime<Utc>,
  ) -> String {
    sign_presigned(
      &PresignRequest {
        method,
        base_url,
        host,
        canonical_uri,
        region: &self.region,
        access_key: &self.access_key,
        secret_key: &self.secret_key,
        expires_in: self.presign_ttl_seconds,
      },
      now,
    )
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

  /// Where a BROWSER finds an object — always the public endpoint.
  fn object_location(&self, key: &str) -> (String, String, String) {
    self.location(&self.endpoint, &uri_encode(key, false))
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

/// What a `HEAD` on the bucket told us.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BucketProbe {
  Exists,
  Missing,
  /// We could not tell, and that is a legitimate answer — not an error.
  ///
  /// The common cause is credentials scoped to objects within one bucket:
  /// they can PUT and GET all day but may not introspect the bucket, so the
  /// probe comes back 403 on a bucket that exists and works fine. Treating
  /// that as "missing" would send us on to `CreateBucket`, which is exactly
  /// how Nextcloud ends up logging "access denied while trying to create a
  /// bucket" on installs whose bucket was there the whole time
  /// (nextcloud/server#36427).
  Unknown,
}

/// Classify a `HeadBucket` response. Pure so every branch is testable without
/// an object store — see `docs/bucket-provisioning-plan.md`.
pub fn classify_head(status: u16) -> BucketProbe {
  match status {
    200..=299 => BucketProbe::Exists,
    404 => BucketProbe::Missing,
    _ => BucketProbe::Unknown,
  }
}

/// What a `CreateBucket` attempt did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateOutcome {
  Created,
  /// It already existed and we own it. Normal when two replicas boot together.
  AlreadyOurs,
  /// These credentials may not create buckets. Common on managed S3, where the
  /// app is deliberately scoped to one bucket someone else provisioned.
  Denied,
  /// The name is taken by ANOTHER account. Only AWS can say this — its bucket
  /// namespace is global across all accounts — and it means the configured
  /// name is unusable, not that something went wrong.
  NameTaken,
  Failed {
    status: u16,
  },
}

/// Classify a `CreateBucket` response.
///
/// Both "already exists" answers are 409 and only the body tells them apart,
/// and AWS returns 200 instead of 409 for `BucketAlreadyOwnedByYou` in
/// us-east-1 alone (documented legacy behaviour) — which the 2xx arm already
/// folds into success.
pub fn classify_create(status: u16, body: &str) -> CreateOutcome {
  match status {
    200..=299 => CreateOutcome::Created,
    403 => CreateOutcome::Denied,
    409 if body.contains("BucketAlreadyOwnedByYou") => CreateOutcome::AlreadyOurs,
    409 => CreateOutcome::NameTaken,
    status => CreateOutcome::Failed { status },
  }
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
  fn bucket_location_path_style_addresses_the_bucket_itself() {
    let config = test_config(true, None);
    let (base_url, host, canonical_uri) = config.bucket_location();
    assert_eq!(base_url, "http://localhost:9000/mica");
    assert_eq!(host, "localhost:9000");
    assert_eq!(canonical_uri, "/mica");
  }

  /// The signature covers the canonical URI, so the bucket-level path has to be
  /// the one actually requested — `/mica`, not the `/mica/<key>` of an object.
  #[test]
  fn bucket_presign_signs_the_bucket_path_not_an_object() {
    let config = test_config(true, None);
    let url = config.presign_head_bucket();
    assert!(url.starts_with("http://localhost:9000/mica?"), "{url}");
    assert!(url.contains("X-Amz-Signature="));
  }

  /// us-east-1 must send no LocationConstraint at all: it is the implicit
  /// default and stating it explicitly is rejected.
  #[test]
  fn create_bucket_body_omits_the_default_region() {
    let mut config = test_config(true, None);
    config.region = "us-east-1".to_string();
    assert_eq!(config.create_bucket_body(), None);

    config.region = "eu-west-1".to_string();
    let body = config.create_bucket_body().expect("non-default region states itself");
    assert!(body.contains("<LocationConstraint>eu-west-1</LocationConstraint>"), "{body}");
  }

  /// The split that makes server-side S3 work at all: what the BROWSER is sent
  /// keeps the public endpoint, what THIS PROCESS sends goes to the internal
  /// one. Getting it backwards fails silently in the worst direction — uploads
  /// keep working (browsers are fine), while bucket provisioning and blob GC
  /// quietly cannot reach the store.
  #[test]
  fn server_side_urls_use_the_internal_endpoint_and_client_urls_do_not() {
    let mut config = test_config(true, None);
    config.internal_endpoint = Some("http://rustfs:9000".to_string());

    let client_upload = config.presign_put("a/b.png").url;
    assert!(
      client_upload.starts_with("http://localhost:9000/"),
      "browsers must keep the public endpoint: {client_upload}"
    );

    // Every URL THIS PROCESS sends. Missing one is invisible: browsers keep
    // working, and only the server-side path quietly cannot reach the store —
    // which is exactly how three of these (avatar, import_url, store_bytes)
    // were left behind when the first two were fixed.
    for server_url in [
      config.presign_head_bucket(),
      config.presign_create_bucket(),
      config.presign_delete("a/b.png"),
      config.presign_put_server("a/b.png").url,
    ] {
      assert!(
        server_url.starts_with("http://rustfs:9000/"),
        "server-side calls must use the internal endpoint: {server_url}"
      );
    }
  }

  /// Unset is the single-host case, where both routes are the same address.
  #[test]
  fn without_an_internal_endpoint_everything_uses_the_public_one() {
    let config = test_config(true, None);
    assert_eq!(config.server_endpoint(), "http://localhost:9000");
    assert!(config
      .presign_head_bucket()
      .starts_with("http://localhost:9000/mica?"));
  }

  #[test]
  fn head_classification_covers_every_branch() {
    assert_eq!(classify_head(200), BucketProbe::Exists);
    assert_eq!(classify_head(404), BucketProbe::Missing);
    // The one that matters: scoped credentials on a bucket that DOES exist.
    // Anything but a clean 404 has to stay Unknown, or we go on to create a
    // bucket that is already there.
    assert_eq!(classify_head(403), BucketProbe::Unknown);
    assert_eq!(classify_head(500), BucketProbe::Unknown);
    assert_eq!(classify_head(301), BucketProbe::Unknown);
  }

  #[test]
  fn create_classification_separates_the_two_409s() {
    assert_eq!(classify_create(200, ""), CreateOutcome::Created);
    assert_eq!(
      classify_create(409, "<Error><Code>BucketAlreadyOwnedByYou</Code></Error>"),
      CreateOutcome::AlreadyOurs
    );
    assert_eq!(
      classify_create(409, "<Error><Code>BucketAlreadyExists</Code></Error>"),
      CreateOutcome::NameTaken
    );
    assert_eq!(classify_create(403, ""), CreateOutcome::Denied);
    assert_eq!(classify_create(500, ""), CreateOutcome::Failed { status: 500 });
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

  /// The compose allow-list now passes this knob through as `"${VAR:-}"`, which
  /// makes BLANK the value production actually sends when the operator sets
  /// nothing — so blank has to mean "use the default", not "zero".
  #[test]
  fn max_upload_bytes_treats_blank_and_garbage_as_unset() {
    const DEFAULT: i64 = 25 * 1024 * 1024;

    assert_eq!(max_upload_bytes(None), DEFAULT, "genuinely unset");
    assert_eq!(
      max_upload_bytes(Some("")),
      DEFAULT,
      "compose resolves an unset ${{VAR:-}} to empty, NOT to absent"
    );
    assert_eq!(max_upload_bytes(Some("   ")), DEFAULT, "whitespace is blank");
    assert_eq!(max_upload_bytes(Some("abc")), DEFAULT, "garbage");

    // A real value wins, whitespace and all.
    assert_eq!(max_upload_bytes(Some("52428800")), 52_428_800);
    assert_eq!(max_upload_bytes(Some(" 52428800 ")), 52_428_800);

    // 0 / negative are NOT taken literally: unlike the workspace quota, where 0
    // means unlimited, a per-file cap of 0 would refuse every upload — an
    // instance bricked by a typo.
    assert_eq!(max_upload_bytes(Some("0")), DEFAULT);
    assert_eq!(max_upload_bytes(Some("-1")), DEFAULT);
  }

  fn test_config(force_path_style: bool, public_base_url: Option<String>) -> S3Config {
    S3Config {
      endpoint: "http://localhost:9000".to_string(),
      internal_endpoint: None,
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
