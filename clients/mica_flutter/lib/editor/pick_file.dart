// Open a native file picker. On web this uses an <input type=file>; off the web
// it returns null.
//
// `pickTextFile()`   → `({String name, String text})`  (Markdown/text)
// `pickImportFile()` → `({String name, Uint8List bytes})` (Markdown or .zip)
export 'pick_file_stub.dart' if (dart.library.html) 'pick_file_web.dart';
