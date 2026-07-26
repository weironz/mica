// File-name / url helpers for the image paths.
//
// The Dart mirror of the server's naming rules (`crates/api-server/src/routes/
// files.rs`: `file_extension` / `name_with_ext` / `safe_file_name`). The client
// names an upload, the server sanitizes and stores it — so the two must agree
// on what counts as an extension. Keep them in step.

/// Whether [name] ends in a USABLE extension, as opposed to merely containing a
/// dot somewhere.
///
/// This distinction is the bug: an AppFlowy blob url's last segment looks like
/// `0QYXMbZ8CjEbSzegIlDdWCGI-zg53UlHuO3v2Vr9X2M=.` — it *contains* a dot, but
/// the dot is bare. A `contains('.')` test accepted it as a filename, so the
/// upload was stored with no extension and its name displayed as `0QYX…X2M.`
/// (matching the server's own `contains('.')` mistake in `url_file_name`).
bool hasUsableFileExt(String name) {
  final dot = name.lastIndexOf('.');
  if (dot < 0 || dot == name.length - 1) return false;
  final ext = name.substring(dot + 1);
  // Mirrors the server: ASCII alphanumeric only, and short enough to be a real
  // extension rather than a trailing sentence.
  return ext.length <= 8 && RegExp(r'^[A-Za-z0-9]+$').hasMatch(ext);
}

/// A url as a human reads it: percent-escapes decoded, so an image named
/// `解锁超时.png` shows as itself instead of
/// `%E8%A7%A3%E9%94%81%E8%B6%85%E6%97%B6.png`.
///
/// DISPLAY ONLY. Fetching and copying must use the raw url — the escapes are
/// what make it valid on the wire. Malformed escapes fall back to the input
/// rather than throwing.
String readableUrl(String url) {
  try {
    return Uri.decodeFull(url);
  } catch (_) {
    return url;
  }
}
