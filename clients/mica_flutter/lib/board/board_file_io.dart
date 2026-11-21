import 'dart:convert';
import 'dart:typed_data';

import 'package:file_picker/file_picker.dart';

Future<({String name, Uint8List bytes})?> pickBoardImportFile() async {
  final f = await FilePicker.pickFile(
    dialogTitle: '导入 draw.io 或 Mica 画板文件',
    type: FileType.custom,
    allowedExtensions: const ['drawio', 'xml', 'svg', 'png', 'json'],
  );
  if (f == null) return null;
  try {
    final bytes = await f.readAsBytes();
    return (name: f.name, bytes: bytes);
  } catch (_) {
    return null;
  }
}

Future<String?> saveBoardTextFile({
  required String suggestedName,
  required String extension,
  required String content,
}) async {
  final uri = await FilePicker.saveFile(
    dialogTitle: '保存文件',
    fileName: suggestedName,
    bytes: Uint8List.fromList(utf8.encode(content)),
  );
  if (uri == null) return null;
  return uri.toFilePath();
}
