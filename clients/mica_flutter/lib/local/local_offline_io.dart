// P2-M3: the desktop local-offline facade (native only).
//
// Wraps the on-device store (`MicaStore`) + the page tree (`local_view`) + the
// active document backend (`LocalDocBackend`) behind a web-safe surface that
// speaks only plain maps/records — so `main.dart` (which is also compiled for
// web) can drive a fully local workspace without statically importing the native
// FFI. The web build gets `local_offline_web.dart` instead, where everything is
// unavailable.
import 'cache_stats.dart';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';
import 'package:flutter/foundation.dart' show visibleForTesting;
import '../editor/operation_history.dart';

// The interface makes every member an override; 35 `@override` annotations
// would drown the real signal in enforcement noise, so the lint is off here.
// ignore_for_file: annotate_overrides
import '../cloud/cloud_doc_store.dart';
import '../cloud/store_cloud_doc_store.dart';
import '../src/rust/api/document.dart';
import '../src/rust/api/pdf.dart';
import '../src/rust/api/store.dart';
import '../src/rust/api/zip.dart' as zip_ffi;
import '../upload/sha256.dart';
import 'local_doc.dart';
import 'local_offline_api.dart';
import 'native_rust.dart';
import '../api/models.dart';

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
        await initNativeRust();
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

  /// Test-only: substitute the store instead of opening the native bridge —
  /// plain `flutter test` cannot load the FlutterRustBridge runtime, so the
  /// Dart half of the vault import (filtering, the blob CAS, error merging)
  /// is locked against a fake.
  @visibleForTesting
  void debugSetStoreForTesting(MicaStore store) => _store = store;

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

  /// A [CloudDocStore] over this on-device store for a cloud document in
  /// [origin] (P2 option C — Phase 1): the local-first mirror so a cloud doc
  /// reads offline across a restart. [origin] is the ACCOUNT-SCOPED server key
  /// from `cloudCacheOrigin` — never a bare server URL — so one account's
  /// replica/outbox/cursor can never be opened as another's (issue #111).
  /// Null if the store isn't open (native bridge unavailable).
  CloudDocStore? cloudDocStore(String origin, String docId) {
    final store = _store;
    return store == null ? null : StoreCloudDocStore(store, origin, docId);
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
    // Local ordering is Rust's, by the same step-of-ten rule as create and as
    // the view tree. A mirrored cloud origin keeps the row-by-row path: its
    // order is the server's, and this only writes the mirror.
    if (origin == 'local') {
      _store?.reorderWorkspaces(orderedIds: ids);
      return;
    }
    final byId = {for (final w in listWorkspaces(origin: origin)) w.id: w};
    for (var i = 0; i < ids.length; i++) {
      final w = byId[ids[i]];
      if (w == null) continue;
      saveWorkspace((
        id: w.id,
        name: w.name,
        position: ((i + 1) * 10).toString().padLeft(10, '0'),
        role: w.role,
      ), origin: origin);
    }
  }

  /// Erase everything mirrored from [origin] — every doc, view and workspace.
  /// Used when a server is removed: its data lives on that server, and what we
  /// keep here is only a mirror of it.
  ///
  /// Scoped by origin throughout (the store has always taken it; the methods
  /// above merely pin it to `'local'`), so one server's removal cannot reach
  /// another's rows, nor the on-device workspaces — which are nobody's mirror
  /// and would be gone for good.
  ///
  /// NOT a "clear cache": it drops the durable outbox too. Settings' clear path
  /// uses [clearMirrorCache], which retains un-pushed documents (issue #110).
  void forgetOrigin(String origin) {
    final store = _store;
    if (store == null || origin == 'local') return;
    for (final v in store.listViews(origin: origin)) {
      store.deleteDoc(origin: origin, docId: v.objectId);
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

  Future<List<SearchResult>> searchLocal(
    String workspaceId,
    String query, {
    int limit = 50,
  }) async {
    final store = _store;
    if (store == null) return const [];
    final hits = await store.searchLocal(
      workspaceId: workspaceId,
      query: query,
      limit: limit,
    );
    return [
      for (final h in hits)
        SearchResult(
          viewId: h.viewId,
          objectId: h.objectId,
          name: h.name,
          snippet: h.snippet,
          titleMatch: h.titleMatch,
          isFolder: h.isFolder,
          parentViewId: h.parentViewId,
        ),
    ];
  }

  Future<List<Backlink>> backlinksLocal(
    String workspaceId,
    String viewId,
  ) async {
    final store = _store;
    if (store == null) return const [];
    return [
      for (final b in await store.backlinksLocal(
        workspaceId: workspaceId,
        viewId: viewId,
      ))
        Backlink(viewId: b.viewId, documentId: b.objectId, title: b.name),
    ];
  }

  Future<PageGraph> graphLocal(String workspaceId) async {
    final store = _store;
    if (store == null) return PageGraph.empty;
    final g = await store.graphLocal(workspaceId: workspaceId);
    return PageGraph(
      nodes: [
        for (final n in g.nodes)
          GraphNode(viewId: n.viewId, name: n.name, degree: n.degree),
      ],
      edges: [
        for (final e in g.edges) GraphEdge(source: e.source, target: e.target),
      ],
      unlinked: g.unlinked,
    );
  }

  Future<int> backfillSearchIndex(int limit) async {
    final store = _store;
    if (store == null) return 0;
    return store.backfillSearchIndex(limit: limit);
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

  /// Replace the mirrored page tree for [origin] with [workspaces]+[views] —
  /// but ONLY the workspaces whose view list this call carries.
  ///
  /// [origin] is the account-scoped key from `cloudCacheOrigin`; the cloud is
  /// authoritative within that scope. Other workspaces keep their mirrored
  /// rows: a refresh of one workspace used to purge EVERY view of the origin
  /// and rewrite only the workspaces loaded in the current session, so a
  /// workspace opened yesterday lost its offline tree today (issue #109).
  ///
  /// [viewWorkspaceIds] names the workspaces whose [views] list is COMPLETE
  /// (including an empty one). When omitted it falls back to the workspaces the
  /// views mention — which cannot express "this workspace became empty", so
  /// callers that know the selection pass the set explicitly.
  void mirrorCloudPageTree(
    String origin,
    List<WorkspaceData> workspaces,
    List<ViewData> views, {
    Set<String> viewWorkspaceIds = const {},
  }) {
    final store = _store;
    if (store == null) return;
    final replaced = viewWorkspaceIds.isNotEmpty
        ? viewWorkspaceIds
        : {for (final v in views) v.workspaceId};
    final live = {for (final w in workspaces) w.id};
    // Stale pages of a replaced workspace disappear, and so do the rows of a
    // workspace the server no longer lists.
    for (final v in store.listViews(origin: origin)) {
      if (replaced.contains(v.workspaceId) || !live.contains(v.workspaceId)) {
        store.purgeView(origin: origin, id: v.id);
      }
    }
    // Metadata is deleted only for workspaces that really vanished. Deleting
    // every row and re-adding the incoming ones would cascade away the views of
    // the workspaces this call does NOT carry.
    for (final w in store.listWorkspaces(origin: origin)) {
      if (!live.contains(w.id)) {
        store.deleteWorkspace(origin: origin, id: w.id);
      }
    }
    for (final w in workspaces) {
      saveWorkspace(w, origin: origin);
    }
    for (final v in views) {
      saveView(v, origin: origin);
    }
  }

  /// The mirrored page tree for [origin], or null if nothing is cached.
  CloudPageTreeCache? cachedCloudPageTree(String origin) {
    final store = _store;
    if (store == null) return null;
    final workspaces = listWorkspaces(origin: origin);
    if (workspaces.isEmpty) return null;
    return (workspaces: workspaces, views: listViews(origin: origin));
  }

  /// Read a cloud document's on-device mirror in [origin] (written by the cloud
  /// session's write-through) as (rootBlockId, blocks), for building an offline
  /// placeholder bootstrap so a cached cloud doc opens with no connectivity
  /// (P1c doc-open). Null if the doc was never opened online (nothing mirrored)
  /// or the store isn't open. Read-only: it does NOT become the active local
  /// backend — cloud docs still sync through [CloudSyncSession], keyed by the
  /// same [docId] under the same account-scoped origin.
  DocData? openCloudDocMirror(String origin, String docId) {
    final store = _store;
    if (store == null) return null;
    final doc = store.loadDoc(origin: origin, docId: docId);
    if (doc == null) return null;
    final blocks = (jsonDecode(doc.toBlocksJson()) as List)
        .cast<Map<String, dynamic>>();
    return (rootBlockId: doc.rootBlockId(), blocks: blocks);
  }

  /// Write a page's title into its LOCAL document, so the offline world keeps
  /// the same authority as the cloud: the name lives in the document and
  /// everything that reads the document sees it.
  ///
  /// The cloud does this inside `PATCH /views/{id}` (server-side); offline there
  /// is no server, so the rename path calls this directly. A no-op — and no
  /// write — when the title already matches, because a yrs map insert is an
  /// operation either way and renaming to the same name should not grow the
  /// document.
  ///
  /// Silent when the store is closed or the document is missing: the caller has
  /// already renamed the view row, which is what the user asked for.
  void setDocTitle(String docId, String title) {
    final store = _store;
    if (store == null) return;
    if (_active?.docId == docId) _active!.flush(); // don't race pending edits
    final doc = store.loadDoc(origin: 'local', docId: docId);
    if (doc == null) return;
    if (!doc.setTitle(title: title)) return;
    store.saveDoc(origin: 'local', docId: docId, doc: doc);
  }

  /// Permanently remove a local view and its document.
  void purgeView(String viewId, String objectId) {
    _store?.purgeView(origin: 'local', id: viewId);
    _store?.deleteDoc(origin: 'local', docId: objectId);
  }

  @override
  List<String> trashViewSubtree(String viewId) =>
      _store?.trashViewSubtree(viewId: viewId) ?? const [];

  @override
  List<String> restoreViewSubtree(String viewId) =>
      _store?.restoreViewSubtree(viewId: viewId) ?? const [];

  @override
  List<String> purgeViewSubtree(String viewId) =>
      _store?.purgeViewSubtree(viewId: viewId) ?? const [];

  @override
  String createView({
    required String workspaceId,
    String? parentId,
    required String objectId,
    required String name,
    required String objectType,
  }) =>
      _store?.createView(
        workspaceId: workspaceId,
        parentId: parentId,
        objectId: objectId,
        name: name,
        objectType: objectType,
      ) ??
      '';

  @override
  void reorderViews(String? parentId, List<String> orderedIds) =>
      _store?.reorderViews(parentId: parentId, orderedIds: orderedIds);

  @override
  String createLocalWorkspace(String name) =>
      _store?.createWorkspace(name: name) ?? '';

  @override
  void renameLocalWorkspace(String id, String name) =>
      _store?.renameWorkspace(id: id, name: name);

  @override
  bool deleteLocalWorkspace(String id) =>
      _store?.deleteWorkspaceCascade(id: id) ?? false;

  /// Detach a mirrored cloud workspace into a NEW independent local workspace
  /// (P3f §6.2, the "换 id 版"): copies the workspace + view rows to origin
  /// 'local' and every mirrored document to a FRESH doc id, so the local fork
  /// shares nothing with the still-present cloud mirror (no session cross-talk,
  /// no sync_cursor/doc_update ties — new ids never had any). Blobs are already
  /// content-addressed in the shared CAS (zero-copy). Un-pushed outbox edits are
  /// INCLUDED in the copy (loadDoc = base + replay(log)) and still push from
  /// the mirror on the next connect — nothing is lost on either side. A doc
  /// never opened online has no mirror: its page is created empty (name kept).
  /// [origin] is the account-scoped mirror key (`cloudCacheOrigin`).
  /// Returns the new local workspace id + copied doc count, or null if the
  /// store isn't open / nothing to detach.
  ({String workspaceId, int docs})? detachCloudWorkspace(
    String origin,
    String cloudWorkspaceId,
    String name,
  ) {
    final store = _store;
    if (store == null) return null;
    final views = [
      for (final v in store.listViews(origin: origin))
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
      final doc = store.loadDoc(origin: origin, docId: v.objectId);
      final newDocId = _id('doc');
      if (doc != null) {
        store.saveDoc(origin: 'local', docId: newDocId, doc: doc);
        docs++;
      } else {
        // Never mirrored — keep the page node with empty content.
        store.saveDoc(
          origin: 'local',
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
    // Forwards to Rust. The subtree walk, id remap, doc copies, sibling-name
    // dedup and positioning used to be reimplemented here — a second copy of
    // rules the server already owns, kept in step by nothing but a comment
    // claiming so. `MicaStore::clone_view` is now the only implementation.
    //
    // The WHOLE operation crosses the bridge, not each step: the dedup helper
    // alone was seven lines, and bridging that would have left the tree logic
    // duplicated while adding a crossing per node.
    final r = _store?.cloneView(viewId: viewId, rootName: rootName);
    if (r == null) return null;
    return (rootViewId: r.rootViewId, newName: r.newName, docs: r.docs);
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
    final backend = LocalDocBackend.open(store, 'local', docId, rootId: rootId);
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
    final backend = LocalDocBackend.open(store, 'local', docId);
    _active = backend;
    return (rootBlockId: backend.rootBlockId, blocks: backend.allBlocks());
  }

  /// Apply the editor's op batch to the active document.
  Future<void> applyOps(
    List<Map<String, dynamic>> ops, {
    bool trackHistory = false,
  }) async {
    await _active?.applyOps(ops, trackHistory: trackHistory);
  }

  Future<EditorHistoryState> applyHistory(HistoryAction action) async {
    final backend = _active;
    if (backend == null) throw StateError('No active local document');
    return backend.applyHistory(action);
  }

  /// Import a pre-walked tree of files as local documents, mirroring the
  /// directory layout as a page tree — folders on the path to a `.md` become
  /// folders; non-`.md` files become blobs and anything under a `.`-dir
  /// (`.obsidian`/`.git`/`.trash`…) is skipped. Each [entries] item is a
  /// forward-slashed relative path + its bytes (as the folder picker produces).
  /// **Read-only w.r.t. the source folder** — nothing is written back. This is
  /// the S-tier "open my vault" import.
  ///
  /// The walk itself moved to Rust (issue #84): `MicaStore.importVaultTree`
  /// plans the tree through `mica_interchange::plan_vault_import` (the same
  /// plan core the archive importer uses) and lays positions with the shared
  /// `next_position` — the Dart twin that walked directories and counted
  /// positions here was a second copy of both. What stays on this side is the
  /// one thing FFI has no writer for: the on-device blob CAS.
  Future<VaultImportResult> importVaultTree(
    List<({String path, List<int> bytes})> entries,
    String workspaceId, {
    String? parentViewId,
  }) async {
    final store = _store;
    if (store == null) {
      return (
        docs: 0,
        folders: 0,
        errors: const <String>['local store not open'],
        viewIds: const <String>[],
      );
    }
    final errors = <String>[];

    final visible = [
      for (final e in entries)
        if (!e.path.split('/').any((s) => s.startsWith('.'))) e,
    ];

    // Non-Markdown entries become BLOBS, keyed by content — first, so the
    // page conversion in Rust gets a path→blob-id map to rewire references
    // against. (They used to be dropped here, silently: a vault whose pages
    // said `![](assets/diagram.png)` imported as pages whose every image was a
    // dead link, with nothing in the result saying a file was discarded.)
    final assetIds = <String, String>{};
    var scanned = 0;
    for (final e in visible) {
      if (e.path.toLowerCase().endsWith('.md')) continue;
      try {
        assetIds[e.path] = putBlob(Uint8List.fromList(e.bytes));
      } catch (err) {
        // Named, not swallowed: an image that failed to store leaves its page
        // pointing at something that will not resolve, and the user should
        // hear which one rather than find out later.
        errors.add('${e.path}: $err');
      }
      // Yield periodically so a big vault doesn't freeze the UI isolate.
      if (++scanned % 20 == 0) await Future<void>.delayed(Duration.zero);
    }

    // Tree planning + positions + per-page conversion, all across the bridge
    // in one call (not one crossing per node). Only Markdown goes over: the
    // plan core never needs attachment bytes, so a vault of images crosses
    // exactly once — into the CAS above — not twice.
    final outcome = await store.importVaultTree(
      entries: [
        for (final e in visible)
          if (e.path.toLowerCase().endsWith('.md'))
            VaultTreeEntry(
              path: e.path,
              bytes: Uint8List.fromList(e.bytes),
            ),
      ],
      workspaceId: workspaceId,
      parentViewId: parentViewId,
      assetIds: assetIds,
    );
    return (
      docs: outcome.docs,
      folders: outcome.folders,
      errors: [...errors, ...outcome.errors],
      viewIds: outcome.viewIds,
    );
  }

  bool get canUnpackZip => true;

  Future<
    ({
      String? wrapperName,
      int markdownCount,
      List<({String path, List<int> bytes})> entries,
    })
  >
  unpackImportZip(Uint8List bytes) async {
    final out = await zip_ffi.unpackImportZip(bytes: bytes);
    return (
      wrapperName: out.wrapperName,
      markdownCount: out.markdownCount,
      entries: [
        for (final e in out.entries)
          (path: e.name, bytes: e.bytes as List<int>),
      ],
    );
  }

  /// Import ONE Markdown file through the same vault-import path the folder
  /// import uses (Rust planning + parsing + the shared link rewriter), then
  /// report the created view so the caller can open it (issue #108).
  @override
  Future<VaultImportResult> importMarkdownPage(
    String fileName,
    String markdown,
    String workspaceId, {
    required String fallbackTitle,
  }) {
    final base = fileName
        .replaceAll(RegExp(r'\.(md|markdown|txt)$', caseSensitive: false), '')
        .trim();
    final title = titleFromMarkdown(
      markdown,
      base.isEmpty ? fallbackTitle : base,
    );
    final safe = title.replaceAll(RegExp(r'[/\\:*?"<>|\r\n\t]'), '_').trim();
    final path = '${safe.isEmpty ? fallbackTitle : safe}.md';
    return importVaultTree([
      (path: path, bytes: utf8.encode(markdown)),
    ], workspaceId);
  }

  /// Revert a document to its last on-device checkpoint (§10 recovery). The
  /// active backend is dropped so the next [openDoc] reloads the restored base.
  void rollbackDoc(String docId) {
    _store?.rollbackDoc(origin: 'local', docId: docId);
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

  /// The on-device origins owned by [accountId] across [servers], in the
  /// canonical form every cloud-cache row is keyed by.
  List<String> _scopedOrigins(List<String> servers, String accountId) => [
    for (final s in servers) cloudCacheOrigin(s, accountId),
  ];

  /// Measure the on-device store, splitting re-downloadable mirrors from
  /// local-only originals, for ONE account.
  ///
  /// Pages come from the store's own `origin` column (`'local'` vs an
  /// account-scoped server key); bytes come from walking `blobs/` once and
  /// classifying each file by its id shape (`cache_stats.dart`). Deliberately
  /// no FFI: everything needed is already either a Dart-side file or an
  /// existing query.
  ///
  /// A blob whose file cannot be stat'd is skipped rather than failing the whole
  /// figure — a size that is slightly low beats a settings pane that shows an
  /// error where a number belongs.
  @override
  Future<LocalCacheStats> cacheStats({
    required List<String> servers,
    required String accountId,
  }) async {
    // Folders are not pages and trashed rows are not either — the same definition
    // the rest of the app counts by. `listViews` includes trashed rows, so that
    // has to be said explicitly here.
    bool isPage(ViewData v) => v.objectType != 'folder' && !v.trashed;

    final localOnlyPages = listViews(origin: 'local').where(isPage).length;
    var mirroredPages = 0;
    for (final origin in _scopedOrigins(servers, accountId)) {
      mirroredPages += listViews(origin: origin).where(isPage).length;
    }

    var mirroredBytes = 0;
    var localOnlyBytes = 0;
    final dir = Directory(_blobsDir());
    if (dir.existsSync()) {
      for (final entity in dir.listSync(followLinks: false)) {
        if (entity is! File) continue;
        final id = entity.uri.pathSegments.last;
        int size;
        try {
          size = entity.lengthSync();
        } catch (_) {
          continue; // unreadable: skip rather than fail the whole number
        }
        if (isCloudMirrorBlobId(id)) {
          mirroredBytes += size;
        } else {
          localOnlyBytes += size;
        }
      }
    }

    return (
      mirroredPages: mirroredPages,
      mirroredBytes: mirroredBytes,
      localOnlyPages: localOnlyPages,
      localOnlyBytes: localOnlyBytes,
    );
  }

  /// Every image `file_id` a document references. Used to keep blobs of
  /// documents the cache clear must NOT touch (pending edits, quarantine).
  Set<String> _imageIdsIn(MicaDocument? doc) {
    if (doc == null) return const {};
    final out = <String>{};
    for (final block in (jsonDecode(doc.toBlocksJson()) as List)) {
      if (block is! Map) continue;
      if (block['type'] != 'image') continue;
      final data = block['data'];
      final fileId = data is Map ? data['file_id'] as String? : null;
      if (fileId != null && fileId.isNotEmpty) out.add(fileId);
    }
    return out;
  }

  /// Legacy (pre-#111) server-only cache rows for [servers]. Visible to NO
  /// account: we cannot verify which account wrote them, so they are kept as a
  /// quarantine and only listed (ids + page counts, never names) for an
  /// explicit, user-initiated export.
  @override
  List<({String origin, String workspaceId, int pages})> quarantinedWorkspaces({
    required List<String> servers,
  }) {
    final store = _store;
    if (store == null) return const [];
    final out = <({String origin, String workspaceId, int pages})>[];
    for (final server in servers) {
      final origin = legacyCloudCacheOrigin(server);
      if (origin == 'local') continue;
      final views = store.listViews(origin: origin);
      final workspaces = store.listWorkspaces(origin: origin);
      if (workspaces.isEmpty &&
          views.isEmpty &&
          store.listDocs(origin: origin).isEmpty) {
        continue;
      }
      final ids = workspaces.isNotEmpty
          ? {for (final w in workspaces) w.id}
          : {for (final v in views) v.workspaceId};
      for (final id in ids) {
        final pages = views
            .where((v) => v.workspaceId == id && v.objectType != 'folder' && !v.trashed)
            .length;
        out.add((origin: origin, workspaceId: id, pages: pages));
      }
    }
    return out;
  }

  /// Drop this ACCOUNT's re-downloadable mirror and report what happened.
  ///
  /// Issue #110: a document with un-pushed outbox rows is NOT a cache — that
  /// outbox is the only copy of the work. Such documents keep their snapshot,
  /// outbox, cursor, version history, AND their page/view rows (and the folder
  /// chain above them), so the user can still find, open and keep editing the
  /// page. Everything else in the scoped mirror is removed.
  ///
  /// Legacy server-only rows are never touched: they are quarantined (owner
  /// unverifiable) and reported in [ClearMirrorResult.quarantinedOrigins].
  @override
  Future<ClearMirrorResult> clearMirrorCache({
    required List<String> servers,
    required String accountId,
  }) async {
    final store = _store;
    if (store == null) {
      // No FFI store (e.g. a test harness or a failed open): there is no
      // document state to protect, but the blob half still works and is what
      // the cache-reclaim contract promises to remove.
      _deleteMirrorBlobs(const {});
      return (
        stats: await cacheStats(servers: servers, accountId: accountId),
        retainedDocs: 0,
        quarantinedOrigins: const <String>[],
      );
    }
    var retainedDocs = 0;
    final protectedBlobs = <String>{};

    for (final origin in _scopedOrigins(servers, accountId)) {
      final pending = <String>{};
      for (final docId in store.listDocs(origin: origin)) {
        final cursor = store.syncCursor(origin: origin, docId: docId);
        final outbox = store.updatesAfter(
          origin: origin,
          docId: docId,
          after: cursor.pushedClock,
        );
        if (outbox.isEmpty) continue;
        pending.add(docId);
        retainedDocs++;
        protectedBlobs.addAll(
          _imageIdsIn(store.loadDoc(origin: origin, docId: docId)),
        );
      }

      final views = store.listViews(origin: origin);
      final byId = {for (final v in views) v.id: v};
      // Keep each pending page and every folder above it, so the page stays
      // navigable. A folder is only metadata; keeping it costs nothing.
      final keep = <String>{};
      for (final v in views) {
        if (!pending.contains(v.objectId)) continue;
        String? id = v.id;
        while (id != null && keep.add(id)) {
          id = byId[id]?.parentId;
        }
      }
      for (final v in views) {
        if (keep.contains(v.id)) continue;
        store.purgeView(origin: origin, id: v.id);
      }
      for (final docId in store.listDocs(origin: origin)) {
        if (pending.contains(docId)) continue;
        store.deleteDoc(origin: origin, docId: docId);
      }
      final keptWorkspaces = {
        for (final v in views)
          if (keep.contains(v.id)) v.workspaceId,
      };
      for (final w in store.listWorkspaces(origin: origin)) {
        if (keptWorkspaces.contains(w.id)) continue;
        store.deleteWorkspace(origin: origin, id: w.id);
      }
    }

    // Quarantine: prove it exists and protect every blob its documents use.
    final quarantinedOrigins = <String>[];
    for (final server in servers) {
      final origin = legacyCloudCacheOrigin(server);
      if (origin == 'local') continue;
      final hasRows =
          store.listWorkspaces(origin: origin).isNotEmpty ||
          store.listViews(origin: origin).isNotEmpty ||
          store.listDocs(origin: origin).isNotEmpty;
      if (!hasRows) continue;
      quarantinedOrigins.add(origin);
      for (final docId in store.listDocs(origin: origin)) {
        protectedBlobs.addAll(
          _imageIdsIn(store.loadDoc(origin: origin, docId: docId)),
        );
      }
    }

    _deleteMirrorBlobs(protectedBlobs);
    flush();
    return (
      stats: await cacheStats(servers: servers, accountId: accountId),
      retainedDocs: retainedDocs,
      quarantinedOrigins: quarantinedOrigins,
    );
  }

  /// Unlink mirrored blobs, keeping every id in [protected] and anything whose
  /// id shape is not positively a server file id (the safe direction).
  void _deleteMirrorBlobs(Set<String> protected) {
    final dir = Directory(_blobsDir());
    if (!dir.existsSync()) return;
    for (final entity in dir.listSync(followLinks: false)) {
      if (entity is! File) continue;
      final id = entity.uri.pathSegments.last;
      if (!isCloudMirrorBlobId(id) || protected.contains(id)) continue;
      try {
        entity.deleteSync();
      } catch (_) {
        // A file we cannot unlink stays counted by the stats, so the number the
        // user sees afterwards stays true rather than optimistic.
      }
    }
  }

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
  List<({String id, String? label, int createdAt})> listDocVersions(
    String docId,
  ) {
    final store = _store;
    if (store == null) return const [];
    return store
        .listLocalVersions(origin: 'local', docId: docId)
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
    final v = store.createLocalVersion(origin: 'local', docId: docId, label: label);
    return v == null
        ? null
        : (id: v.id, label: v.label, createdAt: v.createdAt);
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
    final doc = store.localVersionDoc(
      origin: 'local',
      docId: docId,
      versionId: versionId,
    );
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
    return store.restoreLocalVersion(
          origin: 'local',
          docId: docId,
          versionId: versionId,
        ) !=
        null;
  }

  /// Export a LOCAL page as a self-contained HTML document, through the same
  /// Rust engine the server uses (so a local export matches a cloud one). Images
  /// are read from the on-device blob CAS and inlined as `data:` URIs; a missing
  /// blob just degrades to a broken `<img>`, never a failed export. Returns null
  /// if the doc isn't in the store.
  String? exportDocHtml(String docId, String title, {int contentWidth = 1160}) {
    final store = _store;
    if (store == null) return null;
    // The open editor doc lives in `_active` with a 400ms debounced save. If
    // we're exporting the doc that's currently open, force its pending edits to
    // disk first — otherwise `loadDoc` returns a stale snapshot and the export
    // is a different (older) document than what's on screen.
    if (_active?.docId == docId) _active!.flush();
    final doc = store.loadDoc(origin: 'local', docId: docId);
    if (doc == null) return null;
    final blocks = (jsonDecode(doc.toBlocksJson()) as List)
        .cast<Map<String, dynamic>>();
    final srcs = <String, String>{};
    for (final block in blocks) {
      if (block['type'] != 'image') continue;
      final data = block['data'];
      final fileId = data is Map ? data['file_id'] as String? : null;
      if (fileId == null || srcs.containsKey(fileId)) continue;
      final bytes = loadBlob(fileId);
      if (bytes == null) continue;
      srcs[fileId] =
          'data:${sniffImageMime(bytes)};base64,${base64Encode(bytes)}';
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
    final doc = store.loadDoc(origin: 'local', docId: docId);
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
      // Titled: a `.md` you keep otherwise loses the page name — only the file
      // name carries it, and that survives exactly as long as the file does.
      // The `.zip` branch below gets the same treatment inside the FFI.
      final md = doc.exportMarkdownTitled(title: base);
      return (
        bytes: Uint8List.fromList(utf8.encode(md)),
        name: '$base.md',
        mime: 'text/markdown',
      );
    }
    final zip = doc.exportMarkdownZip(base: base, assets: assets);
    return (bytes: zip, name: '$base.zip', mime: 'application/zip');
  }

  /// The page's Markdown TEXT, with no packaging decision — the clipboard
  /// sibling of [exportDocMarkdown].
  ///
  /// Deliberately does NOT switch to a ZIP when the page has images, which is
  /// the one thing separating it from the export above. A download can carry an
  /// `assets/` folder; a clipboard cannot, and the caller is pasting text
  /// somewhere else anyway. So image references travel as plain Markdown and
  /// dangle on the far side — the same trade every other "copy as Markdown"
  /// makes, and the reason this is a separate method instead of a flag.
  ///
  /// Returns null if the store isn't open or the doc isn't in it.
  ///
  /// [title] is the page's name and leads the text, because a clipboard has no
  /// file name to carry it — paste somewhere else and the name is simply gone.
  /// A document that carries its own title wins over this (handled in Rust).
  String? exportDocMarkdownText(String docId, String title) {
    final store = _store;
    if (store == null) return null;
    if (_active?.docId == docId) _active!.flush(); // flush pending edits first
    final doc = store.loadDoc(origin: 'local', docId: docId);
    if (doc == null) return null;
    return doc.exportMarkdownTitled(title: title);
  }

  /// Export one folder's subtree as a Markdown ZIP through the SAME shared Rust
  /// builder + ZIP writer the cloud uses (so it's the same format and
  /// round-trips). The store reads views + document payloads itself; here we
  /// gather the on-device image blob bytes for the subtree's pages.
  @override
  Uint8List? exportFolderZip(String workspaceId, String folderId) =>
      _exportTreeZip('local', workspaceId, folderId);

  /// Export a WHOLE local (or explicitly chosen origin) workspace as a Markdown
  /// ZIP. [origin] is `'local'` for a local workspace; a quarantined legacy
  /// cache can be exported by passing its legacy origin for recovery.
  @override
  Uint8List? exportWorkspaceZip(String workspaceId, {String origin = 'local'}) =>
      _exportTreeZip(origin, workspaceId, null);

  Uint8List? _exportTreeZip(
    String origin,
    String workspaceId,
    String? folderId,
  ) {
    final store = _store;
    if (store == null) return null;
    final views = store
        .listViews(origin: origin)
        .where((v) => v.workspaceId == workspaceId && !v.trashed)
        .toList();
    // Document object_ids in the requested subtree — the whole workspace when
    // [folderId] is null, otherwise the folder and everything under it.
    final docObjectIds = <String>[];
    if (folderId == null) {
      for (final v in views) {
        if (v.objectType == 'document') docObjectIds.add(v.objectId);
      }
    } else {
      final queue = <String>[folderId];
      while (queue.isNotEmpty) {
        final parent = queue.removeLast();
        for (final v in views.where((v) => v.parentId == parent)) {
          if (v.objectType == 'document') docObjectIds.add(v.objectId);
          queue.add(v.id);
        }
      }
    }
    // Gather each referenced image's bytes once (dedup by file_id — the local
    // CAS is content-addressed, so file_id is the dedup key on the Rust side).
    final images = <FolderExportImage>[];
    final seen = <String>{};
    for (final objId in docObjectIds) {
      if (origin == 'local' && _active?.docId == objId) {
        _active!.flush(); // freshest content
      }
      final doc = store.loadDoc(origin: origin, docId: objId);
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
          FolderExportImage(
            fileId: fileId,
            name: name ?? 'image',
            bytes: bytes,
          ),
        );
      }
    }
    return store.exportFolderZip(
      origin: origin,
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
