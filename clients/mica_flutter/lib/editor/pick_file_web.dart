import 'dart:async';
import 'dart:html' as html;

/// Show the browser file picker (Markdown/text) and read the chosen file's text.
Future<({String name, String text})?> pickTextFile() async {
  final input = html.FileUploadInputElement()
    ..accept = '.md,.markdown,.txt,text/markdown,text/plain'
    ..multiple = false
    ..style.position = 'fixed'
    ..style.left = '-10000px'
    ..style.top = '0';
  html.document.body?.append(input);

  final done = Completer<({String name, String text})?>();
  void finish(({String name, String text})? value) {
    if (!done.isCompleted) done.complete(value);
  }

  input.onChange.listen((_) async {
    final files = input.files;
    if (files == null || files.isEmpty) {
      finish(null);
      return;
    }
    final file = files.first;
    final reader = html.FileReader()..readAsText(file);
    await reader.onLoadEnd.first;
    final result = reader.result;
    finish((name: file.name, text: result is String ? result : ''));
  });
  input.addEventListener('cancel', (_) => finish(null));

  input.click();
  final value = await done.future;
  input.remove();
  return value;
}
