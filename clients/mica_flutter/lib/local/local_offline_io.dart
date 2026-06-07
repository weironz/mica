// P2-M3: the desktop local-offline facade (native only).
//
// Wraps the on-device store (`MicaStore`) + the page tree (`local_view`) + the
// active document backend (`LocalDocBackend`) behind a web-safe surface that
// speaks only plain maps/records — so `main.dart` (which is also compiled for
// web) can drive a fully local workspace without statically importing the native
// FFI. The web build gets `local_offline_web.dart` instead, where everything is
// unavailable.
import 'dart:io';

import '../src/rust/api/store.dart';
import '../src/rust/frb_generated.dart';
import 'local_doc.dart';

/// One page-tree node, as plain data for the UI layer to map onto its own model.
typedef ViewData = ({
  String id,
  String workspaceId,
  String? parentId,
  String objectId,
  String name,
  String position,
  bool trashed,
});

/// One local workspace, as plain data.
typedef WorkspaceData = ({String id, String name, String position});

/// A loaded document: its root block id and full block list (snapshot payload).
typedef DocData = ({String rootBlockId, List<Map<String, dynamic>> blocks});

class LocalOffline {
  MicaStore? _store;
  LocalDocBackend? _active;
  int _seq = 0;
  static bool _frbReady = false;

  bool get available => true;

  /// Open the on-device store (under the per-user app data dir), initialising the
  /// native bridge once. Idempotent across calls.
  Future<void> open() async {
    if (!_frbReady) {
      try {
        await RustLib.init();
      } catch (_) {
        // Already initialised by a prior open() or another entry point.
      }
      _frbReady = true;
    }
    final path = _dbPath();
    Directory(File(path).parent.path).createSync(recursive: true);
    final store = MicaStore.open(path: path);
    if (store == null) {
      throw StateError('cannot open local store at $path');
    }
    _store = store;
  }

  // ── workspaces ─────────────────────────────────────────────────────────────

  List<WorkspaceData> listWorkspaces() {
    final store = _store;
    if (store == null) return const [];
    return [
      for (final w in store.listWorkspaces())
        (id: w.id, name: w.name, position: w.position),
    ];
  }

  void saveWorkspace(WorkspaceData w) {
    _store?.saveWorkspace(
      workspace: LocalWorkspace(id: w.id, name: w.name, position: w.position),
    );
  }

  /// Delete a workspace, its view rows, and all its documents.
  void deleteWorkspace(String id) {
    final store = _store;
    if (store == null) return;
    for (final v in store.listViews()) {
      if (v.workspaceId == id) store.deleteDoc(docId: v.objectId);
    }
    store.deleteWorkspace(id: id);
  }

  // ── page tree ──────────────────────────────────────────────────────────────

  List<ViewData> listViews() {
    final store = _store;
    if (store == null) return const [];
    return [
      for (final v in store.listViews())
        (
          id: v.id,
          workspaceId: v.workspaceId,
          parentId: v.parentId,
          objectId: v.objectId,
          name: v.name,
          position: v.position,
          trashed: v.trashed,
        ),
    ];
  }

  void saveView(ViewData v) {
    _store?.saveView(
      view: LocalView(
        id: v.id,
        workspaceId: v.workspaceId,
        parentId: v.parentId,
        objectId: v.objectId,
        name: v.name,
        position: v.position,
        trashed: v.trashed,
      ),
    );
  }

  /// Permanently remove a view and its document.
  void purgeView(String viewId, String objectId) {
    _store?.purgeView(id: viewId);
    _store?.deleteDoc(docId: objectId);
  }

  // ── documents ────────────────────────────────────────────────────────────

  /// Create a new empty document, make it active, and return its ids + blocks.
  ({String docId, String rootBlockId, List<Map<String, dynamic>> blocks})
      newDoc() {
    final store = _store!;
    final docId = _id('doc');
    final rootId = _id('root');
    final backend = LocalDocBackend.open(store, docId, rootId: rootId);
    _active = backend;
    return (
      docId: docId,
      rootBlockId: backend.rootBlockId,
      blocks: backend.allBlocks(),
    );
  }

  /// Open an existing document by id and make it active. Flushes the previously
  /// active document first.
  DocData? openDoc(String docId) {
    final store = _store;
    if (store == null) return null;
    _active?.flush();
    final backend = LocalDocBackend.open(store, docId);
    _active = backend;
    return (rootBlockId: backend.rootBlockId, blocks: backend.allBlocks());
  }

  /// Apply the editor's op batch to the active document.
  Future<void> applyOps(List<Map<String, dynamic>> ops) async {
    await _active?.applyOps(ops);
  }

  /// Persist any pending edits now (call on page switch / app pause).
  void flush() => _active?.flush();

  /// A new, locally-unique id with the given prefix.
  String _id(String prefix) =>
      '${prefix}_${DateTime.now().microsecondsSinceEpoch}_${_seq++}';

  String _dbPath() {
    final env = Platform.environment;
    String dir;
    if (Platform.isWindows) {
      final appData = env['APPDATA'];
      dir = '${(appData == null || appData.isEmpty) ? '.' : appData}/mica';
    } else if (Platform.isMacOS) {
      dir = '${env['HOME'] ?? '.'}/Library/Application Support/mica';
    } else {
      final xdg = env['XDG_DATA_HOME'];
      dir = (xdg != null && xdg.isNotEmpty)
          ? '$xdg/mica'
          : '${env['HOME'] ?? '.'}/.local/share/mica';
    }
    return '$dir/local/store.db';
  }
}
