part of 'main.dart';

class WorkspaceView extends StatefulWidget {
  const WorkspaceView({
    required this.apiBase,
    required this.onChangeAvatar,
    required this.onRemoveAvatar,
    this.syncPhase,
    this.commentThreads = const [],
    this.commentHighlights = const [],
    this.onAddComment,
    required this.onReplyComment,
    required this.onSetCommentResolved,
    required this.onDeleteCommentThread,
    required this.onFocusCommentThread,
    required this.session,
    required this.entries,
    required this.activeOrigin,
    required this.selectedRef,
    required this.onSelectEntry,
    required this.onRenameEntry,
    required this.onDeleteEntry,
    required this.onExportEntryZip,
    required this.onImportTreeIntoEntry,
    required this.onImportTreeIntoFolder,
    required this.onReorderWorkspaces,
    required this.cloudOriginLabel,
    required this.onSignIn,
    required this.localAvailable,
    required this.isBusy,
    required this.onRefresh,
    required this.onSignOut,
    required this.workspaces,
    required this.selectedWorkspace,
    required this.members,
    required this.views,
    this.treePending = false,
    required this.selectedView,
    required this.selectedBootstrap,
    this.tabs = const [],
    this.activeTabIndex = 0,
    this.onSelectTab,
    this.onCloseTab,
    this.onOpenInNewTab,
    this.onOpenInNewTabById,
    this.onNewTabPage,
    this.searchRecents = const [],
    this.onSelectWorkspaceById,
    this.onSearchAllWorkspaces,
    this.onOpenSearchHit,
    required this.presence,
    required this.message,
    this.importProgress,
    this.onCancelImport,
    required this.onSelectWorkspace,
    required this.onCreateWorkspace,
    required this.onRenameWorkspace,
    required this.onDeleteWorkspace,
    required this.onCreateDocument,
    required this.onCreateChildDocument,
    required this.onCreateFolder,
    required this.onCreateChildFolder,
    this.onExportFolderZip,
    required this.onReorderViews,
    required this.onLoadTrash,
    required this.onRestoreView,
    required this.onPurgeView,
    this.onPurgeAllTrash,
    required this.onSelectView,
    required this.onRenameView,
    this.onSetViewIcon,
    this.homePane,
    this.overviewPane,
    this.onOpenHome,
    required this.onDeleteView,
    required this.onDeleteViews,
    required this.onCloneView,
    required this.onApplyOperations,
    this.onApplyHistory,
    this.onApplyMaintenanceOperations,
    required this.onUploadImage,
    required this.onImportImageUrl,
    required this.onLoadImageBytes,
    required this.onResolveImageUrls,
    required this.onAiStream,
    required this.onAiNewPage,
    this.onAiPreviewEdits,
    required this.onAiCurrentPage,
    required this.onAiNewWorkspace,
    required this.onLoadAiSettings,
    required this.onLoadImportHistory,
    required this.onListAiModels,
    required this.onSaveAiSettings,
    required this.onDeleteAiProvider,
    this.onLoadTokens,
    this.onCreateToken,
    this.onRevokeToken,
    required this.userName,
    required this.userEmail,
    required this.onUpdateProfile,
    required this.onChangePassword,
    required this.onDeleteAccount,
    required this.onSwitchWorld,
    required this.onRemoveServer,
    required this.appearance,
    required this.pageWidth,
    required this.reHostImages,
    required this.onReHostImagesChanged,
    required this.showFormatBar,
    required this.onShowFormatBarChanged,
    required this.showPageTitle,
    required this.onShowPageTitleChanged,
    required this.showAi,
    required this.aiEnabled,
    required this.onAiEnabledChanged,
    required this.onAppearanceChanged,
    this.onSearch,
    required this.onOpenSearchResult,
    this.onLoadBacklinks,
    this.onLoadGraph,
    required this.onExportPage,
    required this.onCopyPageMarkdown,
    required this.onExportPageHtml,
    required this.onExportPagePdf,
    required this.onImportMarkdown,
    required this.onExportWorkspaceZip,
    this.onExportAllWorkspaces,
    this.onLoadExportStats,
    this.onLoadCacheStats,
    this.onClearMirrorCache,
    this.onLoadQuarantinedCaches,
    this.onExportQuarantinedCache,
    required this.onImportWorkspaceZip,
    required this.onImportWorkspaceTreeInto,
    this.invitations,
    this.onLoadWorkspaceUsage,
    this.onUpdateMember,
    this.onRemoveMember,
    this.onRestoreCheckpoint,
    this.onVersionHistory,
    this.onShare,
    this.onTransfer,
    this.onMigrateEntry,
    this.onDetachEntry,
    this.onCursorChanged,
    this.onEditorFault,
    this.editorEpoch = 0,
    super.key,
  });

  /// The live cloud sync phase for the open doc, or null when it doesn't apply
  /// (local world / no cloud session) — then no badge is drawn.
  final SyncPhase? syncPhase;

  /// Comment threads on the open document (server-resolved anchors).
  final List<CommentThread> commentThreads;

  /// The subset render.dart washes behind the text (unresolved + still anchored).
  final List<CommentHighlight> commentHighlights;

  /// Start a thread on a range. Null → commenting unavailable (local world, or no
  /// session), which also hides the editor's "add comment" entry.
  final Future<void> Function(
    String startBlock,
    int startOffset,
    String endBlock,
    int endOffset,
    String quote,
  )?
  onAddComment;
  final Future<void> Function(String threadId, String body) onReplyComment;
  final Future<void> Function(String threadId, bool resolved)
  onSetCommentResolved;
  final Future<void> Function(String threadId) onDeleteCommentThread;

  /// Tapping a thread's quote in the panel emphasises its wash in the document.
  /// Null id clears the emphasis.
  final void Function(String? threadId) onFocusCommentThread;
  final AuthSession? session;

  /// Where this world's server lives — the base every avatar URL resolves
  /// against. Not derivable down here: the local world and each connected
  /// server have different ones.
  final Uri apiBase;

  /// Null in 本地模式, where there is no account to have a picture. Both report
  /// the avatar URL that holds afterwards, so a route can render the result
  /// without waiting on a rebuild it will never receive.
  final Future<String?> Function()? onChangeAvatar;
  final Future<String?> Function()? onRemoveAvatar;

  /// The unified workspace list (P3c): local + cloud entries, grouped by
  /// origin in the switcher. Row actions dispatch on the ROW's entry.
  final List<WorkspaceEntry> entries;

  /// The one world on screen: `'local'` or a server's origin URL. The account
  /// tile picks it, the switcher shows only its workspaces, and the tile itself
  /// names its identity — one world at a time.
  final String activeOrigin;

  /// Derived, not passed: two props for one fact is how the Settings dialog
  /// ended up rebuilding the active origin out of `activeIsLocal` and
  /// `cloudOrigin` — reconstructing a truth that was sitting right there.
  bool get activeIsLocal => activeOrigin == 'local';

  final WorkspaceRef? selectedRef;
  final Future<void> Function(WorkspaceEntry entry) onSelectEntry;
  final Future<void> Function(WorkspaceEntry entry, String name) onRenameEntry;
  final Future<void> Function(WorkspaceEntry entry) onDeleteEntry;
  final Future<Uint8List> Function(WorkspaceEntry entry) onExportEntryZip;
  final Future<void> Function(
    WorkspaceEntry entry,
    List<ArchiveFile> entries, {
    String? sourceName,
    String? container,
  })
  onImportTreeIntoEntry;

  /// Import loose files / a picked folder UNDER a folder view (parent_view_id).
  final Future<void> Function(
    DocumentView folder,
    List<ArchiveFile> entries, {
    String? sourceName,
    String? container,
  })
  onImportTreeIntoFolder;

  /// Persist a new order for the connected world's workspaces (full list in
  /// the intended order) — see the switcher's move up/down.
  final Future<void> Function(List<WorkspaceEntry> ordered) onReorderWorkspaces;

  /// Display label for the cloud section header ("Mica Cloud" or the host).
  final String cloudOriginLabel;

  /// Non-null when not signed in — the switcher's cloud section shows a
  /// sign-in row that invokes it (desktop: auth is a dialog, not a gate).
  final VoidCallback? onSignIn;

  /// Whether this platform has a local world (desktop true, web false — the
  /// local section and local-workspace creation are hidden without it).
  final bool localAvailable;

  final bool isBusy;
  final VoidCallback onRefresh;
  final VoidCallback onSignOut;
  final List<Workspace> workspaces;
  final Workspace? selectedWorkspace;
  final List<WorkspaceMember> members;
  final List<DocumentView> views;

  /// This workspace's tree has never been loaded — [views] being empty says
  /// nothing about whether the workspace has pages. Distinct from "loaded and
  /// genuinely empty", which is the only case that may claim so on screen.
  final bool treePending;
  final DocumentView? selectedView;
  final DocumentBootstrap? selectedBootstrap;

  /// The open tabs, left to right, and which one is showing.
  ///
  /// [selectedView] / [selectedBootstrap] stay the source of truth for what the
  /// editor renders — these are the STRIP's data, not a second copy of the
  /// selection. Keeping them separate is what lets the local world (which has
  /// no tabs) pass an empty list and get the old single-page shell unchanged.
  final List<DocTab> tabs;
  final int activeTabIndex;

  /// Null in the local world, which has no tabs — the strip hides itself rather
  /// than rendering a row that cannot respond.
  final void Function(int index)? onSelectTab;
  final void Function(int index)? onCloseTab;

  /// Open a page in a new tab, from the sidebar row's context menu.
  final void Function(DocumentView view)? onOpenInNewTab;

  /// Same, by id — the tab strip's `+` picks through search, which yields ids.
  ///
  /// [workspaceId] is where the hit lives, and null means "the one we are in".
  /// Without it a hit from another workspace could only be resolved against the
  /// CURRENT workspace's tree, where it does not exist — which is why the `+`
  /// used to hand those to the switch-and-open path instead of opening a tab.
  final void Function(String viewId, String? workspaceId)? onOpenInNewTabById;

  /// Create a page and open it in a new tab. Null hides the `+` entirely, which
  /// is what the local world gets.
  final VoidCallback? onNewTabPage;

  /// Recently edited pages, for the search dialog's pre-typing list. Built by
  /// the host with the same `buildRecents` the home screen uses.
  final List<HomeDocEntry> searchRecents;

  /// Switch workspaces by id — the search panel's workspace matches.
  ///
  /// By id rather than by [Workspace] because the panel only ever holds the
  /// (id, name) pairs it was given; handing it the models would let it start
  /// reading fields that are the host's to interpret.
  final void Function(String workspaceId)? onSelectWorkspaceById;

  /// The same search across every workspace. Null hides the scope toggle.
  final Future<List<SearchResult>> Function(String query)?
  onSearchAllWorkspaces;

  /// Open a hit that belongs to [workspaceId], switching there first.
  ///
  /// Separate from [onOpenSearchResult] because that one resolves the view out
  /// of the CURRENT workspace's page tree — which a workspace the user has
  /// never visited does not have in memory at all.
  final void Function(String viewId, String workspaceId)? onOpenSearchHit;
  final List<PresenceUser> presence;
  final String? message;

  /// Pages finished / planned for a running import, or null when none is in
  /// flight. Optional so the local world (which imports synchronously) simply
  /// doesn't pass it.
  final ({int done, int total})? importProgress;

  /// Stop the running import. Null when none is running.
  final Future<void> Function()? onCancelImport;
  final Future<void> Function(Workspace workspace) onSelectWorkspace;
  final Future<void> Function(String name) onCreateWorkspace;
  final Future<void> Function(Workspace workspace, String name)
  onRenameWorkspace;
  final Future<void> Function(Workspace workspace) onDeleteWorkspace;
  final Future<String?> Function(String name) onCreateDocument;
  final Future<String?> Function(DocumentView parent, String name)
  onCreateChildDocument;
  final Future<String?> Function(String name) onCreateFolder;
  final Future<String?> Function(DocumentView parent, String name)
  onCreateChildFolder;

  /// Export a folder's subtree as a ZIP. Null (local workspace) hides the entry.
  final Future<Uint8List> Function(DocumentView folder)? onExportFolderZip;
  final Future<void> Function(String? parentViewId, List<DocumentView> ordered)
  onReorderViews;
  final Future<List<DocumentView>> Function() onLoadTrash;
  final Future<void> Function(DocumentView view) onRestoreView;
  final Future<void> Function(DocumentView view) onPurgeView;

  /// Empty the whole bin, returning how many views went. Null where no bulk purge
  /// exists (本地模式).
  final Future<int> Function()? onPurgeAllTrash;
  final Future<void> Function(DocumentView view) onSelectView;
  final Future<bool> Function(DocumentView view, String name) onRenameView;

  /// Opens the emoji picker for a view and persists the choice. Null where icons
  /// cannot be stored (the local world), which hides the menu entry entirely.
  final Future<void> Function(DocumentView view)? onSetViewIcon;

  /// The home pane, shown whenever no page is open. Built by the host (see
  /// `ui/home_pane.dart`) because it spans every workspace, which this view —
  /// scoped to one workspace — does not have.
  final Widget? homePane;

  /// The folder-contents overview, non-null only while a folder is being browsed.
  /// Takes precedence over [homePane] — you asked to look inside something.
  final Widget? overviewPane;

  /// Closes the open page so home shows again. Null hides the sidebar entry.
  final VoidCallback? onOpenHome;
  final Future<void> Function(DocumentView view) onDeleteView;

  /// Trash a whole selection. Separate from [onDeleteView] because the cloud
  /// path is ONE request rather than N, and because the per-view callback
  /// returns the refreshed tree while this one refreshes it itself.
  final Future<void> Function(List<String> viewIds) onDeleteViews;

  final Future<void> Function(DocumentView view) onCloneView;
  final Future<void> Function(List<Map<String, dynamic>> operations)
      onApplyOperations;
  final ApplyHistory? onApplyHistory;
  final ApplyOps? onApplyMaintenanceOperations;
  final Future<({String fileId, String name})?> Function(
    Uint8List bytes,
    String fileName,
    String mimeType,
  )
  onUploadImage;
  final Future<({String fileId, String name})?> Function(String url)
  onImportImageUrl;
  final Future<Uint8List?> Function(String fileId) onLoadImageBytes;
  final Future<Map<String, String>> Function(List<String> fileIds)
  onResolveImageUrls;
  final Stream<String> Function(String prompt, {String? system}) onAiStream;
  final Future<void> Function(String markdown) onAiNewPage;
  final Future<AiEditPreview> Function(
    String response,
    EditorAiSnapshot source,
  )?
  onAiPreviewEdits;
  final Future<void> Function(String markdown)? onAiCurrentPage;
  final Future<void> Function(String markdown) onAiNewWorkspace;

