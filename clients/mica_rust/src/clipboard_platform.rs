//! Platform clipboard bridges for rich HTML (`CF_HTML` on Windows) and pasted
//! images (`PastedFile` / local image file URIs).
//!
//! Why this exists:
//! `nsh-platform-windows` v0.6.1 populates `PasteEvent.text`, `PasteEvent.files`
//! (clipboard PNG/BMP), and `PasteEvent.uris` (`CF_HDROP`), but does **not** yet
//! read Windows `HTML Format` (`CF_HTML`) into `PasteEvent.html`. Reading
//! `CF_HTML` here via minimal `user32`/`kernel32` FFI restores 1:1 rich paste
//! from v0.14.11 (Flutter Mica), browsers, Word, Feishu, and same-window copies.

use std::path::Path;

/// Reads Windows clipboard `HTML Format` (`CF_HTML`) when present and slices it
/// down to the selected HTML fragment. Returns `None` on non-Windows platforms
/// or when no valid `CF_HTML` payload is on the clipboard.
#[cfg(target_os = "windows")]
/// Reads the Windows `CF_HTML` fragment only when the OS clipboard matches the
/// incoming `PasteEvent`'s plain text (or when `expected_text` is empty), so
/// synthetic/headless paste events are never hijacked by unrelated host HTML.
pub fn read_windows_cf_html(expected_text: &str) -> Option<String> {
  use std::ffi::c_void;

  const CF_UNICODETEXT: u32 = 13;

  #[link(name = "user32")]
  unsafe extern "system" {
    fn RegisterClipboardFormatW(lpsz_format: *const u16) -> u32;
    fn IsClipboardFormatAvailable(format: u32) -> i32;
    fn OpenClipboard(hwnd_new_owner: *mut c_void) -> i32;
    fn GetClipboardData(u_format: u32) -> *mut c_void;
    fn CloseClipboard() -> i32;
  }

  #[link(name = "kernel32")]
  unsafe extern "system" {
    fn GlobalLock(h_mem: *mut c_void) -> *mut c_void;
    fn GlobalUnlock(h_mem: *mut c_void) -> i32;
    fn GlobalSize(h_mem: *mut c_void) -> usize;
  }

  // UTF-16LE null-terminated "HTML Format"
  let name_u16: Vec<u16> = "HTML Format".encode_utf16().chain(std::iter::once(0)).collect();
  unsafe {
    let fmt = RegisterClipboardFormatW(name_u16.as_ptr());
    if fmt == 0 || IsClipboardFormatAvailable(fmt) == 0 {
      return None;
    }
    // Retry briefly when Windows clipboard history (Win+V), antivirus, or the
    // preceding `nsh-platform-windows` text/image read momentarily holds the lock.
    let mut opened = false;
    for _ in 0..8 {
      if OpenClipboard(std::ptr::null_mut()) != 0 {
        opened = true;
        break;
      }
      std::thread::sleep(std::time::Duration::from_millis(8));
    }
    if !opened {
      return None;
    }

    let text_matches = if expected_text.is_empty() {
      true
    } else {
      let handle = GetClipboardData(CF_UNICODETEXT);
      if handle.is_null() {
        false
      } else {
        let ptr = GlobalLock(handle) as *const u8;
        let os_text = if ptr.is_null() {
          None
        } else {
          let size = GlobalSize(handle);
          let slice = std::slice::from_raw_parts(ptr, size);
          let u16s: Vec<u16> = slice
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| u16::from_le_bytes(c))
            .take_while(|&u| u != 0)
            .collect();
          let _ = GlobalUnlock(handle);
          Some(String::from_utf16_lossy(&u16s))
        };
        os_text.is_some_and(|t| t.replace("\r\n", "\n") == expected_text.replace("\r\n", "\n"))
      }
    };

    let raw_bytes: Option<Vec<u8>> = if !text_matches {
      None
    } else {
      let handle = GetClipboardData(fmt);
      if handle.is_null() {
        None
      } else {
        let ptr = GlobalLock(handle) as *const u8;
        if ptr.is_null() {
          None
        } else {
          let size = GlobalSize(handle);
          let bytes = if size > 0 {
            let slice = std::slice::from_raw_parts(ptr, size);
            let nul_end = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
            Some(slice[..nul_end].to_vec())
          } else {
            None
          };
          let _ = GlobalUnlock(handle);
          bytes
        }
      }
    };
    let _ = CloseClipboard();
    let bytes = raw_bytes?;
    let text = String::from_utf8_lossy(&bytes);
    extract_cf_html_fragment(&text)
  }
}

