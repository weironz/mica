// 丁-1 (2026-07-21): the engine seam of the cloud sync session.
//
// The WS sync-protocol state machine (cloud_sync_session.dart) is pure Dart and
// engine-agnostic; it touches the CRDT replica only through this contract. The
// desktop adapter (sync_replica_io.dart) wraps the Rust yrs FFI [MicaDocument];
// the web adapter (sync_replica_web.dart) wraps the same Rust core as wasm
// ([MicaDocWasm]) — one engine, so both speak the same update/state-vector
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

  /// Blocks whose CONTENT changed since the last call (any source: remote,
  /// local, undo), or null = re-read everything with [toBlocks] (the tree's
  /// shape changed, too many changed, or nothing has been read yet). Decided
  /// in Rust (mica-core `take_changed_block_ids`), so a reader never re-derives
  /// tree order. The session keeps its block list current with this instead
  /// of a full read per remote update.
  List<String>? takeChangedBlockIds();

  /// The named blocks; ids no longer in the document are left out.
  List<Map<String, dynamic>> blocksById(List<String> ids);

  /// Replay one coarse editor op onto the replica.
  void applyEditorOp(DocOp op);

  void beginHistoryBatch();
  void endHistoryBatch();
  void resetHistory();
  bool undo();
  bool redo();
  bool canUndo();
  bool canRedo();
  (int, int) historyDepths();

  /// Combine [updates] into ONE update with the same effect as applying them in
  /// order. Used on reconnect: a long offline stretch queues hundreds of tiny
  /// diffs, and pushing them one by one costs one server round trip EACH — the
  /// server decodes and re-encodes the whole document per push. Measured against
  /// a live database: 200 queued updates take 2702 ms one-by-one versus 19 ms
  /// merged, and the merged bytes are ~73% smaller because the intermediate
  /// states of a repeatedly-edited block fold away.
  ///
  /// Both adapters implement this by applying the updates to a scratch replica
  /// that starts EMPTY and encoding its state. Empty is load-bearing: a scratch
  /// seeded from anything that already carries content (a root block, a `meta`
  /// entry) would fold that content into the merged update as a concurrent
  /// write, and the receiving side would apply it against its own — a way to
  /// corrupt `meta.root` that no test would obviously catch.
  Uint8List mergeUpdates(List<Uint8List> updates);

  /// Release the engine side now; the replica must not be used afterwards.
  /// The session calls it from `dispose`. On the web this is what lets the
  /// next document reuse the memory: wasm linear memory never shrinks, and
  /// leaving the free to the garbage collector (the `--weak-refs` finalizer)
  /// let every page switch raise the high-water mark until a GC ran.
  void dispose();
}
