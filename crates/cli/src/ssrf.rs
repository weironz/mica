//! SSRF guard for client-side fetches (`rehost-images` downloads an URL found
//! in a document over THIS machine's network). Same rule as the server's
//! `fetch_and_store_image_url` (`crates/api-server/src/routes/files.rs`): no
//! redirects, no private/loopback/link-local targets, and — because a DNS name
//! can answer the CHECK publicly and the CONNECTION privately (rebinding) —
//! the vetted addresses are pinned onto the client itself. The code is a
//! deliberate copy of the same-origin rule, not a shared dependency: the CLI
//! stays a thin HTTP client, and adding mica-infra for one predicate buys
//! nothing.

use std::net::IpAddr;

use anyhow::{Context, Result, bail};

/// True for addresses a rehost fetch must never reach: loopback, private,
/// link-local (incl. the 169.254.169.254 cloud-metadata IP), CGNAT, and their
/// IPv6 equivalents. A collaborator who drops an internal URL into an image
/// block must not get the rehost runner's network to fetch it.
fn is_blocked_addr(ip: IpAddr) -> bool {
  match ip {
    IpAddr::V4(v4) => {
      let o = v4.octets();
      v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_broadcast()
        || v4.is_documentation()
        || v4.is_unspecified()
        || o[0] == 0
        // CGNAT 100.64.0.0/10
        || (o[0] == 100 && (64..=127).contains(&o[1]))
    }
    IpAddr::V6(v6) => {
      if let Some(mapped) = v6.to_ipv4_mapped() {
        return is_blocked_addr(IpAddr::V4(mapped));
      }
      let seg0 = v6.segments()[0];
      v6.is_loopback()
        || v6.is_unspecified()
        // ULA fc00::/7
        || (seg0 & 0xfe00) == 0xfc00
        // link-local fe80::/10
        || (seg0 & 0xffc0) == 0xfe80
    }
  }
}

/// Is this resolved address list safe to connect to? Refusing when ANY answer
/// is blocked — rather than dropping the blocked ones and keeping the rest —
/// is the point: a hostname that answers with both a public and a private
/// address is exactly what a DNS-rebinding attempt looks like, and connecting
/// to the public answer would leave the private one live for the next lookup.
/// An EMPTY list is refused too: "no addresses" is not "no blocked addresses".
fn addresses_are_safe(resolved: &[std::net::SocketAddr]) -> bool {
  !resolved.is_empty() && !resolved.iter().any(|addr| is_blocked_addr(addr.ip()))
}

/// Vet `url` for a client-side fetch and return the addresses the connection
/// may be pinned to. Rejects non-http(s) schemes, host-less URLs, unroutable
/// names, and names resolving to any blocked address.
pub fn vetted_pinned_addrs(url: &reqwest::Url) -> Result<Vec<std::net::SocketAddr>> {
  if !matches!(url.scheme(), "http" | "https") {
    bail!("url must be http(s)");
  }
  if url.host_str().is_none() {
    bail!("url must have a host");
  }
  let port = url.port_or_known_default().unwrap_or(80);
  let resolved = url
    .socket_addrs(|| url.port_or_known_default())
    .context("could not resolve the image url's host")?;
  if !addresses_are_safe(&resolved) {
    bail!("refusing to fetch that url: it resolves to a private or loopback address");
  }
  // Re-state each address with the port the request will actually use:
  // `socket_addrs` resolves against the URL's default, which is not necessarily
  // the port an explicit `:8443` names.
  Ok(
    resolved
      .iter()
      .map(|addr| std::net::SocketAddr::new(addr.ip(), port))
      .collect(),
  )
}

