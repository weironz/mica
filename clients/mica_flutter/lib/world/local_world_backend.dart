part of '../main.dart';

/// The local world's backend: the on-device (SQLite via the LocalOffline
/// facade) page tree, selection, bootstrap and editor epoch — the six fields
/// that used to sit on `_WorkspaceShellState` — plus every local-world
/// operation, moved verbatim off the State so the local world owns itself.
/// These mirror the cloud callbacks but route to the LocalOffline facade
/// instead of `_api`.
///
/// The shell handle is needed for the cross-cutting concerns (setState,
/// context, the shared LocalOffline store, the app-exit flush); the world's
/// own state lives entirely here.
class _LocalWorldBackend extends WorldBackend {
  _LocalWorldBackend(this._shell);

  final _WorkspaceShellState _shell;

  /// The shared on-device store — owned by the shell (it also backs the cloud
  /// mirrors), aliased here so the moved method bodies read unchanged.
  LocalOffline get _local => _shell._local;

  // --- Local offline (P2-M3) ---
  // A single implicit local workspace + synthetic identity; the page tree and
  // documents live entirely on-device.
  List<Workspace> _localWorkspaces = const [];
  Workspace? _localSelectedWorkspace;
  List<DocumentView> _localViews = const [];
  DocumentView? _localSelectedView;
  DocumentBootstrap? _localBootstrap;
  // Bumped on rollback to force the local editor to remount fresh.
  int _localEditorEpoch = 0;

  // ── WorldBackend state ───────────────────────────────────────────────────

  @override
  List<Workspace> get workspaces => _localWorkspaces;

  @override
  Workspace? get selectedWorkspace => _localSelectedWorkspace;

  /// The local world keeps one flat view list rather than a per-workspace
  /// map, so it contributes just its selected workspace.
  @override
  Map<String, List<DocumentView>> get viewsByWorkspace => {
    if (_localSelectedWorkspace != null)
      _localSelectedWorkspace!.id: _localViews,
  };

  @override
  List<DocumentView> get views => _localViews;

  /// The local world's tree is read from disk synchronously — it is never
  /// "on its way". Only a cloud workspace we hold no tree for at all is.
  @override
  bool get treePending => false;

  @override
  DocumentView? get selectedView => _localSelectedView;

  @override
  DocumentBootstrap? get bootstrap => _localBootstrap;

  /// A local workspace has no members at all.
  @override
  List<WorkspaceMember> get members => const [];

  /// Single-user world; nobody to show.
  @override
  List<PresenceUser> get presence => const [];

  /// Cloud only — the local world has no tab model (see `_openViewInNewTab`).
  @override
  List<DocTab> get tabs => const [];

  @override
  int get activeTabIndex => 0;

  /// Comments are a cloud feature (they live in Postgres beside the document),
  /// so the local world gets none and shows no entry point.
  @override
  List<CommentThread> get commentThreads => const [];

  @override
  List<CommentHighlight> get commentHighlights => const [];

  /// Cloud-only: the badge is meaningless for a local world or with no live
  /// cloud session, so it's simply absent there.
  @override
  SyncPhase? get syncPhase => null;

  @override
  bool get isBusy => false;

  @override
  bool get showAi => false;

  @override
  int get editorEpoch => _localEditorEpoch;

  @override
  VoidCallback get onRefresh =>
      () => _shell.setUi(() {
        _reloadLocalWorkspaces();
        _reloadLocalViews();
      });

  // ── WorldBackend grouped operations ──────────────────────────────────────

  @override
  late final WorkspaceOps workspaceOps = WorkspaceOps(
    select: _localSelectWorkspace,
    create: _localCreateWorkspace,
    rename: _localRenameWorkspace,
    delete: _localDeleteWorkspace,
  );

