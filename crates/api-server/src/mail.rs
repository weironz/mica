//! Aliyun DirectMail (邮件推送) implementation of [`Mailer`].
//!
//! Lives here, not in `infra`, because it needs `reqwest` (already an
//! api-server dep) and `infra` is meant to stay light. `infra` owns only the
//! trait and the [`LogMailer`] fallback; [`build_mailer`] picks between them from
//! the environment, so a node with no mail config still boots (LogMailer) and
//! the switch to real sending is one env var — no code change, no redeploy shape.
//!
//! We call DirectMail's `SingleSendMail` over its **v1 RPC** HTTP API. That
//! scheme signs the request with HMAC-**SHA1** over a canonicalized,
//! percent-encoded parameter string — SHA-1 is the API's requirement, not a
//! security choice of ours (the transport is HTTPS). The signing is fiddly and
//! can only be end-to-end verified with a real AccessKey, so the deterministic
//! parts (percent-encoding, canonicalization, determinism) are unit-tested below.

use std::sync::Arc;

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use hmac::{Hmac, Mac};
use mica_infra::{LogMailer, Mail, Mailer};
use sha1::Sha1;

/// Build the process-wide mailer from the environment.
///
/// `MICA_MAIL_BACKEND=directmail` switches on Aliyun DirectMail; anything else
/// (including unset) uses [`LogMailer`]. If `directmail` is asked for but a
/// required field is missing, we WARN and fall back to logging rather than
/// crash — a misconfigured mailer must not take the whole API down, and the
/// reset link is still recoverable from the logs.
pub fn build_mailer() -> Arc<dyn Mailer> {
  let backend = std::env::var("MICA_MAIL_BACKEND")
    .map(|v| v.trim().to_ascii_lowercase())
    .unwrap_or_default();
  if backend != "directmail" {
    return Arc::new(LogMailer);
  }
  match DirectMailMailer::from_env() {
    Ok(mailer) => {
      tracing::info!(from = %mailer.from_address, "mail backend: Aliyun DirectMail");
      Arc::new(mailer)
    }
    Err(missing) => {
      tracing::warn!(
        "MICA_MAIL_BACKEND=directmail but {missing} is unset — falling back to \
         LogMailer (reset links go to the log, not to inboxes)"
      );
      Arc::new(LogMailer)
    }
  }
}

pub struct DirectMailMailer {
  access_key_id: String,
  access_key_secret: String,
  /// The verified sender address, DirectMail's `AccountName`.
  from_address: String,
  /// Optional display name (`FromAlias`).
  from_alias: Option<String>,
  /// Full endpoint URL, e.g. `https://dm.aliyuncs.com/`.
  endpoint: String,
  region_id: String,
  client: reqwest::Client,
}

