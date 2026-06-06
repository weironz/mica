/// Desktop window setup: minimum size + remember last size/position.
/// Web has no OS window to manage, so it resolves to a no-op variant — this
/// also keeps the desktop-only `window_manager` package out of the web bundle.
library;

export 'window_setup_desktop.dart' if (dart.library.html) 'window_setup_web.dart';
