part of '../main.dart';

/// One world behind the shell: local-offline or a cloud server.
///
/// Both worlds' state is loaded side by side (P3c); `_WorkspaceShellState`
/// picks the ACTIVE one via `_activeBackend` and wires it into the single
/// [WorkspaceView] instantiation — this interface is what replaced the ~80
/// `local ? ... : ...` ternaries that wiring used to carry. Capability rule
/// (P3 §2.3): a world without a feature surfaces `null` rather than a no-op,
/// because a nullable prop is what hides the UI entry while a no-op still
/// renders and still accepts input. Every member documents, or sits next to,
/// the polarity the old ternary had.
///
/// Function bodies are the pre-existing shell/local ones, moved verbatim
/// (mechanical change; see `_LocalWorldBackend` and `_CloudWorldBackend`).
abstract class WorldBackend {
  // ── State (read live from the owning side on every build) ────────────────

  List<Workspace> get workspaces;
  Workspace? get selectedWorkspace;

  /// The page trees keyed by workspace, for the home pane (it spans every
  /// workspace of the ACTIVE world). The local world keeps one flat view list
  /// rather than a per-workspace map, so it contributes just its selected
  /// workspace.
  Map<String, List<DocumentView>> get viewsByWorkspace;

  /// The live page tree of the SELECTED workspace.
  List<DocumentView> get views;

  bool get treePending;
  DocumentView? get selectedView;
  DocumentBootstrap? get bootstrap;
  List<WorkspaceMember> get members;
  List<PresenceUser> get presence;
  List<DocTab> get tabs;
  int get activeTabIndex;
  List<CommentThread> get commentThreads;
  List<CommentHighlight> get commentHighlights;
  SyncPhase? get syncPhase;
  bool get isBusy;
  bool get showAi;

  /// Bumped to force the editor to remount fresh (local rollback/version
  /// restore); the cloud world has no such epoch.
  int get editorEpoch;

  /// The toolbar's refresh entry.
  VoidCallback get onRefresh;

  // ── Grouped operations ───────────────────────────────────────────────────

  WorkspaceOps get workspaceOps;
  ViewOps get viewOps;
  SearchOps get searchOps;
  ImageOps get imageOps;
  ExportOps get exportOps;
  AiOps get aiOps;

  // ── Entries whose polarity is not "cloud-only, so null locally" ─────────

  /// Version history. Both worlds have a real one — local via the on-device
  /// doc_version timeline (FFI), cloud via the yrs endpoints.
  Future<void> Function()? get versionHistory;

  /// Restore the open document to its last on-device checkpoint — LOCAL ONLY
  /// (inverted polarity: null in the cloud world, which hides the menu item).
  Future<void> Function()? get restoreCheckpoint => null;

  // ── Cloud-only entries (null in the local world hides the UI) ────────────

  /// Null in 本地模式, where there is no account to have a picture.
  Future<String?> Function()? get changeAvatar => null;
  Future<String?> Function()? get removeAvatar => null;

  /// Start a comment thread on a range. Null → commenting unavailable (also
  /// gated on a live session at the wiring site).
  Future<void> Function(
    String startBlock,
    int startOffset,
    String endBlock,
    int endOffset,
    String quote,
  )?
  get addComment => null;

  // Tabs are cloud-only; the local world has no tab model (see
  // `_openViewInNewTab`). Null callbacks hide the strip entirely.
  void Function(int index)? get selectTab => null;
  void Function(int index)? get closeTab => null;
  void Function(DocumentView view)? get openViewInNewTab => null;
  void Function(String viewId, String? workspaceId)? get openViewInNewTabById =>
      null;
  VoidCallback? get newTabPage => null;

  // Settings: AI provider, import history, tokens and the account tab are all
  // server-backed — see the WorkspaceView props' own notes on why these are
  // null, not no-ops, in 本地模式.
  Future<Map<String, dynamic>> Function()? get loadAiSettings => null;
  Future<List<ImportHistoryEntry>> Function()? get loadImportHistory => null;
  ListAiModels? get listAiModels => null;
  Future<Map<String, dynamic>> Function({
    required String providerId,
    String? provider,
    String? baseUrl,
    String? model,
    String? apiKey,
  })?
  get saveAiSettings => null;
  Future<Map<String, dynamic>> Function(String providerId)?
  get deleteAiProvider => null;
  Future<List<Map<String, dynamic>>> Function()? get loadTokens => null;
  Future<Map<String, dynamic>> Function(
    String name,
    List<String> scopes,
    int? expiresInDays,
  )?
  get createToken => null;
  Future<void> Function(String id)? get revokeToken => null;
  Future<void> Function(String displayName)? get updateProfile => null;
  Future<void> Function(String current, String next)? get changePassword =>
      null;
  Future<void> Function(String password)? get deleteAccount => null;