  @override
  late final ViewOps viewOps = ViewOps(
    createDocument: _localCreateDocument,
    createChildDocument: (parent, name) =>
        _localCreateDocument(name, parentViewId: parent.id),
    createFolder: _localCreateFolder,
    createChildFolder: (parent, name) =>
        _localCreateFolder(name, parentViewId: parent.id),
    exportFolderZip: _shell._localExportFolderZip,
    reorder: _localReorderViews,
    loadTrash: _localLoadTrash,
    restore: _localRestoreView,
    purge: _localPurgeView,
    purgeAll: null,
    select: _localSelectView,
    rename: _localRenameView,
    setIcon: null,
    delete: _localDeleteView,
    bulkDelete: _localDeleteViews,
    clone: _localCloneView,
    openViewFromHome: _openViewFromHome,
    closeOpenPage: _closeOpenPage,
  );

  @override
  late final SearchOps searchOps = SearchOps(
    selectWorkspaceById: null,
    searchAll: null,
    openHit: null,
    // Desktop local mode now has a real index (`doc_snapshot.content_text`),
    // so it searches for real. Web local mode still has no on-device store at
    // all, and there `null` is still the honest answer — «this world cannot
    // search» is a different statement from «nothing matched», and the dialog
    // says a different thing for each. A stub returning `const []` used to
    // collapse the two, telling every local user their page was not there.
    search: kIsWeb ? null : _searchLocalWorkspace,
    openResult: _openLocalViewById,
  );

  @override
  late final ImageOps imageOps = ImageOps(
    upload: _localUploadImage,
    importUrl: _localImportImageUrl,
    loadBytes: _localLoadImageBytes,
    resolveUrls: _localResolveImageUrls,
  );

  @override
  late final ExportOps exportOps = ExportOps(
    page: _shell._localExportPage,
    copyMarkdown: _shell._localPageMarkdownText,
    pageHtml: _shell._localExportPageHtml,
    importMarkdown: _localImportMarkdown,
    workspaceZip: _localExportWorkspaceZip,
    importTree: _localImportVaultTree,
  );

  @override
  late final AiOps aiOps = AiOps(
    stream: (_, {system}) => const Stream<String>.empty(),
    newPage: (_) async {},
    previewEdits: null,
    currentPage: null,
    newWorkspace: (_) async {},
  );

  // ── WorldBackend polarity exceptions ─────────────────────────────────────

  @override
  Future<void> Function()? get versionHistory => _openLocalVersionHistory;

  @override
  Future<void> Function()? get restoreCheckpoint => _localRollbackDoc;

  // ── Local world implementation (moved off _WorkspaceShellState) ─────────

  DocumentView _viewFromData(ViewData v) => DocumentView(
    id: v.id,
    parentViewId: v.parentId,
    objectId: v.objectId,
    objectType: v.objectType,
    name: v.name,
    position: v.position,
  );

  Workspace _workspaceFromData(WorkspaceData w) =>
      Workspace(id: w.id, name: w.name, ownerId: 'local', role: 'owner');

  /// Reload the workspace list and keep (or re-anchor) the selection. The store
  /// always has at least one workspace.
  void _reloadLocalWorkspaces() {
    _localWorkspaces = [
      for (final w in _local.listWorkspaces()) _workspaceFromData(w),
    ];
    if (_localWorkspaces.isEmpty) {
      _localSelectedWorkspace = null;
      return;
    }
    // On first load restore the last-selected local workspace (AppFlowy remembers
    // the active workspace per user); after that keep the current selection.
    final selId =
        _localSelectedWorkspace?.id ?? loadPref('lastWorkspaceId:local');
    _localSelectedWorkspace = _localWorkspaces.firstWhere(
      (w) => w.id == selId,
      orElse: () => _localWorkspaces.first,
    );
  }

  /// The local page to open for the selected workspace: the one last viewed there
  /// (AppFlowy remembers it per-workspace), else the first openable page. A saved
  /// id that no longer resolves (deleted) falls through to the first page.
  DocumentView? _localViewToOpen() {
    final wsId = _localSelectedWorkspace?.id;
    final wantId = wsId == null ? null : loadPref('lastViewId:$wsId');
    final saved = _localViews
        .where((v) => v.id == wantId && v.objectType != 'folder')
        .firstOrNull;
    return saved ?? firstOpenableView(_localViews);
  }

