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

/// Write literal text/plain and semantic text/html together. Rich destinations
/// reconstruct formatting from HTML; plain destinations receive only content.
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
    // Fall through to the synchronous copy event (also works on HTTP).
  }
  // execCommand on a textarea alone drops HTML. Supply both flavors through
  // the copy event so HTTP LAN clients and browsers without ClipboardItem keep
  // code semantics too. Restore focus/selection after the temporary input.
  final active = html.document.activeElement;
  final input = active is html.TextAreaElement ? active : null;
  final start = input?.selectionStart;
  final end = input?.selectionEnd;
  final area = html.TextAreaElement()
    ..value = plain
    ..setAttribute('readonly', '')
    ..style.position = 'fixed'
    ..style.left = '-10000px';
  var wrote = false;
  void onCopy(html.Event event) {
    final data = (event as html.ClipboardEvent).clipboardData;
    if (data == null) return;
    data.setData('text/plain', plain);
    data.setData('text/html', richHtml);
    event.preventDefault();
    wrote = true;
  }

  try {
    html.document.addEventListener('copy', onCopy, true);
    html.document.body?.append(area);
    area.focus();
    area.select();
    if (html.document.execCommand('copy') && wrote) return true;
  } catch (_) {
    // Last resort: land the literal text.
  } finally {
    html.document.removeEventListener('copy', onCopy, true);
    area.remove();
    active?.focus();
    if (input != null && start != null && end != null) {
      input.setSelectionRange(start, end);
    }
  }
  return copyTextToClipboard(plain);
}
