// ignore: uri_does_not_exist
import 'dart:js_util' as js_util;
import 'dart:html' as html;

/// Copy [text] to the clipboard. Tries the async Clipboard API (secure
/// contexts) then falls back to execCommand('copy'), which works on plain http.
Future<bool> copyTextToClipboard(String text) async {
  try {
    final clipboard = js_util.getProperty(html.window.navigator, 'clipboard');
    if (clipboard != null) {
      await js_util.promiseToFuture<void>(
        js_util.callMethod(clipboard, 'writeText', <dynamic>[text]),
      );
      return true;
    }
  } catch (_) {
    // Fall through to the legacy path below.
  }
  try {
    final area = html.TextAreaElement()
      ..value = text
      ..setAttribute('readonly', '')
      ..style.position = 'fixed'
      ..style.left = '-10000px'
      ..style.top = '0';
    html.document.body?.append(area);
    area.focus();
    area.select();
    final ok = html.document.execCommand('copy');
    area.remove();
    return ok;
  } catch (_) {
    return false;
  }
}

/// Copy the selection in TWO flavors via a multi-format `ClipboardItem`: [plain]
/// (Markdown-free `text/plain`, what Notepad reads) and [richHtml] (`text/html`,
/// what Typora/Obsidian read and convert back to formatted content). Needs a
/// secure context + ClipboardItem; falls back to writing just [plain] otherwise.
Future<bool> copyRichToClipboard({
  required String plain,
  required String richHtml,
}) async {
  try {
    final clipboard = js_util.getProperty(html.window.navigator, 'clipboard');
    final ctor = js_util.getProperty(html.window, 'ClipboardItem');
    if (clipboard != null &&
        ctor != null &&
        js_util.hasProperty(clipboard, 'write')) {
      final flavors = js_util.newObject<Object>();
      js_util.setProperty(
        flavors,
        'text/plain',
        html.Blob(<dynamic>[plain], 'text/plain'),
      );
      js_util.setProperty(
        flavors,
        'text/html',
        html.Blob(<dynamic>[richHtml], 'text/html'),
      );
      final item = js_util.callConstructor(ctor, <dynamic>[flavors]);
      await js_util.promiseToFuture<void>(
        js_util.callMethod(clipboard, 'write', <dynamic>[
          js_util.jsify(<dynamic>[item]),
        ]),
      );
      return true;
    }
  } catch (_) {
    // Fall through: at least land the plain flavor.
  }
  return copyTextToClipboard(plain);
}
