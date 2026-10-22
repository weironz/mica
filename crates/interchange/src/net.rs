//! Network address policy shared by every place that fetches a URL a document
//! author chose: the server's image import and the CLI's re-hosting sweep.

use std::net::IpAddr;

/// True for addresses an import fetch must never reach: loopback, private,
/// link-local (incl. the 169.254.169.254 cloud-metadata IP), CGNAT, and their
/// IPv6 equivalents. Keeps `import-url` from being turned into an SSRF probe of
/// the server's own network, and `mica-cli rehost-images` from fetching the
/// operator's LAN on a collaborator's say-so. One copy for both.
pub fn is_blocked_addr(ip: IpAddr) -> bool {
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
