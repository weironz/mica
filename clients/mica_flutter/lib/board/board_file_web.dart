import 'dart:async';
import 'dart:convert';
import 'dart:js_interop';
import 'dart:typed_data';

import 'package:web/web.dart' as web;

Future<({String name, Uint8List bytes})?> pickBoardImportFile() {
  final completer = Completer<({String name, Uint8List bytes})?>();
  final input = web.document.createElement('input') as web.HTMLInputElement
    ..type = 'file'
    ..accept = '.drawio,.xml,.svg,.png,.json';
  input.onChange.first.then((_) async {
    final files = input.files;
    if (files == null || files.length == 0) {
      if (!completer.isCompleted) completer.complete(null);
      return;
    }
    final file = files.item(0);
    if (file == null) {
      if (!completer.isCompleted) completer.complete(null);
      return;
    }
    final buf = await file.arrayBuffer().toDart;
    final bytes = buf.toDart.asUint8List();
    if (!completer.isCompleted) {
      completer.complete((name: file.name, bytes: bytes));
    }
  });
  input.click();
  return completer.future;
}

Future<String?> saveBoardTextFile({
  required String suggestedName,
  required String extension,
  required String content,
}) async {
  final bytes = Uint8List.fromList(utf8.encode(content));
  final blob = web.Blob([bytes.toJS].toJS);
  final url = web.URL.createObjectURL(blob);
  final a = web.document.createElement('a') as web.HTMLAnchorElement
    ..href = url
    ..download = suggestedName;
  a.click();
  web.URL.revokeObjectURL(url);
  return suggestedName;
}
