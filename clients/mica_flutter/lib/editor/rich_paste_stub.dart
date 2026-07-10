/// Non-web (desktop/mobile) rich paste. The web variant hooks the DOM `paste`
/// event for HTML + image; off the web there is no such event, so the editor
/// pulls the clipboard explicitly on Ctrl+V (see editor.dart _pasteFromClipboard).
library;

import 'dart:typed_data';

import 'package:pasteboard/pasteboard.dart';

import 'html_to_markdown.dart';

/// Returns true if the paste was consumed by the handler.
typedef RichPasteHandler = bool Function(String markdown, String plain, bool rich);
typedef ImagePasteHandler = void Function(
  Uint8List bytes,
  String mime,
  String name,
);

// The handler-registration API is web-only (DOM event hook); no-op here.
void setRichPasteHandler(RichPasteHandler? handler) {}
void setRichImagePasteHandler(ImagePasteHandler? handler) {}

/// Read a bitmap from the system clipboard (PNG bytes), or null if none.
Future<Uint8List?> readClipboardImage() => Pasteboard.image;

/// Read the clipboard's HTML flavor and convert it to Markdown (so structured
/// content from a browser/Word survives), or null if there is no usable HTML.
Future<String?> readClipboardHtmlAsMarkdown() async {
  final h = await Pasteboard.html;
  if (h == null || h.trim().isEmpty) return null;
  final md = htmlToMarkdown(stripCfHtmlHeader(h));
  return md.trim().isEmpty ? null : md;
}

/// Windows wraps clipboard HTML in the CF_HTML ("HTML Format") clipboard format,
/// whose plaintext descriptor header (`Version:`, `StartHTML:`, `EndHTML:`,
/// `StartFragment:`, `EndFragment:`, optional `SourceURL:`) precedes the real
/// `[!DOCTYPE html]...[html]...` payload. The `pasteboard` plugin returns the
/// whole buffer verbatim, so strip the header before parsing — otherwise it
/// leaks in as a literal first line of the pasted content. A no-op on platforms
/// that hand back bare HTML (macOS/Linux/mobile), where the guard fails.
///
/// We cut at the first `<` (start of markup), NOT the Start/EndFragment numbers:
/// those are UTF-8 *byte* offsets, not Dart char (UTF-16) indices, so a
/// substring by them would mis-slice any non-ASCII content before the fragment.
String stripCfHtmlHeader(String html) {
  // CF_HTML always opens with the `Version:` descriptor; also require a
  // `StartHTML:` marker so real content that merely starts with "Version:" is
  // never mistaken for a header.
  final head = html.length < 256 ? html : html.substring(0, 256);
  if (!html.startsWith('Version:') || !head.contains('StartHTML:')) return html;
  final lt = html.indexOf('<');
  return lt <= 0 ? html : html.substring(lt);
}