  /// Null in 本地模式 — AI settings live on the server, so there is no provider
  /// to configure and Settings drops the tab. Same rule as [onLoadTokens].
  final Future<Map<String, dynamic>> Function()? onLoadAiSettings;

  /// Past imports for the settings screen. Null in 本地模式.
  final Future<List<ImportHistoryEntry>> Function()? onLoadImportHistory;

  /// Fetch the provider's model list. Null in 本地模式, like the settings pair.
  final ListAiModels? onListAiModels;
  final Future<Map<String, dynamic>> Function({
    required String providerId,
    String? provider,
    String? baseUrl,
    String? model,
    String? apiKey,
  })?
  onSaveAiSettings;

  /// Forget one provider entirely. Null in 本地模式, like the pair above.
  final Future<Map<String, dynamic>> Function(String providerId)?
  onDeleteAiProvider;
  final Future<List<Map<String, dynamic>>> Function()? onLoadTokens;
  final Future<Map<String, dynamic>> Function(
    String name,
    List<String> scopes,
    int? expiresInDays,
  )?
  onCreateToken;
  final Future<void> Function(String id)? onRevokeToken;
  final String userName;
  final String userEmail;

  /// Null in 本地模式 — no account there, so Settings drops the Account tab
  /// entirely rather than showing controls that do nothing.
  final Future<void> Function(String displayName)? onUpdateProfile;
  final Future<void> Function(String current, String next)? onChangePassword;
  final Future<void> Function(String password)? onDeleteAccount;

  /// `['local', ...servers]` — this device and every configured server, one
  /// list because they are the same kind of choice: which single world the app
  /// Open the world picker (the sign-in route's pane: pick a server or
  /// 本地模式, add one, remove one). Not named `onSignIn` because it is
  /// reachable while signed in — switching worlds is the point, and
  /// signing in is only what happens if the world you pick wants it.
  final VoidCallback onSwitchWorld;

  /// Confirm-and-remove, owned by the shell. (Adding is no longer offered from
  /// here — the world picker this opens does it, so the menu does not carry a
  /// second copy of that flow.)
  final Future<void> Function(String origin) onRemoveServer;
  final EditorAppearance appearance;
  final double pageWidth;
  final bool reHostImages;
  final void Function(bool value) onReHostImagesChanged;
  final bool showFormatBar;
  final void Function(bool value) onShowFormatBarChanged;
  final bool showPageTitle;
  final void Function(bool value) onShowPageTitleChanged;
  final bool showAi;
  final bool aiEnabled;
  final void Function(bool value) onAiEnabledChanged;
  final void Function(EditorAppearance appearance, double pageWidth)
  onAppearanceChanged;

  /// Null when the active world has no workspace search (本地模式). See the
  /// honest empty state in `_SearchDialog._buildResults`.
  final Future<List<SearchResult>> Function(String query)? onSearch;
  final Future<void> Function(String viewId) onOpenSearchResult;

  /// Load the cloud pages that link TO a view (reverse references), for the
  /// backlinks panel under the editor. Null in 本地模式 — the local world has no
  /// backlinks endpoint, so the panel is hidden there entirely.
  final Future<List<Backlink>> Function(String viewId)? onLoadBacklinks;

  /// The workspace's page-link graph, for the graph view. Null only while there
  /// is no workspace to ask about.
  final Future<PageGraph> Function()? onLoadGraph;
  final Future<({Uint8List bytes, String name, String mime})> Function(
    String title,
  )
  onExportPage;

  /// The open page as Markdown TEXT, for the clipboard. Distinct from
  /// [onExportPage], which packages images into a ZIP — see `_pageMarkdownText`.
  final Future<String> Function() onCopyPageMarkdown;
  final Future<String> Function(String title) onExportPageHtml;
  final Future<Uint8List?> Function(String html) onExportPagePdf;
  final Future<void> Function(String fileName, String markdown)
  onImportMarkdown;
  final Future<Uint8List> Function(String workspaceId) onExportWorkspaceZip;

  /// Null in 本地模式 — cloud-only "export every workspace this account owns"
  /// (`GET /api/workspaces/export.zip`). Null hides the Settings button.
  final Future<void> Function()? onExportAllWorkspaces;

  /// Counts describing the whole-account export. Null in 本地模式.
  final Future<({int workspaces, int pages, int imageBytes})> Function()?
  onLoadExportStats;

  /// What the on-device store holds. Null on web.
  final Future<LocalCacheStats> Function()? onLoadCacheStats;

  /// Drop the re-downloadable half of it; reports what is left and what was
  /// retained (un-pushed edits, quarantined legacy caches).
  final Future<ClearMirrorResult> Function()? onClearMirrorCache;

  final Future<List<({String origin, String workspaceId, int pages})>> Function()?
  onLoadQuarantinedCaches;

  final Future<Uint8List?> Function(String origin, String workspaceId)?
  onExportQuarantinedCache;

  /// Null where importing an archive is not supported — currently local
  /// (offline) mode, whose importer takes a directory of files and has no zip
  /// reader behind it.
  ///
  /// NULL, not a no-op. It used to be `(_, _, {notion}) async {}`, so local
  /// mode kept offering "import a workspace archive" and did nothing at all
  /// when you used it. That is the shape the member form was fixed for two doors
  /// down, and the note there already says why: a no-op still renders, still
  /// accepts the input, and leaves the user believing something happened.
  final Future<void> Function(String fileName, Uint8List bytes, {bool notion})?
  onImportWorkspaceZip;
  final Future<void> Function(Workspace workspace, List<ArchiveFile> entries)
  onImportWorkspaceTreeInto;

  /// Null in 本地模式, where membership does not exist (as are the two member
  /// callbacks below). Absent rather than inert: see the member section in
  /// [_WorkspaceViewState._openWorkspaceSettingsDialog]. Inviting goes here.
  final InvitationsController? invitations;

  /// Storage this workspace occupies + the limit in force. Null in 本地模式:
  /// there is no server to ask and no quota to be near.
  final Future<({int used, int quota, int maxUpload})?> Function(
    Workspace workspace,
  )?
  onLoadWorkspaceUsage;
  final Future<void> Function(WorkspaceMember member, WorkspaceRole role)?
  onUpdateMember;
  final Future<void> Function(WorkspaceMember member)? onRemoveMember;

  /// Restore the open document to its last on-device checkpoint (local mode
  /// only — null elsewhere, which hides the menu item).
  final Future<void> Function()? onRestoreCheckpoint;

  /// Opens the cloud document's version history (null for local workspaces,
  /// which have no server-side history).
  final Future<void> Function()? onVersionHistory;

  /// Opens the share dialog (public link) — cloud-only.
  final Future<void> Function()? onShare;

  /// Move (`copy: false`) or copy (`copy: true`) a sidebar row's subtree into
  /// another cloud workspace. Null for local workspaces — they have no
  /// cross-workspace transfer, so the row menu drops both entries.
  final Future<void> Function(List<DocumentView> views, bool copy)? onTransfer;

  /// Upload a LOCAL workspace row to the cloud (P3f §6.1). Null on web, which
  /// hides the row action.
  final Future<void> Function(WorkspaceEntry entry)? onMigrateEntry;

  /// Detach a CLOUD workspace row into an independent local copy (P3f §6.2).
  /// Null on web, which hides the row action.
  final Future<void> Function(WorkspaceEntry entry)? onDetachEntry;

  /// Bumped to force the editor to remount fresh (e.g. after a rollback, so the
  /// restored content fully replaces the in-memory doc instead of reconciling).
  final int editorEpoch;

  /// Local caret moved (block id + offset) — broadcast as awareness. Null in
  /// single-user (local) mode.
  final void Function(String? blockId, int? offset)? onCursorChanged;

  /// The editor's op pipeline failed to commit a batch (e.g. the durable outbox
  /// write threw). `count` is the running total. Surfaced by the host instead of
  /// being swallowed, so a dropped edit is visible (red line #1).
  final void Function(Object error, int count)? onEditorFault;

  @override
  State<WorkspaceView> createState() => _WorkspaceViewState();
}

class _WorkspaceViewState extends MicaState<WorkspaceView> {
  final _rename = TextEditingController();
  final _pageTitle = TextEditingController();
  final FocusNode _editorFocus = FocusNode(debugLabel: 'MicaEditorBody');
  final FocusNode _pageTitleFocus = FocusNode(debugLabel: 'PageTitle');
  Timer? _pageTitleSaveTimer;

  // An edit stays pending until the document acknowledges its normalized title.
  // The revision prevents a failed older save from releasing newer input.
  bool _pageTitleEditPending = false;
  int _pageTitleEditRevision = 0;
  // How many backlinks the panel below the editor is showing, and which page
  // that answer is about — a count carried over from the previous page would
  // size THIS page's canvas wrongly for one frame. See [_editorAppearance].
  String? _backlinkCountOwner;
  int _backlinkCount = 0;

  /// True once the backlinks panel below the editor has something to draw.
  bool get _hasBacklinksBelow =>
      _backlinkCount > 0 &&
      _backlinkCountOwner != null &&
      _backlinkCountOwner == widget.selectedView?.id;

  /// The canvas claims trailing space on two counts — a
  /// [EditorTheme.minSurfaceHeight] floor so a blank page offers a big
  /// click-to-write area, and [EditorTheme.bottomPad] below the last block so
  /// the caret never types against the bottom edge. Both sit BELOW the text, so
  /// the backlinks panel stacked under the canvas was pushed down by all of it:
  /// a short page drew its text, ~420px of nothing, then the panel stranded
  /// mid-window; a long one still put ~124px of blank between the last line and
  /// the panel, with nothing in it to say the page had ended.
  ///
  /// So the page trims both when it has a panel to show. Obsidian (48px),
  /// Logseq (44) and Roam (36) all land in the same band there, and the panel
  /// draws a hairline of its own — none of the four anchors a link section with
  /// whitespace alone. [_BacklinksPanel] adds the remaining 16.
  EditorAppearance get _editorAppearance => _hasBacklinksBelow
      ? EditorAppearance(
          fontScale: widget.appearance.fontScale,
          fontFamily: widget.appearance.fontFamily,
          tokens: widget.appearance.tokens,
          minSurfaceHeight: 0,
          bottomPad: 24,
        )
      : widget.appearance;

  void _onBacklinksLoaded(String viewId, int count) {
    if (!mounted) return;
    if (_backlinkCountOwner == viewId && _backlinkCount == count) return;
    setState(() {
      _backlinkCountOwner = viewId;
      _backlinkCount = count;
    });
  }

  // Page properties are hidden by default (AFFiNE-style): revealed by the info
  // toggle next to the breadcrumb, so a page with many properties never pushes
  // the body down until you ask for it.
  bool _showProperties = false;

  /// Which tab the right sidebar is showing.
  ///
  /// One sidebar, not two: the outline and the comments used to be separate
  /// surfaces that could both be open, and together they left the text column
  /// about 300px on a normal window.
  _ToolsTab _toolsTab = _ToolsTab.outline;

  /// Live words·chars, published by the editor.
  ///
  /// Held here because the readout is pinned to the PANE: it used to sit at the
  /// end of the CONTENT, which on a short page is the middle of the screen —
  /// a lone floating chip with nothing around it.
  final ValueNotifier<DocCounts> _counts = ValueNotifier(DocCounts.zero);

  /// Open the comments panel for this document.
  ///
  /// Wrapped in a StatefulBuilder so a reply/resolve/delete inside the dialog
  /// re-reads `widget.commentThreads` after the host refetches — the server stays
  /// the single source of truth for anchors and orphan status, and the panel just
  /// shows whatever it last returned.

  // Persisted per-workspace: which nodes are EXPANDED. Absent = collapsed (the
  // default). The tree opens collapsed and remembers what the user expanded;
  // navigating to / creating a nested page reveals its ancestors.
  Set<String> get _expandedViewIds => _tree.expandedIds;
  final PageTreeCache _pageTreeCache = PageTreeCache();

  /// The sidebar tree's expand/focus/pending-reveal state machine — see
  /// ui/sidebar_tree_controller.dart. The legacy names around it
  /// (_expandedViewIds, _focusedNavId, _rootFocused, _pendingRevealId) are
  /// plain passthrough getters, so build helpers read them unchanged and the
  /// listener only triggers a rebuild; every mutation goes through the
  /// controller.
  late final SidebarTreeController _tree;
  // The view whose sidebar name is in inline-rename (edit) mode — null = none.
  // Set right after creating a page/folder (so the user types its name
  // immediately, no dialog) or from the row's "重命名" action; the matching row
  // renders a focused TextField instead of the name Text.
  /// Inline-rename (create-then-name) state — see
  /// ui/inline_rename_controller.dart. [_renamingViewId] is a passthrough
  /// getter onto it.
  late final InlineRenameController _inlineRename;
  String? get _renamingViewId => _inlineRename.renamingId;
  // The sidebar node the user last "located" — a page OR a folder (folders never
  // reach `selectedView`, so this is the only handle on a focused folder). Drives
  // where the top New-page/New-folder buttons create: inside a focused folder, or
  // beside a focused page. Kept in sync with the open doc in didUpdateWidget so a
  // single highlight follows the last interaction. Null = create at the root.
  String? get _focusedNavId => _tree.focusedNavId;
  // The user explicitly "located" the root by tapping the tree's blank area:
  // clears the located node (and its row highlight) even while a doc is open,
  // so the top New-page/New-folder buttons create at the workspace root. Any
  // row tap / opened doc turns this back off. Distinct from `_focusedNavId ==
  // null`, which merely falls back to the open doc.
  bool get _rootFocused => _tree.rootFocused;
  // Ctrl/Shift multi-selection. EMPTY is the ordinary state — this is a second,
  // explicit notion of "chosen", deliberately separate from `_focusedNavId`
  // (which is "where new pages get created" and is never empty for long).
  // Conflating them would make every ordinary click a one-item batch.
  //
  // A single id in here still is not a batch: the batch menu and the multi-drag
  // both go through `actsOnWholeSelection`, which requires two or more.
  //
  // A [TreeSelection] rather than a raw Set here, so the accumulation rules sit
  // somewhere a test can reach — see its doc comment for what went wrong when
  // they lived inline in this State.
  final TreeSelection _selection = TreeSelection();
  // True only while a page is being dragged in the tree. The drop zones overlay
  // each row, so they are mounted only during a drag — otherwise they would
  // intercept ordinary taps on the page rows.
  bool _draggingTree = false;
  // Auto-scroll while dragging near the tree's top/bottom edge, so a long tree
  // can be reordered past what fits on screen (a drag that reaches the edge
  // otherwise stalls — you cannot drop below the last visible row). The list
  // owns its own controller; the key gets its viewport RenderBox to measure how
  // close the pointer is to an edge.
  final ScrollController _treeScroll = ScrollController();