#[cfg(not(target_os = "windows"))]
pub fn read_windows_cf_html(_expected_text: &str) -> Option<String> {
  None
}

/// Extracts the exact selected HTML fragment from either a Windows `CF_HTML`
/// payload (with `Version:` / `StartHTML:` / `StartFragment:` / `EndFragment:`
/// UTF-8 byte offsets) or raw HTML markup.
///
/// Mirrors `main:clients/mica_flutter/lib/editor/cf_html.dart`:
/// - Non-CF_HTML markup is returned unchanged.
/// - Valid CF_HTML byte offsets slice the exact selected fragment (excluding
///   surrounding page context).
/// - Malformed CF_HTML headers fall back to `<!--StartFragment-->`..`<!--EndFragment-->`
///   if present, or `None` so callers fall back safely to plain/markdown text.
pub fn extract_cf_html_fragment(raw: &str) -> Option<String> {
  if raw.is_empty() {
    return None;
  }
  let head_end = raw.floor_char_boundary(raw.len().min(4096));
  let head = &raw[..head_end];
  if !raw.starts_with("Version:") || !head.contains("StartHTML:") {
    return Some(raw.to_owned());
  }

  let parse_offset = |key: &str| -> Option<usize> {
    let idx = head.find(key)?;
    let rest = &head[idx + key.len()..];
    let digits: String = rest
      .chars()
      .skip_while(|c| *c == ' ' || *c == '\t')
      .take_while(|c| c.is_ascii_digit())
      .collect();
    digits.parse::<usize>().ok()
  };

  let bytes = raw.as_bytes();
  let start_html = parse_offset("StartHTML:");
  let end_html = parse_offset("EndHTML:");
  let start_frag = parse_offset("StartFragment:");
  let end_frag = parse_offset("EndFragment:");

  if let (Some(s), Some(e)) = (start_frag, end_frag)
    && s < e
    && e <= bytes.len()
    && start_html.is_none_or(|sh| sh <= s)
    && end_html.is_none_or(|eh| eh >= e)
    && let Ok(fragment) = std::str::from_utf8(&bytes[s..e])
  {
    return Some(fragment.to_owned());
  }

  // Fallback to HTML fragment markers if byte offsets were skewed.
  if let (Some(s), Some(e)) = (
    raw.find("<!--StartFragment-->"),
    raw.find("<!--EndFragment-->"),
  ) && e > s + 20
  {
    return Some(raw[s + 20..e].to_owned());
  }

  None
}

/// Returns `true` when clipboard HTML contains a real data `<table>` without
/// embedded `<img>` tags (e.g. copied from Excel / Google Sheets / Mica tables),
/// so callers prefer pasting the table over a preview bitmap flavor.
pub fn clipboard_html_is_data_table(html: &str) -> bool {
  let lower = html.to_ascii_lowercase();
  let has_table = lower.contains("<table ") || lower.contains("<table>") || lower.contains("<table\n");
  if !has_table {
    return false;
  }
  let has_img = lower.contains("<img ") || lower.contains("<img/") || lower.contains("<img>");
  !has_img
}

