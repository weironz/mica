// Desktop replica adapter: the Rust yrs FFI [MicaDocument] behind the
// [SyncDocReplica] seam. [DocOpMirror] replays coarse editor ops and keeps the
// marks-data cache the editor's text-only update stragglers need; seeding it is
// part of constructing the adapter, so a caller can't forget it.
//
// Not imported on web (FFI): cloud_sync_session.dart picks this file or the
// web variant by conditional import.
import 'dart:convert';
import 'dart:typed_data';

// The typedef `DocOp` exists on both sides of this import (same underlying
// type); the seam's copy is canonical here.
import '../local/doc_ops.dart' hide DocOp;
import '../src/rust/api/document.dart';
import 'sync_doc_replica.dart';

/// Decode [state] into a replica pinned to this device's stable yrs [clientId],
/// so all of a device's edits share one CRDT actor across sessions. Null when
/// the bytes don't decode — the caller treats that as corrupt/absent.
SyncDocReplica? replicaFromState(Uint8List state, BigInt clientId) {
  final doc =
      MicaDocument.fromStateWithClientId(bytes: state, clientId: clientId);
  if (doc == null) return null;
  return _YrsReplica(doc);
}

class _YrsReplica implements SyncDocReplica {
  _YrsReplica(this._doc) {
    _mirror.seedFrom(_doc);
  }

  final MicaDocument _doc;
  final DocOpMirror _mirror = DocOpMirror();

  @override
  String rootBlockId() => _doc.rootBlockId();

  @override
  Uint8List stateVector() => _doc.stateVector();

  @override
  Uint8List encodeState() => _doc.encodeState();

  @override
  Uint8List encodeDiffSince(Uint8List sv) =>
      _doc.encodeDiffSince(stateVector: sv);

  @override
  bool applyUpdate(Uint8List update) => _doc.applyUpdate(update: update);

  @override
  List<Map<String, dynamic>> toBlocks() =>
      (jsonDecode(_doc.toBlocksJson()) as List).cast<Map<String, dynamic>>();

  @override
  void applyEditorOp(DocOp op) => _mirror.apply(_doc, op);

  @override
  Uint8List mergeUpdates(List<Uint8List> updates) {
    // `[0, 0]` is the lib0 v1 encoding of an empty update — zero clients, empty
    // delete set — and decoding it yields a doc with no blocks that re-encodes
    // to exactly those two bytes. `MicaDocument.fromBlocksJson` would NOT do:
    // it writes `meta.root`, which the merge would then carry into the receiving
    // document as a concurrent write. (Verified against mica-core.)
    final scratch = MicaDocument.fromState(bytes: _emptyUpdate)!;
    for (final u in updates) {
      scratch.applyUpdate(update: u);
    }
    return scratch.encodeState();
  }
}

const List<int> _emptyUpdate = [0, 0];
