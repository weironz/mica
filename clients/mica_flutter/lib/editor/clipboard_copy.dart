// Copy text to the system clipboard, working on plain http too. Flutter's
// Clipboard.setData uses navigator.clipboard, which needs a secure context
// (https/localhost) and silently fails over an http LAN address — so we add an
// execCommand('copy') fallback. No-op stub off the web.
export 'clipboard_copy_stub.dart' if (dart.library.html) 'clipboard_copy_web.dart';