  // Whole-account export / quota / membership are cloud-only too.
  Future<void> Function()? get exportAllWorkspaces => null;
  Future<({int workspaces, int pages, int imageBytes})> Function()?
  get exportStats => null;
  Future<void> Function(String fileName, Uint8List bytes, {bool notion})?
  get importWorkspaceZip => null;
  Future<({int used, int quota, int maxUpload})?> Function(Workspace workspace)?
  get loadWorkspaceUsage => null;
  Future<void> Function(WorkspaceMember member, WorkspaceRole role)?
  get updateMember => null;
  Future<void> Function(WorkspaceMember member)? get removeMember => null;

  /// Null, not a no-op: a local workspace has no members at all. The no-op
  /// form still rendered and still accepted an email, because local entries
  /// carry role 'owner' so canManage was true — you could invite someone and
  /// nothing whatsoever happened.
  InvitationsController? get invitations => null;

  /// Opens the share dialog (public link) — cloud-only.
  Future<void> Function()? get share => null;

  /// Cross-workspace move/copy — cloud-only (the destination is another cloud
  /// workspace; local worlds don't participate). Null hides both row-menu
  /// entries in 本地模式.
  Future<void> Function(List<DocumentView> views, bool copy)? get transfer =>
      null;

  /// Caret broadcast as awareness — null in single-user (local) mode.
  void Function(String? blockId, int? offset)? get cursorChanged => null;
}

/// The workspace switcher's create/rename/delete/select bundle.
class WorkspaceOps {
  const WorkspaceOps({
    required this.select,
    required this.create,
    required this.rename,
    required this.delete,
  });

  final Future<void> Function(Workspace workspace) select;
  final Future<void> Function(String name) create;
  final Future<void> Function(Workspace workspace, String name) rename;
  final Future<void> Function(Workspace workspace) delete;
}

/// Page-tree operations: documents, folders, trash and the view-level editor
/// callbacks (block additions the local world accepts as silent no-ops because
/// its editor writes through the local CRDT instead).
class ViewOps {
  const ViewOps({
    required this.createDocument,
    required this.createChildDocument,
    required this.createFolder,
    required this.createChildFolder,
    required this.exportFolderZip,
    required this.reorder,
    required this.loadTrash,
    required this.restore,
    required this.purge,
    required this.purgeAll,
    required this.select,
    required this.rename,
    required this.setIcon,
    required this.delete,
    required this.bulkDelete,
    required this.clone,
    required this.openViewFromHome,
    required this.closeOpenPage,
  });

  final Future<String?> Function(String name) createDocument;
  final Future<String?> Function(DocumentView parent, String name)
  createChildDocument;
  final Future<String?> Function(String name) createFolder;
  final Future<String?> Function(DocumentView parent, String name)
  createChildFolder;

  /// Export a folder's subtree as a ZIP. Null hides the menu entry (see
  /// DocumentListItem) rather than failing after the click.
  final Future<Uint8List> Function(DocumentView folder)? exportFolderZip;
  final Future<void> Function(String? parentViewId, List<DocumentView> ordered)
  reorder;
  final Future<List<DocumentView>> Function() loadTrash;
  final Future<void> Function(DocumentView view) restore;
  final Future<void> Function(DocumentView view) purge;

  /// Empty the whole bin. Cloud only: the on-device store has no bulk purge,
  /// so the button is absent rather than broken in 本地模式.
  final Future<int> Function()? purgeAll;
  final Future<void> Function(DocumentView view) select;
  final Future<bool> Function(DocumentView view, String name) rename;

  /// Cloud-only: the local store's view record has no icon column, so rather
  /// than accept a change it would silently drop, the local world simply does
  /// not offer the entry (see DocumentListItem.onSetIcon).
  final Future<void> Function(DocumentView view)? setIcon;
  final Future<void> Function(DocumentView view) delete;
  final Future<void> Function(List<String> viewIds) bulkDelete;
  final Future<void> Function(DocumentView view) clone;

