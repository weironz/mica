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
import 'local_offline_api.dart';

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

  /// Appends since the last compaction check. Same shape and same numbers as
  /// the cloud store's `_maybeCompact` — this file and that one now persist the
  /// same way, which is the point of the change.
  int _appendsSinceCheck = 0;
  static const _compactCheckEvery = 32;
  static const _compactThreshold = 256;

  /// Open the local document `docId` from `store`, seeding an empty one-paragraph
  /// page if it doesn't exist yet. `rootId`/`seedBlocks` only matter on first
  /// creation; an existing doc keeps its own root.
  static LocalDocBackend open(
    MicaStore store,
    String docId, {
    String rootId = 'root',
    List<Map<String, dynamic>>? seedBlocks,
  }) {
    final MicaDocument? existing;
    try {
      existing = store.loadDoc(docId: docId);
    } catch (_) {
      // The snapshot is PRESENT but corrupt/unreadable (the FFI throws now,
      // distinct from a plain absence). Do NOT fall through to the seed path:
      // seeding + saveDoc + checkpointDoc would launder the corruption AND
      // overwrite the §10 recovery backup, destroying the one thing rollback
      // needs. Surface it so the app can offer rollback / version history.
      throw LocalDocCorruptException(docId);
    }
    if (existing != null) {
      // Fold whatever the last session left in the log BEFORE checkpointing.
      // The checkpoint copies only the base row, and `rollbackDoc` restores that
      // base AND drops the log — so without this the recovery point would sit
      // before the previous session's edits and a rollback would take them with
      // it. It also means every session starts with an empty log.
      store.compactLocal(docId: docId);
      // Snapshot the last-good base before this session mutates it (§10 recovery
      // point — rollback restores it if an edit/merge later corrupts the doc).
      store.checkpointDoc(docId: docId);
      return LocalDocBackend._(store, existing, docId, existing.rootBlockId());
    }
    final blocks = seedBlocks ?? emptyPage(rootId);
    final doc = MicaDocument.fromBlocksJson(
      rootId: rootId,
      blocksJson: jsonEncode(blocks),
    );
    store.saveDoc(docId: docId, doc: doc);
    store.checkpointDoc(docId: docId);
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

  /// Mirror the editor's op batch into the on-device yrs doc and append the
  /// resulting CRDT diff to the store's log. Returning a Future satisfies the
  /// editor's `ApplyOps` contract; the FFI calls are synchronous so this
  /// resolves immediately.
  ///
  /// This used to re-encode and rewrite the WHOLE document on a 400 ms
  /// debounce — cost O(document) per burst of typing, on a file that only grows.
  /// The cloud path has been append + periodic squash since P4-1; this is the
  /// same machinery (`append_update`, `load_doc` replays base + log), so both
  /// worlds now persist one way instead of two.
  ///
  /// Two things fall out of it that are not about speed:
  ///   * **The 400 ms window is gone.** An edit was not on disk until the timer
  ///     fired; a crash inside that window lost it silently. An append is
  ///     durable at the moment of the edit.
  ///   * **`loadDoc` is never stale.** It replays the log, so a reader no longer
  ///     has to remember to [flush] first (several callers in
  ///     `local_offline_io.dart` did exactly that, and forgetting produced an
  ///     export of an older document than the one on screen).
  Future<void> applyOps(List<DocOp> ops) async {
    final sv = _doc.stateVector();
    for (final op in ops) {
      _mirror.apply(_doc, op);
    }
    final diff = _doc.encodeDiffSince(stateVector: sv);
    if (diff.isEmpty) return; // a no-op batch writes nothing
    _store.appendUpdate(docId: docId, update: diff);
    _maybeCompact();
  }

  /// Bound the log without a timer: every [_compactCheckEvery] appends, check
  /// its size and fold past [_compactThreshold].
  ///
  /// `compactLocal`, not `squash`: squash keeps `clock > pushed_clock` because
  /// those updates still owe the server, and a local-only doc's `pushed_clock`
  /// is 0 forever — so squash (and `trimUpdatesThrough`, clamped to the same
  /// mark) would be permanent no-ops here and the log would grow without bound.
  void _maybeCompact() {
    if (++_appendsSinceCheck < _compactCheckEvery) return;
    _appendsSinceCheck = 0;
    final (local, remote) = _store.logSizes(docId: docId);
    if (local + remote > _compactThreshold) _store.compactLocal(docId: docId);
  }

  /// Fold the log into the base now. Safe to call any time (app pause, doc
  /// close, before an export).
  ///
  /// No longer a save: edits are already durable when [applyOps] returns, so
  /// nothing is lost if this never runs. It is compaction, and it leaves a
  /// closed document as a single clean base.
  void flush() {
    _appendsSinceCheck = 0;
    _store.compactLocal(docId: docId);
  }

  /// Current document as the full blocks list (tree order) — for export/debug.
  List<Map<String, dynamic>> allBlocks() =>
      (jsonDecode(_doc.toBlocksJson()) as List).cast<Map<String, dynamic>>();

  void dispose() => flush();
}
