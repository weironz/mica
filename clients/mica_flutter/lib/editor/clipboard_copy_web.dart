import 'dart:js_interop';

import 'package:web/web.dart' as web;

/// Copy [text] to the clipboard. Tries the async Clipboard API (secure
/// contexts) then falls back to execCommand('copy'), which works on plain http.
Future<bool> copyTextToClipboard(String text) async {
  try {
    // In a non-secure context `navigator.clipboard` is undefined; the typed
    // getter hands that back as-is and the call below throws, landing in the
    // legacy path — the same branch the old js_util null check selected.
    await web.window.navigator.clipboard.writeText(text).toDart;
    return true;
  } catch (_) {
    // Fall through to the legacy path below.
  }
  try {
    final area = web.document.createElement('textarea') as web.HTMLTextAreaElement
      ..value = text
      ..setAttribute('readonly', '')
      ..style.position = 'fixed'
      ..style.left = '-10000px'
      ..style.top = '0';
    web.document.body?.append(area);
    area.focus();
    area.select();
    final ok = web.document.execCommand('copy');
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
    final clipboard = web.window.navigator.clipboard;
    // A ClipboardItem is built from a plain {mime: Blob} object; `jsify` of a
    // Dart map is exactly that. Browsers without ClipboardItem (or non-secure
    // contexts) throw here, selecting the copy-event path below.
    final flavors = <String, web.Blob>{
      'text/plain': web.Blob(
        <JSAny>[plain.toJS].toJS,
        web.BlobPropertyBag(type: 'text/plain'),
      ),
      'text/html': web.Blob(
        <JSAny>[richHtml.toJS].toJS,
        web.BlobPropertyBag(type: 'text/html'),
      ),
    };
    final item = web.ClipboardItem(flavors.jsify() as JSObject);
    await clipboard.write(<web.ClipboardItem>[item].toJS).toDart;
    return true;
  } catch (_) {
    // Fall through to the synchronous copy event (also works on HTTP).
  }
  // execCommand on a textarea alone drops HTML. Supply both flavors through
  // the copy event so HTTP LAN clients and browsers without ClipboardItem keep
  // code semantics too. Restore focus/selection after the temporary input.
  final active = web.document.activeElement;
  // package:web keeps focus() on HTMLElement, not Element, so remember the
  // focusable form separately from the selection state we may restore.
  final activeHtml = active is web.HTMLElement ? active : null;
  final input = active is web.HTMLTextAreaElement ? active : null;
  final start = input?.selectionStart;
  final end = input?.selectionEnd;
  final area = web.document.createElement('textarea') as web.HTMLTextAreaElement
    ..value = plain
    ..setAttribute('readonly', '')
    ..style.position = 'fixed'
    ..style.left = '-10000px';
  var wrote = false;
  void onCopy(web.Event event) {
    final data = (event as web.ClipboardEvent).clipboardData;
    if (data == null) return;
    data.setData('text/plain', plain);
    data.setData('text/html', richHtml);
    event.preventDefault();
    wrote = true;
  }

  try {
    web.document.addEventListener('copy', onCopy.toJS, true.toJS);
    web.document.body?.append(area);
    area.focus();
    area.select();
    if (web.document.execCommand('copy') && wrote) return true;
  } catch (_) {
    // Last resort: land the literal text.
  } finally {
    web.document.removeEventListener('copy', onCopy.toJS, true.toJS);
    area.remove();
    activeHtml?.focus();
    if (input != null && start != null && end != null) {
      input.setSelectionRange(start, end);
    }
  }
  return copyTextToClipboard(plain);
}