  /// Open a page the home screen listed. Home carries ids (it spans
  /// workspaces), so the view object is looked up here; a stale id — the page
  /// was deleted between render and tap — is simply ignored rather than
  /// guessed at.
  final void Function(String viewId) openViewFromHome;

  /// Close the open page so home shows again (the sidebar's Home row). Keeps
  /// the workspace selection — home is a view of the world, not a way out of
  /// it.
  final void Function() closeOpenPage;
}

/// Workspace search and hit navigation.
class SearchOps {
  const SearchOps({
    required this.selectWorkspaceById,
    required this.searchAll,
    required this.openHit,
    required this.search,
    required this.openResult,
  });

  /// Cloud only: the local world is a single implicit workspace, so there is
  /// nothing for a name search to disambiguate.
  final void Function(String workspaceId)? selectWorkspaceById;

  /// Cloud only: the local world is a single implicit workspace, so there is
  /// nothing to widen the scope to. Null hides the scope toggle.
  final Future<List<SearchResult>> Function(String query)? searchAll;

  /// Cloud only: open a hit that belongs to another workspace (the local
  /// world's single workspace never needs the switch).
  final void Function(String viewId, String workspaceId)? openHit;

  /// Null when the world has no workspace search (web local mode). See the
  /// local backend's note on why «cannot search» is not «nothing matched».
  final Future<List<SearchResult>> Function(String query)? search;

  /// Open a hit the search dialog returned, within the current workspace.
  final Future<void> Function(String viewId) openResult;
}

/// The editor's image pipeline (upload / re-host / resolve / load bytes).
class ImageOps {
  const ImageOps({
    required this.upload,
    required this.importUrl,
    required this.loadBytes,
    required this.resolveUrls,
  });

  final Future<({String fileId, String name})?> Function(
    Uint8List bytes,
    String fileName,
    String mimeType,
  )
  upload;
  final Future<({String fileId, String name})?> Function(String url) importUrl;
  final Future<Uint8List?> Function(String fileId) loadBytes;
  final Future<Map<String, String>> Function(List<String> fileIds) resolveUrls;
}

/// Page/folder/workspace export and tree import.
class ExportOps {
  const ExportOps({
    required this.page,
    required this.copyMarkdown,
    required this.pageHtml,
    required this.importMarkdown,
    required this.workspaceZip,
    required this.importTree,
  });


  /// Page export chooses `.md` (no assets) vs `.zip` (bundled images) by
  /// content — in BOTH worlds: local goes through the FFI engine + in-house
  /// ZIP writer (see `_localExportPage`), matching the cloud output.
  final Future<({Uint8List bytes, String name, String mime})> Function(
    String title,
  )
  page;
  final Future<String> Function() copyMarkdown;

  /// HTML export works in BOTH worlds — local goes through the FFI engine
  /// (see `_localExportPageHtml`), so unlike the ZIP it isn't gated off local.
  final Future<String> Function(String title) pageHtml;
  final Future<void> Function(String fileName, String markdown) importMarkdown;
  final Future<Uint8List> Function(String workspaceId) workspaceZip;
  final Future<void> Function(Workspace workspace, List<ArchiveFile> entries)
  importTree;
}

/// The AI surface. Settings live on the server, so in 本地模式 the whole tab
/// is hidden (the load/save callbacks stay null) while the in-editor actions
/// degrade to their old local stubs.
class AiOps {
  const AiOps({
    required this.stream,
    required this.newPage,
    required this.previewEdits,
    required this.currentPage,
    required this.newWorkspace,
  });

  final Stream<String> Function(String prompt, {String? system}) stream;
  final Future<void> Function(String markdown) newPage;
  final Future<AiEditPreview> Function(
    String response,
    EditorAiSnapshot source,
  )?
  previewEdits;
  final Future<void> Function(String markdown)? currentPage;
  final Future<void> Function(String markdown) newWorkspace;
}

/// The cloud world's backend: a thin adapter whose state reads the shell's
/// cloud fields live and whose ops are the pre-existing shell methods. Nothing
/// moves out of `_WorkspaceShellState` for this side — the point is that the
/// local and cloud worlds now share one dispatch shape.
class _CloudWorldBackend extends WorldBackend {
  _CloudWorldBackend(this._shell);

  final _WorkspaceShellState _shell;

  // ── State ────────────────────────────────────────────────────────────────

  @override
  List<Workspace> get workspaces => _shell._workspaces;

  @override
  Workspace? get selectedWorkspace => _shell._selectedWorkspace;

