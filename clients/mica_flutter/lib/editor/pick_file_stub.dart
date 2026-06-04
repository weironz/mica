import 'dart:typed_data';

Future<({String name, String text})?> pickTextFile() async => null;

Future<({String name, Uint8List bytes})?> pickImportFile({
  bool zipOnly = false,
}) async =>
    null;

Future<List<({String name, Uint8List bytes})>> pickImportFiles() async =>
    const [];

Future<List<({String path, Uint8List bytes})>> pickImportFolder() async =>
    const [];
