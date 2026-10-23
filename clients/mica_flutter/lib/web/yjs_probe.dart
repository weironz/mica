// P2-M4 (web→yjs): facade — real yjs probe on web, no-op stub elsewhere. Keeps
// `main.dart` free of `dart:js_interop` on desktop.
export 'yjs_probe_stub.dart' if (dart.library.js_interop) 'yjs_probe_web.dart';