  @override
  Map<String, List<DocumentView>> get viewsByWorkspace =>
      _shell._viewsByWorkspace;

  @override
  List<DocumentView> get views => _shell._selectedWorkspace == null
      ? const []
      : _shell._viewsByWorkspace[_shell._selectedWorkspace!.id] ?? const [];

  /// This workspace's tree has never been loaded — [views] being empty says
  /// nothing about whether the workspace has pages.
  @override
  bool get treePending =>
      _shell._selectedWorkspace != null &&
      !_shell._viewsByWorkspace.containsKey(_shell._selectedWorkspace!.id);

  @override
  DocumentView? get selectedView => _shell._selectedView;

  @override
  DocumentBootstrap? get bootstrap => _shell._selectedBootstrap;

  @override
  List<WorkspaceMember> get members => _shell._selectedWorkspace == null
      ? const []
      : _shell._membersByWorkspace[_shell._selectedWorkspace!.id] ?? const [];

  @override
  List<PresenceUser> get presence => _shell._presence;

  @override
  List<DocTab> get tabs => _shell._tabs;

  @override
  int get activeTabIndex => _shell._activeTabIndex;

  @override
  List<CommentThread> get commentThreads => _shell._commentThreads;

  @override
  List<CommentHighlight> get commentHighlights => _shell._commentHighlights;

  /// The current cloud doc's sync phase, driving the quiet header badge. Only
  /// meaningful while a cloud session is live.
  @override
  SyncPhase? get syncPhase =>
      _shell._cloudSession == null ? null : _shell._syncPhase;

  @override
  bool get isBusy => _shell._isBusy;

  @override
  bool get showAi => _shell._aiEnabled && _shell._aiConfigured;

  @override
  int get editorEpoch => 0;

  @override
  VoidCallback get onRefresh => () {
    if (!_shell._isBusy) _shell._refreshWorkspaces();
  };

  // ── Grouped operations ───────────────────────────────────────────────────

  @override
  late final WorkspaceOps workspaceOps = WorkspaceOps(
    select: _shell._selectWorkspace,
    create: _shell._createWorkspace,
    rename: _shell._renameWorkspace,
    delete: _shell._deleteWorkspace,
  );

  @override
  late final ViewOps viewOps = ViewOps(
    createDocument: _shell._createDocument,
    createChildDocument: _shell._createChildDocument,
    createFolder: _shell._createFolder,
    createChildFolder: _shell._createChildFolder,
    exportFolderZip: _shell._exportFolderZip,
    reorder: _shell._reorderViews,
    loadTrash: _shell._loadTrash,
    restore: _shell._restoreView,
    purge: _shell._purgeView,
    purgeAll: _shell._purgeAllTrash,
    select: _shell._selectView,
    rename: _shell._renameView,
    setIcon: _shell._promptSetViewIcon,
    delete: _shell._deleteView,
    bulkDelete: _shell._deleteViews,
    clone: _shell._cloneView,
    openViewFromHome: _openViewFromHome,
    closeOpenPage: _closeOpenPage,
  );

  @override
  late final SearchOps searchOps = SearchOps(
    selectWorkspaceById: _shell._selectWorkspaceById,
    searchAll: _shell._searchAllWorkspaces,
    openHit: (viewId, workspaceId) =>
        unawaited(_shell._openSearchHit(viewId, workspaceId)),
    search: _shell._searchWorkspace,
    openResult: _shell._openViewById,
  );

  @override
  late final ImageOps imageOps = ImageOps(
    upload: _shell._uploadEditorImage,
    importUrl: _shell._importEditorImageUrl,
    loadBytes: _shell._loadEditorImageBytes,
    resolveUrls: _shell._resolveEditorImageUrls,
  );

  @override
  late final ExportOps exportOps = ExportOps(
    page: _shell._exportPage,
    copyMarkdown: _shell._pageMarkdownText,
    pageHtml: _shell._exportPageHtml,
    importMarkdown: _shell._importMarkdownAsPage,
    workspaceZip: _shell._exportWorkspaceZip,
    importTree: _shell._importTreeIntoWorkspace,
  );

  @override
  late final AiOps aiOps = AiOps(
    stream: _shell._aiStream,
    newPage: _shell._aiNewPageFromMarkdown,
    previewEdits: _shell._previewAiEdits,
    currentPage: _shell._aiCurrentFromMarkdown,
    newWorkspace: _shell._aiNewWorkspaceFromMarkdown,
  );

