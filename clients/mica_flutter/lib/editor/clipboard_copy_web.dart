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
