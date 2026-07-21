// 丁-1 (2026-07-21): the engine seam of the cloud sync session.
//
// The WS sync-protocol state machine (cloud_sync_session.dart) is pure Dart and
// engine-agnostic; it touches the CRDT replica only through this contract. The
// desktop adapter (sync_replica_io.dart) wraps the Rust yrs FFI [MicaDocument];
// the web adapter (sync_replica_web.dart) wraps the JS yjs [MicaYDoc] — the two
// engines are wire-compatible, so both speak the same update/state-vector
// bytes. Which adapter backs a session is decided by conditional import;
// changing an engine on a platform means swapping its adapter, and nothing in
// the session or the store moves.
//
// Before this seam existed the whole session lived twice (cloud_sync_io.dart /
// cloud_sync_web.dart, 407 non-comment lines byte-identical) and red line #1's
// semantics were kept in sync by hand. Don't add engine types back into the
// session: every engine touch goes through here.
import 'dart:typed_data';

/// A block op as the editor emits it (see `editor/controller.dart`).
typedef DocOp = Map<String, dynamic>;

/// The CRDT replica surface the sync session needs — nothing more.
abstract interface class SyncDocReplica {
  /// The document's root block id (stable across the doc's life).
  String rootBlockId();

  /// The replica's current state vector (lib0 v1 encoding).
  Uint8List stateVector();

  /// The full doc state as one update (lib0 v1) — the base-snapshot format.
  Uint8List encodeState();

  /// The delta from [sv] to the current state (what a push carries).
  Uint8List encodeDiffSince(Uint8List sv);

  /// Merge a remote update. Returns false when the bytes don't apply — the
  /// session treats that as an integrity fault (red line #1), never a skip.
  bool applyUpdate(Uint8List update);

  /// The full document as a flat block list (tree order).
  List<Map<String, dynamic>> toBlocks();

  /// Replay one coarse editor op onto the replica.
  void applyEditorOp(DocOp op);
}
