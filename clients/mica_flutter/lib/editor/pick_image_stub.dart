/// Desktop/mobile image picker (non-web variant). Web uses pick_image_web.dart.
library;

import 'dart:io';
import 'dart:typed_data';

import 'package:file_picker/file_picker.dart';

Future<({String name, String mime, Uint8List bytes})?> pickImage() async {
  final res = await FilePicker.platform.pickFiles(
    withData: true,
    type: FileType.image,
  );
  if (res == null || res.files.isEmpty) return null;
  final f = res.files.first;
  Uint8List? bytes = f.bytes;
  if (bytes == null && f.path != null) {
    try {
      bytes = await File(f.path!).readAsBytes();
    } catch (_) {}
  }
  if (bytes == null) return null;
  return (name: f.name, mime: _mimeFor(f.extension ?? f.name), bytes: bytes);
}

String _mimeFor(String nameOrExt) {
  final e = nameOrExt.toLowerCase();
  if (e.endsWith('png')) return 'image/png';
  if (e.endsWith('jpg') || e.endsWith('jpeg')) return 'image/jpeg';
  if (e.endsWith('gif')) return 'image/gif';
  if (e.endsWith('webp')) return 'image/webp';
  if (e.endsWith('bmp')) return 'image/bmp';
  if (e.endsWith('svg')) return 'image/svg+xml';
  return 'application/octet-stream';
}
