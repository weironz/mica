/// Client-side preference persistence (appearance, page width, toggles).
/// Web: window.localStorage; other platforms: in-memory no-op stub.
library;

export 'prefs_stub.dart' if (dart.library.html) 'prefs_web.dart';
