/// Windows CF_HTML ("HTML Format") fragment selection — shared by the desktop
/// clipboard pull and the web paste adapter.
///
/// The platform wraps clipboard HTML in a plaintext descriptor header
/// (`Version:`, `StartHTML:`, `EndHTML:`, `StartFragment:`, `EndFragment:`,
/// optional `SourceURL:`). The offsets are UTF-8 BYTE offsets into the whole
/// buffer, and they are the only way to tell the copied SELECTION from the page
/// context an app also serializes — ignoring them pastes "before … after"
/// along with the selected words.
///
/// Returns:
///  * [html] unchanged when it is not CF_HTML (macOS/Linux bare markup);
///  * the exact fragment substring for a valid header;
///  * `null` when the header exists but its bounds are missing or invalid.
///    Callers must then fall back to plain text — never to the whole HTML,
///    which is precisely the page context the user did not copy.
library;

import 'dart:convert';

final RegExp _startHtml = RegExp(r'StartHTML:(\d+)');
final RegExp _endHtml = RegExp(r'EndHTML:(\d+)');
final RegExp _startFragment = RegExp(r'StartFragment:(\d+)');
final RegExp _endFragment = RegExp(r'EndFragment:(\d+)');

int? _offset(RegExp re, String head) {
  final m = re.firstMatch(head);
  return m == null ? null : int.tryParse(m.group(1)!);
}

/// The fragment selected by a CF_HTML clipboard payload, or `null` when the
/// payload claims to be CF_HTML but cannot be sliced safely.
String? cfHtmlFragment(String html) {
  // CF_HTML always opens with the `Version:` descriptor; also require a
  // `StartHTML:` marker so real content that merely starts with "Version:" is
  // never mistaken for a header.
  final head = html.length < 4096 ? html : html.substring(0, 4096);
  if (!html.startsWith('Version:') || !head.contains('StartHTML:')) return html;

  final bytes = utf8.encode(html);
  final startHtml = _offset(_startHtml, head);
  final endHtml = _offset(_endHtml, head);
  final start = _offset(_startFragment, head);
  final end = _offset(_endFragment, head);
  if (start == null || end == null) return null;
  if (start < 0 || end <= start || end > bytes.length) return null;
  if (startHtml != null &&
      (startHtml < 0 || startHtml > bytes.length || start < startHtml)) {
    return null;
  }
  if (endHtml != null && (endHtml < end || endHtml > bytes.length)) {
    return null;
  }
  try {
    return utf8.decode(bytes.sublist(start, end));
  } on FormatException {
    return null;
  }
}