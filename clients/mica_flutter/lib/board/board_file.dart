import 'dart:typed_data';

import 'board_file_io.dart'
    if (dart.library.js_interop) 'board_file_web.dart'
    as impl;

Future<({String name, Uint8List bytes})?> pickBoardImportFile() =>
    impl.pickBoardImportFile();

Future<String?> saveBoardTextFile({
  required String suggestedName,
  required String extension,
  required String content,
}) => impl.saveBoardTextFile(
  suggestedName: suggestedName,
  extension: extension,
  content: content,
);
