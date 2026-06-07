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
import 'doc_ops.dart';

export 'doc_ops.dart' show DocOp;

class LocalDocBackend {
  LocalDocBackend._(this._store, this._doc, this.docId, this.rootBlockId) {
    _mirror.seedFrom(_doc);
  }

  final MicaStore _store;
  final MicaDocument _doc;

  /// Store key for this document.
  final String docId;

  /// The root (page) block id; the editor's nodes are this block's children.
  final String rootBlockId;

  // Shared editor-op → yrs translator (also used by the cloud sync session), so
  // local and cloud interpret edits identically.
  final DocOpMirror _mirror = DocOpMirror();

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

  /// Mirror the editor's op batch into the on-device yrs doc, then persist
  /// (debounced). Returning a Future satisfies the editor's `ApplyOps` contract;
  /// the FFI calls are synchronous so this resolves immediately.
  Future<void> applyOps(List<DocOp> ops) async {
    for (final op in ops) {
      _mirror.apply(_doc, op);
    }
    _scheduleSave();
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