  // Pointer is over the navigation sidebar — reveals the tree's expand
  // toggles (AppFlowy-style: they live in their own slim column, opacity 0
  // at rest so the page icons keep one aligned column).
  bool _navHovered = false;
  bool _toolsExpanded = false;
  bool _navCollapsed = false;
  // Narrow shell only: the sidebar is an overlay, and this is whether it is up.
  // Never true on a roomy shell — there the sidebar is resident and `_navCollapsed`
  // is the control that matters.
  bool _navDrawerOpen = false;
  // Pane widths, drag-resizable via the splitters (long page names need room).
  double _navWidth = 280;
  double _toolsWidth = 300;
  // Once the user drags the nav splitter we stop auto-fitting and honor their
  // width (persisted). Until then, the sidebar fits itself to the longest page
  // name on each workspace switch (min/max clamped) — the "auto width" ask.
  bool _navWidthManual = false;
  static const double _navWidthMin = 220;
  static const double _navWidthMax = 480;

  /// Below this the shell stops being a Row of resident panes and the sidebar
  /// becomes an overlay (see [_narrowShell]).
  ///
  /// 768 is not taste — it is where AppFlowy and AFFiNE independently landed for
  /// the same transition (`PageBreaks.tabletPortrait = 768` gates AppFlowy's
  /// `menuIsDrawer`; AFFiNE's `useResponsiveSidebar` takes `floatThreshold =
  /// 768`). Two products converging on one number from different stacks is worth
  /// more than a number of our own, and it sits right where our sidebar's floor
  /// (220) plus a readable text column stop fitting side by side.
  ///
  /// Note both of them ALSO auto-hide a resident sidebar at a wider tier
  /// (AppFlowy 1024, AFFiNE 540). We deliberately don't: that tier has to be
  /// edge-triggered and remember whether the user collapsed it by hand
  /// (AppFlowy carries a `hasColappsedMenuManually` flag for exactly this), or
  /// it fights the user as they drag. This threshold needs no such state — under
  /// it the sidebar simply is not a pane, which is a pure function of width.
  static const double kNarrowShellWidth = 768;
  final EditorScrollHook _scrollHook = EditorScrollHook();
  final GlobalKey _editorSurfaceKey = GlobalKey();
  final EditorCommandHook _commandHook = EditorCommandHook();
  // Bound once so initState/dispose register and clear the *same* reference in
  // the app-exit flush slot (a fresh `_commandHook.flush` tear-off each access
  // would never compare equal under `identical`).
  late final Future<void> Function() _boundExitFlush = _commandHook.flush;
  // Live document outline (TOC). The editor republishes headings on every edit;
  // the outline panel listens, so it tracks typing instead of only navigation.
  final EditorOutlineHook _outlineHook = EditorOutlineHook();
  // In-page find (Ctrl+F). The editor owns the find bar; the app-level shortcut
  // opens it through this hook even when focus isn't in the editor.
  final EditorFindHook _findHook = EditorFindHook();
  // Focused block's kind/level → highlights the current block type in the
  // (optional) format toolbar.
  final EditorActiveBlockHook _activeBlockHook = EditorActiveBlockHook();

  /// The level the active `after` drop would land at, or null when the pointer
  /// is not over a zone that can re-parent. Tracked in state because the
  /// indicator has to redraw as the pointer moves sideways.
  int? _dropLevel;

  /// Land on a FOLDER that came back from search.
  ///
  /// A folder is not openable — `_selectView` refuses them outright — so a
  /// folder hit routed through the page path did nothing at all, silently. What
  /// "take me to that folder" means here is exactly what tapping it in the tree
  /// means: focus it (which also makes it the target of the New page / New
  /// folder buttons) and open it up.
  ///
  /// Reuses `_focusedNavId` rather than inventing a second notion of "the
  /// current folder" — two of those would drift, and the tree already has one.
  /// The row a reveal is still trying to reach, or null when none is pending.
  String? get _pendingRevealId => _tree.pendingRevealId;

  @override
  void initState() {
    super.initState();
    _inlineRename = InlineRenameController()..addListener(_syncRenameState);
    _tree = SidebarTreeController();
    // Backfill from the initial selection so the very first frame shows the page
    // name (and workspace rename field) instead of an empty "Untitled" hint —
    // didUpdateWidget only fires on later changes, so without this the title
    // looks blank until the next page switch.
    // pageTitle, like didUpdateWidget below — the document is the authority.
    final name = widget.selectedBootstrap?.pageTitle ?? '';
    // Show the placeholder (not solid text) for an untitled page.
    _pageTitle.text = isUntitledPageName(name) ? '' : name;
    final workspace = widget.selectedWorkspace;
    if (workspace != null) _rename.text = workspace.name;
    // The tree opens collapsed; only the open page's ancestors are revealed.
    final sel = widget.selectedBootstrap?.view.id;
    if (sel != null) _tree.revealAncestors(widget.views, sel);
    _tree.addListener(_syncTreeState);
    // Restore a manually-set sidebar width; otherwise fit to the first tree.
    _navWidthManual = loadPref('navWidthManual') == 'true';
    if (_navWidthManual) {
      final saved = double.tryParse(loadPref('navWidth') ?? '');
      if (saved != null) {
        _navWidth = saved.clamp(_navWidthMin, _navWidthMax);
      }
    } else {
      _fitNavWidthToContent();
    }
    _seedOutline();
    // Expose this editor's debounce flush to the app-exit path so quitting right
    // after typing commits the last <=400ms instead of dropping it.
    _activeEditorFlush = _boundExitFlush;
    // GLOBAL, not a Shortcuts binding — see _escKeyEvent for why nothing
    // focus-based can ever see this key.
    HardwareKeyboard.instance.addHandler(_escKeyEvent);
  }

  /// Esc clears the sidebar multi-selection.
  ///
  /// Registered on [HardwareKeyboard] because no focus-based binding can see
  /// the key: three were built and all three were dead, measured in the running
  /// app (the probe story lives in `docs/features/shortcuts.md`). Focus sits on the page
  /// route's own scope — an ancestor of this whole shell — and Flutter
  /// dispatches keys from the focused node UPWARD, so `Shortcuts` anywhere
  /// inside the shell is never on the path. Hardware handlers run regardless of
  /// focus, which is the property everything else lacked.
  ///
  /// ALWAYS returns false: this observes, exactly like the pointer [Listener]
  /// in [build]. Consuming would steal Esc from whoever holds it below (find
  /// bar, slash menu); the guards for the two owners where even observing
  /// would be wrong (dialog on top, inline rename) live in
  /// [escClearsTreeSelection], where a test can reach them.
  bool _escKeyEvent(KeyEvent event) {
    if (event is! KeyDownEvent ||
        event.logicalKey != LogicalKeyboardKey.escape) {
      return false;
    }
    if (!mounted) return false;
    if (escClearsTreeSelection(
      hasSelection: !_selection.isEmpty,
      routeIsCurrent: ModalRoute.of(context)?.isCurrent ?? true,
      renamingInline: _renamingViewId != null,
    )) {
      _clearTreeSelection();
    }
    return false;
  }

  /// Seed the live outline from the current page's bootstrap snapshot so the
  /// headings show the instant a page opens — before the editor mounts and
  /// starts republishing them live. [EditorOutlineHook] change-dedupes, so the
  /// editor's first live publish (identical data) is a no-op.
  void _seedOutline() {
    final blocks = widget.selectedBootstrap?.childBlocks ?? const [];
    _outlineHook.publish([
      for (final b in blocks)
        if (b.kind == 'heading')
          OutlineEntry(id: b.id, text: b.text, level: _headingLevel(b)),
    ]);
  }

  /// Auto-size the sidebar to the widest visible page name (+ the row's fixed
  /// chrome), clamped to [_navWidthMin, _navWidthMax]. Runs on a workspace
  /// switch, not continuously, so the width doesn't jump as you scroll/expand.
  /// A no-op once the user has taken manual control of the width.
  void _fitNavWidthToContent() {
    if (_navWidthManual) return;
    final items = _visibleDocumentTree();
    if (items.isEmpty) return;
    var widest = 0.0;
    for (final it in items) {
      final tp = TextPainter(
        text: TextSpan(
          text: it.view.name,
          style: const TextStyle(fontSize: 14, fontWeight: FontWeight.w600),
        ),
        textDirection: TextDirection.ltr,
        maxLines: 1,
      )..layout();
      // left pad 2 + indent + toggle 18 + icon 18 + gap 6 + text + right pad 4
      // + a little slack so the glyphs never kiss the edge.
      final rowWidth = 2 + it.depth * 16 + 18 + 18 + 6 + tp.width + 4 + 14;
      tp.dispose();
      if (rowWidth > widest) widest = rowWidth;
      if (widest >= _navWidthMax) break;
    }
    _navWidth = widest.clamp(_navWidthMin, _navWidthMax);
  }