/// Persists pasted image bytes so `image(ImageSrc::from(uri))` can render them
/// reliably across restarts without bloating Markdown with multi-megabyte base64.
/// - Desktop: writes to `<data_local_dir>/mica/pasted-images/<uuid>.<ext>` and
///   returns a `file:///...` URI.
/// - WebAssembly: returns a `data:<mime>;base64,...` URI.
pub fn persist_pasted_image(bytes: &[u8], mime: &str, name: &str) -> Option<String> {
  if bytes.is_empty() {
    return None;
  }
  let ext = extension_for_image(bytes, mime, name);

  #[cfg(not(target_arch = "wasm32"))]
  {
    let base_dir = dirs_next::data_local_dir()
      .map(|d| d.join("mica").join("pasted-images"))
      .unwrap_or_else(|| std::env::temp_dir().join("mica-pasted-images"));
    std::fs::create_dir_all(&base_dir).ok()?;
    let filename = format!("img-{}.{ext}", uuid::Uuid::new_v4());
    let path = base_dir.join(filename);
    std::fs::write(&path, bytes).ok()?;
    Some(path_to_file_uri(&path))
  }

  #[cfg(target_arch = "wasm32")]
  {
    let resolved_mime = if mime.starts_with("image/") {
      mime
    } else {
      match ext {
        "jpg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        _ => "image/png",
      }
    };
    Some(format!("data:{resolved_mime};base64,{}", encode_base64(bytes)))
  }
}

/// Recognizes whether a pasted `file://` URI (e.g. copied from File Explorer /
/// Finder) points to a supported image file and returns `(uri, alt_name)`.
pub fn image_uri_from_pasted_file_uri(uri: &str) -> Option<(String, String)> {
  let trimmed = uri.trim();
  if trimmed.is_empty() {
    return None;
  }
  let without_query = trimmed.split(['?', '#']).next().unwrap_or(trimmed);
  let lower = without_query.to_ascii_lowercase();
  let is_img = [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".svg", ".avif"]
    .iter()
    .any(|ext| lower.ends_with(ext));
  if !is_img {
    return None;
  }
  let alt = without_query
    .rsplit(['/', '\\'])
    .next()
    .and_then(|f| Path::new(f).file_stem())
    .and_then(|s| s.to_str())
    .filter(|s| !s.is_empty())
    .unwrap_or("图片")
    .to_owned();
  let normalized = if trimmed.starts_with("file://")
    || trimmed.starts_with("http://")
    || trimmed.starts_with("https://")
  {
    trimmed.to_owned()
  } else {
    path_to_file_uri(Path::new(trimmed))
  };
  Some((normalized, alt))
}

/// Reads the natural `(width, height)` in pixels of a local `file://` image
/// (supports PNG, JPEG, GIF, BMP, WebP headers without external dependencies).
#[cfg(not(target_arch = "wasm32"))]
pub fn probe_local_image_dimensions(uri: &str) -> Option<(u32, u32)> {
  let path = file_uri_to_local_path(uri)?;
  let mut file = std::fs::File::open(path).ok()?;
  let mut buf = vec![0u8; 64 * 1024];
  let n = std::io::Read::read(&mut file, &mut buf).ok()?;
  buf.truncate(n);
  probe_image_dimensions(&buf)
}

#[cfg(target_arch = "wasm32")]
pub fn probe_local_image_dimensions(_uri: &str) -> Option<(u32, u32)> {
  None
}