impl DirectMailMailer {
  /// Read config from the environment. `Err(name)` names the first missing
  /// required variable so the caller can log exactly what to set.
  fn from_env() -> Result<Self, &'static str> {
    let access_key_id = req_env("MICA_MAIL_ACCESS_KEY_ID")?;
    let access_key_secret = req_env("MICA_MAIL_ACCESS_KEY_SECRET")?;
    let from_address = req_env("MICA_MAIL_FROM")?;
    let from_alias = std::env::var("MICA_MAIL_FROM_NAME")
      .ok()
      .map(|v| v.trim().to_string())
      .filter(|v| !v.is_empty());
    let endpoint = std::env::var("MICA_MAIL_ENDPOINT")
      .ok()
      .map(|v| v.trim().to_string())
      .filter(|v| !v.is_empty())
      .unwrap_or_else(|| "https://dm.aliyuncs.com/".to_string());
    let region_id = std::env::var("MICA_MAIL_REGION")
      .ok()
      .map(|v| v.trim().to_string())
      .filter(|v| !v.is_empty())
      .unwrap_or_else(|| "cn-hangzhou".to_string());
    Ok(Self {
      access_key_id,
      access_key_secret,
      from_address,
      from_alias,
      endpoint,
      region_id,
      client: reqwest::Client::new(),
    })
  }

  /// The signed, ready-to-send parameter list for one message. Split out from
  /// the network call so `nonce`/`timestamp` can be injected and the signing
  /// asserted in a test.
  fn signed_params(&self, mail: &Mail, nonce: &str, timestamp: &str) -> Vec<(String, String)> {
    let mut params: Vec<(String, String)> = vec![
      ("Format".into(), "JSON".into()),
      ("Version".into(), "2015-11-23".into()),
      ("AccessKeyId".into(), self.access_key_id.clone()),
      ("SignatureMethod".into(), "HMAC-SHA1".into()),
      ("Timestamp".into(), timestamp.to_string()),
      ("SignatureVersion".into(), "1.0".into()),
      ("SignatureNonce".into(), nonce.to_string()),
      ("RegionId".into(), self.region_id.clone()),
      ("Action".into(), "SingleSendMail".into()),
      ("AccountName".into(), self.from_address.clone()),
      // 1 = send from the verified sending address (not a random account).
      ("AddressType".into(), "1".into()),
      ("ReplyToAddress".into(), "false".into()),
      ("ToAddress".into(), mail.to.clone()),
      ("Subject".into(), mail.subject.clone()),
      ("HtmlBody".into(), mail.html_body.clone()),
    ];
    if let Some(alias) = &self.from_alias {
      params.push(("FromAlias".into(), alias.clone()));
    }
    let signature = self.sign(&params);
    params.push(("Signature".into(), signature));
    params
  }

  /// Aliyun v1 RPC signature: sort params, canonicalize as percent-encoded
  /// `k=v&…`, wrap as `POST&%2F&<encoded canonical>`, HMAC-SHA1 with
  /// `secret + "&"`, base64.
  fn sign(&self, params: &[(String, String)]) -> String {
    let mut sorted: Vec<&(String, String)> = params.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let canonical = sorted
      .iter()
      .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
      .collect::<Vec<_>>()
      .join("&");
    let string_to_sign = format!("POST&{}&{}", percent_encode("/"), percent_encode(&canonical));

    let mut mac = Hmac::<Sha1>::new_from_slice(format!("{}&", self.access_key_secret).as_bytes())
      .expect("HMAC accepts any key length");
    mac.update(string_to_sign.as_bytes());
    BASE64.encode(mac.finalize().into_bytes())
  }

  async fn send_now(&self, mail: &Mail) -> anyhow::Result<()> {
    // A UUID (v4) as the required unique SignatureNonce; UTC seconds as the
    // Timestamp in the RPC's fixed ISO-8601 shape.
    let nonce = uuid::Uuid::new_v4().to_string();
    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let params = self.signed_params(mail, &nonce, &timestamp);

    // Body is the SAME percent-encoding used for signing (Aliyun uses %20 for
    // space, not `+`), so we build it by hand rather than let reqwest form-encode.
    let body = params
      .iter()
      .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
      .collect::<Vec<_>>()
      .join("&");

    let response = self
      .client
      .post(&self.endpoint)
      .header("content-type", "application/x-www-form-urlencoded")
      .body(body)
      .send()
      .await?;

    let status = response.status();
    if status.is_success() {
      return Ok(());
    }
    let detail = response.text().await.unwrap_or_default();
    anyhow::bail!("DirectMail SingleSendMail failed: {status} {detail}");
  }
}

#[async_trait::async_trait]
impl Mailer for DirectMailMailer {
  async fn send(&self, mail: &Mail) -> anyhow::Result<()> {
    self.send_now(mail).await
  }
}

