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

// The interface makes every member an override; 35 `@override` annotations
// would drown the real signal in enforcement noise, so the lint is off here.
// ignore_for_file: annotate_overrides
import '../cloud/cloud_doc_store.dart';
import '../cloud/store_cloud_doc_store.dart';
import '../src/rust/api/document.dart';
import '../src/rust/api/pdf.dart';
import '../src/rust/api/store.dart';
import '../src/rust/frb_generated.dart';
import '../upload/sha256.dart';
import 'local_doc.dart';
import 'local_offline_api.dart';

// The shared plain-data types (ViewData & co.) live in the contract file; keep
// re-exporting them so `import 'local_offline.dart'` users see them as before.
export 'local_offline_api.dart';

class LocalOffline implements LocalOfflineApi {
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
  /// native bridge once. Idempotent AND single-flight — P3c starts the local and
  /// cloud init chains concurrently, and the cloud one may lazily open the store
  /// via [deviceClientId] while this is mid-flight; without single-flight both
  /// would open the same SQLite file and one native handle would leak.
  Future<void> open() {
    if (_store != null) return Future.value();
    return _opening ??= _openOnce().whenComplete(() => _opening = null);
  }

  Future<void>? _opening;

  Future<void> _openOnce() async {
    if (_store != null) return;
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

  /// Renumber workspace positions to match [ids] order (drag-reorder). Positions
  /// are zero-padded `n*10` so lexical order == intended order (same scheme the
  /// server uses). Unknown ids are skipped.
  void reorderWorkspaces(List<String> ids, {String origin = 'local'}) {
    final byId = {for (final w in listWorkspaces(origin: origin)) w.id: w};
    for (var i = 0; i < ids.length; i++) {
      final w = byId[ids[i]];
      if (w == null) continue;
      final pos = ((i + 1) * 10).toString().padLeft(10, '0');
      saveWorkspace(
        (id: w.id, name: w.name, position: pos, role: w.role),
        origin: origin,
      );
    }
  }

  /// Delete a local workspace, its view rows, and all its documents. Local-only:
  /// a cloud mirror is a read cache, never user-deleted through this path.
  void deleteWorkspace(String id) {
    final store = _store;
    if (store == null) return;
    for (final v in store.listViews(origin: 'local')) {
      if (v.workspaceId == id) store.deleteDoc(docId: v.objectId);
    }
    store.deleteWorkspace(origin: 'local', id: id);
  }

  /// Erase everything mirrored from [origin] — every doc, view and workspace.
  /// Used when a server is removed: its data lives on that server, and what we
  /// keep here is only a mirror of it.
  ///
  /// Scoped by origin throughout (the store has always taken it; the methods
  /// above merely pin it to `'local'`), so one server's removal cannot reach
  /// another's rows, nor the on-device workspaces — which are nobody's mirror
  /// and would be gone for good.
  void forgetOrigin(String origin) {
    final store = _store;
    if (store == null || origin == 'local') return;
    for (final v in store.listViews(origin: origin)) {
      store.deleteDoc(docId: v.objectId);
    }
    for (final w in store.listWorkspaces(origin: origin)) {
      store.deleteWorkspace(origin: origin, id: w.id);
    }
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
          objectType: v.objectType,
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
        objectType: v.objectType,
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
    // Drop the previous mirror for this origin, then rewrite it. Explicitly
    // origin-scoped: with the v4 (origin,id) PK it is structurally impossible
    // for this clean-replace to touch another origin's rows.
    for (final v in store.listViews(origin: serverUrl)) {
      store.purgeView(origin: serverUrl, id: v.id);
    }
    for (final w in store.listWorkspaces(origin: serverUrl)) {
      store.deleteWorkspace(origin: serverUrl, id: w.id);
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

  /// Permanently remove a local view and its document.
  void purgeView(String viewId, String objectId) {
    _store?.purgeView(origin: 'local', id: viewId);
    _store?.deleteDoc(docId: objectId);
  }

  /// Detach a mirrored cloud workspace into a NEW independent local workspace
  /// (P3f §6.2, the "换 id 版"): copies the workspace + view rows to origin
  /// 'local' and every mirrored document to a FRESH doc id, so the local fork
  /// shares nothing with the still-present cloud mirror (no session cross-talk,
  /// no sync_cursor/doc_update ties — new ids never had any). Blobs are already
  /// content-addressed in the shared CAS (zero-copy). Un-pushed outbox edits are
  /// INCLUDED in the copy (loadDoc = base + replay(log)) and still push from
  /// the mirror on the next connect — nothing is lost on either side. A doc
  /// never opened online has no mirror: its page is created empty (name kept).
  /// Returns the new local workspace id + copied doc count, or null if the
  /// store isn't open / nothing to detach.
  ({String workspaceId, int docs})? detachCloudWorkspace(
    String serverUrl,
    String cloudWorkspaceId,
    String name,
  ) {
    final store = _store;
    if (store == null) return null;
    final views = [
      for (final v in store.listViews(origin: serverUrl))
        if (v.workspaceId == cloudWorkspaceId) v,
    ];
    final wsId = _id('ws');
    store.saveWorkspace(
      workspace: LocalWorkspace(
        id: wsId,
        name: name,
        position: _nextLocalWorkspacePosition(),
        origin: 'local',
        role: 'owner',
      ),
    );
    // Two passes: mint every view id first so parent links remap correctly
    // regardless of tree order.
    final viewIdMap = {for (final v in views) v.id: _id('view')};
    var docs = 0;
    for (final v in views) {
      final doc = store.loadDoc(docId: v.objectId);
      final newDocId = _id('doc');
      if (doc != null) {
        store.saveDoc(docId: newDocId, doc: doc);
        docs++;
      } else {
        // Never mirrored — keep the page node with empty content.
        store.saveDoc(
          docId: newDocId,
          doc: MicaDocument.fromMarkdown(markdown: ''),
        );
      }
      store.saveView(
        view: LocalView(
          id: viewIdMap[v.id]!,
          workspaceId: wsId,
          parentId: v.parentId == null ? null : viewIdMap[v.parentId],
          objectId: newDocId,
          name: v.name,
          position: v.position,
          trashed: v.trashed,
          origin: 'local',
          objectType: v.objectType,
        ),
      );
    }
    return (workspaceId: wsId, docs: docs);
  }

  /// Duplicate the subtree rooted at [viewId] within its own local workspace.
  /// Every node gets a fresh view id and a fresh doc (loadDoc → saveDoc under a
  /// new id, so the copy shares no CRDT state with the source); blobs stay
  /// shared via the content-addressed CAS (same file_id, zero copy — the same
  /// dedup intent the cloud path relies on). The copied root sits beside the
  /// original (same parent) with [rootName] deduped against its live siblings.
  /// Returns the new root view id, the deduped name, and the copied doc count,
  /// or null if the store isn't open or the view is gone.
  ({String rootViewId, String newName, int docs})? cloneView({
    required String viewId,
    required String rootName,
  }) {
    final store = _store;
    if (store == null) return null;
    final all = store.listViews(origin: 'local');
    final root = all.where((v) => v.id == viewId).firstOrNull;
    if (root == null) return null;

    // Collect the subtree (root + all descendants).
    final subtree = <LocalView>[];
    final queue = <LocalView>[root];
    while (queue.isNotEmpty) {
      final v = queue.removeLast();
      subtree.add(v);
      for (final c in all) {
        if (c.parentId == v.id) queue.add(c);
      }
    }

    // Dedup the copy's name against live siblings under the same parent, and
    // place it after them (max position + 10, matching _localCreate*).
    var maxPos = 0;
    final siblingNames = <String>[];
    for (final v in all) {
      if (v.parentId == root.parentId && !v.trashed) {
        siblingNames.add(v.name);
        final n = int.tryParse(v.position) ?? 0;
        if (n > maxPos) maxPos = n;
      }
    }
    final newName = _dedupName(rootName, siblingNames);
    final rootPosition = (maxPos + 10).toString().padLeft(10, '0');

    // Fresh ids for every node; copy each doc; remap parents. The root keeps its
    // parent (beside the original); inner nodes point at their copied parent.
    final idMap = {for (final v in subtree) v.id: _id('view')};
    var docs = 0;
    for (final v in subtree) {
      final isRoot = v.id == viewId;
      final doc = store.loadDoc(docId: v.objectId);
      final newDocId = _id('doc');
      if (doc != null) {
        store.saveDoc(docId: newDocId, doc: doc);
        docs++;
      } else {
        store.saveDoc(
          docId: newDocId,
          doc: MicaDocument.fromMarkdown(markdown: ''),
        );
      }
      store.saveView(
        view: LocalView(
          id: idMap[v.id]!,
          workspaceId: v.workspaceId,
          parentId: isRoot ? v.parentId : idMap[v.parentId],
          objectId: newDocId,
          name: isRoot ? newName : v.name,
          position: isRoot ? rootPosition : v.position,
          trashed: false,
          origin: 'local',
          objectType: v.objectType,
        ),
      );
    }
    return (rootViewId: idMap[viewId]!, newName: newName, docs: docs);
  }

  /// `base` if free among [siblings], else `base 2`, `base 3`, … (the number is
  /// locale-neutral; the caller supplies the localized base). Mirrors the
  /// server's dedup_sibling_name so cloud and local pick the same shape.
  String _dedupName(String base, List<String> siblings) {
    if (!siblings.contains(base)) return base;
    for (var n = 2;; n++) {
      final candidate = '$base $n';
      if (!siblings.contains(candidate)) return candidate;
    }
  }

  /// Next zero-padded position after the last local workspace.
  String _nextLocalWorkspacePosition() {
    var max = 0;
    for (final w in listWorkspaces()) {
      final n = int.tryParse(w.position) ?? 0;
      if (n > max) max = n;
    }
    return ((max + 10)).toString().padLeft(10, '0');
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
    String workspaceId, {
    String? parentViewId,
  }) async {
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
    // Seed each parent's counter from its EXISTING children's max position, so
    // an import into a NON-EMPTY folder/root (the common case for the folder ⋯
    // "import into folder" menu) doesn't reuse positions already taken —
    // local_view has no (parent, position) unique constraint or tiebreak, so a
    // collision would leave sibling order undefined. Work in position-integer
    // space (10-spaced), matching _nextLocalPosition on the host.
    final lastPosByParent = <String?, int>{};
    for (final v in store.listViews(origin: 'local')) {
      if (v.workspaceId != workspaceId) continue;
      final n = int.tryParse(v.position) ?? 0;
      if (n > (lastPosByParent[v.parentId] ?? 0)) {
        lastPosByParent[v.parentId] = n;
      }
    }
    String nextPos(String? parent) {
      final n = (lastPosByParent[parent] ?? 0) + 10;
      lastPosByParent[parent] = n;
      return n.toString().padLeft(10, '0');
    }

    var folders = 0;
    String? ensureFolder(String relDir) {
      // Top-level nodes hang off the import target (a folder) when given, else
      // the workspace root — the local mirror of the server's `root_parent`.
      if (relDir.isEmpty) return parentViewId;
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
        // A vault directory is a pure container — a folder (F2 makes the server
        // import do the same). The empty doc above is an unused placeholder.
        objectType: 'folder',
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
          objectType: 'document',
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

  /// A local page's version timeline (newest first): auto snapshots + named
  /// checkpoints. `createdAt` is unix millis. Empty if the store isn't open.
  List<({String id, String? label, int createdAt})> listDocVersions(String docId) {
    final store = _store;
    if (store == null) return const [];
    return store
        .listLocalVersions(docId: docId)
        .map((v) => (id: v.id, label: v.label, createdAt: v.createdAt))
        .toList();
  }

  /// Pin the page's current state as a NAMED version. Null if there's no saved
  /// snapshot yet (or the store is closed).
  ({String id, String? label, int createdAt})? createDocVersion(
    String docId,
    String label,
  ) {
    final store = _store;
    if (store == null) return null;
    final v = store.createLocalVersion(docId: docId, label: label);
    return v == null ? null : (id: v.id, label: v.label, createdAt: v.createdAt);
  }

  /// A version's content (blocks in tree order + root id) for a READ-ONLY
  /// preview — decoded into a throwaway doc, never the live one. Null if the
  /// store is closed or the version is gone.
  ({String rootBlockId, List<Map<String, dynamic>> blocks})? docVersionContent(
    String docId,
    String versionId,
  ) {
    final store = _store;
    if (store == null) return null;
    final doc = store.localVersionDoc(docId: docId, versionId: versionId);
    if (doc == null) return null;
    final blocks = (jsonDecode(doc.toBlocksJson()) as List)
        .map((b) => Map<String, dynamic>.from(b as Map))
        .toList();
    return (rootBlockId: doc.rootBlockId(), blocks: blocks);
  }

  /// Restore a local page to a version (persists it + drops the pending log).
  /// Returns whether the version existed; the caller reloads the editor.
  bool restoreDocVersion(String docId, String versionId) {
    final store = _store;
    if (store == null) return false;
    return store.restoreLocalVersion(docId: docId, versionId: versionId) != null;
  }

  /// Export a LOCAL page as a self-contained HTML document, through the same
  /// Rust engine the server uses (so a local export matches a cloud one). Images
  /// are read from the on-device blob CAS and inlined as `data:` URIs; a missing
  /// blob just degrades to a broken `<img>`, never a failed export. Returns null
  /// if the doc isn't in the store.
  /// Backed by a flag FILE, not a pref: the Windows runner reads it BEFORE the
  /// engine starts (long before any Dart runs) to pick the GPU adapter — see
  /// main.cpp, which rebuilds this exact path natively. Keep the two in sync.
  bool? get gpuLowPower {
    if (!Platform.isWindows) return null;
    return File(_gpuFlagPath()).existsSync();
  }

  void setGpuLowPower(bool value) {
    if (!Platform.isWindows) return;
    final flag = File(_gpuFlagPath());
    try {
      if (value) {
        flag.parent.createSync(recursive: true);
        flag.writeAsStringSync('1');
      } else if (flag.existsSync()) {
        flag.deleteSync();
      }
    } catch (_) {
      // A failed toggle just leaves the previous mode; the switch re-reads on
      // next settings open, so the UI can't drift from the file.
    }
  }

  /// Beside (not inside) the local store dir: %APPDATA%/mica/gpu_low_power.
  /// Deliberately ignores [_rootOverride] — this is a machine-level launch
  /// flag for the real runner, meaningless under a test root.
  String _gpuFlagPath() {
    final appData = Platform.environment['APPDATA'];
    return '${(appData == null || appData.isEmpty) ? '.' : appData}/mica/gpu_low_power';
  }

  String? exportDocHtml(String docId, String title, {int contentWidth = 1160}) {
    final store = _store;
    if (store == null) return null;
    // The open editor doc lives in `_active` with a 400ms debounced save. If
    // we're exporting the doc that's currently open, force its pending edits to
    // disk first — otherwise `loadDoc` returns a stale snapshot and the export
    // is a different (older) document than what's on screen.
    if (_active?.docId == docId) _active!.flush();
    final doc = store.loadDoc(docId: docId);
    if (doc == null) return null;
    final blocks =
        (jsonDecode(doc.toBlocksJson()) as List).cast<Map<String, dynamic>>();
    final srcs = <String, String>{};
    for (final block in blocks) {
      if (block['type'] != 'image') continue;
      final data = block['data'];
      final fileId = data is Map ? data['file_id'] as String? : null;
      if (fileId == null || srcs.containsKey(fileId)) continue;
      final bytes = loadBlob(fileId);
      if (bytes == null) continue;
      srcs[fileId] = 'data:${_sniffImageMime(bytes)};base64,${base64Encode(bytes)}';
    }
    return doc.exportHtml(
      title: title,
      imageSrcs: srcs,
      contentWidth: contentWidth,
    );
  }

  /// Export a LOCAL page as Markdown, choosing the shape by content (matches the
  /// cloud `_exportPage`): no bundled images → a clean `.md`; any local image →
  /// a `.zip` (`<base>.md` + `assets/`), built by the SAME Rust engine + ZIP
  /// writer the cloud uses, so the two are byte-compatible. Returns null if the
  /// doc isn't in the store. [base] names the `.md`/`.zip` (the page title).
  ({Uint8List bytes, String name, String mime})? exportDocMarkdown(
    String docId,
    String base,
  ) {
    final store = _store;
    if (store == null) return null;
    if (_active?.docId == docId) _active!.flush(); // flush pending edits first
    final doc = store.loadDoc(docId: docId);
    if (doc == null) return null;
    final blocks = (jsonDecode(doc.toBlocksJson()) as List)
        .cast<Map<String, dynamic>>();
    // Gather on-device image bytes per file_id (first occurrence wins); the FFI
    // names/dedups them under assets/ exactly as the server does.
    final assets = <ZipAsset>[];
    final seen = <String>{};
    for (final block in blocks) {
      if (block['type'] != 'image') continue;
      final data = block['data'];
      final fileId = data is Map ? data['file_id'] as String? : null;
      if (fileId == null || !seen.add(fileId)) continue;
      final bytes = loadBlob(fileId);
      if (bytes == null) continue;
      assets.add(ZipAsset(fileId: fileId, bytes: bytes));
    }
    if (assets.isEmpty) {
      final md = doc.exportMarkdown();
      return (
        bytes: Uint8List.fromList(utf8.encode(md)),
        name: '$base.md',
        mime: 'text/markdown',
      );
    }
    final zip = doc.exportMarkdownZip(base: base, assets: assets);
    return (bytes: zip, name: '$base.zip', mime: 'application/zip');
  }

  /// Export a LOCAL folder's subtree as a Markdown ZIP, through the SAME shared
  /// Rust builder + ZIP writer the cloud uses (so it's the same format and
  /// round-trips). The store reads views + document payloads itself; here we
  /// gather the on-device image blob bytes for the subtree's pages and hand
  /// them in. Returns null if the store isn't open.
  Uint8List? exportFolderZip(String workspaceId, String folderId) {
    final store = _store;
    if (store == null) return null;
    final views = store
        .listViews(origin: 'local')
        .where((v) => v.workspaceId == workspaceId && !v.trashed)
        .toList();
    // Document object_ids in the folder's subtree (pre-order walk).
    final docObjectIds = <String>[];
    final queue = <String>[folderId];
    while (queue.isNotEmpty) {
      final parent = queue.removeLast();
      for (final v in views.where((v) => v.parentId == parent)) {
        if (v.objectType == 'document') docObjectIds.add(v.objectId);
        queue.add(v.id);
      }
    }
    // Gather each referenced image's bytes once (dedup by file_id — the local
    // CAS is content-addressed, so file_id is the dedup key on the Rust side).
    final images = <FolderExportImage>[];
    final seen = <String>{};
    for (final objId in docObjectIds) {
      if (_active?.docId == objId) _active!.flush(); // freshest content
      final doc = store.loadDoc(docId: objId);
      if (doc == null) continue;
      final blocks = (jsonDecode(doc.toBlocksJson()) as List)
          .cast<Map<String, dynamic>>();
      for (final block in blocks) {
        if (block['type'] != 'image') continue;
        final data = block['data'];
        final fileId = data is Map ? data['file_id'] as String? : null;
        if (fileId == null || !seen.add(fileId)) continue;
        final name = data is Map ? data['name'] as String? : null;
        final bytes = loadBlob(fileId);
        if (bytes == null) continue;
        images.add(
          FolderExportImage(fileId: fileId, name: name ?? 'image', bytes: bytes),
        );
      }
    }
    return store.exportFolderZip(
      workspaceId: workspaceId,
      folderId: folderId,
      images: images,
    );
  }

  /// Render a self-contained HTML document to a real PDF (vector, selectable
  /// text, embedded CJK) via the OS-preinstalled WebView2 runtime's headless
  /// print-to-PDF. This isn't page/store specific — it's the one platform-glue
  /// step that turns exported HTML (cloud OR local) into PDF bytes, so it lives
  /// on the native boundary the web build already stubs out. Returns null on
  /// non-Windows platforms / if the runtime is unavailable / print failed.
  Future<Uint8List?> htmlToPdf(String html) => exportPdf(html: html);
}

/// Sniff an image's MIME from its magic bytes — the local blob CAS keys by
/// sha256 and keeps no extension, so a `data:` URI has nowhere else to learn the
/// type. Defaults to PNG (the format editor paste/upload produces).
String _sniffImageMime(Uint8List b) {
  if (b.length >= 3 && b[0] == 0xFF && b[1] == 0xD8 && b[2] == 0xFF) {
    return 'image/jpeg';
  }
  if (b.length >= 4 && b[0] == 0x47 && b[1] == 0x49 && b[2] == 0x46) {
    return 'image/gif';
  }
  if (b.length >= 12 &&
      b[0] == 0x52 &&
      b[1] == 0x49 &&
      b[2] == 0x46 &&
      b[8] == 0x57 &&
      b[9] == 0x45 &&
      b[10] == 0x42 &&
      b[11] == 0x50) {
    return 'image/webp';
  }
  if (b.isNotEmpty && b[0] == 0x3C) {
    return 'image/svg+xml';
  }
  return 'image/png';
}
