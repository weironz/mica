import 'dart:async';
import 'dart:html' as html;
import 'dart:typed_data';

/// Show the browser file picker (Markdown/text) and read the chosen file's text.
Future<({String name, String text})?> pickTextFile() async {
  final file = await _pick('.md,.markdown,.txt,text/markdown,text/plain');
  if (file == null) return null;
  final reader = html.FileReader()..readAsText(file);
  await reader.onLoadEnd.first;
  final result = reader.result;
  return (name: file.name, text: result is String ? result : '');
}

/// Pick a Markdown file or a workspace `.zip`, returning its raw bytes.
Future<({String name, Uint8List bytes})?> pickImportFile() async {
  final file = await _pick('.md,.markdown,.txt,.zip,application/zip');
  if (file == null) return null;
  final reader = html.FileReader()..readAsArrayBuffer(file);
  await reader.onLoadEnd.first;
  final result = reader.result;
  final bytes = result is ByteBuffer
      ? result.asUint8List()
      : (result is Uint8List ? result : Uint8List(0));
  return (name: file.name, bytes: bytes);
}

Future<html.File?> _pick(String accept) async {
  final input = html.FileUploadInputElement()
    ..accept = accept
    ..multiple = false
    ..style.position = 'fixed'
    ..style.left = '-10000px'
    ..style.top = '0';
  html.document.body?.append(input);
  final done = Completer<html.File?>();
  void finish(html.File? f) {
    if (!done.isCompleted) done.complete(f);
  }

  input.onChange.listen((_) {
    final files = input.files;
    finish(files != null && files.isNotEmpty ? files.first : null);
  });
  input.addEventListener('cancel', (_) => finish(null));
  input.click();
  final file = await done.future;
  input.remove();
  return file;
}
