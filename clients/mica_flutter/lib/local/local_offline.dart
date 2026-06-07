// P2-M3: local-offline facade. Native impl on desktop/mobile; web gets a stub
// (no FFI in the web bundle). See `local_offline_io.dart`.
export 'local_offline_io.dart' if (dart.library.html) 'local_offline_web.dart';
