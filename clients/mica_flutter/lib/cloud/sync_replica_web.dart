// Web replica adapter: the Rust document core as wasm ([MicaDocWasm]) behind
// the [SyncDocReplica] seam — the same engine, op replay and edit rules as the
// desktop adapter (sync_replica_io.dart), over wasm instead of FFI.
//
// [clientId] is accepted for signature parity and ignored: the web has no
// device identity (callers pass 0), and pinning every browser to one yrs actor
// would corrupt the CRDT. Each replica takes a random actor, which is
// CRDT-correct (convergence holds) and what the yjs engine did before.
import 'dart:typed_data';

import '../web/mica_doc_wasm.dart';
import 'block_op_mirror.dart';
import 'sync_doc_replica.dart';

/// Decode [state] into a replica. Null when the bytes don't decode or the
/// engine did not load — same contract as desktop.
SyncDocReplica? replicaFromState(Uint8List state, BigInt clientId) {
  final doc = MicaDocWasm.fromState(state);
  return doc == null ? null : _WasmReplica(doc);
}

class _WasmReplica implements SyncDocReplica {
  _WasmReplica(this._doc) {
    _mirror.seedFrom(_doc);
    _doc.trackChanges();
  }

  final MicaDocWasm _doc;
  final BlockOpMirror _mirror = BlockOpMirror();

  @override
  String rootBlockId() => _doc.rootBlockId();

  @override
  Uint8List stateVector() => _doc.stateVector();

  @override
  Uint8List encodeState() => _doc.encodeState();

  @override
  Uint8List encodeDiffSince(Uint8List sv) => _doc.encodeDiffSince(sv);

  @override
  bool applyUpdate(Uint8List update) => _doc.applyUpdate(update);

  @override
  List<Map<String, dynamic>> toBlocks() => _doc.toBlocks();

  @override
  List<String>? takeChangedBlockIds() => _doc.takeChangedBlockIds();

  @override
  List<Map<String, dynamic>> blocksById(List<String> ids) =>
      _doc.blocksById(ids);

  @override
  void applyEditorOp(DocOp op) => _mirror.apply(_doc, op);

  @override
  void beginHistoryBatch() => _doc.beginHistoryBatch();
  @override
  void endHistoryBatch() => _doc.endHistoryBatch();
  @override
  void resetHistory() {
    _doc.resetHistory();
    _mirror.seedFrom(_doc);
  }

  @override
  bool undo() {
    final changed = _doc.undo();
    _mirror.seedFrom(_doc);
    return changed;
  }

  @override
  bool redo() {
    final changed = _doc.redo();
    _mirror.seedFrom(_doc);
    return changed;
  }

  @override
  bool canUndo() => _doc.canUndo();
  @override
  bool canRedo() => _doc.canRedo();
  @override
  (int, int) historyDepths() => _doc.historyDepths();

  @override
  Uint8List mergeUpdates(List<Uint8List> updates) {
    // `[0, 0]` is the lib0 v1 empty update: a genuinely empty scratch. A doc
    // built from blocks would write `meta.root`, which the merge would carry
    // into the receiver as a concurrent write (see sync_replica_io.dart).
    final scratch = MicaDocWasm.fromState(Uint8List.fromList(const [0, 0]))!;
    try {
      for (final u in updates) {
        scratch.applyUpdate(u);
      }
      return scratch.encodeState();
    } finally {
      scratch.free();
    }
  }
}
