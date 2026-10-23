// Web implementation of rich clipboard paste. Captures the native `paste`
// event (capture phase, before Flutter's hidden input) and, when the clipboard
// carries `text/html`, converts that HTML to Markdown so structure (headings,
// lists, code, tables) survives — matching how Typora pastes web content.
//
// The HTML→Markdown conversion itself lives in html_to_markdown.dart
// (pure Dart via package:html, shared with the desktop clipboard pulls) —
// there used to be a dart:html mirror of it here, and the two drifted.
//
// The DOM access goes through package:web + dart:js_interop (the dart:html
// successor), which is also what the dart2wasm compile target supports.
import 'dart:js_interop';
import 'dart:typed_data';

import 'package:web/web.dart' as web;

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
web.File? _clipboardImage(web.DataTransfer data) {
  final files = data.files;
  for (var i = 0; i < files.length; i++) {
    final f = files.item(i);
    if (f != null && f.type.startsWith('image/')) return f;
  }
  final items = data.items;
  for (var i = 0; i < items.length; i++) {
    final item = items[i];
    if (item.kind == 'file' && item.type.startsWith('image/')) {
      final file = item.getAsFile();
      if (file != null) return file;
    }
  }
  return null;
}

void setRichPasteHandler(RichPasteHandler? handler) {
  _handler = handler;
  if (_installed) return;
  _installed = true;
  web.document.addEventListener('paste', ((web.Event event) {
    final data = (event as web.ClipboardEvent).clipboardData;
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
        () async {
          try {
            final buffer = await image.arrayBuffer().toDart;
            imageHandler(
              buffer.toDart.asUint8List(),
              image.type.isEmpty ? 'image/png' : image.type,
              image.name.isEmpty ? 'pasted-image.png' : image.name,
            );
          } catch (_) {
            // A read that rejects drops the paste, same as the old
            // FileReader path that never reached onLoadEnd.
          }
        }();
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
  }).toJS, true.toJS);
}