  @override
  void didUpdateWidget(covariant WorkspaceView oldWidget) {
    super.didUpdateWidget(oldWidget);
    // Keep the sidebar's "located" node in step with the open doc: opening a
    // page (link / auto-open / navigate) refocuses it, so the single row
    // highlight follows and a stale folder focus doesn't linger.
    if (widget.selectedView?.id != oldWidget.selectedView?.id) {
      _tree.focusView(widget.selectedView?.id); // opening a doc re-locates it
    }
    final selected = widget.selectedWorkspace;
    final wsChanged =
        selected != null && selected.id != oldWidget.selectedWorkspace?.id;
    if (wsChanged) {
      _rename.text = selected.name;
      // Enter a workspace collapsed. Expansion is not carried across a switch:
      // it only ever grew (nothing removed an entry but a manual collapse or a
      // deleted node), so a workspace you had used for a while came back with
      // most of its folders open and a screen-long tree.
      //
      // The page that opens is still remembered, so its ancestors are revealed
      // here — without that the highlighted row does not exist in the tree and
      // "where am I" has no answer. The reveal below covers the usual case
      // (bootstrap/views arrive with or after the switch); this one covers a
      // switch seen AFTER them, where nothing else would fire again. Both are
      // no-ops when the id is not in `views`.
      _tree.clearExpansion();
      final open = widget.selectedBootstrap?.view.id;
      if (open != null) _tree.revealAncestors(widget.views, open);
    }
    // Re-fit the sidebar to content when the tree could have changed shape:
    // switched workspace, added/removed a page, or renamed the open page.
    // (No-op once the width is user-controlled.)
    if (_pageTreeCache.updateViews(widget.views) || wsChanged) {
      _fitNavWidthToContent();
    }

    final bootstrap = widget.selectedBootstrap;
    final idChanged =
        bootstrap?.view.id != oldWidget.selectedBootstrap?.view.id;
    if (idChanged) {
      // Flush the departing page before replacing its field. A timer that
      // reads widget later would see the newly selected page instead.
      if (_pageTitleSaveTimer?.isActive ?? false) {
        final oldBootstrap = oldWidget.selectedBootstrap;
        final title = renamedTo(_pageTitle.text, oldBootstrap?.pageTitle ?? '');
        _pageTitleSaveTimer?.cancel();
        if (oldBootstrap != null && title != null) {
          WidgetsBinding.instance.addPostFrameCallback((_) {
            unawaited(oldWidget.onRenameView(oldBootstrap.view, title));
          });
        }
      }
      _pageTitleEditPending = false;
      _pageTitleEditRevision++;
    }
    // New page open → reseed the outline from its snapshot immediately (the new
    // editor's live publish is one frame away; this avoids a stale-headings flash).
    if (idChanged) _seedOutline();
    // `pageTitle`, not `view.name`: the document is the authority and the column
    // is its projection, so a rename made on ANOTHER device reaches this client
    // as a yrs update long before it would refetch the tree.
    //
    // The COMPARISON has to move with the display. Showing `pageTitle` while
    // still comparing `view.name` would leave the field stale in exactly the
    // case this fixes — the document changed and the column had not caught up.
    if (idChanged ||
        bootstrap?.pageTitle != oldWidget.selectedBootstrap?.pageTitle) {
      // An untitled page renders empty so its placeholder shows instead of
      // solid text.
      final name = bootstrap?.pageTitle ?? '';
      final display = isUntitledPageName(name) ? '' : name;
      // Who wins is [titleFieldSync]'s call, not this widget's: an incoming
      // title is as likely to be the echo of our own debounced save (arriving
      // mid-word, half a title behind the field) as it is a rename from
      // somewhere else. Assigning unconditionally ate the rest of what the
      // user was typing.
      switch (titleFieldSync(
        field: _pageTitle.text,
        documentTitle: name,
        displayTitle: display,
        pageChanged: idChanged,
        // A live IME composition counts as pending even if no keystroke has
        // committed yet: assigning .text mid-composition drops the 拼音 in
        // flight, which is input loss of its own.
        editPending:
            _pageTitleEditPending || _pageTitle.value.composing.isValid,
      )) {
        case TitleFieldSync.keepField:
          break;
        case TitleFieldSync.settled:
          _pageTitleEditPending = false;
        case TitleFieldSync.takeDocument:
          _pageTitle.text = display;
          _pageTitleEditPending = false;
      }
    }
    // Opening a fresh/untitled page with the title shown: land the caret in the
    // title so you can name it right away (instead of on the body), matching the
    // "Untitled is a placeholder" model.
    if (idChanged &&
        widget.showPageTitle &&
        bootstrap != null &&
        bootstrap.view.objectType == 'document' &&
        isUntitledPageName(bootstrap.pageTitle)) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!mounted) return;
        _pageTitleFocus.requestFocus();
        _pageTitle.selection = const TextSelection.collapsed(offset: 0);
      });
    }
    // Reveal the opened page in the sidebar: expand its ancestor chain so a
    // nested selection isn't hidden under collapsed parents. Also fire when the
    // view set first populates — on a cold start the selection can be set
    // before widget.views arrives, so the initState reveal would have walked an
    // empty tree.
    final viewsChanged = widget.views.length != oldWidget.views.length;
    if ((idChanged || viewsChanged) && bootstrap != null) {
      _tree.revealAncestors(widget.views, bootstrap.view.id);
    }
  }

  @override
  void dispose() {
    // Exit-flush the debounced page-title save: a plain `cancel` below would
    // silently drop the last typed name (the same reason didUpdateWidget
    // flushes before replacing the field).
    if (_pageTitleSaveTimer?.isActive ?? false) {
      _pageTitleSaveTimer!.cancel();
      final bootstrap = widget.selectedBootstrap;
      final title = renamedTo(_pageTitle.text, bootstrap?.pageTitle ?? '');
      if (bootstrap != null && title != null) {
        unawaited(widget.onRenameView(bootstrap.view, title));
      }
    }
    _tree.removeListener(_syncTreeState);
    _tree.dispose();
    _inlineRename.removeListener(_syncRenameState);
    _inlineRename.dispose();
    HardwareKeyboard.instance.removeHandler(_escKeyEvent);
    _rename.dispose();
    _pageTitle.dispose();
    _editorFocus.dispose();
    _pageTitleFocus.dispose();
    _pageTitleSaveTimer?.cancel();
    _outlineHook.dispose();
    _activeBlockHook.dispose();
    _treeScroll.dispose();
    // Only clear the exit-flush slot if it still points at ours (a remounting
    // sibling may have already claimed it).
    if (identical(_activeEditorFlush, _boundExitFlush)) {
      _activeEditorFlush = null;
    }
    super.dispose();
  }

  void _syncRenameState() {
    if (!mounted) return;
    setState(() {});
  }

  /// The tree controller changed: rebuild — the field names above are
  /// passthrough getters, so there is nothing to copy.
  void _syncTreeState() {
    if (!mounted) return;
    setState(() {});
  }

  @override
  Widget build(BuildContext context) {
    // P3c: no sign-in gate — a null session just means no cloud account is
    // attached; local workspaces work regardless, and the switcher's cloud
    // section offers sign-in.
    //
    // LayoutBuilder, not MediaQuery: the shell reacts to the space it is
    // actually given, so a widget test can drive the breakpoint by sizing the
    // host — and a desktop window dragged narrow crosses it just like a phone.
    return CallbackShortcuts(
      bindings: _appShortcuts(),
      // Clicking OUTSIDE the sidebar releases the located row AND the
      // multi-selection — the other half of the rule whose first half is the
      // blank-space tap inside it. Moving the mouse away is not enough: you
      // have to act somewhere else. (The selection half was missing at first
      // and user-reported, 2026-08-27: Ctrl-click a few rows, click into the
      // editor, and the rows stayed lit — a selection that survives your
      // leaving is a batch waiting to act on rows you stopped thinking about.
      // Explorer, the feature's stated reference, drops its selection on any
      // outside click.)
      //
      // A Listener, not a GestureDetector: this must not enter the gesture
      // arena at all. Competing there would put it up against every tap in the
      // editor — including text selection and drags — and losing or winning
      // either one would change behaviour that has nothing to do with the
      // sidebar. onPointerDown only observes.
      //
      // Dialogs and context menus live in the app Overlay, which is not inside
      // this subtree — their clicks never reach this Listener, so an open batch
      // menu keeps its selection while you pick from it.
      //
      // `_navHovered` is the test for "inside", reusing the MouseRegion that is
      // already tracking exactly that rather than measuring the pane's rect a
      // second time and letting the two disagree.
      child: Listener(
        behavior: HitTestBehavior.translucent,
        onPointerDown: (_) {
          if (_navHovered) return;
          _tree.focusRoot();
          _clearTreeSelection();
        },
        child: LayoutBuilder(
          builder: (context, c) => c.maxWidth < kNarrowShellWidth
              ? _narrowShell(context, c.maxWidth)
              : _wideShell(context),
        ),
      ),
    );
  }

  /// Roomy: the classic three-pane split, every pane resident at once.
  Widget _wideShell(BuildContext context) {
    return Row(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (_navCollapsed)
          _collapsedNavRail(context)
        else ...[
          SizedBox(width: _navWidth, child: _navigationPane(context)),
          _resizeHandle(
            onDrag: (dx) => setState(() {
              // Dragging takes manual control — stop auto-fitting from here on.
              _navWidthManual = true;
              _navWidth = (_navWidth + dx).clamp(_navWidthMin, _navWidthMax);
            }),
            onDragEnd: () {
              savePref('navWidth', _navWidth.toStringAsFixed(1));
              savePref('navWidthManual', 'true');
            },
          ),
        ],
        Expanded(child: _editorPane(context)),
        if (_toolsExpanded) ...[
          _resizeHandle(
            onDrag: (dx) => setState(
              () => _toolsWidth = (_toolsWidth - dx).clamp(220.0, 480.0),
            ),
          ),
          SizedBox(width: _toolsWidth, child: _workspaceTools(context)),
        ],
      ],
    );
  }

  /// Narrow: one pane at a time. The editor takes the whole width and the
  /// sidebar slides OVER it, summoned from the top bar and dismissed by the
  /// scrim or by opening a page.
  ///
  /// A resident sidebar cannot survive here. It has a 220px floor of its own
  /// ([_navWidthMin]) and does not yield, so at a 390px phone viewport it kept
  /// its full 280 and left the editor ~110px — narrow enough to wrap CJK text
  /// to one glyph per line. Splitting a phone screen between two panes has no
  /// win: whatever the editor loses comes straight out of the text column,
  /// which is the entire point of the app.
  ///
  /// The tools pane is dropped rather than stacked — two overlays competing for
  /// 390px helps nobody, and it is an auxiliary view.
  Widget _narrowShell(BuildContext context, double maxWidth) {
    // Always leave a strip of editor showing beside the drawer: it is the scrim
    // you tap to dismiss, and it keeps "there is a page behind this" legible.
    final drawerWidth = math.min(_navWidth, math.max(maxWidth - 56, 200.0));
    return Stack(
      children: [
        Positioned.fill(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              _narrowTopBar(context),
              Expanded(child: _editorPane(context)),
            ],
          ),
        ),
        if (_navDrawerOpen) ...[
          Positioned.fill(
            child: GestureDetector(
              onTap: _closeNavDrawer,
              child: const ColoredBox(color: Color(0x66000000)),
            ),
          ),
          Positioned(
            top: 0,
            left: 0,
            bottom: 0,
            width: drawerWidth,
            child: Material(
              elevation: 8,
              child: _navigationPane(context, narrow: true),
            ),
          ),
        ],
      ],
    );
  }

  /// Narrow-only top bar: the one always-reachable way back to the sidebar.
  ///
  /// It has to live outside the editor's scroll view — the page header (title +
  /// page menu) scrolls away with the content, so a hamburger parked there
  /// would strand you in a document with no route back to the tree.
  Widget _narrowTopBar(BuildContext context) {
    final title = widget.selectedBootstrap?.pageTitle ?? 'Mica';
    return Material(
      color: MicaTheme.of(context).surface.base,
      child: SizedBox(
        height: 48,
        child: Row(
          children: [
            IconButton(
              tooltip: context.l10n.sidebarOpen,
              onPressed: _openNavDrawer,
              icon: const Icon(Icons.menu, size: 22),
            ),
            Expanded(
              child: Text(
                title,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  fontWeight: FontWeight.w600,
                  color: MicaTheme.of(context).text.primary,
                ),
              ),
            ),
            const SizedBox(width: 8),
          ],
        ),
      ),
    );
  }

  void _openNavDrawer() => setState(() => _navDrawerOpen = true);

  void _closeNavDrawer() {
    if (_navDrawerOpen) setState(() => _navDrawerOpen = false);
  }

  /// App-level keyboard shortcuts (desktop feel): new page, search, settings.
  /// The editor handles its own editing shortcuts (Ctrl+B/I/Z/…) and returns the
  /// rest unhandled, so these fire when a key bubbles past it. Both Control
  /// (Win/Linux) and Meta (macOS) variants are bound.
  Map<ShortcutActivator, VoidCallback> _appShortcuts() {
    void newPage() => _createLocated(folder: false);
    return <ShortcutActivator, VoidCallback>{
      const SingleActivator(LogicalKeyboardKey.keyN, control: true): newPage,
      const SingleActivator(LogicalKeyboardKey.keyN, meta: true): newPage,
      // NO Esc binding for the sidebar selection, deliberately — see the note
      // on the tree's build method. Three ways to bind it were tried and all
      // three were dead on arrival for the same reason.
      // Ctrl/Cmd+F → in-page find within the open document; Ctrl/Cmd+Shift+F →
      // the workspace-wide search (what plain Ctrl+F used to do).
      const SingleActivator(LogicalKeyboardKey.keyF, control: true):
          _openInPageFind,
      const SingleActivator(LogicalKeyboardKey.keyF, meta: true):
          _openInPageFind,
      const SingleActivator(
        LogicalKeyboardKey.keyF,
        control: true,
        shift: true,
      ): _openSearch,
      const SingleActivator(LogicalKeyboardKey.keyF, meta: true, shift: true):
          _openSearch,
      // Ctrl+, is the convention (works on English layouts / macOS Cmd+,), but a
      // Chinese IME grabs Ctrl+,/Ctrl+. at the OS level (punctuation toggle), so
      // it won't reach the app while such an IME is active. Settings is also
      // reachable from the menu.
      const SingleActivator(LogicalKeyboardKey.comma, control: true):
          _openSettings,
      const SingleActivator(LogicalKeyboardKey.comma, meta: true):
          _openSettings,
      // Rename the highlighted sidebar row. Keyboard-only on purpose: binding
      // this to a double-click would put a DoubleTapGestureRecognizer on every
      // tree row, and it holds the gesture arena — taxing EVERY single click
      // with kDoubleTapTimeout (300ms) to serve a rare rename. Single-click-to-
      // open is the sidebar's hot path. AppFlowy (Flutter, same arena) and
      // AFFiNE (React, no such cost) both bind rename to F2/menu and leave
      // double-click unbound, matching Explorer/Finder/VS Code muscle memory
      // where a double-click on a tree row means *open*.
      const SingleActivator(LogicalKeyboardKey.f2): _renameLocated,
    };
  }

  /// Ctrl/Cmd+F: open the editor's in-page find bar (no-op when no document is
  /// open — e.g. a folder view). Workspace-wide search moved to Ctrl+Shift+F.
  void _openInPageFind() => _findHook.open();

  /// A slim draggable splitter between panes (the divider line stays 1px;
  /// the grab area is wider for easy targeting).
  Widget _resizeHandle({
    required void Function(double dx) onDrag,
    VoidCallback? onDragEnd,
  }) {
    return MouseRegion(
      cursor: SystemMouseCursors.resizeLeftRight,
      child: GestureDetector(
        behavior: HitTestBehavior.translucent,
        onHorizontalDragUpdate: (d) => onDrag(d.delta.dx),
        onHorizontalDragEnd: onDragEnd == null ? null : (_) => onDragEnd(),
        child: const SizedBox(
          width: 7,
          child: Center(child: VerticalDivider(width: 1)),
        ),
      ),
    );
  }

  /// [narrow] = this pane is the overlay drawer, not a resident column. It only
  /// changes what the header's sidebar button means: there is no "collapse to a
  /// rail" on a phone, so it dismisses the drawer instead.
  Widget _navigationPane(BuildContext context, {bool narrow = false}) {
    final canEdit = matchesEditRole(widget.selectedWorkspace?.role);

    return ColoredBox(
      color: MicaTheme.of(context).surface.base,
      child: MouseRegion(
        onEnter: (_) => setState(() => _navHovered = true),
        // Merely LEAVING the sidebar does not release the location.
        //
        // It used to, and that was wrong twice over: the highlight vanished the
        // instant the mouse drifted off — you never got to see the thing you
        // had just located — and "released" is a decision, not a side effect of
        // where the cursor happens to be. A click is the decision. So the
        // release lives on taps: blank space inside the sidebar (the wrapper
        // below), or a click anywhere outside it (the Listener in `build`).
        onExit: (_) => setState(() => _navHovered = false),
        // A tap on ANY blank part of the sidebar releases the location, not
        // just blank space inside the tree. The gap beside the search box is
        // sidebar too, and a click there that left a row lit was claiming a
        // location the next New button would not use.
        //
        // Translucent, and leaning on the gesture arena exactly as the tree
        // already does: rows and buttons carry their own tap recognizers and
        // win it, so this only ever sees taps that hit nothing. It MUST stay
        // that way — firing alongside the New buttons would clear the
        // location microseconds before the create reads it.
        child: GestureDetector(
          behavior: HitTestBehavior.translucent,
          onTap: _tree.focusRoot,
          child: Padding(
            // Tighter than the 16 all round it used to be. The sidebar's scarce
            // axis is VERTICAL — every point spent above the tree is a row of
            // pages you cannot see — so the vertical padding is cut hardest and
            // the horizontal is left roomy enough that rows keep a comfortable
            // hit area away from the window edge.
            padding: const EdgeInsets.fromLTRB(12, 10, 12, 6),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                // Top row: HOME on the left, the collapse button on the right.
                //
                // No logo/wordmark — the window title bar already says "Mica" with
                // the same icon directly above, and two of them read as two apps
                // stacked. Removing it left the row holding nothing but a Spacer,
                // i.e. a blank band across the most valuable strip in the sidebar;
                // home moved up into it rather than sitting a row lower under
                // whitespace.
                Row(
                  children: [
                    if (widget.onOpenHome != null)
                      Expanded(
                        child: _HomeNavRow(
                          label: context.l10n.navHome,
                          selected: widget.selectedBootstrap == null,
                          onTap: widget.onOpenHome!,
                        ),
                      )
                    else
                      const Spacer(),
                    if (widget.isBusy)
                      const Padding(
                        padding: EdgeInsets.only(right: 4),
                        child: SizedBox(
                          height: 16,
                          width: 16,
                          child: CircularProgressIndicator(strokeWidth: 2),
                        ),
                      ),
                    IconButton(
                      tooltip: narrow
                          ? context.l10n.sidebarCloseDrawer
                          : context.l10n.sidebarCollapse,
                      visualDensity: VisualDensity.compact,
                      onPressed: narrow
                          ? _closeNavDrawer
                          : () => setState(() => _navCollapsed = true),
                      icon: Icon(
                        narrow ? Icons.close : Icons.view_sidebar_outlined,
                        size: 20,
                      ),
                    ),
                  ],
                ),
                const SizedBox(height: 10),
                // Search still sits ABOVE the workspace switcher (design 03), and
                // that order is the meaning: it spans every workspace, so putting
                // it under a workspace picker would imply it is scoped to the one
                // you happen to have selected. Same reason home is on the row
                // above rather than inside the workspace section.
                _searchBox(context),
                const SizedBox(height: 10),
                Divider(height: 1, color: MicaTheme.of(context).border.subtle),
                const SizedBox(height: 10),
                Row(
                  crossAxisAlignment: CrossAxisAlignment.center,
                  children: [
                    Expanded(
                      child: _WorkspaceSelector(
                        entries: widget.entries,
                        activeIsLocal: widget.activeIsLocal,
                        // Counts PAGES, not rows: a switcher that says "12 个页面"
                        // while four of them are folders is lying in a way the user
                        // can check against the tree right below it.
                        //
                        // The second half names the WORLD, not the host. Using
                        // `cloudOriginLabel` here shipped "802 个页面 ·
                        // mica.cloudcele...." — a truncated hostname that fills the
                        // line and says nothing. The design's own meta reads
                        // "已同步"/"云端": which world this is, not which server.
                        activeMeta: widget.selectedWorkspace == null
                            ? null
                            : '${context.l10n.pageCount(countPages(widget.views))}'
                                  ' · '
                                  '${widget.activeIsLocal ? context.l10n.worldLocalName : context.l10n.worldCloudLabel}',
                        selectedRef: widget.selectedRef,
                        cloudEmail: widget.session?.user.email,
                        onSignIn: widget.onSignIn,
                        onSelect: widget.onSelectEntry,
                        onRename: _promptRenameWorkspace,
                        onDelete: _confirmDeleteWorkspace,
                        onExport: _exportWorkspaceFile,
                        onCreate: _promptCreateWorkspace,
                        onImport: widget.onImportWorkspaceZip == null
                            ? null
                            : (notion) => _importWorkspaceFile(notion: notion),
                        onImportFilesInto: _importFilesIntoWorkspace,
                        onImportFolderInto: _importFolderIntoWorkspace,
                        onMigrate: widget.onMigrateEntry,
                        onDetach: widget.onDetachEntry,
                        onReorder: widget.onReorderWorkspaces,
                      ),
                    ),
                    const SizedBox(width: 4),
                    // The graph is WORKSPACE-scoped, so it belongs here rather
                    // than next to search: search sits above the switcher because
                    // it spans every workspace, and this one shows the links
                    // inside the workspace you have selected.
                    IconButton(
                      tooltip: context.l10n.graphTitle,
                      visualDensity: VisualDensity.compact,
                      onPressed:
                          widget.onLoadGraph == null ||
                              widget.selectedWorkspace == null
                          ? null
                          : _openGraph,
                      icon: const Icon(Icons.hub_outlined, size: 20),
                    ),
                    IconButton(
                      tooltip: context.l10n.workspaceSettings,
                      visualDensity: VisualDensity.compact,
                      onPressed: widget.selectedWorkspace == null
                          ? null
                          : _openWorkspaceSettingsDialog,
                      icon: const Icon(Icons.tune, size: 20),
                    ),
                  ],
                ),
                if (widget.invitations case final invitations?)
                  ReceivedInvitationsCard(controller: invitations),
                if (widget.message != null) ...[
                  const SizedBox(height: 12),
                  ErrorBanner(widget.message!),
                ],
                if (widget.importProgress case final p?) ...[
                  const SizedBox(height: 12),
                  MicaProgressRow(
                    label: context.l10n.importProgress(p.done, p.total),
                    done: p.done,
                    total: p.total,
                  ),
                  if (widget.onCancelImport case final cancel?)
                    Align(
                      alignment: Alignment.centerLeft,
                      child: TextButton(
                        onPressed: cancel,
                        child: Text(context.l10n.importCancel),
                      ),
                    ),
                ],
                const SizedBox(height: 10),
                // Section label + actions. This was a label plus ONE visible
                // action (new page) with refresh / new folder / recycle bin
                // folded into a `⋯` overflow, on the theory that four equal icons
                // read as a toolbar competing with the tree.
                //
                // In use that traded the wrong way round: new folder is a primary
                // action here (the tree is folders), and burying it behind a menu
                // costs a click on the sidebar's whole reason for existing. So
                // new page + new folder + refresh are all visible now, and the
                // overflow is gone rather than left holding one stray item — the
                // recycle bin moved down beside the account row, which is where
                // the other "not part of the tree" entries live.
                Row(
                  children: [
                    Padding(
                      padding: const EdgeInsets.only(left: 4),
                      child: Text(
                        context.l10n.sidebarPagesLabel,
                        style: TextStyle(
                          fontSize: 12,
                          letterSpacing: 1.5,
                          fontWeight: FontWeight.w500,
                          color: MicaTheme.of(context).text.faint,
                        ),
                      ),
                    ),
                    const Spacer(),
                    if (canEdit) ...[
                      IconButton(
                        tooltip: context.l10n.newPage,
                        visualDensity: VisualDensity.compact,
                        // Creates relative to the located sidebar node (see
                        // _createLocated) — inside a focused folder, or beside a
                        // focused page.
                        onPressed: () => _createLocated(folder: false),
                        icon: const Icon(Icons.add, size: 18),
                      ),
                      IconButton(
                        tooltip: context.l10n.newFolder,
                        visualDensity: VisualDensity.compact,
                        onPressed: () => _createLocated(folder: true),
                        icon: const Icon(
                          Icons.create_new_folder_outlined,
                          size: 18,
                        ),
                      ),
                    ],
                    IconButton(
                      tooltip: context.l10n.recycleRefresh,
                      visualDensity: VisualDensity.compact,
                      onPressed: widget.onRefresh,
                      icon: const Icon(Icons.refresh, size: 18),
                    ),
                  ],
                ),
                const SizedBox(height: 2),
                Expanded(child: _pageTree(context, canEdit)),
                const Divider(height: 16),
                // AI entry points exist only when the feature is enabled in
                // Settings AND a provider is configured.
                if (widget.showAi) ...[
                  SizedBox(
                    width: double.infinity,
                    child: FilledButton.tonalIcon(
                      onPressed: _openAiDialog,
                      icon: const Icon(Icons.auto_awesome, size: 18),
                      label: Text(context.l10n.aiAskTitle),
                    ),
                  ),
                  const SizedBox(height: 12),
                ],
                // Account + recycle bin share the bottom row. The bin is not part
                // of the page tree — it is where pages GO when they leave it — so
                // it sits with the other whole-app entries rather than in the
                // tree's own action row.
                Row(
                  children: [
                    Expanded(child: _accountTile(context)),
                    if (updateSupported)
                      UpdateButton(controller: _desktopUpdates),
                    IconButton(
                      tooltip: context.l10n.recycleBinTitle,
                      visualDensity: VisualDensity.compact,
                      onPressed: _openRecycleBin,
                      icon: Icon(
                        Icons.delete_outline,
                        size: 20,
                        color: MicaTheme.of(context).text.muted,
                      ),
                    ),
                  ],
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  /// Search-box-shaped button: looks like an input, opens the search dialog
  /// (Notion-style — the real query field lives in the dialog).
  Widget _searchBox(BuildContext context) {
    final enabled = widget.selectedWorkspace != null;
    return InkWell(
      onTap: enabled ? _openSearch : null,
      borderRadius: BorderRadius.circular(8),
      child: Container(
        width: double.infinity,
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 9),
        decoration: BoxDecoration(
          color: MicaTheme.of(context).surface.sunken,
          borderRadius: BorderRadius.circular(8),
        ),
        child: Row(
          children: [
            Icon(
              Icons.search,
              size: 18,
              color: enabled
                  ? MicaTheme.of(context).text.muted
                  : MicaTheme.of(context).border.strong,
            ),
            const SizedBox(width: 8),
            Text(
              context.l10n.search,
              style: TextStyle(
                color: enabled
                    ? MicaTheme.of(context).text.muted
                    : MicaTheme.of(context).border.strong,
              ),
            ),
          ],
        ),
      ),
    );
  }

  /// The sidebar collapsed to a slim rail: just an expand button.
  ///
  /// No logo here either. It was the same duplicate as the expanded header's —
  /// collapsing the sidebar brought back the second Mica mark that had just been
  /// removed, which reads as the app re-introducing itself.
  Widget _collapsedNavRail(BuildContext context) {
    return ColoredBox(
      color: MicaTheme.of(context).surface.base,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 10),
        child: Column(
          children: [
            IconButton(
              tooltip: context.l10n.sidebarExpand,
              visualDensity: VisualDensity.compact,
              onPressed: () => setState(() => _navCollapsed = false),
              icon: const Icon(Icons.view_sidebar_outlined, size: 20),
            ),
          ],
        ),
      ),
    );
  }

  /// The sidebar's content width (280 panel − 16 padding each side), so the
  /// menu lines up with the tile that opens it.
  static const _kAccountMenuWidth = 248.0;

  final _accountMenu = MenuController();

  /// Account row pinned to the bottom of the left sidebar, and the app's ONE
  /// world switcher: which world you are in, plus the way to change it.
  ///
  /// It lives here because this tile is the only control in the app whose
  /// content already IS "which world am I in" — [accountIdentity] is a pure
  /// function of (activeOrigin, session), no fetch. It is supposed to change
  /// when you switch. Every other home put the switcher inside a container
  /// whose contents it changes: the workspace dropdown (which then had to list
  /// both worlds side by side), and Settings (whose Account / API Tokens / AI
  /// tabs belong to whichever server the switcher was pointing at).
  /// The signed-in user's picture, or null when there is none (and in 本地模式,
  /// which has no account at all).
  String? _myAvatarUrl() {
    final user = widget.session?.user;
    if (user == null) return null;
    return avatarUrl(
      base: widget.apiBase,
      userId: user.id,
      version: user.avatarVersion,
    );
  }

  Widget _accountTile(BuildContext context) {
    final id = accountIdentity(
      local: widget.activeIsLocal,
      user: widget.session?.user,
    );
    final initial = widget.activeIsLocal
        ? context.l10n.accountLocalInitial
        : (id.name.isNotEmpty ? id.name.substring(0, 1).toUpperCase() : '?');
    // Width comes from the rows (each a SizedBox of _kAccountMenuWidth), not
    // from a MenuStyle: that is how _WorkspaceSelector — the other menu in this
    // same sidebar — does it, and a panel constrained to the same width as its
    // children has nowhere to put its own padding.
    return MenuAnchor(
      controller: _accountMenu,
      menuChildren: [
        // Desktop only — the same gate the Settings page had. Web has no local
        // store to switch to (_local.available is false there) and is served
        // BY a server, so its menu stays exactly what it has always been.
        // One entry, not a list of worlds plus an add-server row. The world
        // picker already exists as a full screen (the sign-in route: tabs,
        // server list, add, remove, health probes) and it was the better one —
        // this menu had a second, smaller implementation of the same idea that
        // could only ever drift from it.
        //
        // Reachable while SIGNED IN, which is the whole point: the sign-in
        // screen is otherwise only shown when there is no session, so before
        // this the only way into 本地模式 from a signed-in app was to sign out.
        if (!kIsWeb) ...[
          _menuAction(
            Icons.swap_horiz,
            context.l10n.accountSwitchWorld,
            widget.onSwitchWorld,
          ),
          const Divider(height: 8),
        ],
        _menuAction(
          Icons.settings_outlined,
          context.l10n.settingsTitle,
          _openSettings,
        ),
        // Signing in stays a top-level action, not a per-row one: you sign in
        // to the world you are in. In 本地模式 there is neither — and now the
        // reason is one line above it (pick a world first) instead of a comment
        // pointing somewhere else.
        if (id.canSignOut)
          _menuAction(
            Icons.logout,
            context.l10n.commonSignOut,
            widget.onSignOut,
          )
        else if (id.canSignIn && widget.onSignIn != null)
          _menuAction(
            Icons.login,
            context.l10n.workspaceRowSignInCloud,
            widget.onSignIn!,
          ),
      ],
      builder: (context, controller, child) => InkWell(
        onTap: () => controller.isOpen ? controller.close() : controller.open(),
        child: Padding(
          // Slimmer than it was (vertical 6, radius-16 avatar): this row is
          // always on screen and never scrolls, so its height comes straight
          // out of the page tree's.
          padding: const EdgeInsets.symmetric(vertical: 3, horizontal: 4),
          child: Row(
            children: [
              UserAvatar(url: _myAvatarUrl(), fallback: initial, radius: 14),
              const SizedBox(width: 8),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Text(
                      id.name,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: const TextStyle(
                        fontSize: 14,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                    if (id.email != null)
                      Text(
                        id.email!,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontSize: 12,
                          color: MicaTheme.of(context).text.muted,
                        ),
                      ),
                  ],
                ),
              ),
              Icon(
                Icons.more_vert,
                size: 18,
                color: MicaTheme.of(context).text.faint,
              ),
            ],
          ),
        ),
      ),
    );
  }

  /// One connection: what it IS, and nothing else. Deliberately no sign-in
  /// state — signing in does not happen here (it is the action below, in the
  /// world you are actually in), and a server you just added is always
  /// signed-out, so the marker could only ever state the obvious.
  Widget _menuAction(IconData icon, String label, VoidCallback onTap) =>
      SizedBox(
        width: _kAccountMenuWidth,
        child: InkWell(
          onTap: () {
            _accountMenu.close();
            onTap();
          },
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
            child: Row(
              children: [
                Icon(icon, size: 18, color: MicaTheme.of(context).text.muted),
                const SizedBox(width: 10),
                Text(
                  label,
                  style: TextStyle(color: MicaTheme.of(context).text.primary),
                ),
              ],
            ),
          ),
        ),
      );

  void _schedulePageTitleSave() {
    // The field is now ahead of the document. Latched BEFORE the timer, not
    // inside it: the point is to hold from the first keystroke, and what this
    // guards against is the echo of a save an EARLIER timer already sent.
    _pageTitleEditPending = true;
    final revision = ++_pageTitleEditRevision;
    _pageTitleSaveTimer?.cancel();
    _pageTitleSaveTimer = Timer(const Duration(milliseconds: 700), () async {
      final bootstrap = widget.selectedBootstrap;
      if (bootstrap == null) return;
      // Compared against the SAME value the field is showing. Against
      // view.name instead, a rename that arrived from another device would
      // look like the user had just typed one, and this would push it back.
      final title = renamedTo(_pageTitle.text, bootstrap.pageTitle);
      if (title == null) {
        // Nothing to send (blank, or back to the name it already had), so no
        // echo is coming to settle the latch. Release it here, or the field
        // would refuse every later update for the rest of this page's life.
        _pageTitleEditPending = false;
        return;
      }

      final saved = await widget.onRenameView(bootstrap.view, title);
      // A failed request has no echo. Release only its own edit, never newer
      // keystrokes or a different page selected while the request ran.
      if (!saved && mounted && revision == _pageTitleEditRevision) {
        _pageTitleEditPending = false;
      }
    });
  }

  /// Right panel — the current page's outline (table of contents). Rebuilds
  /// off [_outlineHook] so headings track live edits, not just navigation.
  /// The ONE right sidebar: outline and comments as tabs, not two competing
  /// panels.
  ///
  /// They used to be separate surfaces — the outline lived here, the comment
  /// rail was mounted inside the editor pane — so opening both squeezed the
  /// text column to ~300px on a normal window. Two auxiliary views fighting the
  /// document for width is the wrong trade every time; AFFiNE puts them in one
  /// rail behind icon tabs for the same reason.
  Widget _workspaceTools(BuildContext context) {
    final canComment = widget.onAddComment != null;
    final tab = (!canComment) ? _ToolsTab.outline : _toolsTab;
    return ColoredBox(
      color: MicaTheme.of(context).surface.base,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (canComment)
            Padding(
              padding: const EdgeInsets.fromLTRB(12, 10, 8, 4),
              child: Row(
                children: [
                  _ToolsTabButton(
                    icon: Icons.toc,
                    label: context.l10n.pageOutlineTitle,
                    active: tab == _ToolsTab.outline,
                    onTap: () => setState(() => _toolsTab = _ToolsTab.outline),
                  ),
                  const SizedBox(width: 4),
                  _ToolsTabButton(
                    icon: Icons.chat_bubble_outline,
                    label: context.l10n.commentsTitle,
                    active: tab == _ToolsTab.comments,
                    // The unresolved count rides the tab now: with the panel
                    // behind a tab you would otherwise have to open it to learn
                    // there is anything in it.
                    badge: widget.commentThreads
                        .where((t) => !t.isResolved)
                        .length,
                    onTap: () => setState(() => _toolsTab = _ToolsTab.comments),
                  ),
                  const Spacer(),
                ],
              ),
            ),
          Expanded(
            child: tab == _ToolsTab.comments
                ? CommentPanel(
                    threads: widget.commentThreads,
                    currentUserId: widget.session?.user.id,
                    onReply: widget.onReplyComment,
                    onSetResolved: widget.onSetCommentResolved,
                    onDelete: widget.onDeleteCommentThread,
                    onFocusThread: (t) => widget.onFocusCommentThread(t.id),
                  )
                : _outlineTab(context),
          ),
        ],
      ),
    );
  }

  Widget _outlineTab(BuildContext context) {
    return ListenableBuilder(
      listenable: _outlineHook,
      builder: (context, _) {
        final outline = _pageOutlineItems(context, _outlineHook.headings);
        return ColoredBox(
          color: MicaTheme.of(context).surface.base,
          child: outline.isEmpty
              ? EmptyState(
                  icon: Icons.toc,
                  title: context.l10n.pageOutlineTitle,
                  detail: context.l10n.pageOutlineEmptyDetail,
                )
              : SingleChildScrollView(
                  padding: const EdgeInsets.all(16),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [...outline],
                  ),
                ),
        );
      },
    );
  }

  /// Inline workspace settings (rename + members), shown in the left panel when
  /// its gear is toggled — kept in the tree so member edits refresh live.
  /// Workspace settings as a centered modal dialog (rename + members), instead
  /// of expanding inline in the sidebar. Member add/remove/role-change await the
  /// (async) callback then rebuild the dialog via its own StatefulBuilder, so the
  /// list stays live without leaning on the parent's setState reaching the route.
  Future<void> _openWorkspaceSettingsDialog() async {
    final workspace = widget.selectedWorkspace;
    if (workspace == null) return;
    _rename.text = workspace.name;
    final l10n = context.l10n;
    // Fetched BEFORE the dialog opens: one call, no spinner inside a
    // StatefulBuilder, and a failure just omits the row instead of putting
    // rename + members behind an unrelated error.
    final usage = await widget.onLoadWorkspaceUsage?.call(workspace);
    if (!mounted) return;
    await showDialog<void>(
      context: context,
      builder: (dialogContext) => StatefulBuilder(
        builder: (dialogContext, setLocal) {
          final ws = widget.selectedWorkspace ?? workspace;
          // The role check alone was not enough: local entries are minted as
          // 'owner', so it said yes in a world with no membership API.
          final canManage =
              matchesManageRole(ws.role) && widget.invitations != null;
          final members = widget.members;
          return AlertDialog(
            title: Text(l10n.workspaceSettings),
            content: SizedBox(
              width: 440,
              child: SingleChildScrollView(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    DetailRow(label: l10n.widgetRoleLabel, value: ws.role),
                    DetailRow(label: 'ID', value: ws.id),
                    if (usage != null) ...[
                      DetailRow(
                        label: l10n.workspaceStorage,
                        // A quota of 0 means the server disabled quotas. Showing
                        // "8.6 MB / 0 B" there would read as "you are over the
                        // limit", so that case says only what is true.
                        // BINARY units on this row: the quota is configured,
                        // documented and enforced in GiB, and decimal units
                        // printed "1.1 GB" for a limit everything else calls
                        // 1 GiB. Both sides use the same base — mixing them in
                        // one "used / limit" string is worse than either.
                        value: usage.quota > 0
                            ? '${formatBytesBinary(usage.used)} / ${formatBytesBinary(usage.quota)}'
                            : formatBytesBinary(usage.used),
                      ),
                      // Nothing stored → no bar. An empty track is a 1px line
                      // directly under a row of text, and it reads as a divider
                      // someone added by mistake; a brand-new workspace would
                      // wear it forever.
                      if (usage.quota > 0 && usage.used > 0)
                        Padding(
                          padding: const EdgeInsets.only(top: 6),
                          child: ClipRRect(
                            borderRadius: BorderRadius.circular(3),
                            child: LinearProgressIndicator(
                              value: (usage.used / usage.quota).clamp(0.0, 1.0),
                              minHeight: 6,
                            ),
                          ),
                        ),
                    ],
                    const SizedBox(height: 16),
                    TextField(
                      controller: _rename,
                      decoration: InputDecoration(
                        labelText: l10n.workspaceRename,
                        prefixIcon: const Icon(Icons.edit),
                        border: const OutlineInputBorder(),
                        isDense: true,
                      ),
                    ),
                    const SizedBox(height: 10),
                    Align(
                      alignment: Alignment.centerLeft,
                      child: FilledButton.icon(
                        onPressed: () async {
                          await widget.onRenameWorkspace(ws, _rename.text);
                          setLocal(() {});
                        },
                        icon: const Icon(Icons.save, size: 18),
                        label: Text(l10n.commonSave),
                      ),
                    ),
                    const SizedBox(height: 22),
                    if (widget.invitations == null)
                      EmptyState(
                        icon: Icons.group_outlined,
                        title: l10n.workspaceMembersLocalTitle,
                        detail: l10n.workspaceMembersLocalDetail,
                      )
                    else ...[
                      Row(
                        children: [
                          const Icon(Icons.group, size: 18),
                          const SizedBox(width: 8),
                          Text(
                            l10n.workspaceMembers,
                            style: Theme.of(
                              dialogContext,
                            ).textTheme.titleMedium,
                          ),
                        ],
                      ),
                      const SizedBox(height: 12),
                      if (canManage) ...[
                        MemberInviteSection(
                          controller: widget.invitations!,
                          workspaceId: ws.id,
                        ),
                        const SizedBox(height: 14),
                      ],
                      if (members.isEmpty)
                        Text(
                          l10n.workspaceNoMembers,
                          style: TextStyle(
                            color: MicaTheme.of(context).text.faint,
                          ),
                        )
                      else
                        for (final member in members)
                          Padding(
                            padding: const EdgeInsets.only(bottom: 8),
                            child: MemberListItem(
                              member: member,
                              avatarUrl: avatarUrl(
                                base: widget.apiBase,
                                userId: member.userId,
                                version: member.avatarVersion,
                              ),
                              canManage: canManage,
                              canRemove: member.role != 'owner',
                              onRoleChanged: (role) async {
                                await widget.onUpdateMember!(member, role);
                                setLocal(() {});
                              },
                              onRemove: () async {
                                await widget.onRemoveMember!(member);
                                setLocal(() {});
                              },
                            ),
                          ),
                    ],
                  ],
                ),
              ),
            ),
            actions: [
              TextButton(
                onPressed: () => Navigator.of(dialogContext).pop(),
                child: Text(l10n.commonClose),
              ),
            ],
          );
        },
      ),
    );
  }

  Future<void> _promptCreateWorkspace() async {
    // Create in the CURRENT world — no local/cloud picker. `onCreateWorkspace`
    // is already routed to the active origin by the host, so a workspace made
    // while you're in local mode is local, and cloud while you're in cloud.
    final l10n = context.l10n;
    final name = await showDialog<String>(
      context: context,
      builder: (context) => DialogTextControllers(
        count: 1,
        builder: (context, fields) => AlertDialog(
          title: Text(l10n.workspaceRowNewWorkspace),
          content: TextField(
            controller: fields[0],
            autofocus: true,
            decoration: InputDecoration(
              labelText: l10n.workspaceNameLabel,
              border: const OutlineInputBorder(),
            ),
            onSubmitted: (value) => Navigator.of(context).pop(value),
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.of(context).pop(),
              child: Text(l10n.commonCancel),
            ),
            FilledButton(
              onPressed: () => Navigator.of(context).pop(fields[0].text),
              child: Text(l10n.workspaceCreate),
            ),
          ],
        ),
      ),
    );

    final trimmed = name?.trim() ?? '';
    if (trimmed.isNotEmpty) {
      await widget.onCreateWorkspace(trimmed);
    }
  }

  Future<void> _promptRenameWorkspace(WorkspaceEntry entry) async {
    final workspace = entry.workspace;
    final l10n = context.l10n;
    final name = await showDialog<String>(
      context: context,
      builder: (context) {
        return DialogTextControllers(
          count: 1,
          initialTexts: [workspace.name],
          builder: (context, fields) => AlertDialog(
            title: Text(l10n.workspaceRename),
            content: TextField(
              controller: fields[0],
              autofocus: true,
              decoration: InputDecoration(
                labelText: l10n.workspaceNameLabel,
                border: const OutlineInputBorder(),
              ),
              onSubmitted: (value) => Navigator.of(context).pop(value),
            ),
            actions: [
              TextButton(
                onPressed: () => Navigator.of(context).pop(),
                child: Text(l10n.commonCancel),
              ),
              FilledButton.icon(
                onPressed: () => Navigator.of(context).pop(fields[0].text),
                icon: const Icon(Icons.save),
                label: Text(l10n.commonSave),
              ),
            ],
          ),
        );
      },
    );

    final trimmed = name?.trim() ?? '';
    if (trimmed.isNotEmpty && trimmed != workspace.name) {
      await widget.onRenameEntry(entry, trimmed);
    }
  }

  // ── Rename ─────────────────────────────────────────────────────────────────
  // Two shapes, split by operation:
  //  - Renaming an EXISTING row (row menu / F2) opens a centered dialog. An
  //    inline field can't edit a long name: the sidebar is narrow AND flush
  //    against the screen's left edge, and a TextField only scrolls in
  //    proportion to how far the pointer is dragged past its edge — so the
  //    pointer hits x=0 while the name's head is still off-screen, and you can
  //    never select back to it. A centered dialog has drag room on both sides.
  //    (AppFlowy renames through a centered dialog for every view, for the
  //    same reason; nobody in the field auto-widens an inline field.)
  //  - Naming a JUST-CREATED row stays inline (`_createThenRename`) — that name
  //    is empty, so it cannot overflow, and a dialog per create is heavier.
  // `_renamingViewId` therefore only ever drives the create-then-name flow.

  /// Rename [view] through a centered dialog ([_RenameDialog]), pre-filled and
  /// select-all so the common "replace the whole name" case is still one
  /// keystroke.
  Future<void> _promptRenameView(DocumentView view) async {
    final name = await showDialog<String>(
      context: context,
      builder: (_) => _RenameDialog(initialName: view.name),
    );
    if (name == null || !mounted) return; // cancelled / Esc → no change
    await _commitRename(view, name);
  }

  /// F2: rename the highlighted sidebar row — the located node, i.e. the same
  /// row `_navigationPane` highlights (last-tapped folder/page, else the open
  /// doc), so what you see is what gets renamed.
  void _renameLocated() {
    if (!matchesEditRole(widget.selectedWorkspace?.role)) return;
    final view = _locatedView();
    if (view == null) return;
    // The located row can sit inside a collapsed parent — a folder located
    // earlier stays located after its parent is collapsed. Reveal it so the row
    // being renamed is actually on screen behind the dialog.
    setState(() => _tree.revealAncestors(widget.views, view.id));
    _promptRenameView(view);
  }

  Future<void> _commitRename(DocumentView view, String name) async {
    // Empty or unchanged → nothing to send (the controller's [renamedTo]).
    final next = _inlineRename.commit(view, name);
    if (next == null) return;
    await widget.onRenameView(view, next);
  }

  /// Create a page/folder via [create], then drop its new sidebar row straight
  /// into inline-rename so the user just types the name — no naming dialog.
  Future<void> _createThenRename(Future<String?> Function() create) async {
    final id = await create();
    if (id == null || !mounted) return;
    _inlineRename.begin(id);
    // Scroll the new row into view. Without this, creating inside a folder that
    // already holds many pages left the row below the fold: the rename box had
    // focus and swallowed the typing, but nothing on screen moved, so the page
    // read as "not created" until you scrolled down looking for it.
    //
    // `revealInTree` rather than a direct scroll, because the row does not
    // exist yet at this moment — the tree only gains it on a later rebuild.
    // That machinery already retries per frame until the row is laid out, which
    // is exactly the shape of this problem.
    _tree.revealInTree(widget.views, id);
  }

  DocumentView? _viewById(String? id) {
    if (id == null) return null;
    for (final v in widget.views) {
      if (v.id == id) return v;
    }
    return null;
  }

  /// The node the top New-page/New-folder buttons create relative to: the last
  /// folder/page the user tapped, else the open doc, else null (root).
  DocumentView? _locatedView() =>
      _rootFocused ? null : _viewById(_focusedNavId ?? widget.selectedView?.id);

  /// Create a page ([folder] false) or folder ([folder] true) relative to the
  /// currently located sidebar node, then drop it into inline-rename:
  ///  - located a folder → create INSIDE it (a child),
  ///  - located a page (a leaf can't hold children) → create BESIDE it (under its
  ///    parent, so it lands in the same group, in order),
  ///  - nothing located → create at the workspace root (the old behaviour).
  void _createLocated({required bool folder}) {
    final located = _locatedView();
    final parent = createParentForLocated(widget.views, located);
    if (parent != null) {
      // Reveal the new child.
      setState(() => _tree.expandForChildOf(widget.views, parent.id));
    }
    final folderName = context.l10n.folderNewDefault;
    final untitled = context.l10n.untitledPage;
    // Located a PAGE → the new row goes on the next line, not at the end of the
    // group. Creation carries no position (neither the HTTP API nor the local
    // store takes one), so the row is created under the right parent and then
    // slid into place with the SAME reorder drag-and-drop uses. Reusing that
    // rather than adding a position parameter down three layers keeps one
    // answer to "how does a row change places".
    final after = located != null && located.objectType != 'folder'
        ? located
        : null;
    _createThenRename(() async {
      final String? id;
      if (parent == null) {
        id = folder
            ? await widget.onCreateFolder(folderName)
            : await widget.onCreateDocument(untitled);
      } else {
        id = folder
            ? await widget.onCreateChildFolder(parent, folderName)
            : await widget.onCreateChildDocument(parent, untitled);
      }
      if (id != null && after != null) await _placeRightAfter(id, after);
      return id;
    });
  }

  /// Create inside [parent] from that folder row's own New buttons.
  ///
  /// Still "in THIS folder" — the button is on the folder's row and means
  /// nothing else. What it additionally honours is WHERE in the folder: if the
  /// located row is a page sitting in this same folder, the new row goes on the
  /// next line rather than at the bottom of the group. Same rule the top New
  /// buttons follow; having the two disagree was the complaint.
  ///
  /// A location pointing anywhere else (another folder, the root, released)
  /// does not reach in here: the anchor has to be a child of [parent], so the
  /// fallback is the plain append this button always did.
  void _createInFolder(DocumentView parent, {required bool folder}) {
    // Reveal the new child.
    setState(() => _tree.expandForChildOf(widget.views, parent.id));
    final located = _locatedView();
    final after =
        located != null &&
            located.objectType != 'folder' &&
            located.parentViewId == parent.id
        ? located
        : null;
    final folderName = context.l10n.folderNewDefault;
    final untitled = context.l10n.untitledPage;
    _createThenRename(() async {
      final id = folder
          ? await widget.onCreateChildFolder(parent, folderName)
          : await widget.onCreateChildDocument(parent, untitled);
      if (id != null && after != null) await _placeRightAfter(id, after);
      return id;
    });
  }

  /// Move the freshly created [newId] to sit directly below [after].
  ///
  /// Best-effort on purpose: if the new row is not in `widget.views` yet, or
  /// the anchor moved, this returns and the row keeps the position the create
  /// gave it (last in the group) — the behaviour before this rule existed. A
  /// failed tidy-up must not look like a failed create.
  Future<void> _placeRightAfter(String newId, DocumentView after) async {
    // The create's setState has run on the HOST; this widget sees the new view
    // only after the next build. Wait for it rather than reading a list that
    // provably does not contain the row yet.
    await WidgetsBinding.instance.endOfFrame;
    if (!mounted) return;
    final created = _viewById(newId);
    if (created == null) return;
    final ordered = orderedSiblingsPlacingAfter(
      views: widget.views,
      anchor: after,
      created: created,
    );
    if (ordered == null) return; // already in place, or the anchor is gone
    await widget.onReorderViews(after.parentViewId, ordered);
  }

  Future<void> _confirmDeleteWorkspace(WorkspaceEntry entry) async {
    final workspace = entry.workspace;
    final l10n = context.l10n;
    final shouldDelete = await showDialog<bool>(
      context: context,
      builder: (context) {
        return AlertDialog(
          title: Text(l10n.workspaceDeleteTitle),
          content: Text(l10n.workspaceDeleteConfirm(workspace.name)),
          actions: [
            TextButton(
              onPressed: () => Navigator.of(context).pop(false),
              child: Text(l10n.commonCancel),
            ),
            FilledButton.icon(
              style: FilledButton.styleFrom(
                backgroundColor: MicaTheme.of(context).status.danger,
              ),
              onPressed: () => Navigator.of(context).pop(true),
              icon: const Icon(Icons.delete_outline),
              label: Text(l10n.commonDelete),
            ),
          ],
        );
      },
    );

    if (shouldDelete != true) {
      return;
    }

    await widget.onDeleteEntry(entry);
  }

  void _openRecycleBin() {
    showDialog<void>(
      context: context,
      builder: (context) => _RecycleBinDialog(
        onLoad: widget.onLoadTrash,
        onRestore: widget.onRestoreView,
        onPurge: widget.onPurgeView,
        onPurgeAll: widget.onPurgeAllTrash,
        canEdit: matchesEditRole(widget.selectedWorkspace?.role),
        // The live tree, so a row can say where restoring puts it back.
        liveViews: widget.views,
        relativeStrings: RelativeTimeStrings(
          justNow: context.l10n.homeJustNow,
          minutesAgo: context.l10n.homeMinutesAgo,
          hoursAgo: context.l10n.homeHoursAgo,
          yesterday: context.l10n.homeYesterday,
          daysAgo: context.l10n.homeDaysAgo,
        ),
      ),
    );
  }

  /// Page title menu: Markdown export/import + ZIP export.
  Future<void> _onPageMenu(String value) async {
    switch (value) {
      case 'copy-md':
        await _copyPageMarkdown();
      case 'export-zip':
        await _exportPageFile();
      case 'export-html':
        await _exportPageHtmlFile();
      case 'export-pdf':
        await _exportPagePdfFile();
      case 'import-md':
        await _importMarkdownFile();
      case 'share':
        await widget.onShare?.call();
      case 'version-history':
        await widget.onVersionHistory?.call();
      case 'restore-checkpoint':
        final restore = widget.onRestoreCheckpoint;
        if (restore == null) return;
        final ok = await showDialog<bool>(
          context: context,
          builder: (context) => AlertDialog(
            title: Text(context.l10n.pageRestoreCheckpointTitle),
            content: Text(context.l10n.pageRestoreCheckpointBody),
            actions: [
              TextButton(
                onPressed: () => Navigator.pop(context, false),
                child: Text(context.l10n.commonCancel),
              ),
              FilledButton(
                onPressed: () => Navigator.pop(context, true),
                child: Text(context.l10n.versionRestore),
              ),
            ],
          ),
        );
        if (ok == true) await restore();
    }
  }

  Future<void> _importMarkdownFile() async {
    final picked = await pickTextFile();
    if (picked == null || !mounted) return;
    await widget.onImportMarkdown(picked.name, picked.text);
  }

  /// Download a whole workspace as a Markdown ZIP (page-tree folders + assets).
  /// Export one folder's subtree as a ZIP, named after the folder.
  Future<void> _exportFolderFile(DocumentView view) async {
    final export = widget.onExportFolderZip;
    if (export == null) return;
    final l10n = context.l10n;
    try {
      final bytes = await export(view);
      if (bytes.isEmpty) throw ApiException(l10n.exportEmptyContent);
      final name = view.name.trim().isEmpty ? 'folder' : view.name.trim();
      downloadImage(bytes, '$name.zip', 'application/zip');
    } catch (error) {
      if (mounted) {
        ScaffoldMessenger.of(
          context,
        ).showSnackBar(SnackBar(content: Text(l10n.exportFailed('$error'))));
      }
    }
  }

  Future<void> _exportWorkspaceFile(WorkspaceEntry entry) async {
    final l10n = context.l10n;
    try {
      final bytes = await widget.onExportEntryZip(entry);
      // Same guard as the page/folder exports: an empty archive is a failure,
      // not a file worth saving.
      if (bytes.isEmpty) throw ApiException(l10n.exportEmptyContent);
      final name = entry.workspace.name.trim().isEmpty
          ? 'workspace'
          : entry.workspace.name.trim();
      downloadImage(bytes, '$name.zip', 'application/zip');
    } catch (error) {
      if (mounted) {
        ScaffoldMessenger.of(
          context,
        ).showSnackBar(SnackBar(content: Text(l10n.exportFailed('$error'))));
      }
    }
  }

  /// Export the open page as a ZIP (markdown + its images under `assets/`).
  /// There is no markdown-only export any more: a lone .md silently dropped
  /// every image, emitting `![](photo.png)` for a file it never handed over.
  /// Export the open page as a self-contained `.html` file, named after the
  /// page. Works in both worlds (the local path runs through the FFI engine).
  Future<void> _exportPageHtmlFile() async {
    final l10n = context.l10n;
    try {
      final title = _pageTitle.text.trim();
      final html = await widget.onExportPageHtml(title);
      if (html.isEmpty) throw ApiException(l10n.exportEmptyContent);
      final base = title.isEmpty ? 'page' : title;
      downloadImage(
        Uint8List.fromList(utf8.encode(html)),
        '$base.html',
        'text/html',
      );
    } catch (error) {
      if (mounted) {
        ScaffoldMessenger.of(
          context,
        ).showSnackBar(SnackBar(content: Text(l10n.exportFailed('$error'))));
      }
    }
  }

  /// Export the open page as a real (vector, selectable) `.pdf`. Reuses the same
  /// self-contained HTML the HTML export produces, then hands it to the OS
  /// WebView2 runtime's headless print (desktop only). A null result means the
  /// runtime is unavailable / the platform isn't supported yet.
  Future<void> _exportPagePdfFile() async {
    final l10n = context.l10n;
    try {
      final title = _pageTitle.text.trim();
      final html = await widget.onExportPageHtml(title);
      if (html.isEmpty) throw ApiException(l10n.exportEmptyContent);
      if (kIsWeb) {
        // Web: hand the self-contained HTML to the browser's print dialog
        // (the user picks "Save as PDF"). No native WebView2 here; the browser
        // renders the same HTML (math/mermaid SVG inline), so it matches.
        await printHtml(html);
        return;
      }
      final bytes = await widget.onExportPagePdf(html);
      if (bytes == null || bytes.isEmpty) {
        throw ApiException(l10n.exportPdfUnsupported);
      }
      final base = title.isEmpty ? 'page' : title;
      downloadImage(bytes, '$base.pdf', 'application/pdf');
    } catch (error) {
      if (mounted) {
        ScaffoldMessenger.of(
          context,
        ).showSnackBar(SnackBar(content: Text(l10n.exportFailed('$error'))));
      }
    }
  }

  /// Put the open page's Markdown on the clipboard.
  ///
  /// Reports BOTH outcomes. `copyTextToClipboard` returns false rather than
  /// throwing when the platform refuses (the web path can be denied outright),
  /// and a copy that silently did nothing is worse than one that failed loudly:
  /// the user walks away and pastes whatever was on the clipboard before.
  Future<void> _copyPageMarkdown() async {
    final l10n = context.l10n;
    try {
      final markdown = await widget.onCopyPageMarkdown();
      if (markdown.trim().isEmpty) throw ApiException(l10n.exportEmptyContent);
      final ok = await copyTextToClipboard(markdown);
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: Text(ok ? l10n.pageCopyContentDone : l10n.pageCopyFailed),
        ),
      );
    } catch (error) {
      if (mounted) {
        ScaffoldMessenger.of(
          context,
        ).showSnackBar(SnackBar(content: Text(l10n.exportFailed('$error'))));
      }
    }
  }

  Future<void> _exportPageFile() async {
    final l10n = context.l10n;
    try {
      // Content-aware: a page with no bundled images downloads as a clean
      // `.md`; one with images downloads as a `.zip` (md + assets).
      final out = await widget.onExportPage(_pageTitle.text.trim());
      if (out.bytes.isEmpty) throw ApiException(l10n.exportEmptyContent);
      downloadImage(out.bytes, out.name, out.mime);
    } catch (error) {
      if (mounted) {
        ScaffoldMessenger.of(
          context,
        ).showSnackBar(SnackBar(content: Text(l10n.exportFailed('$error'))));
      }
    }
  }

  /// The graph opens as a large dialog rather than a route: it is something you
  /// glance at and dismiss, and a route would lose the page you were reading.
  void _openGraph() {
    final load = widget.onLoadGraph;
    if (load == null) return;
    showDialog<void>(
      context: context,
      builder: (context) => _GraphDialog(
        load: load,
        currentViewId: widget.selectedView?.id,
        onOpen: (viewId) {
          Navigator.of(context).pop();
          widget.onOpenSearchResult(viewId);
        },
      ),
    );
  }

  /// [inNewTab] forces EVERY hit into a fresh tab — the tab strip's `+` is the
  /// only caller that passes it, because "open an existing page" is that menu
  /// item's whole job. From an ordinary search the choice is per-click instead:
  /// holding Ctrl/Cmd asks for a tab (see `_SearchDialog._accelHeld`), which is
  /// how you follow a hit in ANOTHER workspace without the shell moving there.
  ///
  /// Folder hits reveal in the sidebar either way: there is no document behind
  /// a folder, so "open it in a tab" has nothing to mean.
  void _openSearch({String? initialQuery, bool inNewTab = false}) {
    final openInNewTab = widget.onOpenInNewTabById;
    // Captured under a different name: the closure below takes its own
    // `inNewTab` (the per-click one), which would otherwise shadow this.
    final forceNewTab = inNewTab;
    showDialog<void>(
      context: context,
      builder: (context) => _SearchDialog(
        onSearch: widget.onSearch,
        onSearchAll: widget.onSearchAllWorkspaces,
        views: widget.views,
        workspaceName: widget.selectedWorkspace?.name,
        workspaceId: widget.selectedWorkspace?.id,
        initialQuery: initialQuery,
        recents: widget.searchRecents,
        workspaces: [
          for (final w in widget.workspaces) (id: w.id, name: w.name),
        ],
        onOpenWorkspace: widget.onSelectWorkspaceById == null
            ? null
            : (id) {
                Navigator.of(context).pop();
                widget.onSelectWorkspaceById!(id);
              },
        onOpen: (viewId, workspaceId, {bool inNewTab = false}) {
          Navigator.of(context).pop();
          // The `+` menu forces it for every hit; an ordinary search leaves it
          // to whether Ctrl/Cmd was down at the click.
          final newTab = inNewTab || forceNewTab;
          final other = elsewhereWorkspace(
            workspaceId,
            widget.selectedWorkspace?.id,
          );
          final elsewhere = other != null;
          // Asked for in a TAB, and it lives somewhere else: hand the workspace
          // over with the id and stay where we are. The tab carries that
          // workspace and `_selectTab` switches only when it is clicked, so the
          // `+` can queue a page from elsewhere without moving you off what you
          // are doing. This used to fall into the branch below, which switched
          // immediately and opened in place — the tab was never created.
          if (elsewhere && newTab && openInNewTab != null) {
            openInNewTab(viewId, other);
            // Deliberately no reveal: the id is not in the tree we are showing,
            // and `revealInTree` would park the request until that workspace's
            // tree eventually loads — jumping the sidebar at some unrelated
            // later moment.
            return;
          }
          // Anything else from another workspace still goes through the host's
          // switch-then-open path; the in-place callback resolves the view out
          // of the CURRENT workspace's tree.
          if (other != null) {
            widget.onOpenSearchHit?.call(viewId, other);
          } else if (newTab && openInNewTab != null) {
            openInNewTab(viewId, null);
          } else {
            widget.onOpenSearchResult(viewId);
          }
          // Locate it in the sidebar too, not just open it. A hit arrives with
          // no context — the tree is what says where the page lives, and
          // leaving it parked wherever it happened to be scrolled makes every
          // search feel like it teleported you somewhere unrelated. Folder hits
          // already did this; pages did not.
          //
          // Safe for a cross-workspace hit: the request is pending until that
          // workspace's tree loads (see [_flushPendingReveal]).
          _tree.revealInTree(widget.views, viewId);
        },
        onReveal: (viewId, workspaceId) {
          Navigator.of(context).pop();
          // A folder in ANOTHER workspace has to be switched to before it can
          // be located — `revealInTree` alone just parks the request against a
          // tree that will never contain it, which is a click that does
          // nothing. Reported 2026-08-28, and it became the COMMON case the
          // moment search started spanning every workspace by default: the top
          // hits for a word like "claude" are folders named after it.
          final other = elsewhereWorkspace(
            workspaceId,
            widget.selectedWorkspace?.id,
          );
          // Switch FIRST, then park the reveal. `revealInTree` already knows
          // how to wait for a tree that has not arrived ("a search hit from
          // another workspace arrives before that workspace's views do") — what
          // was missing is that nothing ever made that workspace arrive, so the
          // request waited out its ~2s budget against a tree it was never in.
          if (other != null) widget.onSelectWorkspaceById?.call(other);
          _tree.revealInTree(widget.views, viewId);
        },
      ),
    );
  }

  /// Settings for the world we are in. Which world that is gets chosen on the
  /// account tile, NOT in here — the control that changes what a container
  /// holds cannot live inside it. (It did, and the question "these settings
  /// apply to whom?" had no answer while it did.)
  void _openSettings() {
    showDialog<void>(
      context: context,
      // Named so _setActiveConnection can close it if a switch ever reaches
      // this app from somewhere that is not the tile behind this barrier.
      routeSettings: const RouteSettings(name: 'settings'),
      builder: (context) => _SettingsDialog(
        onLoadAiSettings: widget.onLoadAiSettings,
        onLoadImportHistory: widget.onLoadImportHistory,
        onListAiModels: widget.onListAiModels,
        onSaveAiSettings: widget.onSaveAiSettings,
        onDeleteAiProvider: widget.onDeleteAiProvider,
        onLoadTokens: widget.onLoadTokens,
        onCreateToken: widget.onCreateToken,
        onRevokeToken: widget.onRevokeToken,
        userName: widget.userName,
        userEmail: widget.userEmail,
        onUpdateProfile: widget.onUpdateProfile,
        // A getter, not a value: the dialog is a route, so it does not rebuild
        // when the session upstream changes. Reading through it after an upload
        // is what makes the new picture appear without reopening Settings.
        currentAvatarUrl: _myAvatarUrl,
        onChangeAvatar: widget.onChangeAvatar,
        onRemoveAvatar: widget.onRemoveAvatar,
        onChangePassword: widget.onChangePassword,
        onDeleteAccount: widget.onDeleteAccount,
        appearance: widget.appearance,
        pageWidth: widget.pageWidth,
        reHostImages: widget.reHostImages,
        onReHostImagesChanged: widget.onReHostImagesChanged,
        showFormatBar: widget.showFormatBar,
        showPageTitle: widget.showPageTitle,
        onShowPageTitleChanged: widget.onShowPageTitleChanged,
        aiEnabled: widget.aiEnabled,
        onAiEnabledChanged: widget.onAiEnabledChanged,
        onShowFormatBarChanged: widget.onShowFormatBarChanged,
        onAppearanceChanged: widget.onAppearanceChanged,
        onImportWorkspace: widget.onImportWorkspaceZip == null
            ? null
            : () => _importWorkspaceFile(fromSettings: true),
        onLoadExportStats: widget.onLoadExportStats,
        onLoadCacheStats: widget.onLoadCacheStats,
        onClearMirrorCache: widget.onClearMirrorCache,
        onLoadQuarantinedCaches: widget.onLoadQuarantinedCaches,
        onExportQuarantinedCache: widget.onExportQuarantinedCache,
        onExportAllWorkspaces: widget.onExportAllWorkspaces == null
            ? null
            : () async {
                Navigator.of(context).pop(); // close settings first
                await widget.onExportAllWorkspaces!();
              },
      ),
    );
  }

  /// Switch to [view], flushing the editor's pending (debounced) edits first so
  /// the last typing reaches the backend before the editor is torn down on the
  /// document change. Awaits the flush so the local backend has applied the ops
  /// (and the cloud session has pushed them) BEFORE the host loads the next doc
  /// — otherwise the switch races the async apply and drops the edits. The host
  /// then drains the cloud session before disposing it.
  Future<void> _navigateToView(DocumentView view) async {
    // Locate the tapped row HERE, not as a side effect of the document
    // changing. `didUpdateWidget` moves the highlight when `selectedView`
    // changes, and that covers most taps — but not a tap on the page that is
    // already open: `_selectView` short-circuits it (needsBootstrapOnSelect),
    // `selectedView` never changes, and the highlight stayed wherever it was.
    //
    // Visible as: tap a folder (which DOES locate itself, in _toggleViewExpand),
    // then tap the first page under it — the page never lit up and the folder
    // stayed lit. Tapping a second page and back "fixed" it, because that round
    // trip does change `selectedView`.
    //
    // So: tapping a row locates that row, always. One rule, one place — the
    // folder path already worked this way, the page path only appeared to.
    _tree.locateRow(view.id);
    await _commandHook.flush();
    if (!mounted) return;
    // On the narrow shell the drawer covers the page it just opened — get out of
    // the way. Only for an actual page: toggling a folder open leaves the drawer
    // up, because you are still browsing the tree.
    _closeNavDrawer();
    widget.onSelectView(view);
  }

  /// Navigate to a page targeted by an internal `mica://page/<viewId>` link.
  void _openPageLink(String viewId) {
    for (final v in widget.views) {
      if (v.id == viewId) {
        _navigateToView(v);
        return;
      }
    }
  }

  /// Pick a workspace ZIP and rebuild it as a new workspace. [notion] forces
  /// Notion adaptation (otherwise auto-detected from the contents).
  Future<void> _importWorkspaceFile({
    bool fromSettings = false,
    bool notion = false,
  }) async {
    // Unsupported here (local mode) — the entry points below are hidden when
    // this is null, so reaching it means one of them was missed.
    final import = widget.onImportWorkspaceZip;
    if (import == null) return;
    final picked = await pickImportFile(zipOnly: true);
    if (picked == null || !mounted) return;
    if (fromSettings) {
      Navigator.of(context).pop(); // close settings before the import flow runs
    }
    await import(picked.name, picked.bytes, notion: notion);
  }

  /// Multi-select import into an existing workspace: .md files (plus images
  /// they reference) append pages at the root; ZIPs ride along as-is — the
  /// server expands nested archives.
  Future<void> _importFilesIntoWorkspace(WorkspaceEntry entry) async {
    final picked = await pickImportFiles();
    if (picked.isEmpty || !mounted) return;
    await widget.onImportTreeIntoEntry(entry, [
      for (final f in picked) ArchiveFile(f.name, f.bytes),
    ]);
  }

  /// Folder import (recursive) into an existing workspace: the folder's
  /// contents become pages, its subfolders the page tree.
  /// Ask whether a picked folder imports as ONE container ("wrap") or spills
  /// its contents straight into the destination ("spill", the default).
  /// Returns the `container` value for the import API, or null if cancelled.
  Future<String?> _askImportContainer() async {
    final l10n = context.l10n;
    var asFolder = false;
    final choice = await showDialog<bool>(
      context: context,
      builder: (context) => StatefulBuilder(
        builder: (context, setInner) => AlertDialog(
          title: Text(l10n.importFolderTitle),
          content: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              CheckboxListTile(
                contentPadding: EdgeInsets.zero,
                controlAffinity: ListTileControlAffinity.leading,
                value: asFolder,
                onChanged: (v) => setInner(() => asFolder = v ?? false),
                title: Text(l10n.importAsSingleFolder),
              ),
              Padding(
                padding: const EdgeInsets.only(left: 4, top: 4),
                child: Text(
                  l10n.importAsSingleFolderHint,
                  style: Theme.of(context).textTheme.bodySmall,
                ),
              ),
            ],
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.of(context).pop(),
              child: Text(l10n.commonCancel),
            ),
            FilledButton(
              onPressed: () => Navigator.of(context).pop(asFolder),
              child: Text(l10n.commonImport),
            ),
          ],
        ),
      ),
    );
    if (choice == null) return null; // cancelled
    return choice ? 'wrap' : 'spill';
  }

  Future<void> _importFolderIntoWorkspace(WorkspaceEntry entry) async {
    final picked = await pickImportFolder();
    if (picked.isEmpty || !mounted) return;
    final entries = <ArchiveFile>[];
    String? folderName;
    for (final f in picked) {
      // The picker includes the chosen folder itself as the first segment —
      // drop it so the folder's contents land under the container, and reuse
      // that segment as the container name (see ImportMode::IntoContainer).
      final parts = f.path.split('/');
      if (folderName == null && parts.length > 1 && parts.first.isNotEmpty) {
        folderName = parts.first;
      }
      entries.add(
        ArchiveFile(
          parts.length > 1 ? parts.sublist(1).join('/') : f.path,
          f.bytes,
        ),
      );
    }
    if (!mounted) return;
    final container = await _askImportContainer();
    if (container == null || !mounted) return;
    await widget.onImportTreeIntoEntry(
      entry,
      entries,
      sourceName: folderName,
      container: container,
    );
  }

  /// Multi-select import UNDER a folder: .md files (+ referenced images) become
  /// pages beneath it; ZIPs ride along as-is (server expands nested archives).
  Future<void> _importFilesIntoFolder(DocumentView folder) async {
    final picked = await pickImportFiles();
    if (picked.isEmpty || !mounted) return;
    await widget.onImportTreeIntoFolder(folder, [
      for (final f in picked) ArchiveFile(f.name, f.bytes),
    ]);
  }

  /// Folder import (recursive) UNDER a folder: the picked folder's name becomes
  /// a container under [folder]; its contents/subfolders rebuild the page tree.
  Future<void> _importFolderIntoFolder(DocumentView folder) async {
    final picked = await pickImportFolder();
    if (picked.isEmpty || !mounted) return;
    final entries = <ArchiveFile>[];
    String? folderName;
    for (final f in picked) {
      final parts = f.path.split('/');
      if (folderName == null && parts.length > 1 && parts.first.isNotEmpty) {
        folderName = parts.first;
      }
      entries.add(
        ArchiveFile(
          parts.length > 1 ? parts.sublist(1).join('/') : f.path,
          f.bytes,
        ),
      );
    }
    if (!mounted) return;
    final container = await _askImportContainer();
    if (container == null || !mounted) return;
    await widget.onImportTreeIntoFolder(
      folder,
      entries,
      sourceName: folderName,
      container: container,
    );
  }

  void _openAiDialog() {
    final documentId = widget.selectedBootstrap?.document.id;
    // No engine → no current-page target (an empty body would read as empty).
    final body = withEngineOrNull(() => _commandHook.documentMarkdown() ?? '');
    final onCurrentPage = body == null ? null : widget.onAiCurrentPage;
    showDialog<void>(
      context: context,
      builder: (context) => _AiDialog(
        canEdit: matchesEditRole(widget.selectedWorkspace?.role),
        hasWorkspace: widget.selectedWorkspace != null,
        onStream: widget.onAiStream,
        onNewPage: widget.onAiNewPage,
        onCurrentPage: onCurrentPage,
        currentPage: onCurrentPage == null
            ? null
            : (title: _pageTitle.text, markdown: body!),
        onNewWorkspace: widget.onAiNewWorkspace,
        onPreviewEdits: body == null ? null : widget.onAiPreviewEdits,
        readPageForEdits: widget.onAiPreviewEdits == null
            ? null
            : () {
                final source = _commandHook.captureAiSnapshot();
                if (source == null ||
                    documentId != widget.selectedBootstrap?.document.id) {
                  throw const AiEditConflict();
                }
                return (
                  title: _pageTitle.text,
                  markdown: _commandHook.documentMarkdown() ?? '',
                  source: source,
                );
              },
        onApplyEdits: (source, preview) async {
          if (documentId != widget.selectedBootstrap?.document.id) {
            throw const AiEditConflict();
          }
          await _commandHook.applyAiEdits(
            source,
            preview,
            canApply: () =>
                mounted && documentId == widget.selectedBootstrap?.document.id,
          );
        },
      ),
    );
  }
}