  /// The workspace a stored view belongs to (views carry it; DocumentView does
  /// not), falling back to the selected workspace.
  String _workspaceIdOfView(String viewId) {
    for (final v in _local.listViews()) {
      if (v.id == viewId) return v.workspaceId;
    }
    return _localWsId;
  }

  DocumentBootstrap _localBootstrapFrom(
    String docId,
    String rootBlockId,
    List<Map<String, dynamic>> blocks,
    DocumentView view,
  ) {
    return DocumentBootstrap(
      document: DocumentRecord(
        id: docId,
        rootBlockId: rootBlockId,
        currentSeq: 0,
      ),
      view: view,
      snapshot: DocumentSnapshot(
        versionSeq: 1,
        schemaVersion: 1,
        payload: {'blocks': blocks},
      ),
    );
  }

  /// Reload the live (non-trashed) page tree of the selected workspace.
  void _reloadLocalViews() {
    final wsId = _localSelectedWorkspace?.id;
    _localViews = [
      for (final v in _local.listViews())
        if (!v.trashed && v.workspaceId == wsId) _viewFromData(v),
    ];
  }

  /// Next sibling position under [parentViewId] (zero-padded, 10-spaced).
  /// Open the on-device store, load the page tree, and select (or seed) a page.
  /// Index documents saved before the search projection existed.
  ///
  /// Every save writes the projection from now on, so this only ever runs
  /// through the backlog once — but that backlog is every page the user already
  /// had (588 snapshots on this machine), and none of them would be findable by
  /// body text until something walked them.
  ///
  /// **Batched, off the first frame, and never surfaced as an error.** Search
  /// being briefly incomplete is invisible; a startup that stalls decoding
  /// hundreds of CRDT documents is not. The yields between batches are the whole
  /// point — without them this is one long block with extra steps.
  Future<void> _backfillLocalSearchIndex() async {
    const batch = 25;
    try {
      while (_shell.mounted) {
        final done = await _local.backfillSearchIndex(batch);
        if (done < batch) return; // caught up
        await Future<void>.delayed(Duration.zero);
      }
    } catch (_) {
      // An index that did not finish building costs body matches on some pages.
      // It is not worth a message, and it is certainly not worth failing local
      // mode over — the notes open either way.
    }
  }

  Future<void> _initLocalOffline() async {
    if (_shell._localReady) return;
    final l10n = _shell.context.l10n;
    try {
      await _local.open();
    } catch (error) {
      if (_shell.mounted) {
        _shell.setUi(() {
          _shell._message = l10n.snackLocalStoreOpenFailed('$error');
          // Unblock the shell (P3c gates on _localReady) — the local world
          // shows empty with the error banner; the cloud world still works.
          _shell._localReady = true;
        });
      }
      return;
    }
    _reloadLocalWorkspaces();
    _reloadLocalViews();
    unawaited(_backfillLocalSearchIndex());
    if (_localViews.isEmpty) {
      await _localCreateDocument(l10n.pageWelcomeName);
    } else {
      // Open the page last viewed in this workspace (else the first openable
      // one), skipping folders (a folder has no doc to open; _localSelectView
      // would early-return, leaving the editor blank).
      final toOpen = _localViewToOpen();
      if (toOpen != null) await _localSelectView(toOpen);
    }
    if (_shell.mounted) _shell.setUi(() => _shell._localReady = true);
  }

  Future<void> _localCreateWorkspace(String name) async {
    final title = name.trim().isEmpty
        ? _shell.context.l10n.workspaceDefaultName
        : name.trim();
    // Rust mints the id and the position, by the same step-of-ten rule the
    // view tree uses.
    final id = _local.createLocalWorkspace(title);
    if (!_shell.mounted) return;
    _shell.setUi(() {
      _reloadLocalWorkspaces();
      _localSelectedWorkspace = _localWorkspaces.firstWhere(
        (w) => w.id == id,
        orElse: () => _localWorkspaces.first,
      );
      _localSelectedView = null;
      _localBootstrap = null;
      _reloadLocalViews();
    });
    // A new workspace starts empty — seed a first page.
    await _localCreateDocument(_shell.context.l10n.pageWelcomeName);
  }

