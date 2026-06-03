import 'dart:async';
import 'dart:html' as html;
import 'dart:typed_data';

/// Show the browser file picker (images only) and read the chosen file's bytes.
Future<({String name, String mime, Uint8List bytes})?> pickImage() async {
  final input = html.FileUploadInputElement()
    ..accept = 'image/*'
    ..multiple = false
    // Keep it in the DOM (some browsers require it) but off-screen rather than
    // display:none, which can stop Chrome associating the chosen file.
    ..style.position = 'fixed'
    ..style.left = '-10000px'
    ..style.top = '0';
  html.document.body?.append(input);

  final done = Completer<({String name, String mime, Uint8List bytes})?>();

  void finish(({String name, String mime, Uint8List bytes})? value) {
    if (!done.isCompleted) done.complete(value);
  }

  input.onChange.listen((_) async {
    final files = input.files;
    if (files == null || files.isEmpty) {
      finish(null);
      return;
    }
    final file = files.first;
    final reader = html.FileReader()..readAsArrayBuffer(file);
    await reader.onLoadEnd.first;
    // dart2js types readAsArrayBuffer's result inconsistently (often not a
    // plain ByteBuffer), so accept the common byte representations.
    final result = reader.result;
    Uint8List? bytes;
    if (result is ByteBuffer) {
      bytes = result.asUint8List();
    } else if (result is Uint8List) {
      bytes = result;
    } else if (result is List<int>) {
      bytes = Uint8List.fromList(result);
    }
    if (bytes == null) {
      finish(null);
      return;
    }
    finish((
      name: file.name,
      mime: file.type.isEmpty ? 'application/octet-stream' : file.type,
      bytes: bytes,
    ));
  });
  // Newer Chrome fires a 'cancel' event when the dialog is dismissed.
  input.addEventListener('cancel', (_) => finish(null));

  input.click();

  final value = await done.future;
  input.remove();
  return value;
}
