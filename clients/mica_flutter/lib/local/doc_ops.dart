// Desktop entry to the shared op replay (`cloud/block_op_mirror.dart`): the
// FFI [MicaDocument] as a [BlockOpTarget]. Used by the local (offline) backend
// and the desktop cloud replica, so local and cloud interpret edits
// identically — and, through the shared mirror, identically to the web.
//
// Not imported on web (depends on the native FFI).
import '../cloud/block_op_mirror.dart';
import '../cloud/sync_doc_replica.dart' show DocOp;
import '../src/rust/api/document.dart';

export '../cloud/sync_doc_replica.dart' show DocOp;

/// [BlockOpMirror] over the FFI document.
class DocOpMirror {
  final BlockOpMirror _mirror = BlockOpMirror();

  /// Reset the marks cache from a document's current blocks.
  void seedFrom(MicaDocument doc) => _mirror.seedFrom(_FfiTarget(doc));

  /// Replay one editor op onto [doc].
  void apply(MicaDocument doc, DocOp op) => _mirror.apply(_FfiTarget(doc), op);
}

/// The FFI class is generated, so it cannot declare `implements`; this
/// forwards the five calls.
class _FfiTarget implements BlockOpTarget {
  _FfiTarget(this._doc);
  final MicaDocument _doc;

  @override
  String toBlocksJson() => _doc.toBlocksJson();
  @override
  void insertBlockJson({
    required String parentId,
    required int index,
    required String blockJson,
  }) => _doc.insertBlockJson(
    parentId: parentId,
    index: index,
    blockJson: blockJson,
  );
  @override
  void updateBlock({
    required String id,
    String? kind,
    String? text,
    String? dataJson,
  }) => _doc.updateBlock(id: id, kind: kind, text: text, dataJson: dataJson);
  @override
  void deleteBlock({required String id, required bool bringChildren}) =>
      _doc.deleteBlock(id: id, bringChildren: bringChildren);
  @override
  void moveBlock({
    required String id,
    required String newParent,
    required int index,
  }) => _doc.moveBlock(id: id, newParent: newParent, index: index);
}
