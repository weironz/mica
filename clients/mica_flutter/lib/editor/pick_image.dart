// Open a native file picker for images. On web this uses an <input type=file>;
// off the web it returns null (no desktop/mobile picker wired yet).
//
// `pickImage()` resolves to a record `({String name, String mime, Uint8List
// bytes})` for the chosen file, or null if the user cancels.
export 'pick_image_stub.dart' if (dart.library.html) 'pick_image_web.dart';
