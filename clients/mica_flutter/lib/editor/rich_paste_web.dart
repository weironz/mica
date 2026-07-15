// Web implementation of rich clipboard paste. Captures the native `paste`
// event (capture phase, before Flutter's hidden input) and, when the clipboard
// carries `text/html`, converts that HTML to Markdown so structure (headings,
// lists, code, tables) survives — matching how Typora pastes web content.
//
// The HTML→Markdown conversion itself lives in html_to_markdown.dart
// (pure Dart via package:html, shared with the desktop clipboard pulls) —
// there used to be a dart:html mirror of it here, and the two drifted.
//
// dart:html is legacy but dependency-free and sufficient for reading the
// clipboard's HTML flavor.
// ignore_for_file: deprecated_member_use, avoid_web_libraries_in_flutter
import 'dart:html' as html;
import 'dart:typed_data';

import 'html_to_markdown.dart';

typedef RichPasteHandler = bool Function(String markdown, String plain, bool rich);
typedef ImagePasteHandler = void Function(
  Uint8List bytes,
  String mime,
  String name,
);

RichPasteHandler? _handler;
ImagePasteHandler? _imageHandler;
bool _installed = false;

// Web reads the clipboard through the DOM `paste` event (setRichPasteHandler),
// not by explicit pull, so these facade pulls are unused on web.
Future<Uint8List?> readClipboardImage() async => null;
Future<String?> readClipboardHtmlAsMarkdown() async => null;
Future<String?> readClipboardTableAsMarkdown() async => null;

void setRichImagePasteHandler(ImagePasteHandler? handler) {
  _imageHandler = handler;
}

/// Extract a pasted image file from `files` (preferred) or `items`.
html.File? _clipboardImage(html.DataTransfer data) {
  final files = data.files;
  if (files != null) {
    for (final f in files) {
      if (f.type.startsWith('image/')) return f;
    }
  }
  final items = data.items;
  if (items != null) {
    final n = items.length ?? 0;
    for (var i = 0; i < n; i++) {
      final item = items[i];
      if (item.kind == 'file' && (item.type?.startsWith('image/') ?? false)) {
        final file = item.getAsFile();
        if (file != null) return file;
      }
    }
  }
  return null;
}

/// dart2js types FileReader's readAsArrayBuffer result inconsistently; accept
/// the common byte representations.
Uint8List? _bytesOf(Object? result) {
  if (result is ByteBuffer) return result.asUint8List();
  if (result is Uint8List) return result;
  if (result is List<int>) return Uint8List.fromList(result);
  return null;
}

void setRichPasteHandler(RichPasteHandler? handler) {
  _handler = handler;
  if (_installed) return;
  _installed = true;
  html.document.addEventListener('paste', (event) {
    final data = (event as html.ClipboardEvent).clipboardData;
    if (data == null) return;

    // HTML that carries a real DATA <table> (Excel / Sheets / a web table)
    // wins over the bitmap: Excel also puts a picture of the copied cells on
    // the clipboard, and image-first pasted spreadsheets as screenshots. An
    // <img> inside the table means it's a LAYOUT table wrapping a picture
    // (Word/Outlook) — the bitmap must win or the image is silently lost.
    final htmlFlavor = data.getData('text/html');
    final lowerHtml = htmlFlavor.toLowerCase();
    final hasTable = RegExp(r'<table[\s>]').hasMatch(lowerHtml) &&
        !RegExp(r'<img[\s>/]').hasMatch(lowerHtml);

    // A pasted bitmap (screenshot, copied image) arrives as a file — in the
    // clipboard's `files` and/or `items`. Upload it rather than dropping as text.
    final imageHandler = _imageHandler;
    if (imageHandler != null && !hasTable) {
      final image = _clipboardImage(data);
      if (image != null) {
        event.preventDefault();
        event.stopPropagation();
        final reader = html.FileReader()..readAsArrayBuffer(image);
        reader.onLoadEnd.first.then((_) {
          final bytes = _bytesOf(reader.result);
          if (bytes != null) {
            imageHandler(
              bytes,
              image.type.isEmpty ? 'image/png' : image.type,
              image.name.isEmpty ? 'pasted-image.png' : image.name,
            );
          }
        });
        return;
      }
    }

    final handler = _handler;
    if (handler == null) return;
    final plain = data.getData('text/plain');
    final hasHtml = htmlFlavor.trim().isNotEmpty;
    final content = hasHtml ? htmlToMarkdown(htmlFlavor) : plain;

    if (handler(content, plain, hasHtml)) {
      event.preventDefault();
      event.stopPropagation();
    }
  }, true);
}