  // ── Polarity exceptions + cloud-only entries ─────────────────────────────

  @override
  Future<void> Function()? get versionHistory => _shell._openVersionHistory;

  @override
  Future<String?> Function()? get changeAvatar => _shell._changeAvatar;

  @override
  Future<String?> Function()? get removeAvatar => _shell._removeAvatar;

  @override
  Future<void> Function(String, int, String, int, String)? get addComment =>
      _shell._addComment;

  @override
  void Function(int index)? get selectTab => _shell._selectTab;

  @override
  void Function(int index)? get closeTab => _shell._closeTab;

  @override
  void Function(DocumentView view)? get openViewInNewTab =>
      _shell._openViewInNewTab;

  @override
  void Function(String viewId, String? workspaceId)? get openViewInNewTabById =>
      _shell._openViewByIdInNewTab;

  @override
  VoidCallback? get newTabPage =>
      () => _shell._createDocumentInNewTab(_shell.context.l10n.newPage);

  @override
  Future<Map<String, dynamic>> Function()? get loadAiSettings =>
      _shell._loadAiSettings;

  @override
  Future<List<ImportHistoryEntry>> Function()? get loadImportHistory =>
      _shell._loadImportHistory;

  @override
  ListAiModels? get listAiModels => _shell._listAiModels;

  @override
  Future<Map<String, dynamic>> Function({
    required String providerId,
    String? provider,
    String? baseUrl,
    String? model,
    String? apiKey,
  })?
  get saveAiSettings => _shell._saveAiSettings;

  @override
  Future<Map<String, dynamic>> Function(String providerId)?
  get deleteAiProvider => _shell._deleteAiProvider;

  @override
  Future<List<Map<String, dynamic>>> Function()? get loadTokens =>
      _shell._loadTokens;

  @override
  Future<Map<String, dynamic>> Function(String, List<String>, int?)?
  get createToken => _shell._createToken;

  @override
  Future<void> Function(String id)? get revokeToken => _shell._revokeToken;

  @override
  Future<void> Function(String displayName)? get updateProfile =>
      _shell._updateProfile;

  @override
  Future<void> Function(String current, String next)? get changePassword =>
      _shell._changePassword;

  @override
  Future<void> Function(String password)? get deleteAccount =>
      _shell._deleteAccount;

  @override
  Future<void> Function()? get exportAllWorkspaces =>
      _shell._exportAllWorkspaces;

  @override
  Future<({int workspaces, int pages, int imageBytes})> Function()?
  get exportStats => _shell._exportAllStats;

  @override
  Future<void> Function(String fileName, Uint8List bytes, {bool notion})?
  get importWorkspaceZip => _shell._importWorkspaceZip;

  @override
  Future<({int used, int quota, int maxUpload})?> Function(Workspace)?
  get loadWorkspaceUsage => _shell._loadWorkspaceUsage;

  @override
  Future<void> Function(WorkspaceMember, WorkspaceRole)? get updateMember =>
      _shell._updateWorkspaceMember;

  @override
  Future<void> Function(WorkspaceMember)? get removeMember =>
      _shell._removeWorkspaceMember;

  @override
  InvitationsController? get invitations => _shell._invites;

  @override
  Future<void> Function()? get share => _shell._openShare;

  @override
  Future<void> Function(List<DocumentView>, bool)? get transfer =>
      _shell._openTransfer;

  @override
  void Function(String? blockId, int? offset)? get cursorChanged =>
      _shell._onEditorSelection;

  // The cloud half of the home/overview open+close pair (the local half lives
  // in local_world_backend.dart; the two were one `_activeIsLocal`-flagged
  // method before).

  void _openViewFromHome(String viewId) {
    // Which workspace the hit came from is what this used to throw away by
    // flattening `.values` — and it is exactly what the open needs.
    final workspaceId = workspaceIdOfView(
      viewsByWorkspace: _shell._viewsByWorkspace,
      viewId: viewId,
    );
    if (workspaceId == null) return;
    for (final view
        in _shell._viewsByWorkspace[workspaceId] ?? const <DocumentView>[]) {
      if (view.id != viewId) continue;
      unawaited(_shell._openAcrossWorkspaces(workspaceId, view));
      return;
    }
  }

  void _closeOpenPage() {
    _shell.setUi(() {
      _shell._selectedView = null;
      _shell._selectedBootstrap = null;
    });
  }
}
