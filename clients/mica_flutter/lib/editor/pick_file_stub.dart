/// Desktop/mobile file pickers (non-web variant). Web uses pick_file_web.dart.
/// Backed by the file_picker plugin's native dialogs; bytes are read eagerly.
library;

import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:file_picker/file_picker.dart';

Future<Uint8List?> _bytesOf(PlatformFile f) async {
  if (f.bytes != null) return f.bytes;
  if (f.path != null) {
    try {
      return await File(f.path!).readAsBytes();
    } catch (_) {}
  }
  return null;
}

Future<({String name, String text})?> pickTextFile() async {
  final res = await FilePicker.platform.pickFiles(
    withData: true,
    type: FileType.custom,
    allowedExtensions: const ['md', 'markdown', 'txt'],
  );
  if (res == null || res.files.isEmpty) return null;
  final f = res.files.first;
  final bytes = await _bytesOf(f);
  if (bytes == null) return null;
  return (name: f.name, text: utf8.decode(bytes, allowMalformed: true));
}

Future<({String name, Uint8List bytes})?> pickImportFile({
  bool zipOnly = false,
}) async {
  final res = await FilePicker.platform.pickFiles(
    withData: true,
    type: zipOnly ? FileType.custom : FileType.any,
    allowedExtensions: zipOnly ? const ['zip'] : null,
  );
  if (res == null || res.files.isEmpty) return null;
  final f = res.files.first;
  final bytes = await _bytesOf(f);
  if (bytes == null) return null;
  return (name: f.name, bytes: bytes);
}

Future<List<({String name, Uint8List bytes})>> pickImportFiles() async {
  final res = await FilePicker.platform.pickFiles(
    withData: true,
    allowMultiple: true,
  );
  if (res == null) return const [];
  final out = <({String name, Uint8List bytes})>[];
  for (final f in res.files) {
    final bytes = await _bytesOf(f);
    if (bytes != null) out.add((name: f.name, bytes: bytes));
  }
  return out;
}

Future<List<({String path, Uint8List bytes})>> pickImportFolder() async {
  final dir = await FilePicker.platform.getDirectoryPath();
  if (dir == null) return const [];
  final root = Directory(dir);
  final out = <({String path, Uint8List bytes})>[];
  try {
    await for (final e in root.list(recursive: true, followLinks: false)) {
      if (e is! File) continue;
      try {
        final bytes = await e.readAsBytes();
        var rel = e.path.substring(root.path.length).replaceAll('\\', '/');
        if (rel.startsWith('/')) rel = rel.substring(1);
        out.add((path: rel, bytes: bytes));
      } catch (_) {}
    }
  } catch (_) {}
  return out;
}
