// P2-M3: the desktop local-offline facade (native only).
//
// Wraps the on-device store (`MicaStore`) + the page tree (`local_view`) + the
// active document backend (`LocalDocBackend`) behind a web-safe surface that
// speaks only plain maps/records — so `main.dart` (which is also compiled for
// web) can drive a fully local workspace without statically importing the native
// FFI. The web build gets `local_offline_web.dart` instead, where everything is
// unavailable.
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import '../cloud/cloud_doc_store.dart';
import '../cloud/store_cloud_doc_store.dart';
import '../src/rust/api/document.dart';
import '../src/rust/api/store.dart';
import '../src/rust/frb_generated.dart';
import '../upload/sha256.dart';
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

/// One local workspace, as plain data. [role] is the user's membership role,
/// mirrored from the server so an offline start knows whether editing is allowed
/// (P2d); local workspaces are the user's own (owner).
typedef WorkspaceData = ({String id, String name, String position, String role});

/// A mirrored page tree read back from the store for one `origin` (a server
/// URL) — workspaces + views, for offline navigation (P2 option C, P1c).
typedef CloudPageTreeCache = ({
  List<WorkspaceData> workspaces,
  List<ViewData> views,
});

/// A loaded document: its root block id and full block list (snapshot payload).
typedef DocData = ({String rootBlockId, List<Map<String, dynamic>> blocks});

/// Outcome of a vault import (S-tier read-only scan): documents + folder-pages
/// created, and any per-file errors (unreadable files, etc.).
typedef VaultImportResult = ({int docs, int folders, List<String> errors});

class LocalOffline {
  /// [rootDirOverride] redirects the data dir (store + blob CAS) — for tests
  /// only; production uses the per-user app data dir.
  LocalOffline({String? rootDirOverride}) : _rootOverride = rootDirOverride;

  final String? _rootOverride;
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

  /// This device's stable yrs client id, used to pin the CRDT actor for cloud
  /// sync too (so a device's edits share one actor everywhere). Opens the store
  /// if needed; returns null if the native bridge is unavailable.
  Future<BigInt?> deviceClientId() async {
    if (_store == null) {
      try {
        await open();
      } catch (_) {
        return null;
      }
    }
    return _store?.clientId();
  }

  /// A [CloudDocStore] over this on-device store for a cloud document (P2 option
  /// C — Phase 1): the local-first mirror so a cloud doc reads offline across a
  /// restart. Null if the store isn't open (native bridge unavailable) — the
  /// cloud session then just runs online, as before. The store is opened for
  /// online mode as a side effect of [deviceClientId] (called before the session
  /// is built), so this returns a live store there.
  CloudDocStore? cloudDocStore(String docId) {
    final store = _store;
    return store == null ? null : StoreCloudDocStore(store, docId);
  }

  // ── workspaces ─────────────────────────────────────────────────────────────

  /// Workspaces for [origin] ("local" for on-device, or a server URL for a
  /// cloud mirror). Defaults to the local set so existing callers are unchanged.
  List<WorkspaceData> listWorkspaces({String origin = 'local'}) {
    final store = _store;
    if (store == null) return const [];
    return [
      for (final w in store.listWorkspaces(origin: origin))
        (id: w.id, name: w.name, position: w.position, role: w.role),
    ];
  }

  void saveWorkspace(WorkspaceData w, {String origin = 'local'}) {
    _store?.saveWorkspace(
      workspace: LocalWorkspace(
        id: w.id,
        name: w.name,
        position: w.position,
        origin: origin,
        role: w.role,
      ),
    );
  }

  /// Delete a local workspace, its view rows, and all its documents. Local-only:
  /// a cloud mirror is a read cache, never user-deleted through this path.
  void deleteWorkspace(String id) {
    final store = _store;
    if (store == null) return;
    for (final v in store.listViews(origin: 'local')) {
      if (v.workspaceId == id) store.deleteDoc(docId: v.objectId);
    }
    store.deleteWorkspace(id: id);
  }

  // ── page tree ──────────────────────────────────────────────────────────────

