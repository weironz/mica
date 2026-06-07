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
  final md = htmlToMarkdown(h);
  return md.trim().isEmpty ? null : md;
}
