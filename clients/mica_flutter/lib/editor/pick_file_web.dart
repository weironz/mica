import 'dart:async';
import 'dart:js_interop';
import 'dart:typed_data';

import 'package:web/web.dart' as web;

/// Show the browser file picker (Markdown/text) and read the chosen file's text.
Future<({String name, String text})?> pickTextFile() async {
  final file = await _pick('.md,.markdown,.txt,text/markdown,text/plain');
  if (file == null) return null;
  return (name: file.name, text: (await file.text().toDart).toDart);
}

/// Pick a workspace `.zip` (or also Markdown when [zipOnly] is false),
/// returning its raw bytes.
Future<({String name, Uint8List bytes})?> pickImportFile({
  bool zipOnly = false,
}) async {
  final files = await _pickFiles(zipOnly
      ? '.zip,application/zip'
      : '.md,.markdown,.txt,.zip,application/zip');
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
    // `webkitRelativePath` is "" for a plain (non-directory) pick, which is
    // what the old dart:html `relativePath ?? name` null check guarded.
    final path = f.webkitRelativePath.isNotEmpty
        ? f.webkitRelativePath
        : f.name;
    final parts = path.split('/');
    // Skip hidden files AND anything inside dot-folders (.obsidian, .git,
    // .trash…) — only the picked folder itself (first segment) may be hidden.
    if (parts.skip(1).any((s) => s.startsWith('.')) ||
        (parts.length == 1 && parts.first.startsWith('.'))) {
      continue;
    }
    final base = parts.last;
    final ext = base.contains('.') ? base.split('.').last.toLowerCase() : '';
    if (!keep.contains(ext)) continue;
    out.add((path: path, bytes: await _readBytes(f)));
  }
  return out;
}

Future<Uint8List> _readBytes(web.File file) async =>
    (await file.arrayBuffer().toDart).toDart.asUint8List();

Future<web.File?> _pick(String accept) async {
  final files = await _pickFiles(accept);
  return files.isEmpty ? null : files.first;
}

Future<List<web.File>> _pickFiles(
  String accept, {
  bool multiple = false,
  bool directory = false,
}) async {
  final input = web.document.createElement('input') as web.HTMLInputElement
    ..type = 'file'
    ..multiple = multiple
    ..style.position = 'fixed'
    ..style.left = '-10000px'
    ..style.top = '0';
  if (accept.isNotEmpty) input.accept = accept;
  if (directory) input.setAttribute('webkitdirectory', '');
  web.document.body?.append(input);
  final done = Completer<List<web.File>>();
  void finish(List<web.File> f) {
    if (!done.isCompleted) done.complete(f);
  }

  input.onChange.listen((_) => finish(_listed(input.files)));
  input.addEventListener('cancel', ((web.Event _) => finish(const [])).toJS);
  input.click();
  final files = await done.future;
  input.remove();
  return files;
}

/// FileList exposes `item(i)`/`length` only; project it into a Dart list.
List<web.File> _listed(web.FileList? list) => [
      if (list != null)
        for (var i = 0; i < list.length; i++) list.item(i)!,
    ];