/// Mime sniffing by magic number, for when a host serves images as
/// `application/octet-stream` (or lies). Returns the extension to save under.
pub fn sniff_image_ext(bytes: &[u8]) -> Option<&'static str> {
  match bytes {
    [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, ..] => Some("png"),
    [0xff, 0xd8, 0xff, ..] => Some("jpg"),
    [b'G', b'I', b'F', b'8', ..] => Some("gif"),
    [b'B', b'M', ..] => Some("bmp"),
    [
      b'R',
      b'I',
      b'F',
      b'F',
      _,
      _,
      _,
      _,
      b'W',
      b'E',
      b'B',
      b'P',
      ..,
    ] => Some("webp"),
    // AVIF/HEIF: ISO BMFF with an `ftyp` box carrying the brand.
    [
      _,
      _,
      _,
      _,
      b'f',
      b't',
      b'y',
      b'p',
      b'a',
      b'v',
      b'i',
      b'f',
      ..,
    ] => Some("avif"),
    _ => None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Copied from the server's `ssrf_guard_blocks_private_and_metadata_addresses`
  /// so the two guards stay pinned to the same rule.
  #[test]
  fn private_and_metadata_addresses_are_blocked() {
    let blocked = [
      "127.0.0.1",              // loopback
      "0.0.0.0",                // unspecified
      "10.1.2.3",               // private A
      "172.16.0.1",             // private B
      "172.31.255.255",         // private B (upper)
      "192.168.1.1",            // private C
      "169.254.169.254",        // link-local / cloud metadata
      "100.64.0.1",             // CGNAT
      "::1",                    // v6 loopback
      "::",                     // v6 unspecified
      "fc00::1",                // v6 ULA
      "fd12:3456::1",           // v6 ULA
      "fe80::1",                // v6 link-local
      "::ffff:127.0.0.1",       // v4-mapped loopback
      "::ffff:169.254.169.254", // v4-mapped metadata
    ];
    for s in blocked {
      let ip: IpAddr = s.parse().unwrap();
      assert!(is_blocked_addr(ip), "{s} should be blocked");
    }

    let allowed = [
      "8.8.8.8",
      "1.1.1.1",
      "93.184.216.34",
      "2606:4700:4700::1111",
    ];
    for s in allowed {
      let ip: IpAddr = s.parse().unwrap();
      assert!(!is_blocked_addr(ip), "{s} should be allowed");
    }
  }

  /// A host answering with BOTH a public and a private address is the
  /// rebinding shape: the whole name is refused, not just filtered, and order
  /// must not matter. An empty resolution is refused rather than read as
  /// "nothing blocked".
  #[test]
  fn a_host_with_any_private_answer_is_refused_whole() {
    let public: std::net::SocketAddr = "93.184.216.34:443".parse().unwrap();
    let private: std::net::SocketAddr = "10.0.0.7:443".parse().unwrap();
    let metadata: std::net::SocketAddr = "169.254.169.254:443".parse().unwrap();

    assert!(addresses_are_safe(&[public]));
    assert!(!addresses_are_safe(&[private]));
    assert!(!addresses_are_safe(&[public, private]));
    assert!(!addresses_are_safe(&[private, public]));
    assert!(!addresses_are_safe(&[public, public, metadata]));
    assert!(!addresses_are_safe(&[]));
  }

  /// IP literals and schemes are refused without any DNS: the vetting itself
  /// is what these exercise.
  #[test]
  fn vetting_refuses_private_literals_and_foreign_schemes() {
    let private = reqwest::Url::parse("http://127.0.0.1/x.png").unwrap();
    let err = vetted_pinned_addrs(&private).unwrap_err().to_string();
    assert!(err.contains("private or loopback"), "{err}");

    let metadata = reqwest::Url::parse("http://169.254.169.254/latest/meta-data/").unwrap();
    assert!(vetted_pinned_addrs(&metadata).is_err());

    let ftp = reqwest::Url::parse("ftp://93.184.216.34/x.png").unwrap();
    let err = vetted_pinned_addrs(&ftp).unwrap_err().to_string();
    assert!(err.contains("http(s)"), "{err}");

    let public = reqwest::Url::parse("https://93.184.216.34:8443/x.png").unwrap();
    let pinned = vetted_pinned_addrs(&public).expect("public literal pins");
    assert_eq!(pinned, vec!["93.184.216.34:8443".parse().unwrap()]);
  }

  #[test]
  fn sniffs_real_images_and_rejects_other_bytes() {
    assert_eq!(sniff_image_ext(b"\x89PNG\r\n\x1a\nrest"), Some("png"));
    assert_eq!(sniff_image_ext(b"\xff\xd8\xff\xe0rest"), Some("jpg"));
    assert_eq!(sniff_image_ext(b"GIF89a..."), Some("gif"));
    assert_eq!(sniff_image_ext(b"RIFF....WEBP..."), Some("webp"));
    // HTML payloads and empty bodies are NOT images.
    assert_eq!(sniff_image_ext(b"<!doctype html>"), None);
    assert_eq!(sniff_image_ext(b""), None);
  }
}
