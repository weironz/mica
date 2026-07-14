/// Non-web (desktop/mobile) rich paste. The web variant hooks the DOM `paste`
/// event for HTML + image; off the web there is no such event, so the editor
/// pulls the clipboard explicitly on Ctrl+V (see editor.dart _pasteFromClipboard).
library;

import 'dart:ffi';
import 'dart:io' show Platform;
import 'dart:typed_data';

import 'package:ffi/ffi.dart';
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

/// Read an image from the system clipboard (PNG bytes), or null if none.
///
/// On Windows, prefer the clipboard's registered "PNG" format over
/// [Pasteboard.image]: pasteboard reads the flattened CF_DIB, which bakes a
/// transparent image's alpha to BLACK, but browsers (Chromium/Firefox) also
/// register a "PNG" format holding the original image WITH its alpha — so
/// pasting a transparent PNG copied from a web page keeps its transparency.
Future<Uint8List?> readClipboardImage() async {
  if (Platform.isWindows) {
    final png = _windowsClipboardPng();
    if (png != null && png.isNotEmpty) return png;
  }
  return Pasteboard.image;
}

/// Bytes of the Windows clipboard's registered "PNG" format, or null. Any
/// failure (format absent, clipboard busy) returns null → caller falls back to
/// [Pasteboard.image], so this can only improve on the flattened bitmap.
// Raw Win32 clipboard reads via dart:ffi (HANDLE/HGLOBAL as IntPtr = Dart int,
// so no win32-package type churn). user32: OpenClipboard/GetClipboardData/
// CloseClipboard/RegisterClipboardFormatW; kernel32: GlobalLock/Size/Unlock.
Uint8List? _windowsClipboardPng() {
  final user32 = DynamicLibrary.open('user32.dll');
  final kernel32 = DynamicLibrary.open('kernel32.dll');
  final registerFmt = user32.lookupFunction<Uint32 Function(Pointer<Utf16>),
      int Function(Pointer<Utf16>)>('RegisterClipboardFormatW');
  final openClip = user32
      .lookupFunction<Int32 Function(IntPtr), int Function(int)>('OpenClipboard');
  final getData = user32.lookupFunction<IntPtr Function(Uint32),
      int Function(int)>('GetClipboardData');
  final closeClip =
      user32.lookupFunction<Int32 Function(), int Function()>('CloseClipboard');
  final gLock = kernel32.lookupFunction<Pointer<Uint8> Function(IntPtr),
      Pointer<Uint8> Function(int)>('GlobalLock');
  final gUnlock = kernel32
      .lookupFunction<Int32 Function(IntPtr), int Function(int)>('GlobalUnlock');
  final gSize = kernel32
      .lookupFunction<IntPtr Function(IntPtr), int Function(int)>('GlobalSize');

  final fmtName = 'PNG'.toNativeUtf16();
  try {
    final fmt = registerFmt(fmtName);
    if (fmt == 0) return null;
    if (openClip(0) == 0) return null;
    try {
      final handle = getData(fmt);
      if (handle == 0) return null;
      final ptr = gLock(handle);
      if (ptr == nullptr) return null;
      try {
        final size = gSize(handle);
        if (size <= 0) return null;
        // Copy out of the OS-owned buffer before unlocking it.
        return Uint8List.fromList(ptr.asTypedList(size));
      } finally {
        gUnlock(handle);
      }
    } finally {
      closeClip();
    }
  } catch (_) {
    return null;
  } finally {
    malloc.free(fmtName);
  }
}

/// Read the clipboard's HTML flavor and convert it to Markdown (so structured
/// content from a browser/Word survives), or null if there is no usable HTML.
Future<String?> readClipboardHtmlAsMarkdown() async {
  final h = await Pasteboard.html;
  if (h == null || h.trim().isEmpty) return null;
  final md = htmlToMarkdown(stripCfHtmlHeader(h));
  return md.trim().isEmpty ? null : md;
}

/// Clipboard HTML converted to Markdown, but only when it holds a real DATA
/// `<table>` — the Excel/Sheets/web-table paste path. Excel ALSO puts a bitmap
/// of the copied cells on the clipboard, so the caller checks this BEFORE the
/// image flavor (image-first pasted spreadsheets as pictures).
Future<String?> readClipboardTableAsMarkdown() async {
  final h = await Pasteboard.html;
  if (h == null || h.trim().isEmpty) return null;
  final raw = stripCfHtmlHeader(h);
  if (!clipboardHtmlIsDataTable(raw)) return null;
  final md = htmlToMarkdown(raw);
  // Must actually convert to a pipe table (not just contain a tag) — otherwise
  // fall through to the image/HTML flavors.
  if (!md.contains('| --- ')) return null;
  return md.trim().isEmpty ? null : md;
}

/// Whether clipboard [html] carries a real DATA table worth converting:
/// a genuine `<table` tag (not `<tablet…>` etc.) and NO `<img>` inside — an
/// image wrapped in a layout table (Word/Outlook copy of a picture, email
/// signatures) must lose to the bitmap flavor or the pasted image is silently
/// replaced by a broken/empty one-cell table. Excel/Sheets tables carry no img.
bool clipboardHtmlIsDataTable(String html) {
  final lower = html.toLowerCase();
  if (!RegExp(r'<table[\s>]').hasMatch(lower)) return false;
  if (RegExp(r'<img[\s>/]').hasMatch(lower)) return false;
  return true;
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