  Future<void> _localSelectWorkspace(Workspace workspace) async {
    savePref('lastWorkspaceId:local', workspace.id);
    _shell.setUi(() {
      _localSelectedWorkspace = workspace;
      _reloadLocalViews();
      _localSelectedView = null;
      _localBootstrap = null;
    });
    final toOpen = _localViewToOpen();
    if (toOpen != null) await _localSelectView(toOpen);
  }

  Future<void> _localRenameWorkspace(Workspace workspace, String name) async {
    final title = name.trim().isEmpty ? workspace.name : name.trim();
    // Position and role are preserved by Rust. This used to read the row back
    // to keep the position, and when that read missed it invented
    // `0000000010` — a value that can collide with a real neighbour.
    _local.renameLocalWorkspace(workspace.id, title);
    if (_shell.mounted) {
      _shell.setUi(() {
        _reloadLocalWorkspaces();
        if (_localSelectedWorkspace?.id == workspace.id) {
          _localSelectedWorkspace = _localWorkspaces.firstWhere(
            (w) => w.id == workspace.id,
            orElse: () => _localWorkspaces.first,
          );
        }
      });
    }
  }

  Future<void> _localDeleteWorkspace(Workspace workspace) async {
    // "Keep at least one workspace" is enforced in Rust, beside the delete
    // itself — a UI-side check only covers the paths that go through the UI.
    // What stays here is saying so.
    if (!_local.deleteLocalWorkspace(workspace.id)) {
      if (_shell.mounted) {
        _shell.setUi(
          () =>
              _shell._message = _shell.context.l10n.snackKeepOneLocalWorkspace,
        );
      }
      return;
    }
    if (!_shell.mounted) return;
    final wasSelected = _localSelectedWorkspace?.id == workspace.id;
    _shell.setUi(() {
      _reloadLocalWorkspaces();
      if (wasSelected) {
        _localSelectedView = null;
        _localBootstrap = null;
      }
      _reloadLocalViews();
    });
    if (wasSelected) {
      final firstDoc = firstOpenableView(_localViews);
      if (firstDoc != null) await _localSelectView(firstDoc);
    }
  }

  Future<String?> _localCreateDocument(
    String name, {
    String? parentViewId,
  }) async {
    final title = name.trim().isEmpty
        ? _shell.context.l10n.untitledPage
        : name.trim();
    final created = _local.newDoc();
    // Rust assigns the id and the position (after the last live sibling) —
    // the same rule clone and reorder use.
    final workspaceId = _localSelectedWorkspace?.id ?? 'local';
    final viewId = _local.createView(
      workspaceId: workspaceId,
      parentId: parentViewId,
      objectId: created.docId,
      name: title,
      objectType: 'document',
    );
    final data = (
      id: viewId,
      workspaceId: workspaceId,
      parentId: parentViewId,
      objectId: created.docId,
      name: title,
      position: _local.listViews().firstWhere((v) => v.id == viewId).position,
      trashed: false,
      objectType: 'document',
    );
    final view = _viewFromData(data);
    if (!_shell.mounted) return null;
    _shell.setUi(() {
      _reloadLocalViews();
      _localSelectedView = view;
      _localBootstrap = _localBootstrapFrom(
        created.docId,
        created.rootBlockId,
        created.blocks,
        view,
      );
    });
    return viewId;
  }

  /// Create a local folder (pure container) — a view with object_type='folder'
  /// and no document. Not opened in the editor; pages are created under it.
  Future<String?> _localCreateFolder(
    String name, {
    String? parentViewId,
  }) async {
    final title = name.trim().isEmpty
        ? _shell.context.l10n.folderNewDefault
        : name.trim();
    // A folder has no document; object_id is an unused placeholder.
    final viewId = _local.createView(
      workspaceId: _localSelectedWorkspace?.id ?? 'local',
      parentId: parentViewId,
      objectId: 'folder_${DateTime.now().microsecondsSinceEpoch}',
      name: title,
      objectType: 'folder',
    );
    if (!_shell.mounted) return null;
    _shell.setUi(_reloadLocalViews);
    return viewId;
  }

