/// Desktop/mobile image actions (non-web variant). Web uses image_actions_web.dart.
library;

import 'dart:io';
import 'dart:typed_data';

import 'package:file_picker/file_picker.dart';
import 'package:pasteboard/pasteboard.dart';

/// Save [bytes] to disk via a native "save as" dialog. Fire-and-forget to keep
/// the void contract; the save runs async.
void downloadImage(Uint8List bytes, String filename, String mime) {
  _save(bytes, filename);
}

Future<void> _save(Uint8List bytes, String filename) async {
  try {
    final path = await FilePicker.platform.saveFile(
      dialogTitle: 'Save image',
      fileName: filename.isEmpty ? 'image' : filename,
      bytes: bytes,
    );
    // Some file_picker versions return the chosen path without writing on
    // desktop; write it ourselves so the file always lands.
    if (path != null) {
      await File(path).writeAsBytes(bytes, flush: true);
    }
  } catch (_) {}
}

/// Web-only (the browser print dialog is the web PDF path). Desktop/mobile
/// export PDF through the native WebView2 FFI instead, so this is never called
/// here — a no-op keeps the cross-platform surface identical.
Future<void> printHtml(String html) async {}

/// Copy a raster image to the system clipboard via pasteboard.
Future<bool> copyImageToClipboard(Uint8List bytes, String mime) async {
  try {
    await Pasteboard.writeImage(bytes);
    return true;
  } catch (_) {
    return false;
  }
}
