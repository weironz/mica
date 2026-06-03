// Open a native file picker for a text file (e.g. Markdown) and read it as a
// string. On web this uses an <input type=file>; off the web it returns null.
//
// `pickTextFile()` resolves to `({String name, String text})` for the chosen
// file, or null if cancelled.
export 'pick_file_stub.dart' if (dart.library.html) 'pick_file_web.dart';