/// Parses `(width, height)` from PNG / GIF / BMP / JPEG / WebP header bytes.
pub fn probe_image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
  // PNG: 8-byte signature + IHDR chunk (width @ 16..20, height @ 20..24)
  if bytes.len() >= 24 && &bytes[0..8] == b"\x89PNG\r\n\x1a\n" {
    let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    return (w > 0 && h > 0).then_some((w, h));
  }
  // GIF87a / GIF89a: width @ 6..8, height @ 8..10 (LE)
  if bytes.len() >= 10 && (&bytes[0..6] == b"GIF87a" || &bytes[0..6] == b"GIF89a") {
    let w = u16::from_le_bytes(bytes[6..8].try_into().ok()?) as u32;
    let h = u16::from_le_bytes(bytes[8..10].try_into().ok()?) as u32;
    return (w > 0 && h > 0).then_some((w, h));
  }
  // BMP: "BM" + DIB header width @ 18..22 (i32 LE), height @ 22..26 (i32 LE)
  if bytes.len() >= 26 && &bytes[0..2] == b"BM" {
    let w = i32::from_le_bytes(bytes[18..22].try_into().ok()?).unsigned_abs();
    let h = i32::from_le_bytes(bytes[22..26].try_into().ok()?).unsigned_abs();
    return (w > 0 && h > 0).then_some((w, h));
  }
  // WebP: RIFF....WEBP
  if bytes.len() >= 30 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
    match &bytes[12..16] {
      b"VP8 " if bytes.len() >= 30 => {
        let w = (u16::from_le_bytes([bytes[26], bytes[27]]) & 0x3FFF) as u32;
        let h = (u16::from_le_bytes([bytes[28], bytes[29]]) & 0x3FFF) as u32;
        return (w > 0 && h > 0).then_some((w, h));
      }
      b"VP8L" if bytes.len() >= 25 => {
        let bits = u32::from_le_bytes(bytes[21..25].try_into().ok()?);
        let w = (bits & 0x3FFF) + 1;
        let h = ((bits >> 14) & 0x3FFF) + 1;
        return Some((w, h));
      }
      b"VP8X" if bytes.len() >= 30 => {
        let w = (bytes[24] as u32 | ((bytes[25] as u32) << 8) | ((bytes[26] as u32) << 16)) + 1;
        let h = (bytes[27] as u32 | ((bytes[28] as u32) << 8) | ((bytes[29] as u32) << 16)) + 1;
        return Some((w, h));
      }
      _ => {}
    }
  }
  // JPEG: scan SOF0..SOF15 markers (excluding DHT/DAC/JPG)
  if bytes.len() >= 4 && bytes[0] == 0xFF && bytes[1] == 0xD8 {
    let mut i = 2usize;
    while i + 9 < bytes.len() {
      if bytes[i] != 0xFF {
        break;
      }
      let marker = bytes[i + 1];
      if marker == 0xFF {
        i += 1;
        continue;
      }
      let seg_len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
      if seg_len < 2 {
        break;
      }
      let is_sof = matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
      if is_sof && i + 8 < bytes.len() {
        let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
        let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
        return (w > 0 && h > 0).then_some((w, h));
      }
      i += 2 + seg_len;
    }
  }
  None
}

fn extension_for_image(bytes: &[u8], mime: &str, name: &str) -> &'static str {
  if bytes.len() >= 8 && &bytes[0..8] == b"\x89PNG\r\n\x1a\n" {
    return "png";
  }
  if bytes.len() >= 3 && bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF {
    return "jpg";
  }
  if bytes.len() >= 6 && (&bytes[0..6] == b"GIF87a" || &bytes[0..6] == b"GIF89a") {
    return "gif";
  }
  if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
    return "webp";
  }
  if bytes.len() >= 2 && &bytes[0..2] == b"BM" {
    return "bmp";
  }
  match mime.to_ascii_lowercase().as_str() {
    "image/jpeg" | "image/jpg" => "jpg",
    "image/gif" => "gif",
    "image/webp" => "webp",
    "image/bmp" => "bmp",
    "image/svg+xml" => "svg",
    _ => {
      let lower = name.to_ascii_lowercase();
      if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "jpg"
      } else if lower.ends_with(".gif") {
        "gif"
      } else if lower.ends_with(".webp") {
        "webp"
      } else if lower.ends_with(".bmp") {
        "bmp"
      } else if lower.ends_with(".svg") {
        "svg"
      } else {
        "png"
      }
    }
  }
}

pub fn path_to_file_uri(path: &Path) -> String {
  let raw = path.to_string_lossy().replace('\\', "/");
  let escaped = raw.replace(' ', "%20").replace('#', "%23");
  if escaped.starts_with('/') {
    format!("file://{escaped}")
  } else {
    format!("file:///{escaped}")
  }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn file_uri_to_local_path(uri: &str) -> Option<std::path::PathBuf> {
  let rest = uri.strip_prefix("file://")?;
  let rest = rest.strip_prefix("localhost").unwrap_or(rest);
  let decoded = percent_decode(rest);
  #[cfg(target_os = "windows")]
  {
    let cleaned = decoded.strip_prefix('/').unwrap_or(&decoded);
    Some(std::path::PathBuf::from(cleaned.replace('/', "\\")))
  }
  #[cfg(not(target_os = "windows"))]
  {
    Some(std::path::PathBuf::from(decoded))
  }
}

#[cfg(not(target_arch = "wasm32"))]
fn percent_decode(s: &str) -> String {
  let bytes = s.as_bytes();
  let mut out = Vec::with_capacity(bytes.len());
  let mut i = 0;
  while i < bytes.len() {
    if bytes[i] == b'%'
      && i + 2 < bytes.len()
      && let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2]))
    {
      out.push((h << 4) | l);
      i += 3;
      continue;
    }
    out.push(bytes[i]);
    i += 1;
  }
  String::from_utf8_lossy(&out).into_owned()
}

