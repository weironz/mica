import 'dart:async';
import 'dart:js_interop';
import 'dart:typed_data';

import 'package:web/web.dart' as web;

/// Trigger a browser download of [bytes] as [filename].
void downloadImage(Uint8List bytes, String filename, String mime) {
  final blob = web.Blob(
    <JSAny>[bytes.toJS].toJS,
    web.BlobPropertyBag(type: mime),
  );
  final url = web.URL.createObjectURL(blob);
  final anchor = web.document.createElement('a') as web.HTMLAnchorElement
    ..href = url
    ..download = filename
    ..style.display = 'none';
  web.document.body?.append(anchor);
  anchor.click();
  anchor.remove();
  web.URL.revokeObjectURL(url);
}

/// Print [htmlContent] (a complete, self-contained HTML document) via the
/// browser's print dialog — the web PDF-export path (the user picks "Save as
/// PDF"). Renders into a hidden, same-origin iframe so only the document prints,
/// not the app shell. Kept alive briefly while the dialog is open, then removed.
Future<void> printHtml(String htmlContent) async {
  final iframe = web.document.createElement('iframe') as web.HTMLIFrameElement
    ..style.position = 'fixed'
    ..style.right = '0'
    ..style.bottom = '0'
    ..style.width = '0'
    ..style.height = '0'
    ..style.border = '0'
    ..setAttribute('aria-hidden', 'true');
  web.document.body?.append(iframe);
  // `srcdoc` renders the self-contained doc in an isolated, same-origin frame.
  iframe.srcdoc = htmlContent.toJS;
  await iframe.onLoad.first;
  // package:web dispatches straight through to the underlying JS window — the
  // old dart:html wrapper around `contentWindow` is what used to fail with
  // `NoSuchMethodError: method not found: 'focus'`.
  final win = iframe.contentWindow;
  win?.focus();
  win?.print();
  Future<void>.delayed(const Duration(minutes: 1), () => iframe.remove());
}

/// Copy [bytes] to the system clipboard as an image. Tries the modern async
/// Clipboard API (secure contexts), then falls back to the legacy
/// execCommand('copy') on a selected <img>, which works on plain http too.
Future<bool> copyImageToClipboard(Uint8List bytes, String mime) async {
  if (await _copyViaClipboardApi(bytes, mime)) return true;
  return _copyViaExecCommand(bytes, mime);
}

/// Modern path — requires a secure context (https/localhost).
Future<bool> _copyViaClipboardApi(Uint8List bytes, String mime) async {
  try {
    final clipboard = web.window.navigator.clipboard;
    final blob = web.Blob(
      <JSAny>[bytes.toJS].toJS,
      web.BlobPropertyBag(type: mime),
    );
    final data = <String, web.Blob>{mime: blob}.jsify() as JSObject;
    final item = web.ClipboardItem(data);
    await clipboard.write(<web.ClipboardItem>[item].toJS).toDart;
    return true;
  } catch (_) {
    return false;
  }
}

/// Legacy path — select an off-screen <img> and execCommand('copy'). Works in
/// non-secure (http) contexts where the async Clipboard API is unavailable.
Future<bool> _copyViaExecCommand(Uint8List bytes, String mime) async {
  final blob = web.Blob(
    <JSAny>[bytes.toJS].toJS,
    web.BlobPropertyBag(type: mime),
  );
  final url = web.URL.createObjectURL(blob);
  final holder = web.document.createElement('div') as web.HTMLDivElement
    ..contentEditable = 'true'
    ..style.position = 'fixed'
    ..style.left = '-10000px'
    ..style.top = '0';
  final img = web.document.createElement('img') as web.HTMLImageElement;
  final loaded = Completer<bool>();
  void finishLoading(bool success) {
    if (!loaded.isCompleted) loaded.complete(success);
  }

  final loadSub = img.onLoad.listen((_) => finishLoading(true));
  final errorSub = img.onError.listen((_) => finishLoading(false));
  holder.append(img);
  web.document.body?.append(holder);
  try {
    img.src = url;
    if (!await loaded.future.timeout(
      const Duration(seconds: 10),
      onTimeout: () => false,
    )) {
      return false;
    }
    final range = web.document.createRange()..selectNode(img);
    final selection = web.window.getSelection();
    selection
      ?..removeAllRanges()
      ..addRange(range);
    final ok = web.document.execCommand('copy');
    selection?.removeAllRanges();
    return ok;
  } catch (_) {
    return false;
  } finally {
    await loadSub.cancel();
    await errorSub.cancel();
    holder.remove();
    web.URL.revokeObjectURL(url);
  }
}
