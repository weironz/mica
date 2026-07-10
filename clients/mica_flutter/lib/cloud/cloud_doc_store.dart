import 'dart:typed_data';

/// Local-first persistence for one cloud document (P2, option C — Phase 1).
///
/// A [CloudSyncSession] mirrors its yrs replica through this so the doc opens
/// from the on-device store — read offline across a restart, and resume the sync
/// stream from the saved cursor instead of always cold-bootstrapping. Deals in
/// raw yrs *state bytes* (not the FFI `MicaDocument`) so it is platform-agnostic;
/// desktop backs it with the SQLite `MicaStore`, web passes null (CanvasKit has
/// no on-device store yet — that stays online, gated by `kIsWeb`).
abstract class CloudDocStore {
  /// The persisted replica bytes + how far it had synced (the highest stream
  /// `rid` applied), or null if this doc has never been mirrored locally.
  ({Uint8List state, int cursor})? load();

  /// Persist the current replica bytes + synced cursor. The session debounces
  /// calls, so this can write straight through.
  void save(Uint8List state, int cursor);
}