#[cfg(not(target_arch = "wasm32"))]
fn hex_val(b: u8) -> Option<u8> {
  match b {
    b'0'..=b'9' => Some(b - b'0'),
    b'a'..=b'f' => Some(b - b'a' + 10),
    b'A'..=b'F' => Some(b - b'A' + 10),
    _ => None,
  }
}

/// Sniff MIME type from image magic bytes, falling back to file extension.
pub fn mime_for_image(bytes: &[u8], name: &str) -> &'static str {
  if bytes.len() >= 8 && &bytes[0..8] == b"\x89PNG\r\n\x1a\n" {
    return "image/png";
  }
  if bytes.len() >= 3 && bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF {
    return "image/jpeg";
  }
  if bytes.len() >= 6 && (&bytes[0..6] == b"GIF87a" || &bytes[0..6] == b"GIF89a") {
    return "image/gif";
  }
  if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
    return "image/webp";
  }
  if bytes.len() >= 12
    && &bytes[4..8] == b"ftyp"
    && (&bytes[8..12] == b"avif" || &bytes[8..12] == b"avis")
  {
    return "image/avif";
  }
  if bytes.len() >= 2 && &bytes[0..2] == b"BM" {
    return "image/bmp";
  }
  let head = std::str::from_utf8(&bytes[..bytes.len().min(256)]).unwrap_or("");
  if head.trim_start().starts_with("<svg") || head.contains("<svg ") {
    return "image/svg+xml";
  }
  let lower = name.to_ascii_lowercase();
  if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
    "image/jpeg"
  } else if lower.ends_with(".gif") {
    "image/gif"
  } else if lower.ends_with(".webp") {
    "image/webp"
  } else if lower.ends_with(".avif") {
    "image/avif"
  } else if lower.ends_with(".bmp") {
    "image/bmp"
  } else if lower.ends_with(".svg") {
    "image/svg+xml"
  } else {
    "image/png"
  }
}

/// Standard base64 encoder (shared across desktop clipboard & Web `data:` URIs).
pub fn encode_base64(bytes: &[u8]) -> String {
  const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
  for chunk in bytes.chunks(3) {
    let b0 = chunk[0] as u32;
    let b1 = *chunk.get(1).unwrap_or(&0) as u32;
    let b2 = *chunk.get(2).unwrap_or(&0) as u32;
    let n = (b0 << 16) | (b1 << 8) | b2;
    out.push(TABLE[((n >> 18) & 0x3F) as usize] as char);
    out.push(TABLE[((n >> 12) & 0x3F) as usize] as char);
    if chunk.len() > 1 {
      out.push(TABLE[((n >> 6) & 0x3F) as usize] as char);
    } else {
      out.push('=');
    }
    if chunk.len() > 2 {
      out.push(TABLE[(n & 0x3F) as usize] as char);
    } else {
      out.push('=');
    }
  }
  out
}

/// Standard base64 decoder (ignores ASCII whitespace; used for `data:` URIs).
pub fn decode_base64(input: &str) -> Option<Vec<u8>> {
  let clean: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
  if clean.is_empty() || !clean.len().is_multiple_of(4) {
    return None;
  }
  let val = |b: u8| -> Option<u32> {
    match b {
      b'A'..=b'Z' => Some((b - b'A') as u32),
      b'a'..=b'z' => Some((b - b'a' + 26) as u32),
      b'0'..=b'9' => Some((b - b'0' + 52) as u32),
      b'+' => Some(62),
      b'/' => Some(63),
      _ => None,
    }
  };
  let mut out = Vec::with_capacity(clean.len() / 4 * 3);
  for chunk in clean.as_chunks::<4>().0 {
    let a = val(chunk[0])?;
    let b = val(chunk[1])?;
    out.push(((a << 2) | (b >> 4)) as u8);
    if chunk[2] != b'=' {
      let c = val(chunk[2])?;
      out.push((((b & 0x0F) << 4) | (c >> 2)) as u8);
      if chunk[3] != b'=' {
        let d = val(chunk[3])?;
        out.push((((c & 0x03) << 6) | d) as u8);
      }
    }
  }
  Some(out)
}

