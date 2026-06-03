// Open an external URL (Cmd/Ctrl+click on a link). On web this opens a new
// browser tab; off the web it is a no-op.
export 'open_url_stub.dart' if (dart.library.html) 'open_url_web.dart';
