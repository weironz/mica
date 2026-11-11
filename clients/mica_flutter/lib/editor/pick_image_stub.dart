/// Desktop/mobile image picker (non-web variant). Web uses pick_image_web.dart.
library;

import 'dart:typed_data';

import 'package:file_picker/file_picker.dart';

Future<({String name, String mime, Uint8List bytes})?> pickImage() async {
  final file = await FilePicker.pickFile(type: FileType.image);
  if (file == null) return null;
  Uint8List bytes;
  try {
    bytes = await file.readAsBytes();
  } catch (_) {
    return null;
  }
  return (
    name: file.name,
    mime: _mimeFor(file.extension ?? file.name),
    bytes: bytes,
  );
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