/// Parse a `data:<mime>;base64,<payload>` URI into `(mime, decoded_bytes)`.
pub fn decode_data_uri(uri: &str) -> Option<(String, Vec<u8>)> {
  let rest = uri.trim().strip_prefix("data:")?;
  let (meta, body) = rest.split_once(',')?;
  if !meta.ends_with(";base64") {
    return None;
  }
  let mime = meta.strip_suffix(";base64")?.trim();
  let mime = if mime.is_empty() { "image/png" } else { mime };
  let bytes = decode_base64(body)?;
  Some((mime.to_owned(), bytes))
}

/// Write an encoded raster image onto the system clipboard across all platforms:
/// - **Windows**: writes BOTH `"PNG"` (alpha-preserving, Chrome/Mica/Feishu)
///   and `CF_DIB` (32-bit BGRA, WeChat/Paint/Word/PowerPoint) atomically.
/// - **macOS / Linux**: ensures PNG and delegates to `nsh_core::element::set_clipboard_image`.
/// - **WebAssembly**: delegates to `nsh_core::element::set_clipboard_image` (`ClipboardItem` + PNG transcode).
pub fn copy_image_bytes_to_clipboard(bytes: &[u8], mime: &str) -> bool {
  if bytes.is_empty() {
    return false;
  }

  #[cfg(target_os = "windows")]
  {
    let (png_bytes, dib_bytes) = prepare_windows_clipboard_image(bytes);
    if write_windows_clipboard_image(png_bytes.as_deref(), dib_bytes.as_deref()) {
      return true;
    }
    if let Some(png) = png_bytes {
      let uri = format!("data:image/png;base64,{}", encode_base64(&png));
      nsh_core::element::set_clipboard_image(uri);
      return true;
    }
    let _ = mime;
    false
  }

  #[cfg(all(not(target_arch = "wasm32"), not(target_os = "windows")))]
  {
    let png = ensure_png_bytes(bytes).unwrap_or_else(|| bytes.to_vec());
    let resolved_mime = if png.starts_with(b"\x89PNG\r\n\x1a\n") {
      "image/png"
    } else {
      mime
    };
    let uri = format!("data:{resolved_mime};base64,{}", encode_base64(&png));
    nsh_core::element::set_clipboard_image(uri);
    true
  }

  #[cfg(target_arch = "wasm32")]
  {
    let resolved_mime = if mime.starts_with("image/") {
      mime
    } else {
      mime_for_image(bytes, "")
    };
    let uri = format!("data:{resolved_mime};base64,{}", encode_base64(bytes));
    nsh_core::element::set_clipboard_image(uri);
    true
  }
}

#[cfg(not(target_arch = "wasm32"))]
fn ensure_png_bytes(bytes: &[u8]) -> Option<Vec<u8>> {
  if bytes.len() >= 8 && &bytes[0..8] == b"\x89PNG\r\n\x1a\n" {
    return Some(bytes.to_vec());
  }
  let decoded = image::load_from_memory(bytes).ok()?;
  let mut buf = std::io::Cursor::new(Vec::new());
  decoded.write_to(&mut buf, image::ImageFormat::Png).ok()?;
  Some(buf.into_inner())
}

