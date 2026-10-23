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
    required this.onUpdateRootBlockText,
    required this.onAddBlock,
    required this.onUpdateBlock,
    required this.onDeleteBlock,
    required this.onMoveBlock,
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
    required this.onExportPageZip,
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
  final Future<void> Function(String text) onUpdateRootBlockText;
  final Future<void> Function(DocumentBlockKind kind, String text) onAddBlock;
  final Future<void> Function(
    DocumentBlock block,
    DocumentBlockKind kind,
    String text,
  )
  onUpdateBlock;
  final Future<void> Function(DocumentBlock block) onDeleteBlock;
  final Future<void> Function(DocumentBlock block, int targetIndex) onMoveBlock;
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
  final Future<Uint8List> Function() onExportPageZip;
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

  /// Drop the re-downloadable half of it; reports what is left, so the panel can
  /// show the result without waiting on a rebuild it never receives.
  final Future<LocalCacheStats> Function()? onClearMirrorCache;

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

class _WorkspaceViewState extends State<WorkspaceView> {
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
  final Set<String> _expandedViewIds = {};
  final PageTreeCache _pageTreeCache = PageTreeCache();

  /// The sidebar tree's expand/focus/pending-reveal state machine — see
  /// ui/sidebar_tree_controller.dart. The legacy fields around it
  /// (_expandedViewIds, _focusedNavId, _rootFocused, _pendingRevealId,
  /// _revealAttempts) are synced view-model copies for the build helpers;
  /// every mutation goes through the controller.
  late final SidebarTreeController _tree;
  // The view whose sidebar name is in inline-rename (edit) mode — null = none.
  // Set right after creating a page/folder (so the user types its name
  // immediately, no dialog) or from the row's "重命名" action; the matching row
  // renders a focused TextField instead of the name Text.
  /// Inline-rename (create-then-name) state — see
  /// ui/inline_rename_controller.dart. [_renamingViewId] is the synced copy.
  late final InlineRenameController _inlineRename;
  String? _renamingViewId;
  // The sidebar node the user last "located" — a page OR a folder (folders never
  // reach `selectedView`, so this is the only handle on a focused folder). Drives
  // where the top New-page/New-folder buttons create: inside a focused folder, or
  // beside a focused page. Kept in sync with the open doc in didUpdateWidget so a
  // single highlight follows the last interaction. Null = create at the root.
  String? _focusedNavId;
  // The user explicitly "located" the root by tapping the tree's blank area:
  // clears the located node (and its row highlight) even while a doc is open,
  // so the top New-page/New-folder buttons create at the workspace root. Any
  // row tap / opened doc turns this back off. Distinct from `_focusedNavId ==
  // null`, which merely falls back to the open doc.
  bool _rootFocused = false;
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
    _syncTreeStateQuiet();
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
      if (open != null) _revealAncestors(open);
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
      _revealAncestors(bootstrap.view.id);
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
    if (identical(_activeEditorFlush, _boundExitFlush))
      _activeEditorFlush = null;
    super.dispose();
  }

  void _syncRenameState() {
    if (!mounted) return;
    setState(() => _renamingViewId = _inlineRename.renamingId);
  }

  /// Copy the tree controller state into this State's view-model fields,
  /// so every build helper reading them sees the same values it did
  /// before the controller existed.
  void _syncTreeState() {
    if (!mounted) return;
    setState(() {
      _expandedViewIds
        ..clear()
        ..addAll(_tree.expandedIds);
      _focusedNavId = _tree.focusedNavId;
      _rootFocused = _tree.rootFocused;
      _pendingRevealId = _tree.pendingRevealId;
    });
  }

  /// The initState variant: same copy, no setState (legal only before the
  /// first build; the listener added after this call does the notifying).
  void _syncTreeStateQuiet() {
    _expandedViewIds.addAll(_tree.expandedIds);
    _focusedNavId = _tree.focusedNavId;
    _rootFocused = _tree.rootFocused;
    _pendingRevealId = _tree.pendingRevealId;
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
          _focusRoot();
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
          onTap: _focusRoot,
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

  Widget _pageTree(BuildContext context, bool canEdit) {
    // Which of the four states this is, is decided by `pageTreeStateFor` in
    // ui/page_tree_state.dart, NOT by inline ifs here — that function is the
    // only part of this rule a test can reach, because `_pageTree` cannot be
    // built without the 97-parameter widget around it. Re-inlining a branch
    // here silently un-tests it.
    final treeState = pageTreeStateFor(
      hasWorkspace: widget.selectedWorkspace != null,
      viewsEmpty: widget.views.isEmpty,
      treePending: widget.treePending,
    );

    if (treeState == PageTreeState.noWorkspace) {
      return EmptyState(
        icon: Icons.ads_click,
        title: context.l10n.workspaceRowSelectWorkspace,
        detail: context.l10n.sidebarSelectWorkspaceDetail,
      );
    }

    if (treeState == PageTreeState.skeleton) {
      return const PageTreeSkeleton();
    }

    if (treeState == PageTreeState.empty) {
      return EmptyState(
        icon: Icons.note_add,
        title: context.l10n.sidebarNoPagesTitle,
        detail: context.l10n.sidebarNoPagesDetail,
        // The rule's other half (design 19): an empty tree should offer the way
        // OUT of being empty, not just describe it. Offered only when the user
        // may actually create — a dead button is worse than none.
        actionLabel: canEdit ? context.l10n.newPage : null,
        onAction: canEdit ? () => _createLocated(folder: false) : null,
      );
    }

    // The single highlighted row = the located node (last-tapped folder/page),
    // falling back to the open doc. So a focused folder highlights just like the
    // open page, and the two never light up at once.
    //
    // Released location = NO highlight, deliberately. The highlight's whole job
    // is to answer "where will the New buttons put this"; leaving it lit on the
    // open page after the location was released answers a question nobody asked
    // and lies about the one that was.
    //
    // (This was briefly the other way round — falling back to the open document
    // so the row stayed lit. It reads better in a screenshot and worse in use:
    // you cannot tell a located row from a merely-open one, which is the single
    // thing this highlight exists to tell you.)
    final activeId = _rootFocused
        ? null
        : (_focusedNavId ?? widget.selectedView?.id);
    // Tapping the tree's blank area (below/around the rows) deselects the
    // located node so the top New buttons create at the root. Rows keep their
    // own tap handlers (they win the gesture arena); only taps that miss a row
    // A reveal is requested before the row it wants exists — ancestors expand
    // in the same setState, and a cross-workspace hit waits on that tree
    // loading. Retrying after each frame is what makes both land.
    if (_pendingRevealId != null) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) _flushPendingReveal();
      });
    }
    // Once, not once per row: each row needs to know its NEIGHBOUR's depth, and
    // rebuilding the whole flattened tree inside the map would make drawing the
    // sidebar quadratic — on the 3700-page workspace that is not a nicety.
    final treeRows = _visibleDocumentTree();
    // reach this. Opaque so the empty space below the last row is hittable.
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: () {
        _clearTreeSelection();
        _focusRoot();
      },
      // Esc ALSO clears the selection — via [_escKeyEvent], a global
      // HardwareKeyboard handler, NOT any binding here. Three focus-based
      // bindings were built first and all three were dead (a local
      // `Shortcuts`/`Actions` pair, the same plus a requested FocusNode, and
      // `_appShortcuts` where Ctrl+N works); the probe story is in
      // `docs/features/shortcuts.md`. Short version: when Esc arrives, focus sits on the
      // page route's own scope — an ANCESTOR of this shell — and key dispatch
      // walks from the focused node UPWARD, so nothing registered in here is
      // ever on the path. The feature shipped without Esc for that reason
      // (a documented key that does nothing is worse than no key), until the
      // user asked for it (2026-08-27) — which is what the global handler's
      // dialog/rename caveats were traded against.
      child: _buildTreeList(treeRows, canEdit, activeId),
    );
  }

  Widget _buildTreeList(
    List<({DocumentView view, int depth, bool hasChildren})> treeRows,
    bool canEdit,
    String? activeId,
  ) {
    return
    // The auto-scroll region wraps the list EXACTLY, so its box is the
    // viewport the edge zones are measured against.
    DragAutoScrollRegion(
      enabled: _draggingTree,
      controller: _treeScroll,
      child: CustomScrollView(
        controller: _treeScroll,
        slivers: [
          SliverList(
            delegate: SliverChildBuilderDelegate(
              (context, index) {
                final item = treeRows[index];
                // The NEXT row's depth bounds how far left an `after` drop under this
                // row may pop out; null means "this is the last row", which is what
                // lets the last row reach the workspace root.
                final nextDepth = index + 1 < treeRows.length
                    ? treeRows[index + 1].depth
                    : null;
                final row = DocumentListItem(
                  key: ValueKey(item.view.id),
                  view: item.view,
                  depth: item.depth,
                  hasChildren: item.hasChildren,
                  revealToggle: _navHovered,
                  isCollapsed: !_expandedViewIds.contains(item.view.id),
                  isSelected: item.view.id == activeId,
                  isMultiSelected: _selection.contains(item.view.id),
                  // Read-only trees get no selection: nothing in the batch menu is
                  // something a viewer could do.
                  onSelectClick: canEdit
                      ? ({required extendRange}) => _handleSelectClick(
                          item.view,
                          extendRange: extendRange,
                        )
                      : null,
                  onPlainTap: () => _anchorTreeSelection(item.view),
                  batchActions:
                      canEdit && _selection.actsOnWholeSelection(item.view.id)
                      ? () => _batchMenuItems(item.view)
                      : null,
                  canEdit: canEdit,
                  isRenaming: item.view.id == _renamingViewId,
                  onToggle: () => _toggleViewExpand(item.view),
                  onPressed: () => _navigateToView(item.view),
                  onCreateChild: () =>
                      _createInFolder(item.view, folder: false),
                  onCreateChildFolder: () =>
                      _createInFolder(item.view, folder: true),
                  onExportFolder: widget.onExportFolderZip == null
                      ? null
                      : () => _exportFolderFile(item.view),
                  // Import md / images / a nested folder UNDER this folder — both worlds
                  // (server validates parent_view_id; local prefixes the tree).
                  onImportFilesIntoFolder: item.view.objectType == 'folder'
                      ? () => _importFilesIntoFolder(item.view)
                      : null,
                  onImportFolderIntoFolder: item.view.objectType == 'folder'
                      ? () => _importFolderIntoFolder(item.view)
                      : null,
                  // Cross-workspace move/copy — cloud-only (onTransfer is null in a
                  // local workspace, which hides both entries). Works for pages and
                  // folders alike; the folder carries its subtree server-side.
                  onTransferMove: widget.onTransfer == null
                      ? null
                      : () => widget.onTransfer!([item.view], false),
                  onTransferCopy: widget.onTransfer == null
                      ? null
                      : () => widget.onTransfer!([item.view], true),
                  onClone: () => widget.onCloneView(item.view),
                  onRename: () => _promptRenameView(item.view),
                  onSetIcon: widget.onSetViewIcon == null
                      ? null
                      : () => widget.onSetViewIcon!(item.view),
                  // Pages only — a folder has no document to open in a tab. Also null
                  // in the local world, where the host passes no tab callbacks.
                  onOpenInNewTab:
                      (widget.onOpenInNewTab == null ||
                          item.view.objectType == 'folder')
                      ? null
                      : () => widget.onOpenInNewTab!(item.view),
                  onRenameSubmit: (name) => _commitRename(item.view, name),
                  onRenameCancel: _cancelRename,
                  onDelete: () => widget.onDeleteView(item.view),
                );
                if (!canEdit) {
                  return Padding(
                    key: ValueKey(item.view.id),
                    padding: const EdgeInsets.only(bottom: 8),
                    child: row,
                  );
                }
                // `childWhenDragging` only fades the row under the pointer. The rest
                // of the selection is travelling too, so it fades with it — otherwise
                // four rows sit there looking like they stayed behind.
                final travelling =
                    _draggingTree &&
                    _selection.contains(item.view.id) &&
                    _selection.length > 1;
                return KeyedSubtree(
                  key: ValueKey(item.view.id),
                  child: _draggableTreeRow(
                    item.view,
                    item.depth,
                    nextDepth,
                    travelling ? Opacity(opacity: 0.4, child: row) : row,
                  ),
                );
              },
              childCount: treeRows.length,
              findChildIndexCallback: (key) => key is ValueKey<String>
                  ? _pageTreeCache.visibleIndexOf(key.value)
                  : null,
            ),
          ),
          // The root drop zone. Every other drop target takes its parent from the
          // ROW it sits on (`before`/`after` mean "sibling of that row"), so when
          // the bottom of the tree is a nested row there was no way to say "below
          // all of this, at the workspace root" — reported as 「放在整个页面树最底
          // 部，让他与根路径齐平，似乎做不到」.
          //
          // The blank area under the tree already MEANS root: tapping it releases
          // the located node, and the New buttons then create at the root.
          // Accepting a drop there says the same thing with a drag, so there is
          // nothing new to learn.
          //
          // `SliverFillRemaining(hasScrollBody: false)` and not a trailing box of
          // some chosen height: it takes the LEFTOVER viewport when the tree is
          // short — which is the whole blank area the user was aiming at — and
          // only its own height when the tree overflows, so it adds no phantom
          // scroll extent to a long tree. The first attempt was a 40px box and
          // the drop silently did nothing, because the pointer was below it in
          // blank space that still belonged to no one.
          if (_draggingTree)
            SliverFillRemaining(hasScrollBody: false, child: _rootDropZone()),
        ],
      ),
    );
  }

  /// "Move to the workspace root, at the end." Exists only during a drag.
  ///
  /// Deliberately NOT the full depth picker (drag left/right to choose the
  /// nesting level, the way Notion does). That is the general answer, and it is
  /// a rework rather than an addition: depth here is derived from which row's
  /// DragTarget the pointer is over, and a DragTarget cannot see the pointer's
  /// x — it would take one overlay tracking the pointer and computing (index,
  /// depth) itself. It would also have to respect this tree's invariant that
  /// only a folder may hold children, so the indicator must snap to the LEGAL
  /// depths rather than to every depth, or it draws a line you cannot drop on.
  /// Worth doing if picking a MIDDLE level turns out to be wanted too; the
  /// case actually reported is the root.
  Widget _rootDropZone() {
    return DragTarget<List<DocumentView>>(
      hitTestBehavior: HitTestBehavior.opaque,
      // A row already at the root, dropped here, is a reorder to the end —
      // a legitimate thing to want, so it is accepted rather than refused.
      onWillAcceptWithDetails: (_) => true,
      onAcceptWithDetails: (details) {
        _clearTreeSelection();
        widget.onReorderViews(null, rootDropOrder(widget.views, details.data));
      },
      builder: (context, candidate, rejected) {
        // No label any more. A caption explaining that this area means "root"
        // was a way of describing the model instead of making it operable —
        // and it only ever spoke about ONE level. Now the same dot that walks
        // out to level 0 under the last row does the explaining, and this zone
        // is just the rest of that gesture's surface.
        return Align(
          alignment: Alignment.topLeft,
          child: Padding(
            padding: const EdgeInsets.only(top: 6),
            child: _DropIndicator(
              inset: dropIndicatorInset(0),
              // This zone accepts everything, so `blocked` cannot arise here —
              // still routed through the same enum rather than a bool, so there
              // is one vocabulary for "what is the indicator saying".
              state: candidate.isNotEmpty
                  ? DropIndicatorState.allowed
                  : DropIndicatorState.idle,
            ),
          ),
        );
      },
    );
  }

  /// Placeholder rows for a tree that has not arrived yet.
  ///
  /// Static on purpose: an animated shimmer would be a second thing running
  /// during the very frames this is meant to keep cheap, and the one controlled
  /// comparison anyone cites found skeletons no better liked than a spinner.
  /// The job here is only to not lie about the workspace being empty.
  /// Tapping the tree's blank area deselects the located node: nothing is
  /// highlighted and the top New-page/New-folder buttons create at the
  /// workspace root (the only way to reach the root while a doc stays open).
  void _focusRoot() => _tree.focusRoot();

  /// Wrap a page row so it can be dragged to reorder among its siblings (its
  /// subtree follows, since children render under their parent). Touch moves
  /// scroll the tree; long-press then move reorders. Mouse drags start at once.
  /// The top/bottom half of each sibling row is a drop slot (before/after).
  Widget _draggableTreeRow(
    DocumentView view,
    int depth,
    int? nextDepth,
    Widget row,
  ) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 8),
      child: NavigationDraggable<List<DocumentView>>(
        // Grabbing a row inside a multi-selection drags the whole selection —
        // the same rule the context menu uses, so what you get is what the
        // highlight already showed you.
        data: _rowsActedOnBy(view.id),
        dragAnchorStrategy: pointerDragAnchorStrategy,
        // No onDragUpdate: it is mounted-guarded, and auto-scrolling this very
        // list unmounts the row being dragged. DragAutoScrollRegion reads the
        // raw pointer stream instead — see its doc comment.
        onDragStarted: () => setState(() => _draggingTree = true),
        onDragEnd: (_) => _endTreeDrag(),
        onDraggableCanceled: (_, _) => _endTreeDrag(),
        onDragCompleted: _endTreeDrag,
        feedback: _dragFeedback(_rowsActedOnBy(view.id)),
        childWhenDragging: Opacity(opacity: 0.4, child: row),
        child: Stack(
          children: [
            row,
            // Drop zones exist only during a drag, so they never block taps.
            if (_draggingTree)
              Positioned.fill(
                child: Column(
                  children: [
                    // The indicator is drawn at the depth the row would LAND
                    // at, which for before/after is this row's own depth (they
                    // mean "become its sibling").
                    Expanded(
                      flex: 3,
                      child: _dropSlot(view, _DropMode.before, depth),
                    ),
                    Expanded(
                      flex: 4,
                      child: _dropSlot(view, _DropMode.into, depth),
                    ),
                    // `nextDepth` is what bounds how far left this drop may
                    // pop out — see [reparentLevelFor].
                    Expanded(
                      flex: 3,
                      child: _dropSlot(
                        view,
                        _DropMode.after,
                        depth,
                        nextDepth: nextDepth,
                      ),
                    ),
                  ],
                ),
              ),
          ],
        ),
      ),
    );
  }

  /// The chip that follows the pointer during a tree drag.
  ///
  /// One row: name and icon, as it always was. Several: the SAME chip with two
  /// stacked cards peeking out behind it and a count underneath — a deck, not a
  /// list. The alternative (render all N rows) was rejected on sight: five rows
  /// is a panel that covers the drop indicator you are aiming with, and the
  /// indicator is the only thing telling you where the drop lands.
  ///
  /// The count is what makes the deck honest, because the cards behind are a
  /// fixed two whether you grabbed three rows or thirty.
  Widget _dragFeedback(List<DocumentView> rows) {
    final tokens = MicaTheme.of(context);
    final lead = rows.isEmpty ? null : rows.first;
    Widget card({double inset = 0, double opacity = 1, Widget? child}) =>
        Padding(
          padding: EdgeInsets.only(left: inset, top: inset),
          child: Opacity(
            opacity: opacity,
            child: Material(
              elevation: 6,
              borderRadius: BorderRadius.circular(6),
              child: Container(
                padding: const EdgeInsets.symmetric(
                  horizontal: 12,
                  vertical: 8,
                ),
                constraints: const BoxConstraints(maxWidth: 260, minWidth: 120),
                decoration: BoxDecoration(
                  color: tokens.surface.overlay,
                  borderRadius: BorderRadius.circular(6),
                ),
                child: child ?? const SizedBox(height: 20),
              ),
            ),
          ),
        );

    final top = card(
      inset: rows.length > 1 ? 8 : 0,
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(
            lead != null && lead.objectType == 'folder'
                ? Icons.folder_outlined
                : Icons.description_outlined,
            size: 18,
            color: tokens.accent.primary,
          ),
          const SizedBox(width: 8),
          Flexible(
            child: Text(lead?.name ?? '', overflow: TextOverflow.ellipsis),
          ),
        ],
      ),
    );
    if (rows.length <= 1) return top;
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Stack(
          children: [
            // Back to front, so the named card ends up on top.
            card(opacity: 0.45),
            card(inset: 4, opacity: 0.7),
            top,
          ],
        ),
        const SizedBox(height: 6),
        Material(
          color: Colors.transparent,
          child: Padding(
            padding: const EdgeInsets.only(left: 8),
            child: Text(
              context.l10n.batchDragLabel(rows.length),
              style: TextStyle(fontSize: 12, color: tokens.text.muted),
            ),
          ),
        ),
      ],
    );
  }

  /// Ends the drag. The auto-scroll stops with it: `DragAutoScrollRegion` is
  /// driven by `_draggingTree`, so clearing the flag is what turns it off.
  void _endTreeDrag() {
    setState(() => _draggingTree = false);
  }

  // ── Ctrl/Shift multi-selection ─────────────────────────────────────────────

  /// A Ctrl (toggle) or Shift (extend) click landed on [view].
  void _handleSelectClick(DocumentView view, {required bool extendRange}) {
    setState(() {
      _selection.click(
        view.id,
        extendRange: extendRange,
        // The tree AS DISPLAYED: a Shift range is only meaningful over rows the
        // user can see. A collapsed folder's children are simply absent.
        visibleRows: [
          for (final row in _visibleDocumentTree())
            (id: row.view.id, parentId: row.view.parentViewId),
        ],
      );
    });
  }

  /// An unmodified click landed on [view]: drop the selection, but leave the
  /// row as the anchor a following Shift-click measures from.
  ///
  /// A no-op when it is already the state, so it can sit in the hot path of
  /// every row tap without causing a rebuild per click.
  void _anchorTreeSelection(DocumentView view) {
    if (_selection.anchorOn(view.id)) setState(() {});
  }

  /// Drop the selection AND the anchor — for a click that hit no row (the blank
  /// area below the tree), or after a drop relocated the rows.
  void _clearTreeSelection() {
    if (_selection.clear()) setState(() {});
  }

  /// The rows a gesture on [id] acts on: the whole selection when [id] is part
  /// of a multi-selection, otherwise just [id]. Nested rows are dropped — see
  /// [independentSelection].
  ///
  /// Returned in TREE order, not selection order, so a batch lands at the
  /// destination looking the way it looked in the sidebar. (Selection order is
  /// whatever sequence the user happened to Ctrl-click in, which nobody
  /// remembers by the time they see the result.)
  List<DocumentView> _rowsActedOnBy(String id) {
    if (!_selection.actsOnWholeSelection(id)) {
      final one = _pageTreeCache.viewById(id);
      return one == null ? const [] : [one];
    }
    final inTreeOrder = [
      for (final row in _visibleDocumentTree())
        if (_selection.contains(row.view.id)) row.view.id,
    ];
    final kept = independentSelection(
      inTreeOrder,
      (child) => _pageTreeCache.viewById(child)?.parentViewId,
    );
    return [for (final k in kept) ?_pageTreeCache.viewById(k)];
  }

  /// The context menu shown when a right-click lands inside a multi-selection.
  ///
  /// Counts come from [_rowsActedOnBy], not from `_selection.length`:
  /// selecting a folder and a page inside it is two highlighted rows but ONE
  /// thing to move, and a menu that says 3 while doing 2 is the kind of lie
  /// that only shows up after the deed.
  List<PopupMenuEntry<VoidCallback>> _batchMenuItems(DocumentView row) {
    final rows = _rowsActedOnBy(row.id);
    final l10n = context.l10n;
    final transfer = widget.onTransfer;
    return [
      if (transfer != null) ...[
        PopupMenuItem<VoidCallback>(
          value: () => transfer(rows, false),
          child: _MenuRow(
            icon: Icons.drive_file_move_outlined,
            label: l10n.batchTransferMove(rows.length),
          ),
        ),
        PopupMenuItem<VoidCallback>(
          value: () => transfer(rows, true),
          child: _MenuRow(
            icon: Icons.copy_all_outlined,
            label: l10n.batchTransferCopy(rows.length),
          ),
        ),
        const PopupMenuDivider(),
      ],
      PopupMenuItem<VoidCallback>(
        value: () => _deleteSelection(rows),
        child: _MenuRow(
          icon: Icons.delete_outline,
          label: l10n.batchDelete(rows.length),
          danger: true,
        ),
      ),
    ];
  }

  /// Trash a whole selection after confirming, in ONE request.
  ///
  /// Confirmed even though the destination is the recycle bin: a per-row delete
  /// is one row you are looking at, while this one can carry folders whose
  /// contents are off screen — the count in the prompt is the only place those
  /// are ever mentioned.
  Future<void> _deleteSelection(List<DocumentView> rows) async {
    if (rows.isEmpty) return;
    final l10n = context.l10n;
    final ok = await showDestructiveConfirm(
      context,
      title: l10n.batchDelete(rows.length),
      body: l10n.batchDeleteConfirm(rows.length),
      confirmLabel: l10n.batchDelete(rows.length),
      cancelLabel: l10n.commonCancel,
    );
    if (!ok || !mounted) return;
    _clearTreeSelection();
    await widget.onDeleteViews([for (final r in rows) r.id]);
  }

  /// The level the active `after` drop would land at, or null when the pointer
  /// is not over a zone that can re-parent. Tracked in state because the
  /// indicator has to redraw as the pointer moves sideways.
  int? _dropLevel;

  Widget _dropSlot(
    DocumentView target,
    _DropMode mode,
    int depth, {
    int? nextDepth,
  }) {
    // A Builder so `onMove` can measure against THIS SLOT's box. Using the
    // State's own `context` measured against the whole pane instead, which put
    // every pointer position past the last level and pinned the indicator at
    // the row's own depth — the level never moved, in either direction.
    return Builder(
      builder: (slotContext) => DragTarget<List<DocumentView>>(
        // The whole zone must be droppable; DragTarget defaults to deferToChild,
        // which would limit the hit area to the thin indicator. These overlays
        // only exist during a drag, so opaque is safe (no taps to intercept).
        hitTestBehavior: HitTestBehavior.opaque,
        // Refused if the target sits inside ANY of the dragged rows: dropping a
        // batch into one of its own folders is the same impossibility as with
        // one row, and only needs to be true of one of them to be true.
        onWillAcceptWithDetails: (details) => TreeDropPlanner(
          widget.views,
        ).canDrop(details.data, target, into: mode == _DropMode.into),
        // Only `after` re-parents, and only sideways. `details.offset` IS the
        // pointer here because the Draggable uses `pointerDragAnchorStrategy`;
        // with the default strategy it would be the feedback's top-left corner
        // and every level would read one chip-width to the right.
        onMove: mode != _DropMode.after
            ? null
            : (details) {
                final box = slotContext.findRenderObject() as RenderBox?;
                if (box == null || !box.hasSize) return;
                final local = box.globalToLocal(details.offset);
                final level = reparentLevelFor(
                  pointerX: local.dx,
                  rowDepth: depth,
                  nextRowDepth: nextDepth,
                );
                if (level != _dropLevel) setState(() => _dropLevel = level);
              },
        onLeave: (_) {
          if (_dropLevel != null) setState(() => _dropLevel = null);
        },
        onAcceptWithDetails: (details) {
          // A level is just a different target for the SAME `after` drop:
          // "sibling of my ancestor at depth L".
          final resolved = TreeDropPlanner(widget.views).resolveAfterTarget(
            target,
            depth,
            mode == _DropMode.after ? _dropLevel : null,
          );
          setState(() => _dropLevel = null);
          _handleDrop(details.data, resolved, mode);
          // The rows have landed somewhere new; keeping them highlighted would
          // leave a selection describing where they used to be.
          _clearTreeSelection();
        },
        builder: (context, candidate, rejected) {
          // `rejected` is how a DragTarget says "something is over me and I
          // refused it" — the case that used to draw nothing at all.
          final state = candidate.isNotEmpty
              ? DropIndicatorState.allowed
              : rejected.isNotEmpty
              ? DropIndicatorState.blocked
              : DropIndicatorState.idle;
          if (mode == _DropMode.into) {
            // Nesting: highlight the whole target row. Warning-coloured when the
            // nest is refused — dropping a folder inside its own subtree is the
            // one people actually try, and it is exactly where silence was worst.
            final tokens = MicaTheme.of(context);
            final edge = switch (state) {
              DropIndicatorState.idle => Colors.transparent,
              DropIndicatorState.allowed => tokens.accent.primary,
              DropIndicatorState.blocked => tokens.status.warning,
            };
            return Container(
              decoration: BoxDecoration(
                borderRadius: BorderRadius.circular(6),
                border: Border.all(color: edge, width: 2),
                color: edge == Colors.transparent
                    ? Colors.transparent
                    : edge.withValues(alpha: 0.08),
              ),
            );
          }
          // Indented to the level this drop lands at, with a terminal dot at the
          // head — AFFiNE's tree indicator, which is Atlassian's
          // pragmatic-drag-and-drop shape: an 8px hollow circle, and a 2px line
          // leaving its centre.
          //
          // The dot is not decoration. It is the thing you aim with: while
          // dragging under the last row of a subtree, moving the pointer left
          // steps the dot out one level at a time, and where it lands is where
          // the row lands.
          //
          // When blocked, the mark is drawn at the level you were ASKING for, in
          // the warning colour — the desired instruction, not nothing.
          return Align(
            alignment: mode == _DropMode.before
                ? Alignment.topLeft
                : Alignment.bottomLeft,
            child: _DropIndicator(
              inset: dropIndicatorInset(
                state == DropIndicatorState.idle ? depth : _dropLevel ?? depth,
              ),
              state: state,
            ),
          );
        },
      ),
    );
  }

  void _handleDrop(
    List<DocumentView> dragged,
    DocumentView target,
    _DropMode mode,
  ) {
    if (dragged.isEmpty) return;
    final moving = {for (final d in dragged) d.id};
    if (mode == _DropMode.into) {
      final children =
          widget.views
              .where(
                (v) => v.parentViewId == target.id && !moving.contains(v.id),
              )
              .toList()
            ..sort((a, b) => a.position.compareTo(b.position));
      children.addAll(dragged);
      setState(() => _expandForChildOf(target.id)); // reveal the drop target
      widget.onReorderViews(target.id, children);
      return;
    }

    final parentId = target.parentViewId;
    final siblings =
        widget.views.where((v) => v.parentViewId == parentId).toList()
          ..sort((a, b) => a.position.compareTo(b.position));
    // The ordering itself is a tested pure function: getting "remove first,
    // then read the index" the wrong way round puts a block one slot off,
    // which is invisible for one row and obvious for five.
    final order = siblingOrderAfterDrop(
      siblings: [for (final s in siblings) s.id],
      dragged: [for (final d in dragged) d.id],
      targetId: target.id,
      before: mode == _DropMode.before,
    );
    if (order == null) return;
    final byId = {
      for (final v in widget.views) v.id: v,
      for (final d in dragged) d.id: d,
    };
    widget.onReorderViews(parentId, [for (final id in order) ?byId[id]]);
  }

  List<({DocumentView view, int depth, bool hasChildren})>
  _visibleDocumentTree() {
    _pageTreeCache.updateViews(widget.views);
    return _pageTreeCache.visibleRows(_expandedViewIds);
  }

  /// Toggling a folder also locates it (the "target" for the top
  /// New-page/New-folder buttons); expansion lives in the tree controller.
  void _toggleViewExpand(DocumentView view) {
    _tree.toggle(view.id);
    _pruneSelectionToVisible();
  }

  /// Drop selected rows that are no longer on screen.
  ///
  /// Collapsing a folder hides its children, and a selection is only honest
  /// while you can see what is in it: without this, "删除 5 项" would delete
  /// three, because everything downstream reads the VISIBLE tree. Keeping the
  /// hidden ones instead is worse — that is a batch acting on rows the user
  /// folded away and stopped thinking about.
  void _pruneSelectionToVisible() {
    if (_selection.isEmpty) return;
    final visible = {for (final row in _visibleDocumentTree()) row.view.id};
    if (_selection.retainVisible(visible)) setState(() {});
  }

  /// `workspace/folder/.../page` for the open page — GitHub's "copy path", for
  /// telling someone (or an agent) WHERE something is.
  ///
  /// The workspace leads because a path without it is ambiguous the moment
  /// there is more than one, and whoever you paste it to cannot guess which you
  /// meant. Segments and separator rationale live on [pagePathSegments], which
  /// the breadcrumb reads too.
  String? _pagePath() => pagePathSegments(
    workspaceName: widget.selectedWorkspace?.name,
    view: widget.selectedView,
    views: widget.views,
  )?.join('/');

  /// Put [_pagePath] on the clipboard.
  Future<bool> _copyPagePath() async {
    final l10n = context.l10n;
    final path = _pagePath();
    if (path == null) {
      ScaffoldMessenger.of(
        context,
      ).showSnackBar(SnackBar(content: Text(l10n.pageOpenFirst)));
      return false;
    }
    final ok = await copyTextToClipboard(path);
    if (!mounted) return ok;
    // SUCCESS says nothing here. The button that was just pressed turns into a
    // green check where the cursor already is (GitHub's "Copied!" affordance) —
    // a snackbar for a working copy throws a black bar across the bottom of the
    // window to report that the expected thing happened, which is the loudest
    // possible way to say the least.
    //
    // FAILURE still speaks: it is unexpected, it needs words, and there is no
    // icon state that can carry "the clipboard refused".
    if (!ok) {
      ScaffoldMessenger.of(
        context,
      ).showSnackBar(SnackBar(content: Text(l10n.pageCopyFailed)));
    }
    return ok;
  }

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
  String? _pendingRevealId;

  /// Expand a view's ancestors and scroll it into view in the sidebar.
  ///
  /// The expand half already existed for folder hits; the scroll half did not,
  /// so "locate this" quietly did nothing whenever the row sat below the fold —
  /// which on a 434-page workspace is most of them.
  ///
  /// Kept as a PENDING id rather than scrolling here, because the row usually
  /// does not exist yet at this moment: its ancestors are expanded in the same
  /// `setState`, and a cross-workspace hit has to wait for the other
  /// workspace's tree to load at all. [_flushPendingReveal] retries per frame
  /// until the row is on screen.
  /// Locate [viewId] when its tree has it, and remember it either way —
  /// the row may belong to a workspace whose views have not arrived yet.
  void _revealInTree(String viewId) => _tree.revealInTree(widget.views, viewId);

  /// Scroll the pending row into view, then stop asking.
  ///
  /// Computed from the row's INDEX, not from its widget. The first version
  /// asked `Scrollable.ensureVisible` for the row's `BuildContext` via a
  /// GlobalKey — which is null for exactly the rows that need scrolling: the
  /// tree is a lazy `ListView`, so a row below the fold has no element at all.
  /// It worked only for rows already on screen, i.e. never usefully. Reported
  /// as "看着没效果".
  ///
  /// Rows are a uniform height inside one workspace ([_treeRowExtent]), so
  /// index × extent lands within a pixel or two, and centring absorbs that.
  void _flushPendingReveal() {
    final id = _tree.pendingRevealId;
    if (id == null) return;
    if (!_tree.bumpRevealAttempt()) {
      // Gave up after about 2s of frames (deleted page, a workspace that
      // failed to load); the controller stopped rather than run a
      // callback every frame forever.
      return;
    }
    // The tree may only have arrived now (workspace switch): expand ancestors,
    // then WAIT A FRAME before scrolling.
    //
    // Scrolling in the same pass lands short, which is what the user saw:
    // expanding adds rows, so the list gets taller — but `maxScrollExtent`
    // still describes the layout measured BEFORE those rows existed, and the
    // clamp below pins the target to that stale, smaller maximum. The shortfall
    // is exactly the height of the rows just revealed, which is why it looked
    // like "scrolled, but you still have to drag a bit further".
    if (widget.views.any((v) => v.id == id) && !_tree.isExpanded(id)) {
      _revealFolder(id);
      return; // re-entered next frame, against the grown list
    }
    final rows = _visibleDocumentTree();
    final index = rows.indexWhere((r) => r.view.id == id);
    // Not in the tree yet, or still inside a parent this frame's expand has not
    // rendered through — keep waiting.
    if (index < 0 || !_treeScroll.hasClients) return;

    _tree.clearPendingReveal();
    final position = _treeScroll.position;
    unawaited(
      _treeScroll.animateTo(
        treeRevealOffset(
          index: index,
          rowExtent: _treeRowExtent,
          viewportHeight: position.viewportDimension,
          maxScrollExtent: position.maxScrollExtent,
          minScrollExtent: position.minScrollExtent,
        ),
        duration: const Duration(milliseconds: 200),
        curve: Curves.easeOut,
      ),
    );
  }

  /// Both editable drag wrappers and read-only rows add an 8px gap to the
  /// DocumentListItem's 38px floor. Keep locating distant rows in this same unit.
  double get _treeRowExtent => 46.0;

  /// Locate [viewId] and expand it plus its ancestors; an id not in this
  /// workspace's tree does nothing. Logic in the tree controller.
  void _revealFolder(String viewId) => _tree.revealFolder(widget.views, viewId);

  /// Expand every ANCESTOR of [id] so a (possibly nested) node is revealed in
  /// the sidebar — used on navigate/create so the active/new page is never
  /// hidden under a collapsed parent (AppFlowy/Notion "reveal current page").
  /// The node itself is not expanded (that would show ITS children).
  void _revealAncestors(String id) => _tree.revealAncestors(widget.views, id);

  /// Expand [id] itself (so a freshly-created/dropped child under it is visible)
  /// plus its ancestor chain.
  void _expandForChildOf(String id) => _tree.expandForChildOf(widget.views, id);

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

  /// The editor pane, with the tab strip on top of whatever it renders.
  ///
  /// The strip wraps [_editorPaneBody] rather than living inside it because the
  /// body returns early for every empty state — no workspace, no page open. A
  /// strip nested below those returns would vanish the moment one of its tabs
  /// held nothing, which is exactly when the user needs it to switch away.
  Widget _editorPane(BuildContext context) {
    final onSelectTab = widget.onSelectTab;
    final onCloseTab = widget.onCloseTab;
    final body = _editorPaneBody(context);
    // Only the local world skips the strip now — it has no tab model at all.
    // The "fewer than two tabs" case used to skip it too; the strip is resident
    // since 2026-08-12 (user's call), because hiding it also hid the `+`, and
    // that left a single-tab window with no visible way to open a second one.
    if (onSelectTab == null || onCloseTab == null || widget.tabs.isEmpty) {
      return body;
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        DocTabStrip(
          tabs: widget.tabs,
          activeIndex: widget.activeTabIndex,
          onSelect: onSelectTab,
          onClose: onCloseTab,
          untitledLabel: context.l10n.untitledPage,
          onNewTab: widget.onNewTabPage == null ? null : _showNewTabMenu,
          newTabTooltip: context.l10n.tabNewTooltip,
          foreignWorkspaceName: _foreignWorkspaceName,
        ),
        Expanded(child: body),
      ],
    );
  }

  /// The workspace name to show on a tab that does not belong to the workspace
  /// on screen — null for every ordinary tab.
  ///
  /// Named rather than merely marked: "this tab is from somewhere else" leaves
  /// you having to click it to find out where, and clicking is the thing that
  /// moves you.
  String? _foreignWorkspaceName(DocTab tab) {
    final id = tab.workspaceId;
    if (id == null || id == widget.selectedWorkspace?.id) return null;
    for (final w in widget.workspaces) {
      if (w.id == id) return w.name;
    }
    // Stamped with a workspace this account can no longer see. Say nothing
    // rather than invent a name — the tab is already in the state
    // `_workspaceForTab` refuses to sync.
    return null;
  }

  /// The tab strip's `+`: create a page, or pick an existing one — both into a
  /// new tab.
  ///
  /// The picker is the app's EXISTING search dialog, not a list built here.
  /// AppFlowy's `+` opens its command palette (search + recents + "new page")
  /// and Mica already has the search half of that; a second page list would be
  /// the two-representations trap this repo keeps paying for. The visible
  /// difference from AppFlowy is that ours has no recents — search shows
  /// nothing until you type. Adding recents is a change to search itself, and
  /// it would then apply everywhere search is opened from, which is the right
  /// way to get it and not something to bolt onto this menu.
  Future<void> _showNewTabMenu(Offset globalPosition) async {
    final onNewPage = widget.onNewTabPage;
    if (onNewPage == null) return;
    final overlay =
        Overlay.of(context).context.findRenderObject() as RenderBox?;
    if (overlay == null) return;
    final local = overlay.globalToLocal(globalPosition);
    final picked = await showMenu<String>(
      context: context,
      position: RelativeRect.fromLTRB(
        local.dx,
        local.dy,
        overlay.size.width - local.dx,
        0,
      ),
      items: [
        PopupMenuItem(
          value: 'new',
          child: _MenuRow(icon: Icons.add, label: context.l10n.tabNewPage),
        ),
        PopupMenuItem(
          value: 'open',
          child: _MenuRow(
            icon: Icons.search,
            label: context.l10n.tabOpenExisting,
          ),
        ),
      ],
    );
    if (!mounted) return;
    switch (picked) {
      case 'new':
        onNewPage();
      case 'open':
        _openSearch(inNewTab: true);
    }
  }

  Widget _editorPaneBody(BuildContext context) {
    final workspace = widget.selectedWorkspace;
    if (workspace == null) {
      return EmptyState(
        icon: Icons.ads_click,
        title: context.l10n.editorSelectWorkspaceTitle,
        detail: context.l10n.editorSelectWorkspaceDetail,
      );
    }

    final bootstrap = widget.selectedBootstrap;
    if (bootstrap == null) {
      // Nothing open → HOME (design 03). This used to be a "pick a page" empty
      // state, which told the user to do something instead of helping them do
      // it; home offers the create action and the recent pages right there.
      // Falls back to the old copy only if the host supplies no home pane.
      return widget.overviewPane ??
          widget.homePane ??
          EmptyState(
            icon: Icons.description_outlined,
            title: context.l10n.editorSelectPageTitle,
            detail: context.l10n.editorSelectPageDetail,
          );
    }

    final canEdit = matchesEditRole(workspace.role);

    return ColoredBox(
      color: MicaTheme.of(context).surface.base,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          // ONE top row for the pane, so the page tools line up with the left
          // sidebar's collapse button instead of sitting 37px below it in the
          // document column. Notion puts the same cluster here; the row exists
          // whether or not the format bar is on, because the tools do.
          Padding(
            padding: const EdgeInsets.fromLTRB(8, 4, 8, 0),
            child: LayoutBuilder(
              builder: (context, header) => Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Row(
                    children: [
                      // The path comes first and takes what it needs, inside two
                      // bounds — see [_pathBudget]. It used to be a flat 260, which
                      // dropped ancestors on paths that had most of the row sitting
                      // empty beside them: an ellipsis with no visible cause.
                      //
                      // The pixel-based collapse in PageBreadcrumb stays as the last
                      // resort. It is NOT a depth rule ("collapse past 3 segments") —
                      // this repo tried that and recorded why it failed: `tools ›
                      // 笔记软件 › mica › 单机手动部署（IP 直连，不用 Traefik）` is
                      // three segments and still overruns. AppFlowy gets away with a
                      // depth rule because their breadcrumb has no width cap at all
                      // and simply scrolls, which a shared row cannot do.
                      if (widget.selectedView != null)
                        ConstrainedBox(
                          constraints: BoxConstraints(
                            maxWidth: _pathBudget(
                              header.maxWidth,
                              barOn:
                                  widget.showFormatBar &&
                                  canEdit &&
                                  header.maxWidth >= 720,
                            ),
                          ),
                          child: PageBreadcrumb(
                            views: widget.views,
                            current: widget.selectedView!,
                            onSelect: widget.onSelectView,
                            // Renaming from the breadcrumb tail (AppFlowy does this).
                            // Gated on the editor role: a viewer's rename would 403,
                            // and an edit affordance that cannot succeed is worse than
                            // none — same rule as everywhere else here.
                            onRename: canEdit ? widget.onRenameView : null,
                            onCopyPath: _copyPagePath,
                            workspaceName: widget.selectedWorkspace?.name,
                            trailing: Row(
                              mainAxisSize: MainAxisSize.min,
                              children: [
                                // Quiet sync status (nothing when synced). It STAYS
                                // on the breadcrumb while the tools moved to the pane
                                // header: it reports on this document, not on the app
                                // — same reason the breadcrumb itself lives here.
                                if (widget.syncPhase != null)
                                  _SyncBadge(widget.syncPhase!),
                              ],
                            ),
                          ),
                        ),
                      // The format bar takes whatever the path left. It is already a
                      // horizontal SingleChildScrollView, so a narrow share does not
                      // break it — it scrolls. [_pathBudget] guarantees that share
                      // never drops below _formatBarFloor.
                      Expanded(
                        child:
                            (widget.showFormatBar &&
                                canEdit &&
                                header.maxWidth >= 720)
                            ? ListenableBuilder(
                                listenable: _activeBlockHook,
                                builder: (context, _) => _formatBar(context),
                              )
                            : const SizedBox.shrink(),
                      ),
                      // The right-hand twin of the sidebar collapse button.
                      // It used to live one row lower, on the title row, so
                      // the "symmetric pair" the old comment claimed was
                      // visibly off — different row, different size. Same
                      // line, same density, same icon size as the left one.
                      IconButton(
                        tooltip: _toolsExpanded
                            ? context.l10n.pageHideSidePanel
                            : context.l10n.pageShowSidePanel,
                        visualDensity: VisualDensity.compact,
                        onPressed: () =>
                            setState(() => _toolsExpanded = !_toolsExpanded),
                        icon: Transform.flip(
                          flipX: true,
                          child: const Icon(
                            Icons.view_sidebar_outlined,
                            size: 20,
                          ),
                        ),
                      ),
                      if (widget.onAddComment != null)
                        _CommentsButton(
                          openCount: widget.commentThreads
                              .where((t) => !t.isResolved)
                              .length,
                          active:
                              _toolsExpanded && _toolsTab == _ToolsTab.comments,
                          onTap: () {
                            final showing =
                                _toolsExpanded &&
                                _toolsTab == _ToolsTab.comments;
                            setState(() {
                              // From anywhere else this means "show me the
                              // comments", not "toggle the sidebar" — so it
                              // opens the sidebar AND selects the tab. Only
                              // a second press on an already-showing comment
                              // tab closes it.
                              _toolsExpanded = !showing;
                              _toolsTab = _ToolsTab.comments;
                            });
                            // Closing drops the emphasis: it means "the
                            // thread I am reading", and with the panel gone
                            // there is no such thing — leaving it on would
                            // strand a stronger wash with nothing to explain
                            // it.
                            if (showing) widget.onFocusCommentThread(null);
                          },
                        ),
                      _PropertiesToggle(
                        active: _showProperties,
                        hasProperties: bootstrap.rootFrontMatter
                            .trim()
                            .isNotEmpty,
                        onTap: () =>
                            setState(() => _showProperties = !_showProperties),
                      ),
                      PopupMenuButton<String>(
                        tooltip: context.l10n.pageMenu,
                        // Same density as its neighbours — it used to sit alone
                        // on the title row at the default size.
                        padding: EdgeInsets.zero,
                        icon: const Icon(Icons.more_horiz, size: 20),
                        onSelected: _onPageMenu,
                        itemBuilder: (context) => [
                          // Copy sits above the exports and gets its own group:
                          // it is the cheap, frequent one (grab the text, paste
                          // it somewhere), while everything below produces a
                          // file. Grouping it with the exports would bury it.
                          PopupMenuItem(
                            value: 'copy-md',
                            child: ListTile(
                              dense: true,
                              contentPadding: EdgeInsets.zero,
                              leading: const Icon(Icons.copy_all_outlined),
                              title: Text(context.l10n.pageCopyContent),
                            ),
                          ),
                          const PopupMenuDivider(),
                          // One export, always a ZIP. The old "Export Markdown"
                          // handed back a lone .md whose images pointed at
                          // `![](photo.png)` — a file that was nowhere in the
                          // download. A zip can carry them; a .md cannot.
                          PopupMenuItem(
                            value: 'export-zip',
                            child: ListTile(
                              dense: true,
                              contentPadding: EdgeInsets.zero,
                              leading: const Icon(Icons.folder_zip_outlined),
                              title: Text(context.l10n.rowExportZipImages),
                            ),
                          ),
                          PopupMenuItem(
                            value: 'export-html',
                            child: ListTile(
                              dense: true,
                              contentPadding: EdgeInsets.zero,
                              leading: const Icon(Icons.html_outlined),
                              title: Text(context.l10n.rowExportHtml),
                            ),
                          ),
                          // PDF export: desktop drives the OS WebView2 runtime's
                          // headless print (native bytes → download); web hands
                          // the same self-contained HTML to the browser's own
                          // print dialog ("Save as PDF"). Available on both.
                          PopupMenuItem(
                            value: 'export-pdf',
                            child: ListTile(
                              dense: true,
                              contentPadding: EdgeInsets.zero,
                              leading: const Icon(
                                Icons.picture_as_pdf_outlined,
                              ),
                              title: Text(context.l10n.rowExportPdf),
                            ),
                          ),
                          const PopupMenuDivider(),
                          PopupMenuItem(
                            value: 'import-md',
                            child: ListTile(
                              dense: true,
                              contentPadding: EdgeInsets.zero,
                              leading: const Icon(Icons.upload_file_outlined),
                              title: Text(context.l10n.pageImportMarkdown),
                            ),
                          ),
                          if (widget.onShare != null) ...[
                            const PopupMenuDivider(),
                            PopupMenuItem(
                              value: 'share',
                              child: ListTile(
                                dense: true,
                                contentPadding: EdgeInsets.zero,
                                leading: const Icon(Icons.public),
                                title: Text(context.l10n.shareTitle),
                              ),
                            ),
                          ],
                          if (widget.onVersionHistory != null) ...[
                            const PopupMenuDivider(),
                            PopupMenuItem(
                              value: 'version-history',
                              child: ListTile(
                                dense: true,
                                contentPadding: EdgeInsets.zero,
                                leading: const Icon(Icons.history),
                                title: Text(context.l10n.versionHistoryTitle),
                              ),
                            ),
                          ],
                          if (widget.onRestoreCheckpoint != null) ...[
                            const PopupMenuDivider(),
                            PopupMenuItem(
                              value: 'restore-checkpoint',
                              child: ListTile(
                                dense: true,
                                contentPadding: EdgeInsets.zero,
                                leading: const Icon(Icons.restore_outlined),
                                title: Text(context.l10n.pageRestoreCheckpoint),
                              ),
                            ),
                          ],
                        ],
                      ),
                    ],
                  ),
                  if (widget.showFormatBar && canEdit && header.maxWidth < 720)
                    ListenableBuilder(
                      listenable: _activeBlockHook,
                      builder: (context, _) => _formatBar(context),
                    ),
                ],
              ),
            ),
          ),
          // The editor column is capped at widget.pageWidth (a fixed page-width
          // step) inside _editorScroll, and the window caps it below that on a
          // narrow pane — so no measured max is needed here.
          Expanded(
            child: Stack(
              children: [
                Positioned.fill(
                  child: _editorScroll(context, canEdit, bootstrap),
                ),
                // Bottom-right of the PANE, so it reads as a status
                // readout that is always where you left it — not a chip
                // that lands wherever the text happens to stop.
                Positioned(
                  right: 12,
                  bottom: 8,
                  child: ValueListenableBuilder<DocCounts>(
                    valueListenable: _counts,
                    builder: (context, counts, _) => counts.chars == 0
                        ? const SizedBox.shrink()
                        : _WordCountBadge(counts: counts),
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  /// Enter in the page title: split at the caret — the remainder becomes a
  /// new first body line (pushing the body down), the caret follows it.
  void _titleEnter() {
    final text = _pageTitle.text;
    final sel = _pageTitle.selection;
    final at = sel.isValid ? sel.start.clamp(0, text.length) : text.length;
    final rest = text.substring(at);
    if (rest.isNotEmpty) {
      _pageTitle.text = text.substring(0, at);
      _schedulePageTitleSave();
    }
    _commandHook.insertTopParagraph(rest);
  }

  /// The optional formatting toolbar (Settings -> Appearance, off by
  /// default): one-click access to the high-frequency Markdown actions,
  /// driven through [_commandHook] so focus/selection semantics stay in the
  /// editor.
  /// Narrowest share of the header the format bar may be squeezed to.
  ///
  /// MEASURED, not chosen: the full strip is 19 buttons over 606px. 360 keeps
  /// roughly the first eleven — undo/redo, the three heading levels, bold /
  /// italic / strike, code, link — visible without scrolling, which is the set
  /// you reach for without looking. Past that the bar scrolls, which it has
  /// always been able to do (it is a horizontal SingleChildScrollView).
  static const _formatBarFloor = 360.0;

  /// The right-hand cluster (side-panel toggle, comments, properties, page
  /// menu) shares this row too, and is not flexible. Reserved alongside the
  /// bar's floor so the floor is a floor on what the BAR gets, not on what is
  /// left over before the icons take their cut.
  static const _headerIconsWidth = 160.0;

  /// How wide the breadcrumb may get.
  ///
  /// "Path first" — it takes what it needs — under two bounds, and each one is
  /// the answer to a question that was actually asked:
  ///
  ///  * A ceiling at 70% of the header, so a pathological title cannot span the
  ///    whole thing. Real names and real nesting stay well under it; the one
  ///    that reached it was a hand-typed `asdfsadfasfdasfds…`.
  ///  * With the bar on, a reserve for the bar: the path may not eat the last
  ///    [_formatBarFloor] + [_headerIconsWidth] of the row. That is a bound on
  ///    what the PATH may take — not a promise about the bar's rendered width,
  ///    which `_formatBar` narrows again with its own `Center(maxWidth:
  ///    pageWidth)`. Measured on a 1050-wide window the bar ends up around
  ///    317px and shows ten buttons before it scrolls.
  ///
  /// The 200 floor at the end is for a genuinely narrow pane, where the two
  /// bounds cross and something has to give; there the path keeps enough to
  /// show the page name and the bar scrolls from the start.
  ///
  /// Measured, bar on, this page's path needing 221px:
  ///
  ///   window 1600 -> path 221, bar 606 (everything fits, no scroll)
  ///   window 1400 -> path 221, bar 562
  ///   window 1280 -> path 221, bar 442
  ///   window 1050 -> path yields, bar 317
  ///
  /// The path holding its natural 221 rather than its whole budget is the point
  /// of `MainAxisSize.min` on PageBreadcrumb's Row — a cap is not a demand.
  double _pathBudget(double header, {required bool barOn}) {
    final ceiling = header * 0.7;
    if (!barOn)
      return math.max(0, math.min(ceiling, header - _headerIconsWidth));
    return math.max(
      math.min(ceiling, header - _formatBarFloor - _headerIconsWidth),
      200.0,
    );
  }

  Widget _formatBar(BuildContext context) {
    Widget btn(
      IconData icon,
      String tip,
      VoidCallback onTap, {
      bool active = false,
    }) {
      return Tooltip(
        message: tip,
        waitDuration: const Duration(milliseconds: 500),
        child: InkWell(
          borderRadius: BorderRadius.circular(6),
          onTap: onTap,
          child: Container(
            decoration: active
                ? BoxDecoration(
                    color: MicaTheme.of(
                      context,
                    ).accent.primary.withValues(alpha: 0.08),
                    borderRadius: BorderRadius.circular(6),
                  )
                : null,
            padding: const EdgeInsets.all(6),
            child: Icon(
              icon,
              size: 18,
              color: active
                  ? MicaTheme.of(context).accent.primary
                  : MicaTheme.of(context).text.muted,
            ),
          ),
        ),
      );
    }

    Widget divider() => Container(
      width: 1,
      height: 18,
      margin: const EdgeInsets.symmetric(horizontal: 4),
      color: MicaTheme.of(context).border.normal,
    );

    final h = _commandHook;
    final k = _activeBlockHook.kind;
    final lvl = _activeBlockHook.level;
    bool onHeading(int n) => k == 'heading' && lvl == n;

    /// A heading button TOGGLES: pressing the level you are already on puts the
    /// block back to body text.
    ///
    /// Reported 2026-08-12. Each level used to re-apply itself unconditionally,
    /// which made the lit button the only control on this bar that did nothing
    /// when pressed — every neighbour (bold, quote, list) has always toggled.
    ///
    /// One builder rather than three near-identical `btn(...)` calls, because
    /// the toggle has to be identical across the levels and this bar and the
    /// floating one already disagreed once by being written out twice.
    Widget headingBtn(IconData icon, String label, int level) =>
        btn(icon, label, () {
          final target = headingButtonTarget(
            kind: k,
            // Null level on a non-heading block; 0 is never a heading level, so
            // it can only ever answer "not the one you pressed".
            level: lvl ?? 0,
            pressed: level,
          );
          h.setBlock(target.kind, target.data);
        }, active: onHeading(level));
    // TextFieldTapRegion: clicking the toolbar must not read as a tap OUTSIDE
    // the table-cell editor's TextField — onTapOutside would unfocus/commit the
    // cell on pointer-down, so the command would land after the cell closed.
    return TextFieldTapRegion(
      child: Container(
        decoration: BoxDecoration(
          color: MicaTheme.of(context).surface.base,
          border: Border(
            bottom: BorderSide(color: MicaTheme.of(context).border.normal),
          ),
        ),
        padding: const EdgeInsets.symmetric(vertical: 3),
        // Keep the compact toolbar centered and let its controls scroll within
        // the available header width; narrow panes give it a separate row.
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 4),
          child: Center(
            child: ConstrainedBox(
              constraints: BoxConstraints(maxWidth: widget.pageWidth),
              child: Padding(
                padding: EdgeInsets.zero,
                child: SingleChildScrollView(
                  scrollDirection: Axis.horizontal,
                  child: Row(
                    children: [
                      btn(Icons.undo, context.l10n.pageFormatUndo, h.undo),
                      btn(Icons.redo, context.l10n.pageFormatRedo, h.redo),
                      divider(),
                      btn(
                        Icons.notes,
                        context.l10n.pageFormatText,
                        () => h.setBlock('paragraph'),
                        active: k == 'paragraph',
                      ),
                      headingBtn(
                        Icons.looks_one_outlined,
                        context.l10n.pageFormatHeading1,
                        1,
                      ),
                      headingBtn(
                        Icons.looks_two_outlined,
                        context.l10n.pageFormatHeading2,
                        2,
                      ),
                      headingBtn(
                        Icons.looks_3_outlined,
                        context.l10n.pageFormatHeading3,
                        3,
                      ),
                      divider(),
                      btn(
                        Icons.format_bold,
                        context.l10n.pageFormatBold,
                        () => h.toggleMark('bold'),
                      ),
                      btn(
                        Icons.format_italic,
                        context.l10n.pageFormatItalic,
                        () => h.toggleMark('italic'),
                      ),
                      btn(
                        Icons.format_strikethrough,
                        context.l10n.pageFormatStrikethrough,
                        () => h.toggleMark('strike'),
                      ),
                      btn(
                        Icons.code,
                        context.l10n.pageFormatInlineCode,
                        () => h.toggleMark('code'),
                      ),
                      btn(Icons.link, context.l10n.pageFormatLink, h.editLink),
                      divider(),
                      btn(
                        Icons.format_list_bulleted,
                        context.l10n.pageFormatBulletedList,
                        () => h.setBlock('bulleted_list'),
                        active: k == 'bulleted_list',
                      ),
                      btn(
                        Icons.format_list_numbered,
                        context.l10n.pageFormatNumberedList,
                        () => h.setBlock('numbered_list'),
                        active: k == 'numbered_list',
                      ),
                      btn(
                        Icons.check_box_outlined,
                        context.l10n.pageFormatTodoList,
                        () => h.setBlock('todo', {'checked': false}),
                        active: k == 'todo',
                      ),
                      btn(
                        Icons.format_quote,
                        context.l10n.pageFormatQuote,
                        () => h.setBlock('quote'),
                        active: k == 'quote',
                      ),
                      btn(
                        Icons.terminal,
                        context.l10n.pageFormatCodeBlock,
                        () => h.setBlock('code_block'),
                        active: k == 'code_block',
                      ),
                      divider(),
                      btn(
                        Icons.horizontal_rule,
                        context.l10n.pageFormatDivider,
                        () => h.insert('divider'),
                      ),
                      btn(
                        Icons.grid_on,
                        context.l10n.pageFormatTable,
                        () => h.insert('table'),
                      ),
                      btn(
                        Icons.image_outlined,
                        context.l10n.pageFormatImage,
                        () => h.insert('image'),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }

  Widget _editorScroll(
    BuildContext context,
    bool canEdit,
    DocumentBootstrap bootstrap,
  ) {
    return Listener(
      // A press anywhere on the page that is NOT inside the editor canvas
      // (margins beside/above the page column) resets diagram zoom/pan —
      // clicks INSIDE the canvas are judged by the editor itself.
      behavior: HitTestBehavior.translucent,
      onPointerDown: (e) {
        final box =
            _editorSurfaceKey.currentContext?.findRenderObject() as RenderBox?;
        if (box == null || !box.attached) return;
        final local = box.globalToLocal(e.position);
        if (!(Offset.zero & box.size).contains(local)) {
          _commandHook.resetDiagramViews();
        }
      },
      child: LayoutBuilder(
        builder: (context, constraints) => SingleChildScrollView(
          padding: EdgeInsets.fromLTRB(
            constraints.maxWidth < 600 ? 12 : 28,
            8,
            constraints.maxWidth < 600 ? 12 : 28,
            28,
          ),
          child: Center(
            child: ConstrainedBox(
              constraints: BoxConstraints(maxWidth: widget.pageWidth),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  // The editor canvas reserves EditorTheme.gutter on the left
                  // for the block drag handles, so its text starts at x=gutter.
                  // Inset the title + meta rows by the same amount to keep the
                  // page's text column aligned (handles float in the margin).
                  Padding(
                    padding: const EdgeInsets.only(left: EditorTheme.gutter),
                    child: Row(
                      crossAxisAlignment: CrossAxisAlignment.center,
                      children: [
                        // Settings can hide the title; the row itself stays for
                        // the page menu + side-panel toggle.
                        if (!widget.showPageTitle)
                          const Spacer()
                        else
                          Expanded(
                            child: PageTitleField(
                              controller: _pageTitle,
                              focusNode: _pageTitleFocus,
                              appearance: _editorAppearance,
                              hintText: context.l10n.untitledPage,
                              readOnly: !canEdit,
                              onChanged: (_) => _schedulePageTitleSave(),
                              onEnterBody: _titleEnter,
                              onArrowDown: _commandHook.focusFirstLine,
                            ),
                          ),
                      ],
                    ),
                  ),
                  const SizedBox(height: 4),
                  // Collaborator presence, right-aligned under the title row —
                  // shown ONLY when someone else is here. Solo, this was an
                  // "Only you" line that just widened the title↔body gap for no
                  // reason (the space the user flagged).
                  if (widget.presence.isNotEmpty) ...[
                    Padding(
                      padding: const EdgeInsets.only(left: EditorTheme.gutter),
                      child: Row(
                        children: [
                          const Spacer(),
                          _PresenceBar(presence: widget.presence),
                        ],
                      ),
                    ),
                    const SizedBox(height: 4),
                  ],
                  // Page properties (front-matter) — a structured, editable view
                  // of the document's YAML front matter, above the body. Reads the
                  // root block's `data['front_matter']`; commits write it back
                  // through the same self-dispatching op sink the editor uses
                  // (local / cloud-CRDT / cloud-REST all handled). See
                  // docs/features/page-properties.md.
                  if (_showProperties)
                    PropertyPanel(
                      frontMatter: bootstrap.rootFrontMatter,
                      canEdit: canEdit,
                      // Clicking a tag opens workspace search for `#tag` — the
                      // precise token the index stores for list/tag values, so it
                      // finds pages that CARRY the tag, not ones that merely
                      // mention the word in their body (M2).
                      onOpenTag: (v) => _openSearch(initialQuery: '#$v'),
                      onCommit: (fm) {
                        final data = Map<String, dynamic>.from(
                          bootstrap.rootData,
                        );
                        if (fm.isEmpty) {
                          data.remove('front_matter');
                        } else {
                          data['front_matter'] = fm;
                        }
                        return (widget.onApplyMaintenanceOperations ??
                            widget.onApplyOperations)([
                          {
                            'type': 'update_block',
                            'block_id': bootstrap.document.rootBlockId,
                            'data': data,
                          },
                        ]);
                      },
                    ),
                  Padding(
                    key: _editorSurfaceKey,
                    padding: const EdgeInsets.only(top: 4),
                    child: MicaEditor(
                      countsSink: _counts,
                      key: ValueKey(
                        '${bootstrap.document.id}#${widget.editorEpoch}',
                      ),
                      focusNode: _editorFocus,
                      rootBlockId: bootstrap.document.rootBlockId,
                      nodes: _editorNodes(bootstrap.childBlocks),
                      version: bootstrap.snapshot.versionSeq,
                      canEdit: canEdit,
                      onSelectionChanged: widget.onCursorChanged,
                      commentHighlights: widget.commentHighlights,
                      onAddComment: widget.onAddComment == null
                          ? null
                          : (sb, so, eb, eo, quote) async {
                              await widget.onAddComment!(sb, so, eb, eo, quote);
                              // You just wrote it; showing it should not cost
                              // another click. Only for a comment THIS user made:
                              // opening because a collaborator commented would be
                              // an interruption, not a convenience.
                              if (mounted) {
                                setState(() {
                                  _toolsExpanded = true;
                                  _toolsTab = _ToolsTab.comments;
                                });
                              }
                            },
                      remoteCursors: _editorCursors(widget.presence),
                      onApplyOperations: widget.onApplyOperations,
                      onApplyHistory: widget.onApplyHistory,
                      onOpFault: widget.onEditorFault,
                      onUploadImage: widget.onUploadImage,
                      onImportImageUrl: widget.onImportImageUrl,
                      onLoadImageBytes: widget.onLoadImageBytes,
                      onResolveImageUrls: widget.onResolveImageUrls,
                      onAiStream: widget.showAi ? widget.onAiStream : null,
                      reHostImages: widget.reHostImages,
                      scrollHook: _scrollHook,
                      commandHook: _commandHook,
                      outlineHook: _outlineHook,
                      findHook: _findHook,
                      activeBlockHook: _activeBlockHook,
                      onExitTop: _focusPageTitle,
                      appearance: _editorAppearance,
                      onOpenPage: _openPageLink,
                      pageLinks: () => [
                        for (final v in widget.views)
                          if (v.objectType == 'document')
                            PageLinkTarget(id: v.id, title: v.name),
                      ],
                    ),
                  ),
                  // Reverse references — the cloud pages that link to this one.
                  // Cloud + documents only (folders have no body); lazy-loaded so
                  // it never blocks the page opening. Hidden entirely in 本地模式
                  // (onLoadBacklinks null there).
                  if (widget.onLoadBacklinks != null &&
                      widget.selectedView?.objectType == 'document')
                    Padding(
                      padding: const EdgeInsets.only(left: EditorTheme.gutter),
                      child: _BacklinksPanel(
                        key: ValueKey('backlinks#${widget.selectedView!.id}'),
                        viewId: widget.selectedView!.id,
                        load: widget.onLoadBacklinks!,
                        onOpen: widget.onOpenSearchResult,
                        onLoaded: _onBacklinksLoaded,
                      ),
                    ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }

  /// The outline row that stands for the page title. Not a block id — the title
  /// is not a block — so it is a sentinel no document can produce.
  static const String _outlineTitleId = '\u0000outline-page-title';

  /// Put the caret at the end of the page-title field.
  ///
  /// Two callers: pressing Up on the document's first line (`onExitTop`), and
  /// tapping the title row in the outline. Shared rather than duplicated because
  /// of the web quirk below, which took a wrong attempt to find.
  void _focusPageTitle() {
    if (!widget.showPageTitle) return;
    _pageTitleFocus.requestFocus();
    // The web TextField select-alls when focused programmatically; that happens
    // in the focus microtask, so queue ours right behind it — the caret is
    // collapsed before the next frame paints (a post-frame callback here
    // flashed the selection for one frame).
    Future.microtask(() {
      if (!mounted) return;
      _pageTitle.selection = TextSelection.collapsed(
        offset: _pageTitle.text.length,
      );
    });
  }

  /// Tappable outline entries for the current page's headings (no section
  /// header). Tapping scrolls the editor to that heading. Fed the LIVE heading
  /// list from [_outlineHook] (republished by the editor on every edit), so it
  /// tracks typing — not just navigation.
  List<Widget> _pageOutlineItems(
    BuildContext context,
    List<OutlineEntry> headings,
  ) {
    // The page's own title leads its table of contents.
    //
    // It used to be missing entirely, which reads as a bug once you notice it:
    // a document's outline that never mentions the document. The title is not a
    // heading BLOCK — it lives outside the editor's document (see
    // `docs/features/page-title-plan.md`) — so the editor's live heading feed cannot
    // carry it. It is prepended here, where the page is known.
    //
    // Level 1 on purpose: every heading the author writes then indents under it,
    // which is what "this is the root of the page" should look like. An empty or
    // still-untitled name contributes nothing rather than a blank row.
    final title = widget.selectedView?.name.trim() ?? '';
    final entries = <OutlineEntry>[
      if (title.isNotEmpty && !isUntitledPageName(title))
        OutlineEntry(id: _outlineTitleId, text: title, level: 1),
      ...headings,
    ];
    return [
      for (final h in entries)
        if (h.text.trim().isNotEmpty)
          InkWell(
            // The title row has no block to scroll to — it is not in the
            // document — so it puts the caret in the title field instead, which
            // is both "go to the top" and "here is where you rename it".
            onTap: () => h.id == _outlineTitleId
                ? _focusPageTitle()
                : _scrollHook.scrollToBlock(h.id),
            borderRadius: BorderRadius.circular(6),
            child: Padding(
              padding: EdgeInsets.only(
                left: 4.0 + 14 * ((h.level.clamp(1, 6) - 1).clamp(0, 5)),
                top: 5,
                bottom: 5,
                right: 4,
              ),
              child: Text(
                h.text.trim(),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  fontSize: h.level <= 1 ? 14 : 13,
                  fontWeight: h.level <= 1 ? FontWeight.w600 : FontWeight.w400,
                  color: MicaTheme.of(context).text.primary,
                ),
              ),
            ),
          ),
    ];
  }

  int _headingLevel(DocumentBlock b) {
    final level = b.data['level'];
    if (level is int) return level.clamp(1, 6);
    if (level is num) return level.toInt().clamp(1, 6);
    return 1;
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
    setState(() => _revealAncestors(view.id));
    _promptRenameView(view);
  }

  void _cancelRename() => _inlineRename.cancel();

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
    // `_revealInTree` rather than a direct scroll, because the row does not
    // exist yet at this moment — the tree only gains it on a later rebuild.
    // That machinery already retries per frame until the row is laid out, which
    // is exactly the shape of this problem.
    _revealInTree(id);
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
      setState(() => _expandForChildOf(parent.id)); // reveal the new child
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
    setState(() => _expandForChildOf(parent.id)); // reveal the new child
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
            // and `_revealInTree` would park the request until that workspace's
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
          _revealInTree(viewId);
        },
        onReveal: (viewId, workspaceId) {
          Navigator.of(context).pop();
          // A folder in ANOTHER workspace has to be switched to before it can
          // be located — `_revealInTree` alone just parks the request against a
          // tree that will never contain it, which is a click that does
          // nothing. Reported 2026-08-28, and it became the COMMON case the
          // moment search started spanning every workspace by default: the top
          // hits for a word like "claude" are folders named after it.
          final other = elsewhereWorkspace(
            workspaceId,
            widget.selectedWorkspace?.id,
          );
          // Switch FIRST, then park the reveal. `_revealInTree` already knows
          // how to wait for a tree that has not arrived ("a search hit from
          // another workspace arrives before that workspace's views do") — what
          // was missing is that nothing ever made that workspace arrive, so the
          // request waited out its ~2s budget against a tree it was never in.
          if (other != null) widget.onSelectWorkspaceById?.call(other);
          _revealInTree(viewId);
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
    showDialog<void>(
      context: context,
      builder: (context) => _AiDialog(
        canEdit: matchesEditRole(widget.selectedWorkspace?.role),
        hasWorkspace: widget.selectedWorkspace != null,
        onStream: widget.onAiStream,
        onNewPage: widget.onAiNewPage,
        onCurrentPage: widget.onAiCurrentPage,
        currentPage: widget.onAiCurrentPage == null
            ? null
            : (
                title: _pageTitle.text,
                markdown: _commandHook.documentMarkdown() ?? '',
              ),
        onNewWorkspace: widget.onAiNewWorkspace,
        onPreviewEdits: widget.onAiPreviewEdits,
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
