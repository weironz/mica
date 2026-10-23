import 'dart:async';
import 'dart:typed_data';

import 'package:web/web.dart' as web;

/// Show the browser file picker (images only) and read the chosen file's bytes.
Future<({String name, String mime, Uint8List bytes})?> pickImage() async {
  final input = web.document.createElement('input') as web.HTMLInputElement
    ..accept = 'image/*'
    ..multiple = false
    // Keep it in the DOM (some browsers require it) but off-screen rather than
    // display:none, which can stop Chrome associating the chosen file.
    ..style.position = 'fixed'
    ..style.left = '-10000px'
    ..style.top = '0';
  web.document.body?.append(input);

  final done = Completer<({String name, String mime, Uint8List bytes})?>();

  void finish(({String name, String mime, Uint8List bytes})? value) {
    if (!done.isCompleted) done.complete(value);
  }

  input.onChange.listen((_) async {
    final list = input.files;
    final file = list == null || list.length == 0 ? null : list.item(0);
    if (file == null) {
      finish(null);
      return;
    }
    try {
      // File extends Blob, so the promise-based reads replace the old
      // FileReader + onLoadEnd dance (and give errors as rejections).
      final buffer = await file.arrayBuffer().toDart;
      finish((
        name: file.name,
        mime: file.type.isEmpty ? 'application/octet-stream' : file.type,
        bytes: buffer.toDart.asUint8List(),
      ));
    } catch (_) {
      finish(null);
    }
  });
  // Newer Chrome fires a 'cancel' event when the dialog is dismissed.
  input.addEventListener('cancel', ((web.Event _) => finish(null)).toJS);

  input.click();

  final value = await done.future;
  input.remove();
  return value;
}