#[cfg(target_os = "windows")]
fn prepare_windows_clipboard_image(bytes: &[u8]) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
  let png_out = ensure_png_bytes(bytes);
  let dib_out = image::load_from_memory(bytes).ok().map(|img| {
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let pixel_bytes = (w as usize) * (h as usize) * 4;
    let mut dib = Vec::with_capacity(40 + pixel_bytes);
    // BITMAPINFOHEADER (40 bytes)
    dib.extend_from_slice(&40u32.to_le_bytes()); // biSize
    dib.extend_from_slice(&(w as i32).to_le_bytes()); // biWidth
    dib.extend_from_slice(&(h as i32).to_le_bytes()); // biHeight (positive = bottom-up)
    dib.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    dib.extend_from_slice(&32u16.to_le_bytes()); // biBitCount (BGRA)
    dib.extend_from_slice(&0u32.to_le_bytes()); // biCompression = BI_RGB
    dib.extend_from_slice(&(pixel_bytes as u32).to_le_bytes()); // biSizeImage
    dib.extend_from_slice(&0i32.to_le_bytes()); // biXPelsPerMeter
    dib.extend_from_slice(&0i32.to_le_bytes()); // biYPelsPerMeter
    dib.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed
    dib.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant
    for y in (0..h).rev() {
      for x in 0..w {
        let p = rgba.get_pixel(x, y).0;
        // Composite semi-transparent pixels over opaque white for legacy `CF_DIB`
        // consumers (MS Paint, older Word/Office, WeChat) that ignore or black-fill
        // alpha in 32-bit `BI_RGB`, while the companion `"PNG"` format preserves alpha.
        let a = u32::from(p[3]);
        let inv = 255 - a;
        let r = ((u32::from(p[0]) * a + 255 * inv + 127) / 255) as u8;
        let g = ((u32::from(p[1]) * a + 255 * inv + 127) / 255) as u8;
        let b = ((u32::from(p[2]) * a + 255 * inv + 127) / 255) as u8;
        dib.extend_from_slice(&[b, g, r, 0xFF]);
      }
    }
    dib
  });
  (png_out, dib_out)
}

