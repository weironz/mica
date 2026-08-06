// Web replica adapter: the JS yjs [MicaYDoc] behind the [SyncDocReplica] seam.
// yjs and yrs are wire-compatible (verified W1/W2), so a web replica and a
// desktop replica editing the same document converge with no translation.
//
// [clientId] is accepted for signature parity and ignored: yjs assigns a
// per-session actor id, and a fresh actor per page load is CRDT-correct
// (convergence holds), so the web store doesn't pin one.
import 'dart:typed_data';

import '../web/mica_ydoc.dart';
import 'sync_doc_replica.dart';

/// Decode [state] into a yjs-backed replica. Null when the bytes don't apply
/// ([MicaYDoc.fromState] throws on corrupt input) — same contract as desktop.
SyncDocReplica? replicaFromState(Uint8List state, BigInt clientId) {
  try {
    return _YjsReplica(MicaYDoc.fromState(state));
  } catch (_) {
    return null;
  }
}

class _YjsReplica implements SyncDocReplica {
  _YjsReplica(this._doc);

  final MicaYDoc _doc;

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
  void applyEditorOp(DocOp op) => _doc.applyOp(op);

  @override
  Uint8List mergeUpdates(List<Uint8List> updates) {
    // A fresh Y.Doc is genuinely empty — nothing to strip, unlike a doc built
    // from blocks, whose `meta.root` write would ride along in the merged bytes
    // and land on the receiver as a concurrent write.
    final scratch = MicaYDoc.empty();
    for (final u in updates) {
      scratch.applyUpdate(u);
    }
    return scratch.encodeState();
  }
}