  /// S-tier vault import: land a picked folder's `.md` files into the local
  /// store as documents, mirroring the directory tree (read-only — the source
  /// folder is untouched). Wired to the existing "import folder into workspace"
  /// menu; the picker + walk are shared with the cloud path.
  Future<void> _localImportVaultTree(
    Workspace workspace,
    List<ArchiveFile> entries, {
    String? sourceName,
    String? parentViewId,
    String? container,
  }) async {
    final l10n = _shell.context.l10n;
    // Parity with the cloud path: only when [container] is 'wrap' does the
    // picked folder's name become a container folder here — prefix every entry
    // path with it. 'spill' (the default) drops the contents straight in,
    // matching the server's ImportMode::IntoLocation.
    final trimmed = sourceName?.trim() ?? '';
    final wrap = container == 'wrap' && trimmed.isNotEmpty;
    final prefix = wrap ? '$trimmed/' : '';
    final files = <({String path, List<int> bytes})>[
      for (final f in entries) (path: '$prefix${f.name}', bytes: f.bytes),
    ];
    // [parentViewId] imports UNDER a folder instead of the workspace root.
    final result = await _local.importVaultTree(
      files,
      workspace.id,
      parentViewId: parentViewId,
    );
    if (!_shell.mounted) return;
    _shell.setUi(_reloadLocalViews);
    final parts = <String>[
      if (result.docs > 0) l10n.importNotesCount(result.docs),
      if (result.folders > 0) l10n.importFoldersCount(result.folders),
    ];
    final msg = parts.isEmpty
        ? (result.errors.isEmpty
              ? l10n.importNoMarkdown
              : l10n.importFailed(result.errors.first))
        : l10n.importDone(parts.join(l10n.importListSeparator)) +
              (result.errors.isNotEmpty
                  ? l10n.importSkippedSuffix(result.errors.length)
                  : '');
    ScaffoldMessenger.maybeOf(
      _shell.context,
    )?.showSnackBar(SnackBar(content: Text(msg)));
  }

  Future<void> _localSelectView(DocumentView view) async {
    if (view.objectType == 'folder') return; // a folder has no document to open
    // Per-workspace last view (AppFlowy model), same key scheme as the cloud path.
    savePref('lastViewId:${_workspaceIdOfView(view.id)}', view.id);
    final DocData? data;
    try {
      data = _local.openDoc(view.objectId);
    } on LocalDocCorruptException {
      // The on-device copy is corrupt. openDoc deliberately did NOT clobber it
      // with a blank page, so the recovery checkpoint / version history are
      // still intact — select the view and offer to restore instead of dropping
      // the user into a silent blank editor.
      if (!_shell.mounted) return;
      _shell.setUi(() => _localSelectedView = view);
      await _showLocalCorruptDialog(view);
      return;
    }
    if (data == null || !_shell.mounted) return;
    final loaded = data;
    _shell.setUi(() {
      _localSelectedView = view;
      _localBootstrap = _localBootstrapFrom(
        view.objectId,
        loaded.rootBlockId,
        loaded.blocks,
        view,
      );
    });
  }