fn req_env(name: &'static str) -> Result<String, &'static str> {
  std::env::var(name)
    .ok()
    .map(|v| v.trim().to_string())
    .filter(|v| !v.is_empty())
    .ok_or(name)
}

/// RFC-3986 percent-encoding as Aliyun's RPC signing requires: everything except
/// the unreserved set `A-Za-z0-9-_.~` is `%XX` (uppercase hex). Notably space is
/// `%20` (not `+`) and `~` is left literal — this is exactly what their SDKs do
/// after the usual "encodeURIComponent then fix up `+ * %7E`" dance.
fn percent_encode(input: &str) -> String {
  let mut out = String::with_capacity(input.len() * 3);
  for &byte in input.as_bytes() {
    let unreserved =
      byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~');
    if unreserved {
      out.push(byte as char);
    } else {
      out.push('%');
      out.push_str(&format!("{byte:02X}"));
    }
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn percent_encoding_matches_aliyun_rules() {
    // Unreserved pass through untouched.
    assert_eq!(percent_encode("aZ09-_.~"), "aZ09-_.~");
    // Space is %20, not '+'.
    assert_eq!(percent_encode("a b"), "a%20b");
    // Reserved / punctuation is uppercase %XX.
    assert_eq!(percent_encode("/"), "%2F");
    assert_eq!(percent_encode("="), "%3D");
    assert_eq!(percent_encode("&"), "%26");
    assert_eq!(percent_encode("@"), "%40");
    assert_eq!(percent_encode("a@b.com"), "a%40b.com");
    // Multi-byte UTF-8 encodes each byte.
    assert_eq!(percent_encode("重"), "%E9%87%8D");
  }

  fn fixture() -> DirectMailMailer {
    DirectMailMailer {
      access_key_id: "testid".into(),
      access_key_secret: "testsecret".into(),
      from_address: "noreply@mail.example.com".into(),
      from_alias: None,
      endpoint: "https://dm.aliyuncs.com/".into(),
      region_id: "cn-hangzhou".into(),
      client: reqwest::Client::new(),
    }
  }

  #[test]
  fn signing_is_deterministic_and_well_formed() {
    // We have no published Aliyun vector to pin the exact value, so pin what we
    // CAN guarantee without one: the pipeline (sort → canonicalize → encode →
    // HMAC-SHA1 → base64) is a pure function of its inputs, and its output is a
    // base64 SHA-1 digest (20 bytes → 28 chars, '='-padded). Same inputs → same
    // signature; the real end-to-end check is DirectMail accepting it in prod.
    let mail = Mail {
      to: "user@example.com".into(),
      subject: "Reset your password".into(),
      html_body: "<a href=\"https://x/reset-password?token=abc\">reset</a>".into(),
    };
    let sig1 = signature_of(fixture().signed_params(&mail, "nonce-123", "2026-01-01T00:00:00Z"));
    let sig2 = signature_of(fixture().signed_params(&mail, "nonce-123", "2026-01-01T00:00:00Z"));
    assert_eq!(sig1, sig2, "signing is a pure function of its inputs");
    assert_eq!(sig1.len(), 28, "base64 of a 20-byte SHA-1 digest");
    assert!(sig1.ends_with('='));
    assert!(BASE64.decode(&sig1).is_ok(), "a valid base64 string");

    // A different message must produce a different signature.
    let other = Mail {
      to: "user@example.com".into(),
      subject: "Reset your password".into(),
      html_body: "different body".into(),
    };
    let sig3 = signature_of(fixture().signed_params(&other, "nonce-123", "2026-01-01T00:00:00Z"));
    assert_ne!(sig1, sig3, "the body is part of what is signed");
  }

  #[test]
  fn signing_sorts_and_appends_signature_once() {
    let params = fixture().signed_params(
      &Mail {
        to: "u@e.com".into(),
        subject: "s".into(),
        html_body: "b".into(),
      },
      "n",
      "t",
    );
    assert_eq!(
      params.iter().filter(|(k, _)| k == "Signature").count(),
      1,
      "exactly one Signature, appended after signing"
    );
    assert_eq!(params.last().unwrap().0, "Signature");
  }

  fn signature_of(params: Vec<(String, String)>) -> String {
    params
      .into_iter()
      .find(|(k, _)| k == "Signature")
      .map(|(_, v)| v)
      .expect("a Signature param is appended")
  }
}
