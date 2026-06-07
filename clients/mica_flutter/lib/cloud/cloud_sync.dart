// P2-M4.5c: cloud yrs sync session facade. Native impl on desktop/mobile; web
// gets a no-op stub (no FFI in the web bundle, so web keeps the op-based cloud
// path). See `cloud_sync_io.dart`.
export 'cloud_sync_io.dart' if (dart.library.html) 'cloud_sync_stub.dart';
