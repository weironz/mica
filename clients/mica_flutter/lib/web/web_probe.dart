// Web-only startup hooks: the document engine and the browser harness probe.
// Facade so `main.dart` stays free of `dart:js_interop` on desktop.
export 'web_probe_stub.dart' if (dart.library.js_interop) 'web_probe_web.dart';
