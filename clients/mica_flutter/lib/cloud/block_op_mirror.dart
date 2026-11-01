// The ONE bridge from the editor's coarse block-op stream to a CRDT replica,
// for every platform. The self-drawn editor funnels each mutation through one
// `onOps` sink as coarse ops (`insert_block` / `update_block` / `delete_block` /
// `move_block`); this replays each onto a [BlockOpTarget].
//
// Desktop's target is the Rust FFI `MicaDocument` (via `local/doc_ops.dart`);
// the web's is the same Rust core compiled to wasm (`web/mica_doc_wasm.dart`).
// Until 2026-11-01 the web ran yjs with its own Dart copy of this replay and of
// the edit ops beneath it (`web/mica_ydoc.dart`, with marks_diff/text_diff);
// both sides now share this file above and mica-core below.
import 'dart:convert';

import 'sync_doc_replica.dart' show DocOp;

/// The edit surface of a document replica, as both the FFI `MicaDocument` and
/// the wasm `MicaDocument` expose it (same names, same semantics).
abstract interface class BlockOpTarget {
  String toBlocksJson();
  void insertBlockJson({
    required String parentId,
    required int index,
    required String blockJson,
  });
  void updateBlock({
    required String id,
    String? kind,
    String? text,
    String? dataJson,
  });
  void deleteBlock({required String id, required bool bringChildren});
  void moveBlock({
    required String id,
    required String newParent,
    required int index,
  });
}

/// Replays editor block-ops onto a [BlockOpTarget], recovering inline marks for
/// the editor's text-only `update_block` straggler from a per-block `data` cache.
class BlockOpMirror {
  // Last-known `data` map per block id. The editor's debounced text straggler
  // (`{type:update_block, block_id, text}`) omits `data`, but inline marks live
  // *inside* data — a text-only update would otherwise drop them.
  final Map<String, Map<String, dynamic>> _dataById = {};

  /// Reset the marks cache from a document's current blocks (call after building
  /// or replacing the doc, e.g. on bootstrap, and after undo/redo).
  void seedFrom(BlockOpTarget doc) {
    _dataById.clear();
    final all =
        (jsonDecode(doc.toBlocksJson()) as List).cast<Map<String, dynamic>>();
    for (final b in all) {
      final d = b['data'];
      if (d is Map<String, dynamic>) _dataById[b['id'] as String] = d;
    }
  }

  /// Replay one editor op onto [doc].
  void apply(BlockOpTarget doc, DocOp op) {
    switch (op['type'] as String?) {
      case 'insert_block':
        final block = (op['block'] as Map).cast<String, dynamic>();
        final id = block['id'] as String;
        final data = block['data'];
        if (data is Map<String, dynamic>) _dataById[id] = data;
        doc.insertBlockJson(
          parentId: op['parent_id'] as String,
          index: op['index'] as int,
          blockJson: jsonEncode(block),
        );
      case 'update_block':
        final id = op['block_id'] as String;
        Map<String, dynamic>? data;
        if (op['data'] is Map) {
          data = (op['data'] as Map).cast<String, dynamic>();
          _dataById[id] = data;
        } else if (op.containsKey('text')) {
          // text-only straggler: recover marks from the last-known data.
          data = _dataById[id];
        }
        doc.updateBlock(
          id: id,
          kind: op['kind'] as String?,
          text: op['text'] as String?,
          dataJson: data == null ? null : jsonEncode(data),
        );
      case 'delete_block':
        final id = op['block_id'] as String;
        _dataById.remove(id);
        // The editor lifts children itself before deleting, so don't re-parent.
        doc.deleteBlock(id: id, bringChildren: false);
      case 'move_block':
        doc.moveBlock(
          id: op['block_id'] as String,
          newParent: op['parent_id'] as String,
          index: op['index'] as int,
        );
    }
  }
}