#[cfg(target_os = "windows")]
fn write_windows_clipboard_image(png: Option<&[u8]>, dib: Option<&[u8]>) -> bool {
  use std::ffi::c_void;

  const CF_DIB: u32 = 8;
  const GMEM_MOVEABLE: u32 = 0x0002;

  #[link(name = "user32")]
  unsafe extern "system" {
    fn RegisterClipboardFormatW(lpsz_format: *const u16) -> u32;
    fn OpenClipboard(hwnd_new_owner: *mut c_void) -> i32;
    fn EmptyClipboard() -> i32;
    fn SetClipboardData(u_format: u32, h_mem: *mut c_void) -> *mut c_void;
    fn CloseClipboard() -> i32;
  }

  #[link(name = "kernel32")]
  unsafe extern "system" {
    fn GlobalAlloc(u_flags: u32, dw_bytes: usize) -> *mut c_void;
    fn GlobalLock(h_mem: *mut c_void) -> *mut c_void;
    fn GlobalUnlock(h_mem: *mut c_void) -> i32;
    fn GlobalFree(h_mem: *mut c_void) -> *mut c_void;
  }

  if png.is_none() && dib.is_none() {
    return false;
  }

  let png_name: Vec<u16> = "PNG".encode_utf16().chain(std::iter::once(0)).collect();
  unsafe {
    let png_fmt = RegisterClipboardFormatW(png_name.as_ptr());
    let mut opened = false;
    for _ in 0..8 {
      if OpenClipboard(std::ptr::null_mut()) != 0 {
        opened = true;
        break;
      }
      std::thread::sleep(std::time::Duration::from_millis(8));
    }
    if !opened {
      return false;
    }
    if EmptyClipboard() == 0 {
      let _ = CloseClipboard();
      return false;
    }

    let put_fmt = |fmt: u32, data: &[u8]| -> bool {
      if fmt == 0 || data.is_empty() {
        return false;
      }
      let h = GlobalAlloc(GMEM_MOVEABLE, data.len());
      if h.is_null() {
        return false;
      }
      let ptr = GlobalLock(h) as *mut u8;
      if ptr.is_null() {
        let _ = GlobalFree(h);
        return false;
      }
      std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
      let _ = GlobalUnlock(h);
      if SetClipboardData(fmt, h).is_null() {
        let _ = GlobalFree(h);
        return false;
      }
      true
    };

    let mut wrote_any = false;
    if let Some(p) = png {
      wrote_any |= put_fmt(png_fmt, p);
    }
    if let Some(d) = dib {
      wrote_any |= put_fmt(CF_DIB, d);
    }
    let _ = CloseClipboard();
    wrote_any
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn extract_cf_html_fragment_slices_exact_utf8_byte_offsets() {
    let frag = "<h1>泰国项目</h1><p><strong>加粗</strong>与<code>code</code></p>";
    let wrapped = nsh_core::platform::clipboard::cf_html(frag);
    assert_eq!(extract_cf_html_fragment(&wrapped).as_deref(), Some(frag));
  }

  #[test]
  fn extract_cf_html_fragment_passes_raw_html_through() {
    let raw = "<h2>Title</h2><ul><li>Item</li></ul>";
    assert_eq!(extract_cf_html_fragment(raw).as_deref(), Some(raw));
  }

  #[test]
  fn clipboard_html_table_vs_image_discrimination() {
    assert!(clipboard_html_is_data_table("<table><tr><td>A</td></tr></table>"));
    assert!(!clipboard_html_is_data_table("<table><tr><td><img src=\"a.png\"></td></tr></table>"));
    assert!(!clipboard_html_is_data_table("<p>Hello</p>"));
  }

  #[test]
  fn probe_png_and_bmp_dimensions() {
    let png: [u8; 24] = [
      0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A,
      0, 0, 0, 13, b'I', b'H', b'D', b'R',
      0, 0, 0x02, 0x80, // 640
      0, 0, 0x01, 0xE0, // 480
    ];
    assert_eq!(probe_image_dimensions(&png), Some((640, 480)));
  }

  #[test]
  fn image_uri_detection_handles_explorer_files() {
    let hit = image_uri_from_pasted_file_uri("file:///C:/Users/demo/screen%20shot.png");
    assert_eq!(
      hit,
      Some((
        "file:///C:/Users/demo/screen%20shot.png".to_owned(),
        "screen%20shot".to_owned()
      ))
    );
    assert_eq!(image_uri_from_pasted_file_uri("file:///C:/Users/demo/notes.txt"), None);
  }

  #[test]
  fn base64_and_data_uri_round_trip() {
    let raw = b"\x89PNG\r\n\x1a\nhello-mica-image";
    let b64 = encode_base64(raw);
    assert_eq!(decode_base64(&b64).as_deref(), Some(raw.as_slice()));
    let uri = format!("data:image/png;base64,{b64}");
    let (mime, decoded) = decode_data_uri(&uri).expect("decode data uri");
    assert_eq!(mime, "image/png");
    assert_eq!(decoded, raw);
  }

  #[cfg(target_os = "windows")]
  #[test]
  fn windows_clipboard_image_prepares_both_png_and_dib() {
    let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([0x10, 0x20, 0x30, 0xFF]));
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let (out_png, out_dib) = prepare_windows_clipboard_image(png.get_ref());
    assert!(out_png.as_ref().is_some_and(|b| b.starts_with(b"\x89PNG\r\n\x1a\n")));
    let dib = out_dib.expect("CF_DIB bytes");
    // 40-byte BITMAPINFOHEADER + 2*2*4 BGRA pixels
    assert_eq!(dib.len(), 40 + 16);
    assert_eq!(&dib[0..4], &40u32.to_le_bytes());
    assert_eq!(&dib[40..44], &[0x30, 0x20, 0x10, 0xFF]);

    // Transparent pixels must composite over white in CF_DIB (never black in Paint/Word),
    // while PNG stays valid and alpha-preserving.
    let transparent = image::RgbaImage::from_pixel(1, 1, image::Rgba([0x00, 0x00, 0x00, 0x00]));
    let mut t_png = std::io::Cursor::new(Vec::new());
    transparent.write_to(&mut t_png, image::ImageFormat::Png).unwrap();
    let (t_out_png, t_out_dib) = prepare_windows_clipboard_image(t_png.get_ref());
    assert!(t_out_png.is_some());
    let t_dib = t_out_dib.expect("transparent CF_DIB");
    assert_eq!(&t_dib[40..44], &[0xFF, 0xFF, 0xFF, 0xFF]);
  }
}
