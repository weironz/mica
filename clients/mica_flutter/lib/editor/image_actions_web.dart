// dart:js_util resolves only for the web compile target (this file is reached
// solely via a dart.library.html conditional import); the VM-targeted analyzer
// can't see it, so silence that single false-positive here.
// ignore: uri_does_not_exist
import 'dart:js_util' as js_util;
import 'dart:html' as html;
import 'dart:typed_data';

/// Trigger a browser download of [bytes] as [filename].
void downloadImage(Uint8List bytes, String filename, String mime) {
  final blob = html.Blob(<dynamic>[bytes], mime);
  final url = html.Url.createObjectUrlFromBlob(blob);
  final anchor = html.AnchorElement(href: url)
    ..download = filename
    ..style.display = 'none';
  html.document.body?.append(anchor);
  anchor.click();
  anchor.remove();
  html.Url.revokeObjectUrl(url);
}

/// Print [htmlContent] (a complete, self-contained HTML document) via the
/// browser's print dialog — the web PDF-export path (the user picks "Save as
/// PDF"). Renders into a hidden, same-origin iframe so only the document prints,
/// not the app shell. Kept alive briefly while the dialog is open, then removed.
Future<void> printHtml(String htmlContent) async {
  final iframe = html.IFrameElement()
    ..style.position = 'fixed'
    ..style.right = '0'
    ..style.bottom = '0'
    ..style.width = '0'
    ..style.height = '0'
    ..style.border = '0'
    ..setAttribute('aria-hidden', 'true');
  html.document.body?.append(iframe);
  // `srcdoc` renders the self-contained doc in an isolated, same-origin frame.
  iframe.srcdoc = htmlContent;
  await iframe.onLoad.first;
  // `contentWindow` is a `WindowBase` (no typed `print`/`focus`); at runtime it
  // IS the JS window, so dispatch dynamically via js_util.
  final win = iframe.contentWindow;
  if (win != null) {
    js_util.callMethod<void>(win, 'focus', <dynamic>[]);
    js_util.callMethod<void>(win, 'print', <dynamic>[]);
  }
  Future<void>.delayed(const Duration(minutes: 1), iframe.remove);
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
    final clipboard = js_util.getProperty(html.window.navigator, 'clipboard');
    if (clipboard == null) return false;
    final ctor = js_util.getProperty(html.window, 'ClipboardItem');
    if (ctor == null) return false;
    final blob = html.Blob(<dynamic>[bytes], mime);
    final data = js_util.newObject();
    js_util.setProperty(data, mime, blob);
    final item = js_util.callConstructor(ctor, <dynamic>[data]);
    await js_util.promiseToFuture<void>(
      js_util.callMethod(clipboard, 'write', <dynamic>[
        js_util.jsify(<dynamic>[item]),
      ]),
    );
    return true;
  } catch (_) {
    return false;
  }
}

/// Legacy path — select an off-screen <img> and execCommand('copy'). Works in
/// non-secure (http) contexts where the async Clipboard API is unavailable.
Future<bool> _copyViaExecCommand(Uint8List bytes, String mime) async {
  final blob = html.Blob(<dynamic>[bytes], mime);
  final url = html.Url.createObjectUrlFromBlob(blob);
  final holder = html.DivElement()
    ..contentEditable = 'true'
    ..style.position = 'fixed'
    ..style.left = '-10000px'
    ..style.top = '0';
  final img = html.ImageElement(src: url);
  holder.append(img);
  html.document.body?.append(holder);
  try {
    await img.onLoad.first;
    final range = html.Range()..selectNode(img);
    final selection = html.window.getSelection();
    selection
      ?..removeAllRanges()
      ..addRange(range);
    final ok = html.document.execCommand('copy');
    selection?.removeAllRanges();
    return ok;
  } catch (_) {
    return false;
  } finally {
    holder.remove();
    html.Url.revokeObjectUrl(url);
  }
}
