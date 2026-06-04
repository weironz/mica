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
  final files = await _pickFiles('.md,.markdown,.txt,.zip,application/zip');
  if (files.isEmpty) return null;
  final file = files.first;
  return (name: file.name, bytes: await _readBytes(file));
}

/// Pick several import files at once (.md / .zip plus images they reference).
Future<List<({String name, Uint8List bytes})>> pickImportFiles() async {
  final files = await _pickFiles(
    '.md,.markdown,.txt,.zip,application/zip,image/*',
    multiple: true,
  );
  return [
    for (final f in files) (name: f.name, bytes: await _readBytes(f)),
  ];
}

/// Pick a whole folder (recursive). Paths are relative and include the
/// selected folder itself as the first segment. Only markdown and image
/// files are read — anything else in the tree is skipped.
Future<List<({String path, Uint8List bytes})>> pickImportFolder() async {
  final files = await _pickFiles('', directory: true);
  const keep = {
    'md', 'markdown', 'json', //
    'png', 'jpg', 'jpeg', 'gif', 'webp', 'svg', 'bmp',
  };
  final out = <({String path, Uint8List bytes})>[];
  for (final f in files) {
    final path = f.relativePath ?? f.name;
    final base = path.split('/').last;
    if (base.startsWith('.')) continue; // .DS_Store and friends
    final ext = base.contains('.') ? base.split('.').last.toLowerCase() : '';
    if (!keep.contains(ext)) continue;
    out.add((path: path, bytes: await _readBytes(f)));
  }
  return out;
}

Future<Uint8List> _readBytes(html.File file) async {
  final reader = html.FileReader()..readAsArrayBuffer(file);
  await reader.onLoadEnd.first;
  final result = reader.result;
  return result is ByteBuffer
      ? result.asUint8List()
      : (result is Uint8List ? result : Uint8List(0));
}

Future<html.File?> _pick(String accept) async {
  final files = await _pickFiles(accept);
  return files.isEmpty ? null : files.first;
}

Future<List<html.File>> _pickFiles(
  String accept, {
  bool multiple = false,
  bool directory = false,
}) async {
  final input = html.FileUploadInputElement()
    ..multiple = multiple
    ..style.position = 'fixed'
    ..style.left = '-10000px'
    ..style.top = '0';
  if (accept.isNotEmpty) input.accept = accept;
  if (directory) input.setAttribute('webkitdirectory', '');
  html.document.body?.append(input);
  final done = Completer<List<html.File>>();
  void finish(List<html.File> f) {
    if (!done.isCompleted) done.complete(f);
  }

  input.onChange.listen((_) => finish(input.files ?? const []));
  input.addEventListener('cancel', (_) => finish(const []));
  input.click();
  final files = await done.future;
  input.remove();
  return files;
}