  /// Views for [origin] ("local" or a server URL). Defaults to local.
  List<ViewData> listViews({String origin = 'local'}) {
    final store = _store;
    if (store == null) return const [];
    return [
      for (final v in store.listViews(origin: origin))
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

  void saveView(ViewData v, {String origin = 'local'}) {
    _store?.saveView(
      view: LocalView(
        id: v.id,
        workspaceId: v.workspaceId,
        parentId: v.parentId,
        objectId: v.objectId,
        name: v.name,
        position: v.position,
        trashed: v.trashed,
        origin: origin,
      ),
    );
  }

  // ── cloud page-tree mirror (P2 option C — offline nav cache) ─────────────────

  /// Replace the mirrored page tree for [serverUrl] with [workspaces]+[views]
  /// (origin-scoped clean replace, so pages removed on the server disappear).
  /// The cloud is authoritative; this is refreshed after each successful online
  /// load so the tree survives going offline (read back via [cachedCloudPageTree]).
  void mirrorCloudPageTree(
    String serverUrl,
    List<WorkspaceData> workspaces,
    List<ViewData> views,
  ) {
    final store = _store;
    if (store == null) return;
    // Drop the previous mirror for this origin, then rewrite it.
    for (final v in store.listViews(origin: serverUrl)) {
      store.purgeView(id: v.id);
    }
    for (final w in store.listWorkspaces(origin: serverUrl)) {
      store.deleteWorkspace(id: w.id);
    }
    for (final w in workspaces) {
      saveWorkspace(w, origin: serverUrl);
    }
    for (final v in views) {
      saveView(v, origin: serverUrl);
    }
  }

  /// The mirrored page tree for [serverUrl], or null if nothing is cached.
  CloudPageTreeCache? cachedCloudPageTree(String serverUrl) {
    final store = _store;
    if (store == null) return null;
    final workspaces = listWorkspaces(origin: serverUrl);
    if (workspaces.isEmpty) return null;
    return (workspaces: workspaces, views: listViews(origin: serverUrl));
  }

  /// Read a cloud document's on-device mirror (written by the cloud session's
  /// write-through) as (rootBlockId, blocks), for building an offline placeholder
  /// bootstrap so a cached cloud doc opens with no connectivity (P1c doc-open).
  /// Null if the doc was never opened online (nothing mirrored) or the store
  /// isn't open. Read-only: it does NOT become the active local backend — cloud
  /// docs still sync through [CloudSyncSession], keyed by the same [docId].
  DocData? openCloudDocMirror(String docId) {
    final store = _store;
    if (store == null) return null;
    final doc = store.loadDoc(docId: docId);
    if (doc == null) return null;
    final blocks =
        (jsonDecode(doc.toBlocksJson()) as List).cast<Map<String, dynamic>>();
    return (rootBlockId: doc.rootBlockId(), blocks: blocks);
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

  /// Import a pre-walked tree of files as local documents, mirroring the
  /// directory layout as a page tree — folders on the path to a `.md` become
  /// empty pages; non-`.md` files and anything under a `.`-dir
  /// (`.obsidian`/`.git`/`.trash`…) are skipped. Each [entries] item is a
  /// forward-slashed relative path + its bytes (as the folder picker produces).
  /// **Read-only w.r.t. the source folder** — nothing is written back. This is
  /// the S-tier "open my vault" import; parsing is done by the authoritative Rust
  /// engine (`MicaDocument.fromMarkdown`, round-trip-stable with export).
  Future<VaultImportResult> importVaultTree(
    List<({String path, List<int> bytes})> entries,
    String workspaceId,
  ) async {
    final store = _store;
    if (store == null) {
      return (docs: 0, folders: 0, errors: const ['local store not open']);
    }
    final errors = <String>[];

    // Markdown files only, skipping anything under a dot-dir.
    final md = [
      for (final e in entries)
        if (e.path.toLowerCase().endsWith('.md') &&
            !e.path.split('/').any((s) => s.startsWith('.')))
          e,
    ]..sort((a, b) => a.path.compareTo(b.path));

    // Lazily create folder-pages (only ancestors of a real `.md`, so asset-only
    // dirs don't clutter the tree).
    final folderView = <String, String>{}; // relative dir -> view id
    final posByParent = <String?, int>{};
    String nextPos(String? parent) {
      final n = (posByParent[parent] ?? 0) + 1;
      posByParent[parent] = n;
      return (n * 10).toString().padLeft(10, '0');
    }

    var folders = 0;
    String? ensureFolder(String relDir) {
      if (relDir.isEmpty) return null;
      final existing = folderView[relDir];
      if (existing != null) return existing;
      final slash = relDir.lastIndexOf('/');
      final parentRel = slash < 0 ? '' : relDir.substring(0, slash);
      final name = slash < 0 ? relDir : relDir.substring(slash + 1);
      final parentView = ensureFolder(parentRel);
      final docId = _id('doc');
      final viewId = _id('view');
      store.saveDoc(docId: docId, doc: MicaDocument.fromMarkdown(markdown: ''));
      saveView((
        id: viewId,
        workspaceId: workspaceId,
        parentId: parentView,
        objectId: docId,
        name: name,
        position: nextPos(parentView),
        trashed: false,
      ));
      folders++;
      folderView[relDir] = viewId;
      return viewId;
    }

    var docs = 0;
    for (final e in md) {
      try {
        final path = e.path;
        final slash = path.lastIndexOf('/');
        final relDir = slash < 0 ? '' : path.substring(0, slash);
        var fileName = slash < 0 ? path : path.substring(slash + 1);
        if (fileName.toLowerCase().endsWith('.md')) {
          fileName = fileName.substring(0, fileName.length - 3);
        }
        final parentView = ensureFolder(relDir);
        final text = utf8.decode(e.bytes, allowMalformed: true);
        final docId = _id('doc');
        final viewId = _id('view');
        store.saveDoc(
          docId: docId,
          doc: MicaDocument.fromMarkdown(markdown: text),
        );
        saveView((
          id: viewId,
          workspaceId: workspaceId,
          parentId: parentView,
          objectId: docId,
          name: fileName.isEmpty ? 'Untitled' : fileName,
          position: nextPos(parentView),
          trashed: false,
        ));
        docs++;
        // Yield periodically so a large vault doesn't freeze the UI isolate.
        if (docs % 20 == 0) await Future<void>.delayed(Duration.zero);
      } catch (err) {
        errors.add('${e.path}: $err');
      }
    }
    return (docs: docs, folders: folders, errors: errors);
  }

  /// Revert a document to its last on-device checkpoint (§10 recovery). The
  /// active backend is dropped so the next [openDoc] reloads the restored base.
  void rollbackDoc(String docId) {
    _store?.rollbackDoc(docId: docId);
    if (_active?.docId == docId) _active = null;
  }

  /// Persist any pending edits now (call on page switch / app pause).
  void flush() => _active?.flush();

  /// A new, locally-unique id with the given prefix.
  String _id(String prefix) =>
      '${prefix}_${DateTime.now().microsecondsSinceEpoch}_${_seq++}';

  /// The per-user local data dir (`{appdata}/mica/local`). Houses the SQLite
  /// store and the blob CAS.
  String _localDir() {
    if (_rootOverride != null) return '$_rootOverride/local';
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
    return '$dir/local';
  }

  String _dbPath() => '${_localDir()}/store.db';

  // ── blob CAS (P2-M5): content-addressed image store, on-device + offline ─────
  //
  // Images are stored by sha256 of their bytes under `{localDir}/blobs/{sha256}`
  // and the document's image block references that sha256 as its `file_id`
  // (content-addressed = automatic dedup, deterministic, no server). The cloud
  // path keeps its own UUID file ids; local and cloud documents are separate
  // universes today, so the two id schemes never collide.

  String _blobsDir() => '${_localDir()}/blobs';
  String _blobPath(String fileId) => '${_blobsDir()}/$fileId';

  /// Store `bytes` in the local CAS, returning the content id (sha256 hex) to
  /// use as the image block's `file_id`. Idempotent — re-storing the same bytes
  /// is a no-op.
  String putBlob(Uint8List bytes) {
    final id = sha256Hex(bytes);
    final file = File(_blobPath(id));
    if (!file.existsSync()) {
      file.parent.createSync(recursive: true);
      file.writeAsBytesSync(bytes, flush: true);
    }
    return id;
  }

  /// Cache an already-identified blob under an explicit [fileId] key (rather
  /// than content-addressing by sha256). The cloud path uses this to mirror a
  /// downloaded image — whose canonical id is its server file id (a UUID) — into
  /// the same on-device CAS, so the next load (and any offline session) reads
  /// the local copy instead of re-fetching. Idempotent; no-op on empty id.
  ///
  /// UUID keys and sha256 keys share `blobs/` without colliding (distinct
  /// formats), giving §7's single on-device blob store across local & cloud docs.
  void putBlobAs(String fileId, Uint8List bytes) {
    if (fileId.isEmpty) return;
    final file = File(_blobPath(fileId));
    if (!file.existsSync()) {
      file.parent.createSync(recursive: true);
      file.writeAsBytesSync(bytes, flush: true);
    }
  }

  /// Load a blob's bytes by `file_id` (sha256 for local docs, or a cloud file
  /// id cached via [putBlobAs]), or null if it isn't stored.
  Uint8List? loadBlob(String fileId) {
    final file = File(_blobPath(fileId));
    return file.existsSync() ? file.readAsBytesSync() : null;
  }

  /// Whether the local CAS holds a blob for `file_id`.
  bool hasBlob(String fileId) => File(_blobPath(fileId)).existsSync();

  /// A `file://` URI for a stored blob (for copy/export), or null if absent.
  String? blobFileUri(String fileId) {
    final file = File(_blobPath(fileId));
    return file.existsSync() ? file.uri.toString() : null;
  }
}
