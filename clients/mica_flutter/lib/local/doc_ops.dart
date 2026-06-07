// P2-M4.5: the shared bridge from the editor's coarse block-op stream to a yrs
// [MicaDocument]. Used by BOTH the local (offline) backend and the cloud sync
// session, so local and cloud interpret edits identically.
//
// The self-drawn editor funnels every mutation through one `onOps` sink as a
// stream of coarse ops (`insert_block` / `update_block` / `delete_block` /
// `move_block`). This mirror replays each onto the CRDT doc.
//
// Not imported on web (depends on the native FFI); callers guard with `!kIsWeb`.
import 'dart:convert';

import '../src/rust/api/document.dart';

/// A block op as the editor emits it (see `editor/controller.dart`).
typedef DocOp = Map<String, dynamic>;

/// Replays editor block-ops onto a [MicaDocument], recovering inline marks for
/// the editor's text-only `update_block` straggler from a per-block `data` cache.
class DocOpMirror {
  // Last-known `data` map per block id. The editor's debounced text straggler
  // (`{type:update_block, block_id, text}`) omits `data`, but inline marks live
  // *inside* data — a text-only update would otherwise drop them.
  final Map<String, Map<String, dynamic>> _dataById = {};

  /// Reset the marks cache from a document's current blocks (call after building
  /// or replacing the doc, e.g. on bootstrap).
  void seedFrom(MicaDocument doc) {
    _dataById.clear();
    final all =
        (jsonDecode(doc.toBlocksJson()) as List).cast<Map<String, dynamic>>();
    for (final b in all) {
      final d = b['data'];
      if (d is Map<String, dynamic>) _dataById[b['id'] as String] = d;
    }
  }

  /// Replay one editor op onto [doc].
  void apply(MicaDocument doc, DocOp op) {
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
