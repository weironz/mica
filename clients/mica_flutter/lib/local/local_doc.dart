// P2-M3: the desktop editor's local (offline) document backend.
//
// The self-drawn editor funnels every mutation through a single `onOps` sink as
// a stream of coarse block ops (`insert_block` / `update_block` / `delete_block`
// / `move_block`). In cloud mode that sink POSTs to the server. In local offline
// mode it is *this* class: each op is mirrored into an on-device yrs document
// (`MicaDocument`, the `crates/mica-core` CRDT) and the snapshot is persisted to
// the local SQLite store (`MicaStore`). On open we read the doc back out of the
// store and hand its blocks to the editor. That closes the desktop editing loop
// entirely on-device — no account, no network.
//
// Not imported on web (it depends on the native FFI); callers guard with
// `!kIsWeb`.
import 'dart:async';
import 'dart:convert';

import '../src/rust/api/document.dart';
import '../src/rust/api/store.dart';

/// A block op as the editor emits it (see `editor/controller.dart`).
typedef DocOp = Map<String, dynamic>;

class LocalDocBackend {
  LocalDocBackend._(this._store, this._doc, this.docId, this.rootBlockId) {
    _seedDataCache();
  }

  final MicaStore _store;
  final MicaDocument _doc;

  /// Store key for this document.
  final String docId;

  /// The root (page) block id; the editor's nodes are this block's children.
  final String rootBlockId;

  // Last-known `data` map per block id. The editor's debounced text straggler
  // (`{type:update_block, block_id, text}`) omits `data`, but inline marks live
  // *inside* data — applying a text-only update would otherwise drop them. We
  // recover the marks from this cache. Kept in sync from load + every op that
  // does carry data.
  final Map<String, Map<String, dynamic>> _dataById = {};

  Timer? _saveTimer;
  static const _saveDebounce = Duration(milliseconds: 400);

  /// Open the local document `docId` from `store`, seeding an empty one-paragraph
  /// page if it doesn't exist yet. `rootId`/`seedBlocks` only matter on first
  /// creation; an existing doc keeps its own root.
  static LocalDocBackend open(
    MicaStore store,
    String docId, {
    String rootId = 'root',
    List<Map<String, dynamic>>? seedBlocks,
  }) {
    final existing = store.loadDoc(docId: docId);
    if (existing != null) {
      return LocalDocBackend._(store, existing, docId, existing.rootBlockId());
    }
    final blocks = seedBlocks ?? emptyPage(rootId);
    final doc = MicaDocument.fromBlocksJson(
      rootId: rootId,
      blocksJson: jsonEncode(blocks),
    );
    store.saveDoc(docId: docId, doc: doc);
    return LocalDocBackend._(store, doc, docId, rootId);
  }

  /// A fresh page: a root with a single empty paragraph.
  static List<Map<String, dynamic>> emptyPage(String rootId) {
    final bodyId = '$rootId-body';
    return [
      {
        'id': rootId,
        'type': 'page',
        'text': '',
        'data': <String, dynamic>{},
        'children': [bodyId],
      },
      {
        'id': bodyId,
        'type': 'paragraph',
        'text': '',
        'data': <String, dynamic>{},
        'children': <String>[],
      },
    ];
  }

  /// The editor `nodes`: the root block's direct children, in order, as the
  /// `{id,type,text,data,children}` maps the editor consumes.
  List<Map<String, dynamic>> childBlocks() {
    final byId = _blocksById();
    final root = byId[rootBlockId];
    if (root == null) return const [];
    final children = (root['children'] as List?)?.cast<String>() ?? const [];
    return [
      for (final id in children)
        if (byId[id] != null) byId[id]!,
    ];
  }

  Map<String, Map<String, dynamic>> _blocksById() {
    final all =
        (jsonDecode(_doc.toBlocksJson()) as List).cast<Map<String, dynamic>>();
    return {for (final b in all) b['id'] as String: b};
  }

  void _seedDataCache() {
    for (final b in _blocksById().values) {
      final d = b['data'];
      if (d is Map<String, dynamic>) _dataById[b['id'] as String] = d;
    }
  }

  /// Mirror the editor's op batch into the on-device yrs doc, then persist
  /// (debounced). Returning a Future satisfies the editor's `ApplyOps` contract;
  /// the FFI calls are synchronous so this resolves immediately.
  Future<void> applyOps(List<DocOp> ops) async {
    for (final op in ops) {
      _applyOne(op);
    }
    _scheduleSave();
  }

  void _applyOne(DocOp op) {
    switch (op['type'] as String?) {
      case 'insert_block':
        final block = (op['block'] as Map).cast<String, dynamic>();
        final id = block['id'] as String;
        final data = block['data'];
        if (data is Map<String, dynamic>) _dataById[id] = data;
        _doc.insertBlockJson(
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
        _doc.updateBlock(
          id: id,
          kind: op['kind'] as String?,
          text: op['text'] as String?,
          dataJson: data == null ? null : jsonEncode(data),
        );
      case 'delete_block':
        final id = op['block_id'] as String;
        _dataById.remove(id);
        // The editor lifts children itself before deleting, so don't re-parent.
        _doc.deleteBlock(id: id, bringChildren: false);
      case 'move_block':
        _doc.moveBlock(
          id: op['block_id'] as String,
          newParent: op['parent_id'] as String,
          index: op['index'] as int,
        );
    }
  }

  void _scheduleSave() {
    _saveTimer?.cancel();
    _saveTimer = Timer(_saveDebounce, flush);
  }

  /// Persist the current document snapshot to the store immediately. Safe to
  /// call any time (on app pause, doc close, or to force a pending debounce).
  void flush() {
    _saveTimer?.cancel();
    _saveTimer = null;
    _store.saveDoc(docId: docId, doc: _doc);
  }

  /// Current document as the full blocks list (tree order) — for export/debug.
  List<Map<String, dynamic>> allBlocks() =>
      (jsonDecode(_doc.toBlocksJson()) as List).cast<Map<String, dynamic>>();

  void dispose() => flush();
}
