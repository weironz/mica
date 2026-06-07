// P2-M4 cloud yrs sync session facade. Desktop drives a Rust `yrs` replica via
// FFI (cloud_sync_io.dart); web drives a JS `yjs` replica (cloud_sync_web.dart) —
// the two are wire-compatible, so both speak the same WS sync protocol and
// converge on the same document.
export 'cloud_sync_io.dart' if (dart.library.html) 'cloud_sync_web.dart';
