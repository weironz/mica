import 'dart:convert';
import 'dart:ffi';
import 'dart:io' show Platform;

import 'package:ffi/ffi.dart';
import 'package:flutter/services.dart';

/// Copy [text] to the system clipboard via Flutter's framework platform
/// channel — in-house, no plugin. Returns false only if the channel throws.
Future<bool> copyTextToClipboard(String text) async {
  try {
    await Clipboard.setData(ClipboardData(text: text));
    return true;
  } catch (_) {
    return false;
  }
}

/// Copy the selection in TWO flavors: [plain] (Markdown-free text, what a plain
/// editor like Notepad reads) and [richHtml] (what Markdown editors like Typora
/// read and convert back to formatted content). On Windows both flavors are
/// written via the raw Win32 clipboard (CF_UNICODETEXT + CF_HTML); everywhere
/// else only [plain] is set (Flutter can't write multi-flavor) — a graceful
/// degrade, since Windows is the desktop target for this.
Future<bool> copyRichToClipboard({
  required String plain,
  required String richHtml,
}) async {
  if (Platform.isWindows) {
    if (_windowsWriteTextAndHtml(plain, richHtml)) return true;
  }
  return copyTextToClipboard(plain);
}

/// Write CF_UNICODETEXT + CF_HTML to the Windows clipboard via dart:ffi. Returns
/// true iff the plain text landed. Symmetric to the read side in
/// rich_paste_stub.dart (HANDLE/HGLOBAL as IntPtr = Dart int). Any failure
/// (clipboard busy, alloc fail) returns false → caller falls back to plain text.
bool _windowsWriteTextAndHtml(String plain, String html) {
  final user32 = DynamicLibrary.open('user32.dll');
  final kernel32 = DynamicLibrary.open('kernel32.dll');
  final openClip = user32
      .lookupFunction<Int32 Function(IntPtr), int Function(int)>('OpenClipboard');
  final emptyClip =
      user32.lookupFunction<Int32 Function(), int Function()>('EmptyClipboard');
  final setData = user32.lookupFunction<IntPtr Function(Uint32, IntPtr),
      int Function(int, int)>('SetClipboardData');
  final closeClip =
      user32.lookupFunction<Int32 Function(), int Function()>('CloseClipboard');
  final registerFmt = user32.lookupFunction<Uint32 Function(Pointer<Utf16>),
      int Function(Pointer<Utf16>)>('RegisterClipboardFormatW');
  final gAlloc = kernel32.lookupFunction<IntPtr Function(Uint32, IntPtr),
      int Function(int, int)>('GlobalAlloc');
  final gLock = kernel32.lookupFunction<Pointer<Uint8> Function(IntPtr),
      Pointer<Uint8> Function(int)>('GlobalLock');
  final gUnlock = kernel32
      .lookupFunction<Int32 Function(IntPtr), int Function(int)>('GlobalUnlock');

  const cfUnicodeText = 13;
  const gmemMoveable = 0x0002;

  // Allocate a moveable HGLOBAL, copy [bytes] in, return the handle (0 on fail).
  // On success the clipboard TAKES OWNERSHIP — the handle must not be freed.
  int alloc(List<int> bytes) {
    final h = gAlloc(gmemMoveable, bytes.length);
    if (h == 0) return 0;
    final p = gLock(h);
    if (p == nullptr) return 0;
    p.asTypedList(bytes.length).setAll(0, bytes);
    gUnlock(h);
    return h;
  }

  // CF_UNICODETEXT: UTF-16LE, NUL-terminated.
  final textBytes = <int>[];
  for (final u in plain.codeUnits) {
    textBytes.add(u & 0xFF);
    textBytes.add((u >> 8) & 0xFF);
  }
  textBytes.addAll(const [0, 0]);

  final htmlBytes = _cfHtml(html);
  final fmtName = 'HTML Format'.toNativeUtf16();
  try {
    if (openClip(0) == 0) return false;
    try {
      emptyClip();
      var ok = false;
      final hText = alloc(textBytes);
      if (hText != 0 && setData(cfUnicodeText, hText) != 0) ok = true;
      final cfHtmlId = registerFmt(fmtName);
      if (cfHtmlId != 0) {
        final hHtml = alloc(htmlBytes);
        if (hHtml != 0) setData(cfHtmlId, hHtml);
      }
      return ok;
    } finally {
      closeClip();
    }
  } catch (_) {
    return false;
  } finally {
    malloc.free(fmtName);
  }
}

/// Wrap an HTML [fragment] in the CF_HTML clipboard format: a plaintext
/// descriptor (`Version`, `StartHTML`/`EndHTML`, `StartFragment`/`EndFragment`
/// — all UTF-8 *byte* offsets) followed by the markup. The offsets are 10-digit
/// zero-padded so the header's own length is fixed and can be measured up front.
List<int> _cfHtml(String fragment) {
  const marker = '<!--StartFragment-->';
  const endMarker = '<!--EndFragment-->';
  final body = '<html><body>$marker$fragment$endMarker</body></html>';
  String header(int startHtml, int endHtml, int startFrag, int endFrag) =>
      'Version:0.9\r\n'
      'StartHTML:${_pad(startHtml)}\r\n'
      'EndHTML:${_pad(endHtml)}\r\n'
      'StartFragment:${_pad(startFrag)}\r\n'
      'EndFragment:${_pad(endFrag)}\r\n';
  // Fixed-width offsets ⇒ the header length is stable; measure it with zeros.
  final headerLen = utf8.encode(header(0, 0, 0, 0)).length;
  final bodyBytes = utf8.encode(body);
  final startHtml = headerLen;
  final endHtml = headerLen + bodyBytes.length;
  final startFragment = headerLen + utf8.encode('<html><body>$marker').length;
  final endFragment = startFragment + utf8.encode(fragment).length;
  return utf8
      .encode(header(startHtml, endHtml, startFragment, endFragment) + body);
}

String _pad(int n) => n.toString().padLeft(10, '0');
