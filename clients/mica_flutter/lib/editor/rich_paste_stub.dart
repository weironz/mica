/// Non-web (desktop/mobile) rich paste. The web variant hooks the DOM `paste`
/// event for HTML + image; off the web there is no such event, so the editor
/// pulls the clipboard explicitly on Ctrl+V (see editor.dart _pasteFromClipboard).
library;

import 'dart:typed_data';

import 'package:pasteboard/pasteboard.dart';

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

/// Read the clipboard's HTML flavor. TODO(batch 2c): wire a dart:html-free
/// HTML→Markdown so structured web paste survives on desktop too.
Future<String?> readClipboardHtml() async => null;
