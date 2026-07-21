/// Opt-in bug-reproduction capture. Desktop/mobile write files under
/// `{config}/debug`; web has no filesystem and gets no-ops (see
/// `diagnostics_web.dart`). Off unless the user turns it on in Settings.
library;

export 'diagnostics_stub.dart' if (dart.library.html) 'diagnostics_web.dart';