  /// A local page whose on-device snapshot failed to decode. Offer the two
  /// non-destructive recovery paths (last checkpoint / version history) rather
  /// than silently seeding a blank page over it.
  Future<void> _showLocalCorruptDialog(DocumentView view) async {
    final l = _shell.context.l10n;
    final action = await showDialog<String>(
      context: _shell.context,
      builder: (context) => AlertDialog(
        title: Text(l.localCorruptTitle),
        content: Text(l.localCorruptBody),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: Text(l.closeCancel),
          ),
          TextButton(
            onPressed: () => Navigator.pop(context, 'history'),
            child: Text(l.localCorruptHistory),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, 'rollback'),
            child: Text(l.localCorruptRollback),
          ),
        ],
      ),
    );
    if (action == 'rollback') {
      await _localRollbackDoc();
    } else if (action == 'history') {
      await _openLocalVersionHistory();
    }
  }

  /// Restore the open local document to its last checkpoint, then remount the
  /// editor on the restored content (the bumped epoch forces a fresh editor so
  /// in-memory session edits are dropped, not reconciled).
  Future<void> _localRollbackDoc() async {
    final view = _localSelectedView;
    if (view == null) return;
    _local.rollbackDoc(view.objectId);
    if (!_shell.mounted) return;
    _localEditorEpoch++;
    await _localSelectView(view);
  }

  /// Version history for a LOCAL page — same dialog as the cloud path, but
  /// backed by the on-device `doc_version` timeline (FFI). Restore reloads the
  /// editor via the epoch bump, exactly like the checkpoint rollback above.
  Future<void> _openLocalVersionHistory() async {
    final view = _localSelectedView;
    if (view == null) return;
    final docId = view.objectId;
    // A local page persists on a debounce; force a save so "current" is captured
    // before the user browses/creates a version. Awaited now that the flush
    // drains the editor debounce asynchronously too.
    await _shell._flushForExit();
    DocVersion toDocVersion(({String id, String? label, int createdAt}) v) =>
        DocVersion(
          id: v.id,
          label: v.label,
          createdAt: DateTime.fromMillisecondsSinceEpoch(
            v.createdAt,
          ).toUtc().toIso8601String(),
        );
    await showDialog<void>(
      context: _shell.context,
      builder: (context) => _VersionHistoryDialog(
        onList: () async =>
            _local.listDocVersions(docId).map(toDocVersion).toList(),
        onCreate: (name) async {
          if (_local.createDocVersion(docId, name) == null) {
            throw Exception(context.l10n.versionEmpty);
          }
        },
        onRestore: (versionId) async {
          if (!_local.restoreDocVersion(docId, versionId)) {
            throw Exception('version not found');
          }
          if (!_shell.mounted) return;
          _localEditorEpoch++;
          await _localSelectView(view);
        },
        onLoadContent: (versionId) async {
          final content = _local.docVersionContent(docId, versionId);
          if (content == null) throw Exception('version not found');
          return content;
        },
        onLoadImageBytes: _localLoadImageBytes,
        onResolveImageUrls: _localResolveImageUrls,
      ),
    );
  }

  // ── local images (P2-M5): on-device content-addressed store, fully offline ──

  /// Store an inserted/pasted image in the local CAS; the returned `file_id` is
  /// its sha256, which the image block references.
  Future<({String fileId, String name})?> _localUploadImage(
    Uint8List bytes,
    String fileName,
    String mimeType,
  ) async {
    final id = _local.putBlob(bytes);
    if (id.isEmpty) return null;
    return (fileId: id, name: fileName.isEmpty ? 'image' : fileName);
  }

  /// Re-host an externally-pasted image URL into the local CAS by downloading it.
  Future<({String fileId, String name})?> _localImportImageUrl(
    String url,
  ) async {
    try {
      final resp = await sharedHttpClient.get(Uri.parse(url));
      if (resp.statusCode != 200) return null;
      final id = _local.putBlob(resp.bodyBytes);
      if (id.isEmpty) return null;
      final seg = Uri.parse(url).pathSegments;
      final name = seg.isNotEmpty && seg.last.isNotEmpty ? seg.last : 'image';
      return (fileId: id, name: name);
    } catch (_) {
      return null;
    }
  }

  /// Load an image for the canvas: a `file_id` (sha256) from the local CAS, or an
  /// external `http(s)` markdown image fetched directly.
  Future<Uint8List?> _localLoadImageBytes(String key) async {
    if (key.startsWith('http://') || key.startsWith('https://')) {
      try {
        final resp = await sharedHttpClient.get(Uri.parse(key));
        return resp.statusCode == 200 ? resp.bodyBytes : null;
      } catch (_) {
        return null;
      }
    }
    return _local.loadBlob(key);
  }

  /// Map local file ids to `file://` URIs (for copy/export of local images).
  Future<Map<String, String>> _localResolveImageUrls(List<String> ids) async {
    final out = <String, String>{};
    for (final id in ids) {
      final uri = _local.blobFileUri(id);
      if (uri != null) out[id] = uri;
    }
    return out;
  }

  Future<bool> _localRenameView(DocumentView view, String name) async {
    final title = name.trim().isEmpty ? 'Untitled' : name.trim();
    // The name belongs to the DOCUMENT (root block `title`); the view row is the
    // projection. The cloud does this inside `PATCH /views/{id}` server-side —
    // offline there is no server, so the rename writes both here.
    //
    // Documents only: a folder has no document to carry a title (plan §5.2).
    if (view.objectType == 'document') {
      _local.setDocTitle(view.objectId, title);
    }
    _local.saveView((
      id: view.id,
      workspaceId: _workspaceIdOfView(view.id),
      parentId: view.parentViewId,
      objectId: view.objectId,
      name: title,
      position: view.position,
      trashed: false,
      objectType: view.objectType,
    ));
    if (!_shell.mounted) return true;
    _shell.setUi(() {
      _reloadLocalViews();
      if (_localSelectedView?.id == view.id) {
        final renamed = DocumentView(
          id: view.id,
          parentViewId: view.parentViewId,
          objectId: view.objectId,
          objectType: view.objectType,
          name: title,
          position: view.position,
        );
        _localSelectedView = renamed;
        final boot = _localBootstrap;
        if (boot != null && boot.view.id == view.id) {
          _localBootstrap = DocumentBootstrap(
            document: boot.document,
            view: renamed,
            snapshot: boot.snapshot,
          );
        }
      }
    });
    return true;
  }

  /// A view and all its descendants from the on-device set (incl. trashed rows),
  /// so local delete/restore/purge cascade the whole subtree like the server's
  /// recursive-CTE handlers. Without this, trashing a folder orphans its
  /// children: deep descendants vanish from the sidebar (the orphan fallback in
  /// _visibleDocumentTree only lifts direct children) until the parent returns.
  Future<void> _localDeleteView(DocumentView view) async {
    // Soft-delete the page AND its whole subtree (folders carry children).
    // The cascade lives in Rust; what stays here is the UI consequence.
    final ids = _local.trashViewSubtree(view.id).toSet();
    if (!_shell.mounted) return;
    _shell.setUi(() {
      _reloadLocalViews();
      // Close the editor if the open page was anywhere in the trashed subtree.
      if (_localSelectedView != null && ids.contains(_localSelectedView!.id)) {
        _localSelectedView = null;
        _localBootstrap = null;
      }
    });
  }

  /// Trash a selection (local). The Rust store has no batch primitive and needs
  /// none — this is an in-process call per root, not a round trip per root.
  Future<void> _localDeleteViews(List<String> viewIds) async {
    final trashed = <String>{};
    for (final id in viewIds) {
      trashed.addAll(_local.trashViewSubtree(id));
    }
    if (!_shell.mounted) return;
    _shell.setUi(() {
      _reloadLocalViews();
      if (_localSelectedView != null &&
          trashed.contains(_localSelectedView!.id)) {
        _localSelectedView = null;
        _localBootstrap = null;
      }
    });
  }

  /// Duplicate [view] in place (local). Reuses the store's loadDoc→saveDoc copy
  /// primitive; blobs stay shared in the on-device CAS.
  Future<void> _localCloneView(DocumentView view) async {
    final copyName = _shell.context.l10n.cloneCopyName(view.name);
    final result = _local.cloneView(viewId: view.id, rootName: copyName);
    if (result == null || !_shell.mounted) return;
    _shell.setUi(() => _reloadLocalViews());
    ScaffoldMessenger.of(_shell.context).showSnackBar(
      SnackBar(content: Text(_shell.context.l10n.cloneDone(result.newName))),
    );
  }

  Future<void> _localReorderViews(
    String? parentViewId,
    List<DocumentView> ordered,
  ) async {
    // Renumbering and reparenting are one move, and the step-of-ten scheme
    // lives with the create/clone paths rather than being spelled out again.
    _local.reorderViews(parentViewId, [for (final v in ordered) v.id]);
    if (_shell.mounted) _shell.setUi(_reloadLocalViews);
  }

  Future<List<DocumentView>> _localLoadTrash() async {
    final wsId = _localSelectedWorkspace?.id;
    return [
      for (final v in _local.listViews())
        if (v.trashed && v.workspaceId == wsId) _viewFromData(v),
    ];
  }

  Future<void> _localRestoreView(DocumentView view) async {
    // Restores the subtree, and lifts the root to the top level when its parent
    // is still trashed — both in Rust now, matching the server's restore_view
    // instead of reimplementing it here.
    _local.restoreViewSubtree(view.id);
    if (_shell.mounted) _shell.setUi(_reloadLocalViews);
  }

  Future<void> _localPurgeView(DocumentView view) async {
    // Permanently remove the page and its subtree from the recycle bin.
    final ids = _local.purgeViewSubtree(view.id).toSet();
    if (!_shell.mounted) return;
    _shell.setUi(() {
      _reloadLocalViews();
      if (_localSelectedView != null && ids.contains(_localSelectedView!.id)) {
        _localSelectedView = null;
        _localBootstrap = null;
      }
    });
  }

  // ── Local search (moved here with the world it queries) ─────────────────

  /// Local-world search: straight to the on-device index, scoped to the open
  /// local workspace — the store also holds the other local workspaces and the
  /// cloud mirrors. Backlinks and the graph take the same [_localWsId].
  String get _localWsId => _localSelectedWorkspace?.id ?? 'local';
  Future<List<SearchResult>> _searchLocalWorkspace(String query) =>
      _local.searchLocal(_localWsId, query);

  /// Open a local search hit.
  ///
  /// Without this, local search finds pages and clicking one does nothing —
  /// which is a worse state than not searching at all: the list proves the page
  /// exists and then refuses to go there.
  Future<void> _openLocalViewById(String viewId) async {
    final view = _localViews.where((v) => v.id == viewId).firstOrNull;
    if (view != null) await _localSelectView(view);
  }

  // The local half of the home/overview open+close pair (the cloud half lives
  // in world_backend.dart's `_CloudWorldBackend`; the two were one
  // `_activeIsLocal`-flagged method before).

  void _openViewFromHome(String viewId) {
    for (final view in _localViews) {
      if (view.id != viewId) continue;
      // A FOLDER is not a document — opening it as one did nothing at all,
      // which is what the home screen's directory list used to do. Show its
      // contents.
      if (view.isFolder) {
        _shell.setUi(() => _shell._overviewFolderId = view.id);
        return;
      }
      unawaited(_localSelectView(view));
      return;
    }
    return;
  }

  void _closeOpenPage() {
    _shell.setUi(() {
      _localSelectedView = null;
      // Home is a STATE, not just a missing selection: the editor pane renders
      // Home only when bootstrap is null. Clearing the view alone left the old
      // page's editor mounted, so clicking Home looked like nothing happened
      // (issue #108).
      _localBootstrap = null;
    });
  }

  /// Import a picked Markdown file as a page in the selected local workspace —
  /// the local counterpart of the cloud `importMarkdown` (issue #108). The
  /// empty Future it used to be silently did nothing; the conversion lives in
  /// the local facade so this library only wires the outcome.
  Future<void> _localImportMarkdown(String fileName, String markdown) async {
    final l10n = _shell.context.l10n;
    final workspace = _localSelectedWorkspace;
    if (workspace == null) return;
    final result = await _local.importMarkdownPage(
      fileName,
      markdown,
      workspace.id,
      fallbackTitle: l10n.untitledPage,
    );
    if (!_shell.mounted) return;
    _shell.setUi(_reloadLocalViews);
    if (result.errors.isNotEmpty) {
      _shell.setUi(
        () => _shell._message = l10n.importFailed(result.errors.first),
      );
      return;
    }
    final id = result.viewIds.firstOrNull;
    final created = id == null
        ? null
        : _localViews.where((v) => v.id == id).firstOrNull;
    if (created != null) await _localSelectView(created);
  }

  /// Export the local workspace as a real Markdown ZIP through the shared Rust
  /// tree builder — the entry that used to return zero bytes and always failed
  /// (issue #108).
  Future<Uint8List> _localExportWorkspaceZip(String workspaceId) async {
    final bytes = _local.exportWorkspaceZip(workspaceId);
    if (bytes == null || bytes.isEmpty) {
      throw ApiException(_shell.context.l10n.exportEmptyContent);
    }
    return bytes;
  }
}
