import 'dart:async';
import 'dart:html' as html;
import 'dart:typed_data';

/// Show the browser file picker (images only) and read the chosen file's bytes.
Future<({String name, String mime, Uint8List bytes})?> pickImage() async {
  final input = html.FileUploadInputElement()
    ..accept = 'image/*'
    ..multiple = false;
  input.click();

  await input.onChange.first;
  final files = input.files;
  if (files == null || files.isEmpty) return null;
  final file = files.first;

  final reader = html.FileReader()..readAsArrayBuffer(file);
  await reader.onLoadEnd.first;
  final result = reader.result;
  if (result is! ByteBuffer) return null;

  return (
    name: file.name,
    mime: file.type.isEmpty ? 'application/octet-stream' : file.type,
    bytes: result.asUint8List(),
  );
}
