part of 'main.dart';

class _WorkspaceShellState extends State<WorkspaceShell> {
  final ApiClient _api = ApiClient();

  /// Which backend we talk to (cloud / self-hosted / local-offline). Loaded
  /// from prefs in [_loadPrefs] and applied to [_api] before any request.
  /// The configured cloud server's origin URL (P3c-2). Always set (defaults to
  /// Mica Cloud); whether the user is signed in to it is a separate question
  /// (the per-origin auth prefs). There is no "mode" anymore — the local world
  /// always exists alongside.
  /// The server the cloud side currently talks to. Always one of [_servers].
  late String _cloudOrigin;

  /// Every configured server, in the order the user added them. Credentials are
  /// already filed per origin (`authToken:$origin`, …), so N servers cost
  /// nothing to keep — switching back to one restores its sign-in.
  ///
  /// `'local'` is deliberately NOT in here: it is a peer of these wherever the
  /// worlds are listed, but it is not a server and has no URL, credentials or
  /// session. The world picker adds it back on its own side (`kLocalOrigin`).
  List<String> _servers = const [];

  /// The server list + bound cloud origin — see server_registry.dart.
  /// [_servers] and [_cloudOrigin] below are synced view-model copies.
  late final ServerRegistry _registry;

  AuthSession? _session;

  /// A fingerprint of the CURRENT sign-in: the server it belongs to, the user it
  /// is for, and the refresh token it holds.
  ///
  /// Captured before an await and re-read after, to answer "is the thing that
  /// came back still about the session I asked on behalf of?". A cheap
  /// generation counter would do the same job, but it needs an increment at
  /// every place a session is assigned, and the first one anybody forgets is a
  /// silent resurrection bug again. Reading the identity out of the session
  /// itself cannot be forgotten.
  ///
  /// The refresh token is part of it deliberately: it ROTATES on every refresh,
  /// so a result computed from a token we no longer hold is exactly the stale
  /// one to discard — and it is also what distinguishes "same user, same server,
  /// new token" (accept) from "different sign-in entirely" (drop).
  String? get _sessionIdentity {
    final account = _accountIdentity;
    return account == null ? null : '$account|${_session!.refreshToken}';
  }

  /// Server + user only — for an await whose result describes the ACCOUNT rather
  /// than the sign-in: the profile endpoint answers "who is this user", and a
  /// refresh token rotating underneath it does not make that answer wrong.
  ///
  /// Using the strict [_sessionIdentity] there would discard a legitimate
  /// profile update every time a refresh landed first, which is a silent
  /// regression of its own (the avatar would simply stop refreshing on a slow
  /// connection, where refreshes overlap polls most).
  String? get _accountIdentity {
    final session = _session;
    if (session == null) return null;
    return '$_cloudOrigin|${session.user.id}';
  }
  /// The session/workspace hub — the four fields every domain used to touch
  /// directly, now owned by [SessionController] (god-class third step; see
  /// docs/code-optimization/OPTIMIZATION_PLAN_ROUND3.md §7). The four legacy
  /// fields right below are synced view-model copies for the build helpers;
  /// every mutation goes through a controller transition.
  late final SessionController _sessionCtrl;
  List<Workspace> _workspaces = const [];
  Map<String, List<WorkspaceMember>> _membersByWorkspace = const {};
  Map<String, List<DocumentView>> _viewsByWorkspace = const {};

  /// Which of several overlapping tree fetches for a workspace may commit. See
  /// [TreeRequestSeq] for why the last-arriving response is not the right one to
  /// believe.
  final TreeRequestSeq _treeRequests = TreeRequestSeq();
  Workspace? _selectedWorkspace;

  /// The open tabs, left to right. Always holds at least one — a tab with a
  /// null [DocTab.view] is the empty state ("no page open"), NOT the absence of
  /// a tab. Closing the last tab is therefore not a thing that can happen, the
  /// same call AFFiNE makes (`closeTab` returns early when one tab is left).
  ///
  /// Single-element today: the tab strip is not built yet. This list existing is
  /// the point — it is what the ~70 `_selectedBootstrap` call sites now read
  /// through, so adding tabs later touches this file in one place instead of 70.
  final List<DocTab> _tabs = [DocTab()];

  /// Index into [_tabs] of the tab the editor is showing.
  ///
  /// Nothing assigns it yet — there is only one tab — so the analyzer suggests
  /// `final`. Do not take that suggestion: switching tabs is the next commit.
  int _activeTabIndex = 0;

  DocTab get _activeTab => _tabs[_activeTabIndex];

  // `_selectedView` / `_selectedBootstrap` used to be plain fields. They are
  // proxies onto the active tab now so the call sites did not have to move in
  // the same commit that introduced the tab model — the two are independent
  // changes and reviewing them together would hide whichever one has the bug.
  DocumentView? get _selectedView => _activeTab.view;
  set _selectedView(DocumentView? value) => _activeTab.view = value;

  DocumentBootstrap? get _selectedBootstrap => _activeTab.bootstrap;

  /// Setting a document also stamps WHICH WORKSPACE it came from.
  ///
  /// Done here, in the one setter, rather than at the ~70 assignment sites: the
  /// invariant is simply "a tab belongs to whichever workspace was current when
  /// its document loaded", and every one of those sites already goes through
  /// here. Stamping at the call sites would mean finding all of them, and then
  /// finding the next one somebody adds.
  ///
  /// Clearing does NOT clear the stamp — `null` is the loading/closed state,
  /// not a change of workspace, and forgetting the workspace there would strand
  /// the tab exactly when it is about to reload its page.
  set _selectedBootstrap(DocumentBootstrap? value) {
    _activeTab.bootstrap = value;
    if (value != null) _activeTab.workspaceId = _selectedWorkspace?.id;
  }

  String? _message;
  bool _isBusy = false;

  /// The server's per-file upload cap, cached from the last `/usage` response.
  /// 0 means never learned — the server has not been asked yet, predates the
  /// field, or has no storage configured. Only used to put a real number in the
  /// `file_too_large` message, so a stale value is harmless and a missing one
  /// just costs a less specific sentence.
  int _maxUploadBytes = 0;

  /// Pages finished / pages planned for a running cloud import, straight from
  /// the job status. Null when no import is in flight — [_isBusy] alone can't
  /// carry this, since it's true for every operation and says nothing about size.
  /// The import domain's logic half (polling, second image pass), extracted
  /// from this State in the god-class pilot — see
  /// docs/code-optimization/OPTIMIZATION_PLAN_ROUND3.md. [_importProgress] and
  /// [_importJobId] below stay as this State's view-model copies, synced from
  /// the controller in [_syncImportState], so the sidebar row and its cancel
  /// button read them unchanged.
  late final ImportController _import;
  ({int done, int total})? _importProgress;

  /// The running import's job id, so the progress row can ask it to stop. Null
  /// whenever no import is in flight.
  String? _importJobId;
  // True while the cloud nav was rebuilt from the on-device mirror because the
  // server was unreachable (P1c). Roles are forced read-only until the server is
  // reached again; [_recoverOnlineNav] then refetches the authoritative nav.
  bool _offlineNav = false;

  // Editor appearance (in-memory; applied live to the editor).
  EditorAppearance _appearance = const EditorAppearance();
  double _pageWidth = kPageWidthDefault;
  // When on, pasted external image URLs are re-hosted into Mica storage.
  bool _reHostImages = true;
  // Formatting toolbar above the page (global setting; off by default).
  bool _showFormatBar = false;
  // Page title block at the top of the page (on by default).
  bool _showPageTitle = true;
  // AI features (off by default). The Ask AI entry points show only when
  // this is on AND a provider is actually configured on the server.
  bool _aiEnabled = false;
  bool _aiConfigured = false;

  // The ACTIVE tab's sockets. Proxies, for the same reason `_selectedBootstrap`
  // is one: every call site here means "the open document's session", and that
  // is exactly the active tab's. Background tabs keep their own pair in
  // [DocTab.sync] / [DocTab.cloudSession] and are reached only by the cap.
  //
  // Switching tabs therefore needs no special case: `_reconcileSync`'s existing
  // "already on this document?" test reads the tab being switched TO, so a tab
  // that kept its socket is recognised as connected and nothing reconnects.
  DocumentSyncClient? get _sync => _activeTab.sync;
  set _sync(DocumentSyncClient? value) => _activeTab.sync = value;

  List<PresenceUser> _presence = const [];
  Timer? _syncRefetchTimer;

  // --- Cloud yrs CRDT sync (P2-M4.5c, desktop only) ---
  // When the server speaks the yrs sync protocol (M4.4+), the desktop cloud
  // editor edits a CRDT replica instead of POSTing block ops: edits push yrs
  // diffs, remote updates merge + reconcile. It activates only once bootstrap
  // succeeds (`isReady`), so against an older server the app falls back to the
  // op/REST path transparently. `DocumentSyncClient` stays up for presence.
  CloudSyncSession? get _cloudSession => _activeTab.cloudSession;
  set _cloudSession(CloudSyncSession? value) => _activeTab.cloudSession = value;

  /// Bumped on every tab activation; each tab records the value it saw. See
  /// [DocTab.lastActivated].
  int _activationTick = 0;

  BigInt? _deviceClientId;

  /// Guards [_sweepPendingOutboxes] so only one cross-document drain runs at a
  /// time (it fires on every online/reconnect).
  bool _sweepingOutboxes = false;

  /// B3: whether the "cloud sync paused" banner is up, so it shows once per
  /// stuck episode and clears on recovery.
  bool _syncBannerShown = false;

  // §7 upstream blob differ (desktop only): images inserted while offline land in
  // the on-device CAS under a sha256 placeholder file_id and queue here. When the
  // doc is next open online, `_reconcilePendingUploads` uploads the bytes, learns
  // the cloud UUID, and rewrites the block's file_id sha256→UUID. Persisted in
  // prefs so the intent survives a restart; `_reconciling` guards re-entrancy.
  PendingUploads _pending = PendingUploads();
  bool _reconciling = false;

  // Awareness: debounce broadcasting the local caret as presence (P2).
  Timer? _cursorTimer;

  // --- Local offline (P2-M3) ---
  // A single implicit local workspace + synthetic identity; the page tree and
  // documents live entirely on-device (SQLite via the LocalOffline facade).
  final LocalOffline _local = LocalOffline();
  bool _localReady = false;
  List<Workspace> _localWorkspaces = const [];
  Workspace? _localSelectedWorkspace;
  List<DocumentView> _localViews = const [];
  DocumentView? _localSelectedView;
  DocumentBootstrap? _localBootstrap;
  // Bumped on rollback to force the local editor to remount fresh.
  int _localEditorEpoch = 0;

  @override
  void initState() {
    super.initState();
    _registry = ServerRegistry(
      readPref: loadPref,
      savePref: savePref,
      forgetOrigin: (origin) async => _local.forgetOrigin(origin),
      rebaseApi: (base) => _api.baseUri = base,
      restoreSession: () => _restoreSession(),
      defaultCloudUrl: kDefaultCloudUrl,
    );
    _loadPrefs();
    _registry.addListener(_syncRegistryState);
    _servers = _registry.servers;
    _cloudOrigin = _registry.cloudOrigin;
    _sessionCtrl = SessionController()..addListener(_syncSessionState);
    _initImportController();
    // Theme and language persist inside their own controllers' setters; the
    // sync write-through has to hear about those changes HERE, because the
    // controllers are global and know nothing of sessions. The apply pass in
    // _syncSettingsFromCloud sets both controllers too — the
    // _applyingCloudSettings guard inside _schedulePushSettings is what keeps
    // that from echoing the cloud's own values back up.
    themeModeController.addListener(_schedulePushSettings);
    localeController.addListener(_schedulePushSettings);
    // Desktop quits via exit(0) (see window_setup_desktop): make that hard exit
    // safe by persisting debounced local state first. No-op on web.
    appExitFlush = _flushForExit;
    // P3c: both worlds live side by side — the local store always opens (it
    // backs local workspaces AND the cloud mirrors), and a cloud session is
    // restored when one was persisted. The active world comes from the
    // persisted `activeOrigin` (written by the P3c-2 legacy migration in
    // _loadPrefs on first run).
    _activeOrigin =
        loadPref('activeOrigin') ?? (_local.available ? 'local' : _cloudOrigin);
    if (!_local.available && _activeOrigin == 'local') {
      // Web has no local world — the cloud origin is the only one.
      _activeOrigin = _cloudOrigin;
    }
    if (_local.available) {
      WidgetsBinding.instance.addPostFrameCallback((_) => _initLocalOffline());
    }
    WidgetsBinding.instance.addPostFrameCallback((_) async {
      await _restoreSession();
      // The demo account only exists on the local dev backend — never try it
      // against cloud/self-hosted servers (it would register a real account).
      if (mounted && _session == null && kDevAutoLogin && _isLocalBackend()) {
        await _devAutoLogin();
      }
    });
  }

  /// True when the configured backend is a local dev server (localhost URL).
  bool _isLocalBackend() {
    final host = _api.baseUri.host;
    return host == '127.0.0.1' || host == 'localhost' || host == '::1';
  }

  /// Persist a new server choice and switch the live client to it. Switching
  /// invalidates the current session (different backend), so we sign out — the
  /// login screen then targets the newly selected server.
  /// Point the app at a different cloud server (P3c-2 — replaces the legacy
  /// mode switch). The current cloud session is disconnected but its stored
  /// credentials are KEPT under the old origin's keys, so switching back signs
  /// in again without retyping; the new origin's stored session (if any) is
  /// restored immediately.
  /// The configured servers, seeded from the single-server world that came
  /// before: whatever `cloudOrigin` the legacy migration settled on becomes
  /// server #1, so nobody's sign-in moves. [seed] is always present, even if a
  /// stale `servers` list somehow lost it — the app must never end up pointing
  /// at a server that isn't in its own list.
  // Server-list load/save lives in ServerRegistry now.

  /// Ask for a URL, then add it. One implementation for both entry points: the
  /// account menu and the sign-in screen. It used to live only in the menu's
  /// State, which is why the sign-in screen — the app's actual entry — could not
  /// offer it at all.
  Future<void> promptAddServer() async {
    // No TextEditingController on purpose: see dialog_controllers.dart. Reading
    // through onChanged removes the lifecycle instead of timing the disposal.
    var typed = '';
    final l10n = context.l10n;
    final url = await showDialog<String>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(l10n.serverAddTitle),
        content: TextField(
          autofocus: true,
          onChanged: (v) => typed = v,
          keyboardType: TextInputType.url,
          autocorrect: false,
          decoration: InputDecoration(
            labelText: l10n.serverUrlLabel,
            hintText: 'https://mica.example.com',
            prefixIcon: const Icon(Icons.link),
            border: const OutlineInputBorder(),
          ),
          onSubmitted: (v) => Navigator.of(context).pop(v),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(),
            child: Text(l10n.commonCancel),
          ),
          FilledButton(
            onPressed: () => Navigator.of(context).pop(typed),
            child: Text(l10n.serverAdd),
          ),
        ],
      ),
    );
    if (url == null || url.trim().isEmpty || !mounted) return;
    // Adding is not switching: the new server joins the list and nothing else
    // moves. Picking it is a separate act.
    final error = await _addServer(url);
    if (!mounted || error == null) return;
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(error)));
  }

  /// Confirm, then forget a server — credentials AND on-device mirror.
  ///
  /// Through `showDestructiveConfirm`, not a hand-rolled red AlertDialog: that
  /// helper exists precisely because the app had grown several, each re-deciding
  /// what "dangerous" looks like.
  Future<void> confirmRemoveServer(String origin) async {
    final l10n = context.l10n;
    final ok = await showDestructiveConfirm(
      context,
      title: l10n.serverRemoveTitle(serverLabel(origin)),
      body: l10n.serverRemoveBody,
      confirmLabel: l10n.commonDelete,
      cancelLabel: l10n.commonCancel,
    );
    if (!ok || !mounted) return;
    await _removeServer(origin);
  }

  /// Make [origin] the one world on screen — `'local'` or one of [_servers].
  /// The single entry point: local and a server are the same kind of choice, so
  /// they go through the same door rather than a bool toggle.
  Future<void> _setActiveConnection(String origin) async {
    if (origin == _activeOrigin) return;
    // A Settings dialog belongs to the world that opened it: three of its tabs
    // are that server's (Account, API Tokens, AI provider), so it cannot
    // survive the switch it would be describing. Today it cannot even be open
    // here — it is modal and the account menu is behind its barrier — so this
    // is a no-op and a fuse for the day that stops being true (Settings as a
    // side panel, say). _importWorkspaceFile has been doing this by hand.
    Navigator.of(
      context,
      rootNavigator: true,
    ).popUntil((r) => r.settings.name != 'settings');
    if (origin == 'local') {
      // Keep the server's credentials — switching back must not cost a re-login.
      _disconnectCloudSession();
      setState(() => _activeOrigin = 'local');
      savePref('activeOrigin', 'local');
      return;
    }
    await _connectCloudServer(origin);
    if (!mounted) return;
    // Land on the server even when it has no session yet: its sign-in row is
    // how you get one, and it lives in the world you just chose.
    if (_activeOrigin != origin) {
      setState(() => _activeOrigin = origin);
      savePref('activeOrigin', origin);
    }
  }

  /// Add a server to the list — and only that. Switching to it is a separate,
  /// explicit act (the button under the dropdown): adding a server you intend
  /// to use later must not yank the whole app over to it.
  ///
  /// Returns an error message, or null on success — a URL we can't parse must
  /// not silently become a dead entry.
  Future<String?> _addServer(String url) async {
    return switch (_registry.addServer(url)) {
      ServerAddResult.added => null,
      ServerAddResult.invalidUrl => context.l10n.serverInvalidUrl,
      ServerAddResult.alreadyAdded => context.l10n.serverAlreadyAdded,
    };
  }

  /// Forget a server completely: its credentials AND its on-device mirror.
  ///
  /// "删除就删除干净,反正 server 云端还有数据在" — true for anything that
  /// reached the server, which is why we push first: [drainOutbox] gives the
  /// pending edits their last chance while we still have a session. What can't
  /// be pushed (offline, server gone) only ever existed here, so the caller
  /// must have said so out loud before we get here.
  Future<void> _removeServer(String origin) async {
    // The drain-and-switch runs FIRST, while a session may still exist;
    // the registry then forgets credentials + mirror and reseeds the
    // build default (never an invented server).
    await _registry.removeServer(
      origin,
      onRemovingActive: () async {
        if (_activeOrigin != origin) return false;
        await _cloudSession?.drainOutbox(timeout: const Duration(seconds: 4));
        await _setActiveConnection('local');
        if (!mounted) return false;
        return true;
      },
    );
  }

  Future<void> _connectCloudServer(String url) async {
    // Persist + rebase + restore live in the registry; the same-origin case
    // still restores the stored session there (cloud → 本地模式 → cloud must
    // not cost a password).
    final changed = await _registry.connect(
      url,
      // Keep the OLD origin's stored credentials while moving.
      onChanging: () async => _disconnectCloudSession(),
    );
    if (!mounted) return;
    // Land on the server even when it has no session yet: its sign-in row is
    // how you get one, and it lives in the world you just chose.
    if (changed && _session != null && _activeOrigin != _registry.cloudOrigin) {
      setState(() => _activeOrigin = _registry.cloudOrigin);
      savePref('activeOrigin', _activeOrigin);
    }
  }

  /// Restore persisted client settings (Settings dialog writes them through
  /// [_savePrefs] on every change).
  void _loadPrefs() {
    var cloudOrigin = loadPref('cloudOrigin');
    if (cloudOrigin == null || cloudOrigin.isEmpty) {
      // One-time migration from the legacy serverMode/serverUrl prefs (P3c-2).
      final legacy = resolveLegacyCloudSetup(
        savedMode: loadPref('serverMode'),
        savedUrl: loadPref('serverUrl') ?? '',
        authToken: loadPref('authToken') ?? '',
        isWeb: kIsWeb,
      );
      cloudOrigin = legacy.cloudOrigin;
      savePref('cloudOrigin', cloudOrigin);
      if ((loadPref('activeOrigin') ?? '').isEmpty) {
        savePref('activeOrigin', legacy.activeOrigin);
      }
      // Move the single-key credentials to per-origin keys so switching
      // servers stops destroying them (回切免重登). The legacy keys are left
      // in place unread — harmless, and a rollback safety net.
      final legacyToken = loadPref('authToken') ?? '';
      if (legacyToken.isNotEmpty &&
          (loadPref('authToken:$cloudOrigin') ?? '').isEmpty) {
        savePref('authToken:$cloudOrigin', legacyToken);
        savePref('authUser:$cloudOrigin', loadPref('authUser') ?? '');
      }
    }
    _registry.bind(cloudOrigin);
    _registry.load(seed: cloudOrigin);
    // A blank origin means "no cloud server configured yet", NOT "point the
    // client at nothing". `Uri.tryParse('')` succeeds — it returns a perfectly
    // valid Uri with an empty host — so this used to overwrite the client's own
    // default with it, and everything that asks the base url a question got a
    // nonsense answer. `_isLocalBackend()` in particular went false on a fresh
    // profile, which silently disabled dev auto-login. Same shape as the
    // deployment bug where `${VAR:-}` made an unset variable arrive as `""` and
    // `env::var` answered `Ok("")` instead of `Err`: blank must mean absent.
    //
    // A build-time `MICA_API_BASE_URL` outranks the saved pref. It is only ever
    // passed deliberately (a dev run, CI) — the shipped app does not set it —
    // and without this a dev build inherits whichever server the installed copy
    // was last pointed at. That is how `flutter run` came up signed in to
    // PRODUCTION, one keystroke away from writing test data into real notes.
    const pinned = String.fromEnvironment('MICA_API_BASE_URL');
    final base = resolveApiBase(pinned: pinned, saved: _cloudOrigin);
    if (base != null) {
      _api.baseUri = base;
    }
    // The palette is NOT seeded here. It used to be, and that is exactly one
    // frame too late: this runs from initState, inside MaterialApp's first
    // build, where notifying an ancestor cannot rebuild it. main() does it
    // before runApp instead.
    _loadAppearancePrefs();
    if (!kIsWeb)
      _pending = PendingUploads.fromJson(loadPref('pendingBlobUploads'));
  }

  /// The appearance subset of [_loadPrefs] — the state that mirrors the synced
  /// prefs. Split out so [_syncSettingsFromCloud] can re-derive exactly this
  /// after writing cloud values into the prefs, without re-running the origin/
  /// server migration half of [_loadPrefs] mid-session.
  void _loadAppearancePrefs() {
    final fontScale = double.tryParse(loadPref('fontScale') ?? '');
    final fontFamily = loadPref('fontFamily');
    _appearance = EditorAppearance(
      // Bounds derived from the settings control's px detents (see kFontBasePx
      // in dialogs.dart) rather than restated as literals here — restated
      // bounds are how a widened slider and a load-time clamp drift apart.
      fontScale: (fontScale ?? 1.0).clamp(
        fontScaleFromPx(kFontPxMin),
        fontScaleFromPx(kFontPxMax),
      ),
      fontFamily: (fontFamily == null || fontFamily.isEmpty)
          ? null
          : fontFamily,
    );
    // Clamp a stored value into the discrete-step range; absent → the readable
    // default. The editor still caps the *render* at the window width, so a wide
    // saved value on a small window simply fills it.
    _pageWidth =
        (double.tryParse(loadPref('pageWidth') ?? '') ?? kPageWidthDefault)
            .clamp(kPageWidthMin, kPageWidthMax);
    _reHostImages = loadPref('reHostImages') != 'false';
    _showFormatBar = loadPref('showFormatBar') == 'true';
    _showPageTitle = loadPref('showPageTitle') != 'false';
    _aiEnabled = loadPref('aiEnabled') == 'true';
  }

  void _savePrefs() {
    savePref('fontScale', _appearance.fontScale.toString());
    savePref('fontFamily', _appearance.fontFamily ?? '');
    savePref('pageWidth', _pageWidth.toString());
    savePref('reHostImages', _reHostImages.toString());
    savePref('showFormatBar', _showFormatBar.toString());
    savePref('showPageTitle', _showPageTitle.toString());
    savePref('aiEnabled', _aiEnabled.toString());
    _schedulePushSettings();
  }

  // ── Settings sync (ui/settings_sync.dart has the policy) ──────────────────
  // The account's settings follow it across devices; local prefs stay as the
  // offline cache and the whole story for 本地模式.

  Timer? _settingsPushTimer;

  /// True while cloud values are being written into local state, so the
  /// controller listeners this apply pass triggers don't push those same
  /// values straight back up.
  bool _applyingCloudSettings = false;

  /// Debounced write-through of the synced prefs. Debounced because the font
  /// and page-width sliders save on every detent; one PUT after the burst.
  /// Failures are logged and dropped — the local save already happened, and a
  /// toggle that flips back because the network blinked would be worse than a
  /// device that catches up on its next change.
  void _schedulePushSettings() {
    if (_applyingCloudSettings) return;
    final session = _session;
    if (session == null || _activeOrigin == kLocalOrigin) return;
    _settingsPushTimer?.cancel();
    _settingsPushTimer = Timer(const Duration(seconds: 1), () async {
      final s = _session;
      if (s == null || _activeOrigin == kLocalOrigin) return;
      try {
        await _api.putUserSettings(s.accessToken, settingsPayload(loadPref));
      } catch (e) {
        debugPrint('settings push failed: $e');
      }
    });
  }

  /// Adopt the account's settings on sign-in/restore. Cloud wins — that is
  /// what "synced" means from the second device's chair; the device that wrote
  /// last IS the cloud value. An account with no blob yet gets seeded from
  /// this device instead, so the first sign-in from the device that holds the
  /// user's real preferences makes those the account's.
  Future<void> _syncSettingsFromCloud(AuthSession session) async {
    try {
      final cloud = await _api.getUserSettings(session.accessToken);
      if (!mounted || _session?.accessToken != session.accessToken) return;
      final apply = cloudSettingsToApply(cloud);
      if (apply.isEmpty) {
        _schedulePushSettings();
        return;
      }
      _applyingCloudSettings = true;
      try {
        apply.forEach(savePref);
        // Re-derive everything that mirrors those prefs. The two controllers
        // notify their own scopes; the shell state needs a rebuild.
        loadPersistedThemeMode();
        loadPersistedLocale();
        setState(_loadAppearancePrefs);
      } finally {
        _applyingCloudSettings = false;
      }
      // A cloud blob from an older client may lack keys this device has: push
      // the merged view back so the account converges instead of ping-ponging.
      _schedulePushSettings();
    } catch (e) {
      debugPrint('settings sync skipped: $e');
    }
  }

  /// Sign in with the dev account on startup. Falls back to registering it the
  /// first time, and silently leaves the login screen up if the API is down.
  Future<void> _devAutoLogin() async {
    if (_session != null) {
      return;
    }
    const form = AuthFormValue(
      email: kDevEmail,
      displayName: 'Demo User',
      password: kDevPassword,
    );
    await _authenticate(AuthMode.login, form);
    if (mounted && _session == null) {
      await _authenticate(AuthMode.register, form);
    }
    // Dev auto-login is a best-effort convenience for a running local backend.
    // When none is up (`just app` with no `just dev`), both attempts fail
    // with a raw ClientException that _run parks in the banner — a scary red
    // toast for what is a no-op. No backend just means "stay signed out in the
    // local world", so clear it. (Nothing else runs before this on startup, so
    // the only message that can be here is the one these two attempts set.)
    if (mounted && _session == null && _message != null) {
      setState(() => _message = null);
    }
    if (mounted && _session != null) _adoptSignedInOrigin();
  }

  /// Record the server we just signed in to as the configured cloud origin.
  ///
  /// Signing in and being ABLE TO REACH what you signed in to are two different
  /// things here: the world switcher lists configured servers, so a session
  /// against an origin that was never written to `cloudOrigin` is an orphan —
  /// the account exists, the token works, and the cloud world simply cannot be
  /// opened. That is what dev auto-login did on a fresh profile: it logged in
  /// successfully and left the user staring at "add a server".
  ///
  /// Only fills a BLANK origin. Someone who has chosen a server keeps it.
  void _adoptSignedInOrigin() {
    if (_cloudOrigin.isNotEmpty) return;
    final origin = apiOrigin(_api.baseUri);
    if (origin.isEmpty) return;
    savePref('cloudOrigin', origin);
    _registry.bind(origin);
    _registry.load(seed: origin);
  }

  /// Persist the access token + user so a restart (desktop) or browser refresh
  /// (web) restores the session instead of forcing re-login. Plaintext in the
  /// same prefs store as other settings — a per-origin `window.localStorage`
  /// entry on web, a JSON file on desktop (DPAPI encryption is a noted hardening
  /// follow-up; the localStorage copy is likewise XSS-exposed). The token is
  /// server-specific — [_signOut] (which a server switch also calls) clears it,
  /// so a saved token always matches the configured backend.
  // Credentials are keyed per cloud origin (P3c-2), so pointing the app at a
  // different server doesn't destroy the previous server's session — switching
  // back restores it without retyping.
  /// Servers we hold credentials for — entering one costs no password.
  ///
  /// Keyed off `authUser:$origin` rather than the access token: the token is the
  /// part that expires (and on web is never on disk at all), while the stored
  /// user is what [_restoreSession] needs to rebuild a session from the refresh
  /// token. Using the token here would mark a perfectly good sign-in as absent
  /// every time it lapsed, which is most mornings.
  Set<String> _signedInOrigins() => {
    for (final origin in _servers)
      if ((loadPref('authUser:$origin') ?? '').isNotEmpty) origin,
  };

  /// The display name stored for [origin], or null if we hold none.
  ///
  /// Read from prefs rather than `_session`, because the two disagree exactly
  /// where it matters: entering 本地模式 drops the in-memory session while the
  /// credentials stay on disk. Asking `_session` there answers "nobody is signed
  /// in", which is how the world picker ended up demanding a password for a
  /// server it could have walked straight into.
  String? _storedDisplayName(String origin) {
    final raw = loadPref('authUser:$origin');
    if (raw == null || raw.isEmpty) return null;
    try {
      return User.fromJson(jsonDecode(raw) as Map<String, dynamic>).displayName;
    } catch (_) {
      return null; // corrupt value: treat as no stored identity
    }
  }

  void _persistSession(AuthSession session) {
    savePref('authToken:$_cloudOrigin', session.accessToken);
    savePref('authUser:$_cloudOrigin', jsonEncode(session.user.toJson()));
    // The refresh token rotates on every use, so this write is what keeps the
    // sign-in alive across a restart: persist the replacement or the next launch
    // spends a burnt token and the user is bounced to the login screen.
    savePref('refreshToken:$_cloudOrigin', session.refreshToken);
  }

  void _clearPersistedSession() {
    savePref('authToken:$_cloudOrigin', '');
    savePref('authUser:$_cloudOrigin', '');
    savePref('refreshToken:$_cloudOrigin', '');
    // Also clear the legacy single-key copies so an explicit sign-out can't be
    // resurrected by a future migration re-run.
    savePref('authToken', '');
    savePref('authUser', '');
  }

  /// Renews the access token before it lapses. See [SessionRefresher] for the
  /// two rules it exists to enforce (renew early; never two at once).
  late final SessionRefresher _refresher = SessionRefresher(
    refresh: (token) => _api.refreshSession(token),
  );

  /// Notices a profile edited on another device — see [ProfileWatch] for why
  /// this is a look and not a push.
  late final ProfileWatch _profileWatch = ProfileWatch(
    fetch: (token) => _api.fetchMe(token),
  );

  /// Adopt a profile change made elsewhere. Rate-limited and change-gated by
  /// [ProfileWatch], so calling it from the app's hot paths is cheap and does
  /// not churn state.
  ///
  /// Silent on failure by design: it rides along on other work, and a blip here
  /// must not put a banner over an action that succeeded.
  Future<void> _refreshProfile() async {
    final session = _session;
    if (session == null) return;
    // Which ACCOUNT this poll is about. Loose on purpose: see [_accountIdentity].
    final identity = _accountIdentity;
    try {
      final user = await _profileWatch.poll(session);
      // Re-read the session: the poll awaited a round trip, and a sign-out or a
      // token rotation in that window would make `session` the wrong thing to
      // build on — copyWith on the stale one would resurrect a dead token.
      //
      // The identity check is the other half, and the one the re-read cannot do
      // on its own: a session that changed USER or SERVER is a different account,
      // and writing this user's profile into it would show the wrong name and
      // avatar — then persist them under the new server's key.
      final current = _session;
      if (user == null || current == null || !mounted) return;
      if (_accountIdentity != identity) return;
      final updated = current.copyWith(user: user);
      _sessionCtrl.adoptSession(updated);
      _persistSession(updated);
      // No ImageCache evict needed: the address carries the version
      // (`ui/avatar_url.dart`), so a new picture is a new URL and the stale
      // entry is simply never asked for again.
    } catch (_) {
      // Offline, or a 401 that the surrounding action already handles.
    }
  }

  Future<void> _ensureFreshSession() async {
    final session = _session;
    if (session == null) return;
    // Bind the round trip to the sign-in it was started for. The refresh takes a
    // network round trip, and a sign-out, a server switch, or signing in as
    // somebody else all land inside that window — after which `setState(() =>
    // _session = next)` would install the OLD account's tokens over the new
    // session, and `_persistSession` would file them under whatever `_cloudOrigin`
    // happens to be by then. That is a resurrection, not a stale read: the user
    // is back on an account they left.
    final identity = _sessionIdentity;
    try {
      final next = await _refresher.ensureFresh(session);
      if (next == null || !mounted) return;
      if (_sessionIdentity != identity) return;
      _sessionCtrl.adoptSession(next);
      // Persist immediately: the refresh token just rotated, and the copy on
      // disk is now burnt. Losing this write costs the sign-in at next launch.
      _persistSession(next);
    } on ApiException catch (error) {
      // The server refused the refresh token: 30 days idle, revoked, or its
      // family burnt by reuse detection. Nothing to recover — say so, rather
      // than firing doomed requests and leaving the user at `unauthorized`.
      // Only for the session that made the call: a refusal aimed at a sign-in
      // the user has already left must not end the one they are on now.
      if (error.isUnauthorized && _sessionIdentity == identity) {
        _endExpiredSession();
      }
    } catch (_) {
      // Offline / server down: keep the session. The token may well still be
      // good, and local-first means there is work to do without a network.
    }
  }

  /// The sign-in is over and cannot be renewed. Drop it and say so plainly.
  void _endExpiredSession() {
    if (!mounted || _session == null) return;
    _clearPersistedSession();
    // Only the session drops — the workspace list survives so the local
    // world can fall back coherently (SessionController.expireSession).
    _sessionCtrl.expireSession();
    setState(() => _message = context.l10n.snackSessionExpired);
    _fallBackToLocalWorld();
  }

  /// Restore a persisted cloud/self-hosted session on startup so the user isn't
  /// forced to re-login every launch. Cheaply rejects an expired token by its
  /// JWT `exp`, then validates the rest by loading workspaces: a 401 (revoked /
  /// server JWT-secret changed) drops the token; a transient network error keeps
  /// it (this launch shows login, the next retries).
  /// Losing the cloud credentials (expired/revoked token) leaves a stale cloud
  /// activeOrigin behind — fall back to the local world (desktop) so a restart
  /// doesn't keep opening an empty cloud pane (matches _signOut's semantics).
  void _fallBackToLocalWorld() {
    if (!_local.available || _activeIsLocal) return;
    setState(() => _activeOrigin = 'local');
    savePref('activeOrigin', _activeOrigin);
  }

  Future<void> _restoreSession() async {
    if (_session != null) return;
    var token = loadPref('authToken:$_cloudOrigin');
    final userJson = loadPref('authUser:$_cloudOrigin');
    // Web keeps the access token in MEMORY only, so a fresh page never has one —
    // that is the point, not a fault: localStorage is readable by any same-origin
    // script, and a stored XSS there was an account takeover. The durable
    // credential is the HttpOnly refresh cookie, which script cannot read and the
    // browser sends anyway. Trade it for a session here.
    //
    // Desktop never takes this branch: it has the token on disk, DPAPI-sealed.
    if (kIsWeb &&
        (token == null || token.isEmpty) &&
        userJson != null &&
        userJson.isNotEmpty) {
      try {
        // Empty string, not the missing token: the server reads an absent/blank
        // body field as "use the cookie".
        final recovered = await _api.refreshSession('');
        if (!mounted) return;
        _persistSession(recovered);
        token = recovered.accessToken;
      } on ApiException catch (error) {
        // The cookie is gone or burnt (signed out elsewhere, 30 days idle,
        // reuse detected). That is a real end of session, not a transient.
        if (error.isUnauthorized) {
          _clearPersistedSession();
          _fallBackToLocalWorld();
        }
        return;
      } catch (_) {
        return; // offline / server down: keep whatever is left, try next load
      }
    }
    if (token == null ||
        token.isEmpty ||
        userJson == null ||
        userJson.isEmpty) {
      return;
    }
    final refreshToken = loadPref('refreshToken:$_cloudOrigin') ?? '';
    AuthSession session;
    try {
      session = AuthSession(
        accessToken: token,
        refreshToken: refreshToken,
        user: User.fromJson(jsonDecode(userJson) as Map<String, dynamic>),
      );
    } catch (_) {
      _clearPersistedSession();
      return;
    }

    // The access token died while the app was closed — which, at a 24h TTL, is
    // most mornings. With a refresh token that is a non-event; without one (an
    // older server, or 30 days idle) the sign-in really is over.
    final exp = jwtExpiry(token);
    if (exp != null && !exp.isAfter(DateTime.now().toUtc())) {
      if (refreshToken.isEmpty) {
        _clearPersistedSession();
        _fallBackToLocalWorld();
        return;
      }
      try {
        // Through _refresher, not _api directly: it holds the single-flight
        // latch, and startup restore can overlap with a _run() the user just
        // triggered. Two refreshes of one token is what the server reads as
        // theft — we would burn the session we came here to rescue.
        session = await _refresher.ensureFresh(session) ?? session;
        if (!mounted) return;
        _persistSession(session);
      } on ApiException catch (error) {
        if (error.isUnauthorized) {
          _clearPersistedSession();
          _fallBackToLocalWorld();
        }
        // Anything else (server down) keeps the session for the next launch.
        return;
      } catch (_) {
        return; // offline: try again next launch, don't destroy credentials
      }
    }
    try {
      // `session.accessToken`, not the `token` we loaded: a refresh above
      // replaced it, and spending the dead one here would 401 and throw the
      // freshly-renewed sign-in away.
      final workspaces = await _api.listWorkspaces(session.accessToken);
      if (!mounted) return;
      // Land on the workspace + page open at last quit, not always the first.
      final savedWsId = loadPref('lastWorkspaceId');
      final restoredWs =
          (savedWsId == null
              ? null
              : workspaces.where((w) => w.id == savedWsId).firstOrNull) ??
          workspaces.firstOrNull;
      // Warm every workspace's tree from the on-device mirror, not just the
      // one being restored: the seed keeps server-fresh entries on top of
      // the mirror, so a first switch to any other workspace is not a cache
      // miss that renders "还没有页面" while the tree is in flight.
      _sessionCtrl.restore(
        session,
        workspaces,
        restoredWs,
        {..._warmTreesFromMirror(session), ..._viewsByWorkspace},
      );
      unawaited(_syncSettingsFromCloud(session));
      unawaited(_refreshAiConfigured());
      // The session we just restored carries the profile as it was at LAST
      // login — it came off disk, which is why a restart was not what fixed a
      // stale avatar. This is the startup counterpart to the _run() call.
      unawaited(_refreshProfile());
      await _loadSelectedWorkspaceMembers();
      await _loadSelectedWorkspaceViews();
      // The startup restore isn't a _run() action, so nothing else wires the
      // doc it just auto-opened: without this, the WS sync session (presence +
      // yrs CRDT + local-first mirror) only starts on the user's FIRST click —
      // typing before that silently rode the REST fallback with no mirror.
      _reconcileSync();
    } catch (error) {
      if (!mounted) return;
      // Revoked/invalid token → drop it; desktop falls back to the local world.
      // Keyed on the status, not on the message text: sniffing for the word
      // "unauthorized" both missed 401s worded differently and would fire on a
      // 400 that merely mentioned the word.
      if (error is ApiException && error.isUnauthorized) {
        _clearPersistedSession();
        _fallBackToLocalWorld();
        return;
      }
      // Transient (offline / server down) → keep the token and fall back to the
      // on-device page-tree mirror so the user still enters the workspace and can
      // read cached cloud content (P1c). No mirror (never synced / web) → stay on
      // the login screen as before.
      await _applyOfflineCloudNav(session);
    }
  }

  /// What to actually show a person for a failed request.
  ///
  /// The server's `message` is English prose meant for logs — `ApiException`'s
  /// own doc says as much — so a failure the user can *act on* is matched by its
  /// machine [ApiException.code] and answered in their language. Matching on the
  /// message text instead would be a second representation of the same fact: the
  /// server could reword a sentence and silently un-translate the client.
  ///
  /// Everything else still falls through to the raw message. A vague 「出错了」
  /// would be worse than an ugly true sentence, and inventing friendly copy for
  /// failures we haven't characterised is how you end up lying.
  String _apiMessage(ApiException error) {
    return switch (error.code) {
      'import_no_markdown' => context.l10n.importNoMarkdownHint,
      // The two ways an upload gets refused for "size". They are easy to
      // confuse and the fix differs: one file is too big (compress it) versus
      // the workspace is full (free space). Worse, the first fires while the
      // storage bar honestly shows room left, so relaying the server's English
      // sentence left the refusal looking like the bar was lying.
      // The cap is named only when it is actually known: `/usage` is fetched
      // lazily (settings dialog), so someone whose first action is an oversized
      // paste has never been told the number. Quoting a guess would be worse
      // than omitting it — the whole point of this copy is to be believable.
      'file_too_large' => _maxUploadBytes > 0
          ? context.l10n.uploadFileTooLargeWithLimit(
              formatBytesBinary(_maxUploadBytes),
            )
          : context.l10n.uploadFileTooLarge,
      'workspace_quota_exceeded' => context.l10n.uploadWorkspaceFull,
      _ => error.toString(),
    };
  }

  Future<void> _run(Future<void> Function() action) async {
    setState(() {
      _isBusy = true;
      _message = null;
    });

    try {
      // Renew ahead of expiry — but do not put that round trip in front of the
      // user. Proactive rather than retry-on-401 on purpose: [action] is not
      // generally idempotent (it may create a document), so replaying one after
      // a refresh could duplicate work.
      //
      // This used to be `await`ed as the FIRST thing here, which meant a whole
      // /auth/refresh sat in front of every action — including the setState
      // that paints a workspace switch, so once per token lifetime the sidebar
      // could not even highlight until the network came back. The renewal
      // window is five minutes wide (SessionRefresher.lead), so the token in
      // hand is still good: let the action go now and let the renewal ride
      // alongside it. Only a token that is ALREADY dead has to be replaced
      // first, because the action would just be refused.
      //
      // Not fire-and-forget-and-swallow: _ensureFreshSession handles its own
      // failures (401 ends the session, anything else keeps it).
      final expiry = _session?.expiresAt;
      final expired =
          expiry != null &&
          !expiry.isAfter(
            DateTime.now().toUtc().add(const Duration(seconds: 30)),
          );
      if (expired) {
        await _ensureFreshSession();
      } else {
        unawaited(_ensureFreshSession());
      }
      await action();
      _reconcileSync();
      // Unawaited: the action is done and the user is waiting on nothing here.
      // This is the app's most frequent "we just talked to the server anyway"
      // moment, which is what makes a profile changed elsewhere show up without
      // anything polling on a timer.
      unawaited(_refreshProfile());
    } on ApiException catch (error) {
      // A 401 that survived the renewal above: revoked, or the server's JWT
      // secret rotated. The session is unusable — say so instead of parroting
      // the server's bare `unauthorized`, which told the user nothing and left
      // the local mirror rendering a half-dead page that merely looked broken.
      if (error.isUnauthorized && _session != null) {
        _endExpiredSession();
      } else {
        setState(() => _message = _apiMessage(error));
      }
    } catch (error) {
      setState(() {
        _message = error.toString();
      });
    } finally {
      setState(() {
        _isBusy = false;
      });
    }
  }

  // --- Tabs ---------------------------------------------------------------
  //
  // The strip is cloud-only. The local world keeps the single-page shell: it
  // has its own `_localSelectedView` / `_localBootstrap` pair that these
  // operations do not touch, and giving it tabs would mean a second copy of
  // every operation below for no user demand.

  /// Append an empty tab and make it active.
  ///
  /// The caller then loads into it through the ordinary open/create path, so
  /// "in a new tab" and "here" reach the server identically — the only
  /// difference is which [DocTab] is active while it happens.
  void _pushTab() {
    setState(() {
      _tabs.add(DocTab()..lastActivated = ++_activationTick);
      _activeTabIndex = _tabs.length - 1;
    });
  }

  /// Open an existing page, by id, in a new tab. The `+` menu's picker.
  ///
  /// [workspaceId] is where the page actually lives. When that is somewhere
  /// else, the tab opens in the BACKGROUND and the shell stays put — see
  /// [_openElsewhereInBackgroundTab].
  void _openViewByIdInNewTab(String viewId, String? workspaceId) {
    // The same question `_selectTab` asks of a tab it is activating: would this
    // require a workspace switch? Reused rather than re-compared, so "belongs
    // somewhere else" has one definition — including its null rule, where a
    // missing workspace means "here" and must not be read as "unknown".
    final elsewhere = workspaceToSwitchTo(
      tabWorkspaceId: workspaceId,
      currentWorkspaceId: _selectedWorkspace?.id,
    );
    if (elsewhere != null) {
      unawaited(_openElsewhereInBackgroundTab(viewId, elsewhere));
      return;
    }
    _pushTab();
    unawaited(_openViewById(viewId));
  }

  /// Queue a page from ANOTHER workspace into a background tab, without moving
  /// the shell.
  ///
  /// The point of the whole thing: the `+` picker searches every workspace, so
  /// "open this in a tab" should not have to mean "and take me there". The tab
  /// carries its own [DocTab.workspaceId], and `_selectTab` already switches the
  /// shell when you click it — this only had to stop forcing that switch up
  /// front.
  ///
  /// The bootstrap is fetched HERE rather than on activation, and that is not an
  /// optimisation: `_selectTab` is explicit that "the editor reads the active
  /// tab's bootstrap, which is already in memory — switching never refetches".
  /// A tab parked with a view and no bootstrap would render as permanently
  /// loading, and clicking it would switch workspace to show nothing.
  ///
  /// Nothing is revealed in the sidebar. `_revealInTree` on an id that is not in
  /// the current tree PARKS the request until that workspace's tree loads, so
  /// revealing here would make the sidebar jump at some unrelated later moment.
  Future<void> _openElsewhereInBackgroundTab(
    String viewId,
    String workspaceId,
  ) async {
    final session = _session;
    if (session == null) return;

    // The tree of a workspace this device has never opened is not in memory —
    // `_viewsByWorkspace` fills per workspace as you visit them. Fetching it is
    // one request, and it is the tree the tab needs anyway the moment you click
    // it.
    var view = _viewsByWorkspace[workspaceId]
        ?.where((v) => v.id == viewId)
        .firstOrNull;
    if (view == null) {
      try {
        final answer = await _api.listViewsIfChanged(
          session.accessToken,
          workspaceId,
        );
        final views = answer.views;
        if (views != null && mounted) {
          setState(() {
            _sessionCtrl.replaceViews(workspaceId, views);
          });
          view = views.where((v) => v.id == viewId).firstOrNull;
        }
      } on ApiException catch (error) {
        if (mounted) setState(() => _message = error.message);
        return;
      }
    }
    if (view == null || !mounted) return;

    // A folder has no document, so there is nothing to put in a tab. Fall back
    // to switch-and-reveal, which is what a folder hit means.
    if (view.isFolder) {
      await _openSearchHit(viewId, workspaceId);
      return;
    }

    final tab = DocTab(view: view)..workspaceId = workspaceId;
    setState(() => _tabs.add(tab));
    try {
      final bootstrap = await _api.bootstrapDocument(
        session.accessToken,
        workspaceId,
        view.objectId,
      );
      if (!mounted) return;
      setState(() => tab.bootstrap = bootstrap);
    } on ApiException catch (error) {
      // Drop the tab rather than leave one that can never load: a tab stuck
      // showing a spinner is a worse answer than the error, because it looks
      // like it is still working.
      if (!mounted) return;
      setState(() {
        _tabs.remove(tab);
        _message = error.message;
      });
    }
  }

  /// Create a page and open it in a new tab. The `+` menu's first entry.
  ///
  /// No parent: the sidebar's placement rules follow where the POINTER is, and
  /// the `+` sits in the tab strip, which is nowhere in the tree. Root is the
  /// only honest answer — guessing the last-focused folder would put pages
  /// somewhere the user never pointed at.
  void _createDocumentInNewTab(String name) {
    _pushTab();
    unawaited(_createDocument(name));
  }

  /// Open [view] in a NEW tab and switch to it. The sidebar row's context menu.
  void _openViewInNewTab(DocumentView view) {
    _pushTab();
    // Deliberately AFTER the new tab is active: `_selectView` skips the load
    // when the target is already open, and it tests that against the ACTIVE
    // tab. A fresh tab has a null view, so the same page opening in a second
    // tab is correctly treated as "not open here" and loads.
    unawaited(_selectView(view));
  }

  void _selectTab(int index) {
    if (index < 0 || index >= _tabs.length || index == _activeTabIndex) return;
    final target = _tabs[index];
    setState(() {
      _activeTabIndex = index;
      _activeTab.lastActivated = ++_activationTick;
    });
    // A tab from ANOTHER workspace takes the workspace with it. Without this the
    // sidebar, the breadcrumb and every create action stay pointed at the
    // workspace we left, while the editor shows a page from this one — and the
    // next action on that page bootstraps against the wrong workspace and 404s.
    //
    // `keepOpenPage`, because the page is the reason for the switch: the plain
    // switch closes the open document, which would blank the tab just clicked.
    final switchTo = workspaceToSwitchTo(
      tabWorkspaceId: target.workspaceId,
      currentWorkspaceId: _selectedWorkspace?.id,
    );
    if (switchTo != null) {
      for (final w in _workspaces) {
        if (w.id != switchTo) continue;
        // Fire-and-forget on purpose: `_reconcileSync` below only depends on
        // the tab's own document, which is already in memory. The switch is
        // catching the SHELL up (sidebar, members), not gating the page.
        unawaited(_selectWorkspace(w, keepOpenPage: true));
        break;
      }
    }
    // The editor reads the active tab's bootstrap, which is already in memory —
    // switching never refetches. The socket follows: a tab still holding its
    // own is recognised as connected and reconnects nothing, while one that was
    // parked under the cap reconnects here.
    _reconcileSync();
  }

  /// Close the tab at [index].
  ///
  /// Refuses to close the last one: [_tabs] is an invariant-non-empty list, and
  /// "no tabs at all" is not a state the shell can render. AFFiNE makes the
  /// same call. Closing the ACTIVE tab activates the one that slides into its
  /// place (the tab to the right), falling back to the new last tab.
  void _closeTab(int index) {
    if (_tabs.length <= 1 || index < 0 || index >= _tabs.length) return;
    // BEFORE the removal, and unconditionally: once the tab is out of `_tabs`
    // nothing reaches its sockets, and `_reconcileSync` below only ever touches
    // the ACTIVE tab — so a closed background tab would keep a live WebSocket
    // and an undrained replica for the rest of the session.
    _parkTabSync(_tabs[index]);
    setState(() {
      final next = activeIndexAfterClose(
        closing: index,
        active: _activeTabIndex,
        count: _tabs.length,
      );
      _tabs.removeAt(index);
      _activeTabIndex = next;
    });
    _reconcileSync();
  }

  /// The workspace [tab]'s document lives in, falling back to the selected one.
  ///
  /// The fallback covers a tab that has not loaded anything yet (no stamp): it
  /// is about to open a page in the current workspace, so that is the right
  /// answer. A stamped tab always wins over the selection.
  Workspace? _workspaceForTab(DocTab tab) {
    final id = tab.workspaceId;
    if (id == null) return _selectedWorkspace;
    if (_selectedWorkspace?.id == id) return _selectedWorkspace;
    for (final w in _workspaces) {
      if (w.id == id) return w;
    }
    // Stamped with a workspace we are no longer a member of. Returning the
    // selected one would sync this tab's document into the wrong workspace, so
    // return nothing and let the caller close the socket instead.
    return null;
  }

  /// Open, switch, or close the document WebSocket so it always tracks the
  /// currently selected document.
  // The tree-change socket for the workspace whose SIDEBAR is showing, plus
  // which workspace that is — so reconcile can tell "same one" from "moved".
  ViewsEventsChannel? _viewsEvents;
  String? _viewsEventsWorkspaceId;

  /// Keep the tree-change socket pointed at the sidebar's workspace.
  ///
  /// Separate from the document socket reconcile below on purpose: that one
  /// keys off the OPEN DOCUMENT and bails when there is none, but an empty
  /// workspace still has a tree that MCP can grow into — the exact case this
  /// feature is for.
  void _reconcileViewsEvents() {
    final session = _session;
    final workspaceId = _activeIsLocal ? null : _selectedWorkspace?.id;
    if (session == null || workspaceId == null) {
      _viewsEvents?.dispose();
      _viewsEvents = null;
      _viewsEventsWorkspaceId = null;
      return;
    }
    if (_viewsEventsWorkspaceId == workspaceId) return;
    _viewsEvents?.dispose();
    _viewsEventsWorkspaceId = workspaceId;
    _viewsEvents = ViewsEventsChannel(
      uri: () async {
        await _ensureFreshSession();
        return viewsSocketUri(
          _api.baseUri,
          workspaceId,
          _session?.accessToken ?? session.accessToken,
        );
      },
      onChanged: () => unawaited(_refreshTreeFromServer(workspaceId)),
    )..connect();
  }

  /// Re-ask for [workspaceId]'s tree because the server said it changed.
  ///
  /// The ETag turns a ping about something this client already knows (its own
  /// action's refetch, a rename it just made) into a 304. Errors are swallowed:
  /// this runs off a background bell, there is nothing for a user to act on,
  /// and the next bell — or the manual refresh button, which stays — retries.
  Future<void> _refreshTreeFromServer(String workspaceId) async {
    final session = _session;
    if (session == null) return;
    // Claim a sequence number for THIS request. Anything older that lands later
    // is answering a question we have already replaced.
    final seq = _treeRequests.claim(workspaceId);
    final held = _viewsByWorkspace[workspaceId];
    final answer = await fetchBackgroundTree(
      () => _api.listViewsIfChanged(
        session.accessToken,
        workspaceId,
        etag: (held == null || held.isEmpty)
            ? null
            : loadPref('viewsEtag:$workspaceId'),
      ),
    );
    if (answer == null) return; // Transient failure; another bell retries.
    final views = answer.views;
    if (views == null || !mounted) return; // 304 — already current.
    // Superseded while we were in flight: a newer request exists, so this
    // response describes a state that has already been replaced. Writing it
    // would install the older tree AND its older ETag — and a stale ETag is
    // the durable half of the damage, because the next cold start sends it,
    // gets a 304, and keeps a tree the server has moved past.
    if (!_treeRequests.isCurrent(workspaceId, seq)) return;
    _commitTree(workspaceId: workspaceId, views: views, etag: answer.etag);
    _cacheCloudPageTree(workspaceId: workspaceId);
  }

  /// Install a freshly fetched tree together with the ETag that describes it.
  ///
  /// One function so the two cannot drift: the invariant the offline mirror
  /// depends on is "_a saved ETag means the mirror already holds that tree_", and
  /// it only holds if the tag is written in the same step as the tree it came
  /// with. Splitting them — as the two call sites used to — is how a tree from
  /// one response ends up paired with an ETag from another.
  void _commitTree({
    required String workspaceId,
    required List<DocumentView> views,
    String? etag,
  }) {
    // Only advance the tag when the server actually sent one. A 304 has no body
    // and no tag, and keeping the old one is exactly right — it still describes
    // the tree we are holding.
    if ((etag ?? '').isNotEmpty) {
      savePref('viewsEtag:$workspaceId', etag!);
    }
    setState(() {
      _sessionCtrl.replaceViews(workspaceId, views);
    });
  }

  void _reconcileSync() {
    _reconcileViewsEvents();
    final session = _session;
    // The socket belongs to the DOCUMENT, so it takes the workspace from the
    // tab holding it — not from `_selectedWorkspace`. Those differ for a moment
    // whenever a tab switch also switches workspaces (the shell catches up
    // asynchronously), and for that moment this would otherwise open
    // `/ws/workspaces/<the one we left>/documents/<this tab's doc>`, which the
    // server rejects because its document lookup is workspace-scoped too.
    final workspace = _workspaceForTab(_activeTab);
    final documentId = _selectedBootstrap?.document.id;

    if (session == null || workspace == null || documentId == null) {
      _closeDocumentSync();
      return;
    }

    if (_sync?.documentId == documentId) {
      return;
    }

    _closeDocumentSync();
    final sync = DocumentSyncClient(
      documentId: documentId,
      uri: documentSocketUri(
        _api.baseUri,
        workspace.id,
        documentId,
        session.accessToken,
      ),
      selfName: session.user.displayName,
      onRemoteSeq: _handleRemoteSeq,
      onPresence: (users) {
        // Only the tab being LOOKED AT owns the presence row. A background tab
        // is still connected under the cap, and without this test its roster
        // would overwrite the visible one — the reader would see the avatars of
        // whoever is editing some other page. `_handleRemoteSeq` already
        // self-filters on documentId; this is the same guard for presence.
        if (mounted && _selectedBootstrap?.document.id == documentId) {
          setState(() => _presence = users);
        }
      },
    );
    _sync = sync;
    setState(() => _presence = const []);
    sync.connect();

    // A new connection just came up — park the least-recently-used ones that no
    // longer fit under the cap.
    _enforceSyncCap();

    // Open a yrs CRDT session for this doc (desktop = Rust FFI replica, web = JS
    // yjs replica — both wire-compatible). P3d: _reconcileSync itself only runs
    // off cloud state (session + _selectedWorkspace + _selectedBootstrap — all
    // cloud-world fields), so the old mode guard is redundant; a local-world
    // selection never reaches here.
    unawaited(_setupCloudYrs(documentId, workspace, session));
  }

  Future<void> _setupCloudYrs(
    String documentId,
    Workspace workspace,
    AuthSession session,
  ) async {
    // Desktop pins the CRDT actor to the device's stable client id; web has no
    // on-device store, so it uses a per-session yjs actor (placeholder id here).
    var clientId = _deviceClientId;
    if (clientId == null) {
      clientId = await _local.deviceClientId() ?? (kIsWeb ? BigInt.zero : null);
      _deviceClientId = clientId;
    }
    if (clientId == null || !mounted) return;
    // The selection may have moved while we awaited the device id.
    if (_selectedBootstrap?.document.id != documentId ||
        _sync?.documentId != documentId) {
      return;
    }
    // C1: unacked local diffs persist per-document, so a crash / hard close
    // recovers edits the server never acked. Restored here; re-pushed on connect.
    final unackedKey = 'cloudUnacked:$documentId';
    // Local-first (Phase 1): mirror this cloud doc to the on-device store so it
    // reads offline across a restart. deviceClientId() above opened the store.
    // P4-2: on web the mirror is IndexedDB-backed instead (null only when
    // IndexedDB is unavailable → online-only, as before).
    var persistence = _local.cloudDocStore(documentId);
    persistence ??= await openWebDocStore(_cloudOrigin, documentId);
    // The selection may have moved while the browser store opened. If it did,
    // dispose the store we just opened — on web it holds a single-writer Web
    // Lock that would otherwise leak (orphaned, never disposed → that doc can
    // never take the writable mirror again this page load).
    if (!mounted ||
        _selectedBootstrap?.document.id != documentId ||
        _sync?.documentId != documentId) {
      persistence?.dispose();
      return;
    }
    if (persistence != null) {
      // P2b: the append-log is now the durable outbox. One-time migration — fold
      // any legacy prefs `cloudUnacked` queue into it (append THEN delete; the
      // reverse order would drop in-flight edits), so unpushed edits from before
      // P2b survive and get pushed on connect.
      final legacy = _loadUnacked(unackedKey);
      if (legacy.isNotEmpty) {
        for (final diff in legacy) {
          persistence.appendOutbox(diff);
        }
        savePref(unackedKey, ''); // the log owns the outbox now
      }
    }
    final yrs = CloudSyncSession(
      // Rebuilt per connection attempt, and renewed first. This session lives as
      // long as the document stays open — hours — while the access token lapses
      // in one. Renewal is otherwise only driven by `_run()` (a user action), so
      // an idle window longer than the TTL used to leave every reconnect
      // replaying a dead token: 401, back off, repeat, silently forever.
      uri: () async {
        await _ensureFreshSession();
        return documentSocketUri(
          _api.baseUri,
          workspace.id,
          documentId,
          _session?.accessToken ?? session.accessToken,
        );
      },
      clientId: clientId,
      onReady: (_, _) {
        _clearSyncBanner(); // B3: a fresh bootstrap means sync recovered.
        _applyCloudBlocks(documentId);
        // Now online for this doc: drain any images inserted offline (§7.1).
        unawaited(_reconcilePendingUploads(documentId));
        // Comments are anchored into this document's CRDT, so they can only be
        // resolved once it is bootstrapped.
        unawaited(_loadComments(documentId));
      },
      onRemoteBlocks: (_) => _applyCloudBlocks(documentId),
      onFault: (reason, count) => _onCloudSyncFault(documentId, reason, count),
      // Reaching the server means we're back online — leave the P1c offline-nav
      // fallback (refetch workspaces/roles) AND drain every other cloud doc's
      // un-pushed offline outbox, not just this active one.
      onServerConnected: _onCloudOnline,
      onSyncPhase: (phase) {
        if (mounted) setState(() => _syncPhase = phase);
      },
      // Desktop's durable outbox is the append-log (persistence); web / no store
      // keeps the in-memory queue + prefs crash-recovery (C1).
      restoreUnacked: persistence == null ? _loadUnacked(unackedKey) : null,
      onPersistUnacked: persistence == null
          ? (unacked) => _saveUnacked(unackedKey, unacked)
          : null,
      persistence: persistence,
    );
    _cloudSession = yrs;
    yrs.connect();
  }

  /// Rebuild the selected bootstrap from the yrs replica's blocks so the editor
  /// reconciles to the CRDT state (preserving unsent local edits).
  void _applyCloudBlocks(String documentId) {
    final session = _cloudSession;
    final boot = _selectedBootstrap;
    if (!mounted ||
        session == null ||
        boot == null ||
        boot.document.id != documentId) {
      return;
    }
    final blocks = session.allBlocks();
    if (blocks.isEmpty) return;
    setState(() {
      _selectedBootstrap = DocumentBootstrap(
        document: boot.document,
        view: boot.view,
        snapshot: DocumentSnapshot(
          // Bump the version so the editor actually reconciles to this CRDT
          // state. The editor only re-reads `nodes` when `version` changes
          // (editor.dart didUpdateWidget); keeping it equal meant the yrs
          // content — the source of truth for a doc whose op snapshot is stale
          // (e.g. edited via the yrs path) — was silently NOT applied, so on
          // (re)opening the page it showed the empty op snapshot = "lost".
          // Monotonic +1 per rebuild; `versionSeq` isn't sent back to the
          // server on the yrs path, and remote-seq tracking uses
          // `document.currentSeq`, so bumping it here is local-only and safe.
          versionSeq: boot.snapshot.versionSeq + 1,
          schemaVersion: boot.snapshot.schemaVersion,
          payload: {...boot.snapshot.payload, 'blocks': blocks},
        ),
      );
    });
  }

  /// The editor's caret moved — broadcast it (debounced) as awareness so other
  /// collaborators see this user's cursor.
  void _onEditorSelection(String? blockId, int? offset) {
    _cursorTimer?.cancel();
    _cursorTimer = Timer(const Duration(milliseconds: 120), () {
      _sync?.sendCursor(blockId, offset);
    });
  }

  /// Installed as [appExitFlush]: a fast, synchronous local-durability flush run
  /// on the desktop quit path just before exit(0), so terminating the process
  /// hard loses no debounced edits. Persists both worlds' active state; the
  /// cloud socket is intentionally not drained (unacked edits already sit in the
  /// local outbox and resend next launch). Each guard is independent so one
  /// failing store never blocks the other — or the quit.
  Future<void> _flushForExit() async {
    // Drain the editor's 400ms text debounce FIRST, so the last-typed segment is
    // committed through onOps (→ local store / cloud outbox) before we persist
    // below. Without this, typing-then-quitting loses the final <=400ms: the
    // edit is still sitting in the controller's dirty set, never handed to a
    // durable store. Awaited (not fire-and-forget) because onOps dispatches on a
    // microtask + async apply — a synchronous exit(0) would beat it. Bounded: the
    // apply is a local FFI / in-memory enqueue, not a network round-trip.
    try {
      final flush = _activeEditorFlush;
      if (flush != null) await flush();
    } catch (_) {}
    try {
      _cloudSession?.flushForExit();
    } catch (_) {}
    try {
      _local.flush();
    } catch (_) {}
  }

  /// Drop [tab]'s sockets while keeping everything the reader can see.
  ///
  /// The tab keeps its `view` and `bootstrap`, so it still renders the page it
  /// was showing — it just stops receiving remote edits until it is activated
  /// again, which reconnects it through [_reconcileSync].
  ///
  /// `drainAndDispose` rather than `dispose`: the outgoing replica may hold
  /// edits the server has not acked, and dropping the session without draining
  /// them loses content. This is the same call [_closeDocumentSync] makes, for
  /// the same reason.
  void _parkTabSync(DocTab tab) {
    tab.sync?.dispose();
    tab.sync = null;
    final cloud = tab.cloudSession;
    tab.cloudSession = null;
    if (cloud != null) unawaited(cloud.drainAndDispose());
  }

  /// Park live tabs past [kMaxLiveSyncTabs]; see [tabsToPark] for the choice.
  void _enforceSyncCap() {
    for (final tab in tabsToPark(_tabs, _activeTab)) {
      _parkTabSync(tab);
    }
  }

  /// Tear down EVERY tab's sockets, not just the active one.
  ///
  /// For sign-out, server switch and app dispose. [_closeDocumentSync] is
  /// active-tab-scoped by design (it is the doc-switch path), so those callers
  /// would otherwise leave background tabs holding open sockets — still
  /// authenticated with the credentials that were just discarded.
  void _closeAllDocumentSync() {
    for (final tab in _tabs) {
      if (!identical(tab, _activeTab)) _parkTabSync(tab);
    }
    _closeDocumentSync();
  }

  void _closeDocumentSync() {
    _syncRefetchTimer?.cancel();
    _syncRefetchTimer = null;
    _cursorTimer?.cancel();
    _sync?.dispose();
    _sync = null;
    final cloud = _cloudSession;
    _cloudSession = null;
    // C2: let the outgoing session flush + drain its outbox before closing, so a
    // doc switch / workspace change / sign-out doesn't hard-drop unacked edits.
    // Fire-and-forget — the page-switch path (_selectView) still awaits an
    // explicit drainOutbox first; the app-close hard case needs C1 (outbox
    // persistence) for a full guarantee.
    if (cloud != null) unawaited(cloud.drainAndDispose());
    if (_presence.isNotEmpty) {
      setState(() => _presence = const []);
    }
  }

  /// A cloud replica hit an integrity fault it wouldn't silently absorb (red line
  /// #1). The session already self-heals with a capped re-bootstrap; here we log
  /// it. Surfacing it to the user (a "sync paused — reload" banner) + resetting
  /// on recovery is B3, the next M-R item.
  void _onCloudSyncFault(String documentId, String reason, int count) {
    debugPrint(
      '[cloud-sync] integrity fault ($reason) on $documentId — #$count',
    );
    // The session auto-heals (capped re-bootstrap) for the first few consecutive
    // faults; past that it's genuinely stuck (B3). Surface it once so the user
    // knows edits may not be reaching the cloud — with a one-tap retry — instead
    // of failing silently (red line #1).
    if (count <= 3 || _syncBannerShown || !mounted) return;
    _syncBannerShown = true;
    final l10n = context.l10n;
    ScaffoldMessenger.maybeOf(context)?.showMaterialBanner(
      MaterialBanner(
        content: Text(l10n.snackCloudSyncPaused),
        leading: const Icon(Icons.cloud_off_outlined),
        actions: [
          TextButton(onPressed: _retryCloudSync, child: Text(l10n.commonRetry)),
          TextButton(
            onPressed: _clearSyncBanner,
            child: Text(l10n.snackDismiss),
          ),
        ],
      ),
    );
  }

  /// The editor's op pipeline (controller `_send`) failed to commit a batch —
  /// most importantly a StoreCloudDocStore.appendOutbox StateError, which used to
  /// be swallowed by a bare `catchError`, silently dropping the edit from the
  /// durable outbox (red line #1: never lose an edit without a trace). Count and
  /// log it; when a cloud session is what's failing, reuse the "sync paused"
  /// banner+retry once past its threshold. In local-only mode there's nothing to
  /// re-bootstrap, so we just log/count (still better than the silent swallow).
  void _onEditorFault(Object error, int count) {
    debugPrint('[editor-op] commit failed — #$count: $error');
    if (_cloudSession != null) {
      _onCloudSyncFault(
        _selectedBootstrap?.document.id ?? 'editor',
        'editor_op_failed',
        count,
      );
    }
  }

  /// Clear the sync-paused banner (B3) — recovery succeeded, doc switched, or the
  /// user dismissed it.
  void _clearSyncBanner() {
    if (!_syncBannerShown) return;
    _syncBannerShown = false;
    if (mounted)
      ScaffoldMessenger.maybeOf(context)?.hideCurrentMaterialBanner();
  }

  /// Retry a stuck cloud sync (B3): tear the session down and reconcile, which
  /// cold-bootstraps a fresh replica (unacked edits persist and replay).
  void _retryCloudSync() {
    _clearSyncBanner();
    _closeDocumentSync();
    _reconcileSync();
  }

  /// Load a document's persisted unacked-diff queue (C1 crash recovery). Stored
  /// as a JSON array of base64 diffs under `cloudUnacked:<docId>`.
  List<Uint8List> _loadUnacked(String key) {
    final raw = loadPref(key);
    if (raw == null || raw.isEmpty) return const [];
    try {
      final list = jsonDecode(raw) as List;
      return [for (final s in list) base64.decode(s as String)];
    } catch (_) {
      return const [];
    }
  }

  void _saveUnacked(String key, List<Uint8List> unacked) {
    savePref(
      key,
      unacked.isEmpty
          ? ''
          : jsonEncode([for (final b in unacked) base64.encode(b)]),
    );
  }

  /// A remote (or our own, echoed) accepted update advanced the server
  /// sequence. If it is ahead of what we hold, pull the latest snapshot. Our
  /// own edits already updated `currentSeq` via their REST response, so their
  /// echo is ignored here.
  void _handleRemoteSeq(String documentId, int serverSeq) {
    // When the yrs session owns this doc, op-model seq notifications are stale
    // noise — remote changes arrive as CRDT updates instead.
    if (_cloudSession?.isReady ?? false) return;
    final bootstrap = _selectedBootstrap;
    if (bootstrap == null ||
        bootstrap.document.id != documentId ||
        serverSeq <= bootstrap.document.currentSeq) {
      return;
    }

    _syncRefetchTimer?.cancel();
    _syncRefetchTimer = Timer(const Duration(milliseconds: 120), () {
      _refreshSelectedBootstrap(documentId);
    });
  }

  Future<void> _refreshSelectedBootstrap(String documentId) async {
    final session = _session;
    final workspace = _selectedWorkspace;
    if (session == null ||
        workspace == null ||
        _selectedBootstrap?.document.id != documentId) {
      return;
    }

    try {
      final fresh = await _api.bootstrapDocument(
        session.accessToken,
        workspace.id,
        documentId,
      );
      if (!mounted || _selectedBootstrap?.document.id != documentId) {
        return;
      }
      setState(() {
        _selectedBootstrap = fresh;
      });
    } catch (_) {
      // Transient sync refetch failures are non-fatal; the next update retries.
    }
  }

  @override
  void dispose() {
    _registry.removeListener(_syncRegistryState);
    _sessionCtrl.removeListener(_syncSessionState);
    _import.removeListener(_syncImportState);
    themeModeController.removeListener(_schedulePushSettings);
    localeController.removeListener(_schedulePushSettings);
    _settingsPushTimer?.cancel();
    _closeAllDocumentSync();
    _viewsEvents?.dispose();
    super.dispose();
  }

  void _syncRegistryState() {
    if (!mounted) return;
    setState(() {
      _servers = _registry.servers;
      _cloudOrigin = _registry.cloudOrigin;
    });
  }

  /// Copy the session hub's state into this State's view-model fields, so
  /// every build helper reading them sees the values it always did.
  void _syncSessionState() {
    if (!mounted) return;
    setState(() {
      _session = _sessionCtrl.session;
      _workspaces = _sessionCtrl.workspaces;
      _viewsByWorkspace = _sessionCtrl.viewsByWorkspace;
      _selectedWorkspace = _sessionCtrl.selectedWorkspace;
    });
  }

  /// Wire the import controller: every remote call is a closure over this
  /// State's api/session helpers, and every user-facing outcome comes back as
  /// a hook implemented here (dialogs, snackbars, the message bus). The
  /// controller itself never touches a context.
  void _initImportController() {
    _import = ImportController(
      jobStatus: (id) =>
          _api.importJobStatus(_requireSession().accessToken, id),
      listWorkspaces: () => _api.listWorkspaces(_requireSession().accessToken),
      rehostImage: (token, workspaceId, documentId,
              {required blockId, required fileName, required bytes}) =>
          _api.rehostImage(
            token,
            workspaceId,
            documentId,
            blockId: blockId,
            fileName: fileName,
            bytes: bytes,
          ),
      cancelJob: (token, id) => _api.cancelImportJob(token, id),
      fetchImageBytes: _loadEditorImageBytes,
    );
    _import.onError = (message) {
      if (mounted) setState(() => _message = message);
    };
    _import.onCancelled = (landed) {
      if (mounted) {
        ScaffoldMessenger.maybeOf(context)?.showSnackBar(
          SnackBar(content: Text(context.l10n.importCancelled(landed))),
        );
      }
    };
    _import.onWorkspacesLoaded = (workspaces) {
      if (mounted) _sessionCtrl.setWorkspaces(workspaces);
    };
    _import.onLanded = (workspace) => _selectWorkspace(workspace);
    _import.onFinished = (job, stillLinked) {
      if (!mounted) return;
      final l10n = context.l10n;
      final skipped = job.skippedTotal;
      // Anything still borrowed outranks the success line: the next thing the
      // user does after importing an export is delete what they exported.
      if (stillLinked.isNotEmpty) {
        _showImportImageFailures(job, stillLinked);
      }
      ScaffoldMessenger.maybeOf(context)?.showSnackBar(
        SnackBar(
          content: Text(
            l10n.importDone(l10n.importNotesCount(job.done)) +
                (skipped > 0 ? l10n.importSkippedSuffix(skipped) : ''),
          ),
          // Only when something was actually dropped, and only when the server
          // sent the paths: "3 skipped" with no way to see which three is a
          // notification the user can do nothing with.
          action: (skipped > 0 && job.skipped.isNotEmpty)
              ? SnackBarAction(
                  label: l10n.importViewSkipped,
                  onPressed: () => _showSkippedFiles(job),
                )
              : null,
        ),
      );
    };
    _import.addListener(_syncImportState);
  }

  /// The State's view-model copies of the controller's progress/job id — the
  /// sidebar row and its cancel button keep reading these fields unchanged.
  void _syncImportState() {
    setState(() {
      _importProgress = _import.progress;
      _importJobId = _import.jobId;
    });
  }

  Future<void> _authenticate(AuthMode mode, AuthFormValue form) {
    return _run(() async {
      // Registering no longer signs you in: the address has to be confirmed
      // first, so there is no session to take. Say so and stop here — navigating
      // into the app would be a lie about what just happened.
      if (mode == AuthMode.register) {
        // The FIRST account on an empty instance is verified server-side on
        // the spot, so telling it to go check its email is two lies in one
        // sentence: nothing was sent, and nothing needs clicking. Ask the
        // server which case this is instead of assuming the common one.
        final verified = await _api.register(form);
        if (!mounted) return;
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(
            content: Text(
              verified
                  ? context.l10n.authFirstAccountReady
                  : context.l10n.authVerifySent,
            ),
          ),
        );
        return;
      }
      final AuthSession session;
      try {
        session = await _api.login(form);
      } on ApiException catch (error) {
        // Translate the two failures a person can actually act on. Done HERE and
        // not in `_apiMessage`, because these codes mean different things
        // elsewhere: a 401 with a live session is an expired token (handled in
        // _run), and `conflict` covers stale writes too. Left alone, the form
        // showed the server's bare English word — 「unauthorized」 in a Chinese UI,
        // which is barely better than the nothing it used to show.
        if (error.isUnauthorized) {
          throw ApiException(context.l10n.loginBadCredentials);
        }
        if (error.statusCode == 409) {
          throw ApiException(context.l10n.loginEmailTaken);
        }
        // The server distinguishes "confirm your email" from "wrong password" with
        // a code precisely so this branch can exist: one of them has a next step
        // the person can take, and it is not the same next step.
        if (error.code == 'email_not_verified') {
          if (mounted) _offerResendVerification(form.email);
          throw ApiException(context.l10n.loginEmailNotVerified);
        }
        rethrow;
      }
      final workspaces = await _api.listWorkspaces(session.accessToken);

      _sessionCtrl.signIn(session, workspaces);
      _persistSession(session);
      unawaited(_syncSettingsFromCloud(session));

      unawaited(_refreshAiConfigured());
      await _loadSelectedWorkspaceMembers();
      await _loadSelectedWorkspaceViews();
    });
  }

  Future<void> _refreshWorkspaces() {
    return _run(() async {
      final session = _requireSession();
      final workspaces = await _api.listWorkspaces(session.accessToken);
      // Reconciling refetch: the selection survives when its id persists,
      // falls to null when it does not (SessionController.replaceWorkspaces).
      _sessionCtrl.replaceWorkspaces(workspaces);

      await _loadSelectedWorkspaceMembers();
      await _loadSelectedWorkspaceViews();
    });
  }

  Future<void> _createWorkspace(String name) {
    return _run(() async {
      final session = _requireSession();
      final workspace = await _api.createWorkspace(session.accessToken, name);
      final created = await _api.createDocument(
        session.accessToken,
        workspace.id,
        'Untitled',
      );
      final bootstrap = await _api.bootstrapDocument(
        session.accessToken,
        workspace.id,
        created.document.id,
      );
      _sessionCtrl.addCreated(workspace, [created.view]);
      setState(() {
        _selectedView = created.view;
        _selectedBootstrap = bootstrap;
      });

      await _loadSelectedWorkspaceMembers();
    });
  }

  Future<void> _renameWorkspace(Workspace workspace, String name) {
    return _run(() async {
      final session = _requireSession();
      final renamed = await _api.updateWorkspace(
        session.accessToken,
        workspace.id,
        name,
      );
      _sessionCtrl.applyRename(renamed);
    });
  }

  Future<void> _deleteWorkspace(Workspace workspace) {
    return _run(() async {
      final session = _requireSession();
      await _api.deleteWorkspace(session.accessToken, workspace.id);
      // Deletion falls the selection to the first remaining workspace;
      // the caller closes the page it held when this was the selected one.
      final wasSelected = _sessionCtrl.remove(workspace);
      if (wasSelected) {
        setState(() {
          _selectedView = null;
          _selectedBootstrap = null;
        });
      }
      if (wasSelected && _selectedWorkspace != null) {
        await _loadSelectedWorkspaceMembers();
        await _loadSelectedWorkspaceViews();
      }
    });
  }

  /// Switch to a workspace by id, ignoring an id that is no longer in the list.
  ///
  /// Silent on a miss rather than raising: the only caller is the search panel,
  /// whose rows are built from this same list, so a miss means the list changed
  /// under the click — and "nothing happened" beats an error about an id the
  /// user never saw.
  void _selectWorkspaceById(String workspaceId) {
    for (final w in _workspaces) {
      if (w.id != workspaceId) continue;
      if (_selectedWorkspace?.id == workspaceId) return;
      unawaited(_selectWorkspace(w));
      return;
    }
  }

  /// [keepOpenPage] leaves the active tab's document alone.
  ///
  /// Normally switching workspaces closes the open page — it belongs to the
  /// workspace being left. A TAB switch is the exception: the page is the
  /// reason for the switch, it is already loaded, and clearing it here would
  /// blank the very tab the user just clicked.
  Future<void> _selectWorkspace(
    Workspace workspace, {
    bool keepOpenPage = false,
  }) {
    savePref('lastWorkspaceId', workspace.id);
    final trace = SwitchTrace.begin(
      workspace.id,
      cached: (_viewsByWorkspace[workspace.id] ?? const []).isNotEmpty,
    );
    final remembered = keepOpenPage ? null : _rememberedViewOf(workspace.id);
    return _run(() async {
      setState(() {
        _sessionCtrl.select(workspace);
        if (!keepOpenPage) {
          // Land on the page this workspace was last on, straight from the
          // tree we already hold, so the switch selects and highlights in the
          // SAME frame as the click. Null when this workspace has never been
          // opened here — then there is genuinely nothing to show yet.
          _selectedView = remembered;
          _selectedBootstrap = null;
        }
      });
      trace.mark('shell');
      // …and the BODY from the on-device replica, so the switch lands on the
      // page rather than dropping to the home pane for a round trip. Also hands
      // the document to its CRDT session, which from here owns the body and
      // merges the server's state into it.
      //
      // NOT awaited. "It is only a local read" turned out to be worth 836ms on
      // a large document — decoding a mirrored doc is real work, and awaiting
      // it here put that in FRONT of the page-tree request, making a warm
      // switch slower than a cold one (1054ms vs 109ms, measured). It races the
      // network instead: whichever body arrives first is shown, and
      // `_seedBodyFromMirror` stands down if the fresher one won.
      if (remembered != null) {
        unawaited(
          _seedBodyFromMirror(remembered).then((_) => trace.mark('mirror')),
        );
      }
      // P3e: offline workspace switching. Already in degraded (offline) nav →
      // read the mirror directly, no per-switch network timeout. Otherwise try
      // the server; fall back to the mirror ONLY on connectivity failures —
      // an ApiException means the server answered (403/404/500) and must
      // surface, not be masked by stale cache (P1c discipline). P4-2: web has
      // a mirror too (localStorage page tree + IndexedDB docs).
      if (_offlineNav) {
        await _openWorkspaceFromMirror(workspace);
        return;
      }
      try {
        // Members are for the share sheet, not for the tree or the page. They
        // used to be awaited FIRST, so every switch spent a whole round trip
        // before it even asked for the content the user is looking at.
        unawaited(
          _loadSelectedWorkspaceMembers().catchError((Object error) {
            // Not awaited, but not swallowed either: the share sheet would
            // otherwise just show an empty member list with no reason given.
            if (mounted) setState(() => _message = '$error');
          }),
        );
        await _loadSelectedWorkspaceViews();
      } on ApiException {
        rethrow;
      } catch (_) {
        await _openWorkspaceFromMirror(workspace);
      } finally {
        trace.end();
      }
    });
  }

  /// Populate the selected cloud workspace's nav from the on-device mirror
  /// (offline switch, P3e — the AFFiNE "signed-in offline opens from cache"
  /// behavior). Members are unknowable offline (empty); the first cached view
  /// opens via its mirrored doc, and the sync session reconciles on reconnect.
  Future<void> _openWorkspaceFromMirror(Workspace workspace) async {
    final cache = _local.cachedCloudPageTree(_api.baseUri.toString());
    if (cache == null) return; // never mirrored — nothing to show
    final rebuilt = rebuildCloudNavFromCache(cache, _session?.user.id ?? '');
    final views = rebuilt.views[workspace.id] ?? const <DocumentView>[];
    // Auto-open the first DOCUMENT, never a folder (a folder has no mirrored
    // doc → blank editor + a folder wrongly highlighted as selected).
    final firstView = firstOpenableView(views);
    final bootstrap = firstView == null
        ? null
        : await _offlineCloudBootstrap(firstView);
    if (!mounted) return;
    setState(() {
      _sessionCtrl.replaceViews(workspace.id, views);
      _membersByWorkspace = {..._membersByWorkspace, workspace.id: const []};
      _selectedView = firstView;
      _selectedBootstrap = bootstrap;
      _offlineNav = true;
    });
    if (bootstrap != null) _reconcileSync();
    // An EMPTY workspace never reaches _reconcileSync (nothing to open), but
    // its sidebar still wants tree-change pings — MCP growing a first page
    // into an empty workspace is this feature's founding case. Idempotent when
    // the line above already ran.
    _reconcileViewsEvents();
  }

  /// Returns the new view's id (null on failure) so the caller can drop it into
  /// inline-rename — the sidebar name becomes editable immediately, no dialog.
  Future<String?> _createDocument(String name, {String? parentViewId}) async {
    String? newId;
    await _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      final created = await _api.createDocument(
        session.accessToken,
        workspace.id,
        name,
        parentViewId: parentViewId,
      );
      final bootstrap = await _api.bootstrapDocument(
        session.accessToken,
        workspace.id,
        created.document.id,
      );

      setState(() {
        final views = _viewsByWorkspace[workspace.id] ?? const [];
        _sessionCtrl.replaceViews(workspace.id, [..._viewsByWorkspace[workspace.id] ?? const [], created.view]);
        _selectedView = created.view;
        _selectedBootstrap = bootstrap;
      });
      newId = created.view.id;
    });
    return newId;
  }

  Future<String?> _createChildDocument(DocumentView parent, String name) {
    return _createDocument(name, parentViewId: parent.id);
  }

  /// Create a cloud folder (pure container) and add it to the tree. Unlike a
  /// document it is NOT opened in the editor (folders have no content); the
  /// user creates pages under it. Mirrored offline via [_cacheCloudPageTree].
  Future<String?> _createFolder(String name, {String? parentViewId}) async {
    String? newId;
    await _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      final view = await _api.createFolder(
        session.accessToken,
        workspace.id,
        name,
        parentViewId: parentViewId,
      );
      if (!mounted) return;
      setState(() {
        final views = _viewsByWorkspace[workspace.id] ?? const [];
        _sessionCtrl.replaceViews(workspace.id, [..._viewsByWorkspace[workspace.id] ?? const [], view]);
      });
      newId = view.id;
      // This workspace only. Unqualified, it wrote every workspace the mirror
      // had preheated into memory — which on a real account is the 2.1 s the
      // method's own comment measured, paid on every new folder.
      _cacheCloudPageTree(workspaceId: workspace.id);
    });
    return newId;
  }

  Future<String?> _createChildFolder(DocumentView parent, String name) {
    return _createFolder(name, parentViewId: parent.id);
  }

  /// Persist a new sibling order: assign evenly spaced, zero-padded positions to
  /// [orderedSiblings] (all sharing [parentViewId]) and push the ones that
  /// changed. Ordering is per-parent, so renumbering one group is self-contained.
  Future<void> _reorderViews(
    String? parentViewId,
    List<DocumentView> orderedSiblings,
  ) {
    String pad(int n) => n.toString().padLeft(10, '0');

    // Show the new order NOW; tell the server after.
    //
    // This used to await one moveView round trip PER row and only then
    // setState, so the rows sat visibly in their old places for N requests
    // before jumping. For a drag that reads as lag; for a freshly created page
    // being slid up beside the located row it read as a bug — "it appears at
    // the bottom and then moves" — and no care in the caller could hide it,
    // because the caller was already done.
    //
    // Optimistic, with the server as the correction: the loop below overwrites
    // these rows with what actually came back, and a failure surfaces through
    // _run like any other write. The local guess uses the SAME step-of-ten rule
    // the request does, so guess and answer agree instead of racing.
    final optimisticWorkspace = _selectedWorkspace;
    if (optimisticWorkspace != null) {
      setState(() {
        final views = [...?_viewsByWorkspace[optimisticWorkspace.id]];
        for (var i = 0; i < orderedSiblings.length; i++) {
          final v = orderedSiblings[i];
          final placed = DocumentView(
            id: v.id,
            parentViewId: parentViewId,
            objectId: v.objectId,
            objectType: v.objectType,
            name: v.name,
            position: pad((i + 1) * 10),
            icon: v.icon,
            updatedAt: v.updatedAt,
          );
          final idx = views.indexWhere((x) => x.id == v.id);
          // Not present yet happens on the create path: the row was added by
          // the create's own setState in this same turn, or is about to be.
          if (idx >= 0) {
            views[idx] = placed;
          } else {
            views.add(placed);
          }
        }
        _sessionCtrl.replaceViews(optimisticWorkspace.id, views);
      });
    }

    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      if (orderedSiblings.isEmpty) return;

      // ONE request for the whole group.
      //
      // This was a loop of `moveView`, awaited per sibling. It looked like it
      // only paid for the rows that actually moved — `if (view.position !=
      // desired)` — but a view is created with a `Uuid::now_v7()` position, so
      // that test is TRUE for every sibling that has not been reordered before.
      // Creating a page or folder beside a located row therefore fired one
      // round trip per existing child of that folder, in series, with `_run`
      // holding the whole app busy throughout. That is the lag: it grows with
      // the size of the folder you are creating in.
      //
      // `/views/reorder` exists for exactly this and has since 0.13.19 — its own
      // comment names the per-view loop as the thing not to do — but no client
      // code had ever called it. It validates every id before writing and does
      // the renumber in one transaction, so it also removes the half-renumbered
      // group a failure used to leave behind.
      //
      // Nothing to reconcile afterwards: the optimistic setState above wrote
      // `pad((i + 1) * 10)`, which is the same value the server assigns
      // (`format!("{:010}", (i + 1) * 10)`), for the same ids in the same order.
      // The reply is a count, not views, so there is nothing to correct from
      // either.
      //
      // One field DOES drift until the next tree load: the server bumps
      // `updated_at` on every reordered row, and the per-view reply used to
      // carry that back. Left as is on purpose — "recently edited" jumping to
      // the top because you dragged a row is the questionable behaviour here,
      // and quietly re-teaching the client to display it is not the place to
      // decide that.
      await _api.reorderViews(
        session.accessToken,
        workspace.id,
        parentViewId: parentViewId,
        orderedViewIds: [for (final v in orderedSiblings) v.id],
      );
    });
  }

  Future<List<DocumentView>> _loadTrash() async {
    final session = _requireSession();
    final workspace = _requireWorkspace();
    return _api.listTrash(session.accessToken, workspace.id);
  }

  Future<void> _restoreView(DocumentView view) {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      final views = await _api.restoreView(
        session.accessToken,
        workspace.id,
        view.id,
      );
      setState(() {
        _sessionCtrl.replaceViews(workspace.id, views);
      });
    });
  }

  /// Ask the running import to stop. It halts at the next page boundary and
  /// keeps what it already wrote — see the server's `cancel_requested`.
  Future<void> _cancelImport() async {
    final session = _session;
    if (session == null) return;
    await _import.cancel(session.accessToken);
  }

  /// The archive entries no imported page referenced.
  ///
  /// The server caps the list, so when the total exceeds what it sent, say so
  /// rather than letting the user count the rows and conclude the rest were fine.
  void _showSkippedFiles(ImportJobStatus job) {
    final l10n = context.l10n;
    showDialog<void>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(l10n.importSkippedTitle(job.skippedTotal)),
        content: SizedBox(
          width: 460,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                l10n.importSkippedBody,
                style: const TextStyle(fontSize: 12.5, height: 1.6),
              ),
              const SizedBox(height: 12),
              Flexible(
                child: ListView(
                  shrinkWrap: true,
                  children: [
                    for (final path in job.skipped)
                      Padding(
                        padding: const EdgeInsets.only(bottom: 4),
                        child: Text(
                          path,
                          style: const TextStyle(
                            fontSize: 12,
                            fontFamily: kMonoFont,
                          ),
                        ),
                      ),
                  ],
                ),
              ),
              if (job.skippedTotal > job.skipped.length)
                Padding(
                  padding: const EdgeInsets.only(top: 8),
                  child: Text(
                    l10n.importSkippedTruncated(
                      job.skippedTotal - job.skipped.length,
                    ),
                    style: TextStyle(
                      fontSize: 12,
                      color: MicaTheme.of(context).text.faint,
                    ),
                  ),
                ),
            ],
          ),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(),
            child: Text(l10n.commonClose),
          ),
        ],
      ),
    );
  }

  /// Second attempt at every image the server could not bring in — fetched
  /// from THIS machine, immediately, as part of the import. The logic lives in
  /// [ImportController.rehostFailures] (with the full history of WHY it runs
  /// here rather than later); this adapter only resolves the session.
  Future<List<ImportImageFailure>> _rehostImportFailures(
    List<ImportImageFailure> failures,
    String? workspaceId,
  ) {
    return _import.rehostFailures(_session?.accessToken, failures, workspaceId);
  }

  /// The images that are still links after both attempts.
  ///
  /// A dialog, not a snackbar: the decision it informs is "can I delete the
  /// thing I exported from now", and a line that fades after four seconds is
  /// not where that belongs.
  void _showImportImageFailures(
    ImportJobStatus job,
    List<ImportImageFailure> remaining,
  ) {
    final l10n = context.l10n;
    // Grouped by url — one dead link on forty pages is one problem, and a
    // forty-row list of the same address reads as forty.
    final byUrl = <String, List<ImportImageFailure>>{};
    for (final f in remaining) {
      byUrl.putIfAbsent(f.url, () => []).add(f);
    }
    final hidden = job.imageFailuresTotal - job.imageFailures.length;
    showDialog<void>(
      context: context,
      builder: (context) {
        final theme = MicaTheme.of(context);
        return AlertDialog(
          title: Text(l10n.importImagesStillLinkedTitle(remaining.length)),
          content: SizedBox(
            width: 520,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  l10n.importImagesStillLinkedBody,
                  style: const TextStyle(fontSize: 12.5, height: 1.6),
                ),
                const SizedBox(height: 12),
                Flexible(
                  child: ListView(
                    shrinkWrap: true,
                    children: [
                      for (final entry in byUrl.entries)
                        Padding(
                          padding: const EdgeInsets.only(bottom: 10),
                          child: Column(
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [
                              Text(
                                entry.key,
                                style: const TextStyle(
                                  fontSize: 12,
                                  fontFamily: kMonoFont,
                                ),
                              ),
                              Text(
                                l10n.importImageFailureWhere(
                                  entry.value.map((f) => f.page).join('、'),
                                  entry.value.first.attempted
                                      ? entry.value.first.reason
                                      : l10n.importImageNeverAttempted,
                                ),
                                style: TextStyle(
                                  fontSize: 11.5,
                                  height: 1.5,
                                  color: theme.text.faint,
                                ),
                              ),
                            ],
                          ),
                        ),
                    ],
                  ),
                ),
                if (hidden > 0)
                  Padding(
                    padding: const EdgeInsets.only(top: 8),
                    child: Text(
                      l10n.importSkippedTruncated(hidden),
                      style: TextStyle(fontSize: 12, color: theme.text.faint),
                    ),
                  ),
              ],
            ),
          ),
          actions: [
            TextButton(
              onPressed: () async {
                Navigator.of(context).pop();
                final left = await _rehostImportFailures(
                  remaining,
                  job.workspaceId,
                );
                if (!mounted) return;
                if (left.isEmpty) {
                  ScaffoldMessenger.maybeOf(this.context)?.showSnackBar(
                    SnackBar(content: Text(l10n.importImagesAllRecovered)),
                  );
                } else {
                  _showImportImageFailures(job, left);
                }
              },
              child: Text(l10n.importImagesRetry),
            ),
            TextButton(
              onPressed: () => Navigator.of(context).pop(),
              child: Text(l10n.commonClose),
            ),
          ],
        );
      },
    );
  }

  /// Counts for the whole-account export, shown under the export row.
  Future<({int workspaces, int pages, int imageBytes})>
  _exportAllStats() async {
    final session = _requireSession();
    return _api.exportAllStats(session.accessToken);
  }

  Future<void> _purgeView(DocumentView view) {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      await _api.purgeView(session.accessToken, workspace.id, view.id);
    });
  }

  /// Empty the cloud workspace's recycle bin; returns how many views went.
  ///
  /// Not wrapped in [_run]: the recycle-bin dialog owns this failure (it shows the
  /// error in its own body and stays open), and routing it through the app-wide
  /// banner would report it behind the dialog the user is looking at.
  Future<int> _purgeAllTrash() async {
    final session = _requireSession();
    final workspace = _requireWorkspace();
    return _api.purgeWorkspaceTrash(session.accessToken, workspace.id);
  }

  // ---------------------------------------------------------------------------
  // AI
  // ---------------------------------------------------------------------------

  Stream<String> _aiStream(String prompt, {String? system}) {
    final session = _session;
    if (session == null) {
      return Stream<String>.error(ApiException(context.l10n.aiNotSignedIn));
    }
    return _api.aiStream(session.accessToken, prompt, system: system);
  }

  Future<AiEditPreview> _previewAiEdits(
    String response,
    EditorAiSnapshot source,
  ) => _api.previewAiEdits(_requireSession().accessToken, response, source);

  Future<Uint8List> _exportPageZip() async {
    final session = _requireSession();
    final workspace = _requireWorkspace();
    final bootstrap = _selectedBootstrap;
    if (bootstrap == null) {
      throw ApiException(context.l10n.pageOpenFirst);
    }
    return _api.exportDocumentZip(
      session.accessToken,
      workspace.id,
      bootstrap.document.id,
    );
  }

  /// Export the open CLOUD page, choosing the shape by content: a page with no
  /// bundled assets exports as a single clean `.md`; one that references local
  /// images exports as a `.zip` (md + `assets/`) — a lone `.md` would dangle
  /// those `![](assets/…)` links. `title` names the file.
  Future<({Uint8List bytes, String name, String mime})> _exportPage(
    String title,
  ) async {
    final session = _requireSession();
    final workspace = _requireWorkspace();
    final bootstrap = _selectedBootstrap;
    if (bootstrap == null) {
      throw ApiException(context.l10n.pageOpenFirst);
    }
    final base = title.trim().isEmpty ? 'page' : title.trim();
    final markdown = await _api.exportMarkdown(
      session.accessToken,
      workspace.id,
      bootstrap.document.id,
      withTitle: true,
    );
    if (_markdownReferencesLocalAssets(markdown)) {
      final bytes = await _api.exportDocumentZip(
        session.accessToken,
        workspace.id,
        bootstrap.document.id,
      );
      return (bytes: bytes, name: '$base.zip', mime: 'application/zip');
    }
    return (
      bytes: Uint8List.fromList(utf8.encode(markdown)),
      name: '$base.md',
      mime: 'text/markdown',
    );
  }

  /// True if the markdown has any image whose target is a LOCAL asset (not an
  /// external `http(s)`/`data:` URL) — those need the `.zip` to travel with
  /// their files.
  bool _markdownReferencesLocalAssets(String markdown) {
    for (final m in RegExp(r'!\[[^\]]*\]\(\s*([^)\s]+)').allMatches(markdown)) {
      final url = m.group(1) ?? '';
      if (!url.startsWith('http://') &&
          !url.startsWith('https://') &&
          !url.startsWith('data:')) {
        return true;
      }
    }
    return false;
  }

  /// Export the open CLOUD page as a self-contained HTML file. `title` (the live
  /// title-bar text) is ignored here — the server names the `<h1>`/`<title>`
  /// from the stored view name; it matters only for the local path below.
  Future<String> _exportPageHtml(String title) async {
    final session = _requireSession();
    final workspace = _requireWorkspace();
    final bootstrap = _selectedBootstrap;
    if (bootstrap == null) {
      throw ApiException(context.l10n.pageOpenFirst);
    }
    return _api.exportDocumentHtml(
      session.accessToken,
      workspace.id,
      bootstrap.document.id,
      // WYSIWYG: export as wide as the editor page the user reads.
      width: _pageWidth.round(),
    );
  }

  /// Export the open LOCAL page as HTML through the FFI engine (same output as
  /// the cloud path). Unlike the ZIP/Markdown exports — which are server-only and
  /// throw `exportLocalUnsupported` locally — HTML works offline, so this closes
  /// that gap for local workspaces.
  Future<String> _localExportPageHtml(String title) async {
    // The LOCAL world tracks its open page in `_localBootstrap`, NOT the
    // cloud/shared `_selectedBootstrap` (the editor renders `_localBootstrap`
    // too — see the `local ? _localBootstrap : _selectedBootstrap` build site).
    // Reading `_selectedBootstrap` here exported a stale, unrelated doc.
    final bootstrap = _localBootstrap;
    if (bootstrap == null) {
      throw ApiException(context.l10n.pageOpenFirst);
    }
    final name = title.trim().isEmpty ? context.l10n.untitledPage : title.trim();
    final html = _local.exportDocHtml(
      bootstrap.document.id,
      name,
      contentWidth: _pageWidth.round(),
    );
    if (html == null || html.isEmpty) {
      throw ApiException(context.l10n.exportEmptyContent);
    }
    return html;
  }

  /// Export the open LOCAL page, choosing `.md` (no assets) vs `.zip` (bundled
  /// images) by content — the local mirror of [_exportPage], via the FFI engine
  /// + the same ZIP writer the cloud uses (byte-compatible). Closes the last
  /// local page-export gap (md/zip were server-only, HTML/PDF already worked).
  Future<({Uint8List bytes, String name, String mime})> _localExportPage(
    String title,
  ) async {
    final bootstrap = _localBootstrap;
    if (bootstrap == null) {
      throw ApiException(context.l10n.pageOpenFirst);
    }
    final base = title.trim().isEmpty ? context.l10n.untitledPage : title.trim();
    final result = _local.exportDocMarkdown(bootstrap.document.id, base);
    if (result == null) {
      throw ApiException(context.l10n.exportEmptyContent);
    }
    return result;
  }

  /// The open page's Markdown TEXT, for the clipboard (CLOUD world).
  ///
  /// Separate from [_exportPage] on purpose: that one is content-aware and
  /// hands back a `.zip` once the page has images, which a clipboard has no use
  /// for. Here image references stay inline as Markdown.
  Future<String> _pageMarkdownText() async {
    final session = _requireSession();
    final workspace = _requireWorkspace();
    final bootstrap = _selectedBootstrap;
    if (bootstrap == null) {
      throw ApiException(context.l10n.pageOpenFirst);
    }
    // The clipboard has no file name to carry the page's name, so the title
    // has to be in the text or it is simply lost on paste.
    return _api.exportMarkdown(
      session.accessToken,
      workspace.id,
      bootstrap.document.id,
      withTitle: true,
    );
  }

  /// Local mirror of [_pageMarkdownText].
  ///
  /// Reads `_localBootstrap`, NOT the cloud `_selectedBootstrap` — the same trap
  /// [_localExportPageHtml] documents, where reading the wrong one silently
  /// exported an unrelated, stale document.
  Future<String> _localPageMarkdownText() async {
    final bootstrap = _localBootstrap;
    if (bootstrap == null) {
      throw ApiException(context.l10n.pageOpenFirst);
    }
    final markdown = _local.exportDocMarkdownText(
      bootstrap.document.id,
      bootstrap.pageTitle,
    );
    if (markdown == null) {
      throw ApiException(context.l10n.exportEmptyContent);
    }
    return markdown;
  }

  Future<Uint8List> _exportFolderZip(DocumentView folder) async {
    final session = _requireSession();
    final workspace = _requireWorkspace();
    return _api.exportFolderZip(session.accessToken, workspace.id, folder.id);
  }

  /// Export a LOCAL folder's subtree as a ZIP via the shared Rust builder — the
  /// local mirror of [_exportFolderZip], closing the last local-export gap.
  Future<Uint8List> _localExportFolderZip(DocumentView folder) async {
    final wsId = _workspaceIdOfView(folder.id);
    final bytes = _local.exportFolderZip(wsId, folder.id);
    if (bytes == null || bytes.isEmpty) {
      throw ApiException(context.l10n.exportEmptyContent);
    }
    return bytes;
  }

  Future<Uint8List> _exportWorkspaceZip(String workspaceId) async {
    final session = _requireSession();
    return _api.exportWorkspaceZip(session.accessToken, workspaceId);
  }

  /// Settings → "Export all workspaces": one zip with every workspace this
  /// account belongs to, each under its own `<name>/` subdir (switcher order)
  /// plus a top-level `workspaces.json` manifest. Cloud-only.
  Future<void> _exportAllWorkspaces() async {
    final session = _requireSession();
    final l10n = context.l10n;
    try {
      final bytes = await _api.exportAllWorkspacesZip(session.accessToken);
      if (bytes.isEmpty) throw ApiException(l10n.exportEmptyContent);
      downloadImage(bytes, 'mica-workspaces.zip', 'application/zip');
    } catch (error) {
      if (mounted) {
        ScaffoldMessenger.of(
          context,
        ).showSnackBar(SnackBar(content: Text(l10n.exportFailed('$error'))));
      }
    }
  }

  /// Local-world search: straight to the on-device index.
  ///
  /// No session, no workspace id — a local world is one device's store, and
  /// there is nothing to scope to.
  Future<List<SearchResult>> _searchLocalWorkspace(String query) =>
      _local.searchLocal(query);

  Future<List<SearchResult>> _searchWorkspace(String query) async {
    final session = _session;
    final workspace = _selectedWorkspace;
    if (session == null || workspace == null) return const [];
    return _api.searchWorkspace(session.accessToken, workspace.id, query);
  }

  /// Every workspace at once, with the open one preferred among equally
  /// relevant hits. This is the DEFAULT scope (2026-08-28) — see
  /// `searchScopeFor` for why the old "current first, widen only when empty"
  /// rule had to go.
  Future<List<SearchResult>> _searchAllWorkspaces(String query) async {
    final session = _session;
    if (session == null) return const [];
    return _api.searchAllWorkspaces(
      session.accessToken,
      query,
      preferWorkspace: _selectedWorkspace?.id,
    );
  }

  /// Open a search hit that may belong to ANOTHER workspace.
  ///
  /// Switching first is not an optimisation, it is the only order that works:
  /// [_openViewById] resolves the view out of the CURRENT workspace's loaded
  /// page tree, and a workspace the user has never visited has no tree in
  /// memory. `_selectWorkspace` is what loads it.
  ///
  /// A null [workspaceId] means the server did not say — an older build, where
  /// the only search was per-workspace. Treat it as "the one we searched", i.e.
  /// the current one, and open in place.
  Future<void> _openSearchHit(String viewId, String? workspaceId) async {
    if (workspaceId != null && workspaceId != _selectedWorkspace?.id) {
      Workspace? target;
      for (final w in _workspaces) {
        if (w.id == workspaceId) {
          target = w;
          break;
        }
      }
      if (target == null) return;
      await _selectWorkspace(target);
      if (!mounted || _selectedWorkspace?.id != workspaceId) return;
    }
    await _openViewById(viewId);
  }

  /// Open a local search hit.
  ///
  /// Without this, local search finds pages and clicking one does nothing —
  /// which is a worse state than not searching at all: the list proves the page
  /// exists and then refuses to go there.
  Future<void> _openLocalViewById(String viewId) async {
    final view = _localViews.where((v) => v.id == viewId).firstOrNull;
    if (view != null) await _localSelectView(view);
  }

  Future<void> _openViewById(String viewId) async {
    final workspace = _selectedWorkspace;
    if (workspace == null) return;
    final views = _viewsByWorkspace[workspace.id] ?? const [];
    final view = views.where((v) => v.id == viewId).firstOrNull;
    if (view != null) {
      await _selectView(view);
    }
  }

  /// The pages that link TO [viewId] (reverse references), in whichever world is
  /// open: 本地模式 answers from the on-device store's own backlink index, the
  /// cloud from `GET .../backlinks`. Both produce the same [Backlink] shape, so
  /// the panel never branches on which world it is in.
  ///
  /// The local world had no answer here at all and the panel was hidden there —
  /// which made a page WITH inbound links look exactly like a page nobody had
  /// linked to. On web `backlinksLocal` returns empty (no on-device store), but
  /// unlike search that needs no special case: a page with no backlinks renders
  /// nothing either way, so the panel just stays hidden.
  /// The page-link graph of the open workspace, from whichever world it is.
  Future<PageGraph> _loadGraph() async {
    if (_activeIsLocal) return _local.graphLocal();
    final session = _session;
    final workspace = _selectedWorkspace;
    if (session == null || workspace == null) return PageGraph.empty;
    return _api.workspaceGraph(session.accessToken, workspace.id);
  }

  Future<List<Backlink>> _loadBacklinks(String viewId) async {
    if (_activeIsLocal) return _local.backlinksLocal(viewId);
    final session = _session;
    final workspace = _selectedWorkspace;
    if (session == null || workspace == null) return const [];
    return _api.backlinks(session.accessToken, workspace.id, viewId);
  }

  Future<void> _updateProfile(String displayName) async {
    final session = _requireSession();
    final user = await _api.updateMe(session.accessToken, displayName);
    if (mounted) {
      // copyWith, not a fresh AuthSession: rebuilding it here dropped the
      // refresh token to '' — renaming yourself would quietly cost you the
      // ability to renew, and you'd be signed out that night.
      final updated = session.copyWith(user: user);
      _sessionCtrl.adoptSession(updated);
      _persistSession(updated); // keep the saved display name fresh for restart
    }
  }

  /// Design 01's brand half, with copy checked against what the product does.
  SignInHero _signInHero(BuildContext context, {required bool offlineIsReal}) =>
      SignInHero(
        strings: SignInHeroStrings(
          tagline: context.l10n.signInTagline,
          pitch: context.l10n.signInPitch,
          features: [
            // On web the offline line carries a 「桌面端」 qualifier, because the
            // platform reading it cannot do that. On desktop it can, so the
            // qualifier would be pointless there — same claim, told truthfully
            // on each platform.
            offlineIsReal
                ? context.l10n.signInFeatureOfflineHere
                : context.l10n.signInFeatureOffline,
            context.l10n.signInFeatureCollab,
            context.l10n.signInFeatureEditor,
          ],
          badge: context.l10n.signInBadge(kAppVersion),
        ),
      );

  SignInPaneStrings _signInPaneStrings(BuildContext context) {
    final l10n = context.l10n;
    return SignInPaneStrings(
      cloudTab: l10n.worldTabCloud,
      localTab: l10n.worldLocalName,
      connected: l10n.serverStatusConnected,
      unreachable: l10n.serverStatusUnreachable,
      checking: l10n.serverStatusChecking,
      serversLabel: l10n.serversSectionLabel,
      addServer: l10n.serverAddTitle,
      removeServer: l10n.commonDelete,
      retry: l10n.commonRetry,
      localTitle: l10n.signInLocalTitle,
      localBody: l10n.signInLocalBody,
      localAction: l10n.signInLocalAction,
      signedIn: l10n.worldSignedInBadge,
    );
  }

  /// Does this server answer? Straight to `/api/health`, unauthenticated — the
  /// point is to tell "wrong address / not running" apart from "wrong password",
  /// which is the failure a first-time user actually hits.
  /// Whether the server at [_regOrigin] accepts new registrations, as of the
  /// last probe. Null — including "we asked a server too old to answer" — means
  /// DON'T KNOW, and the registration entry then stays exactly as it was.
  /// Guessing "closed" would hide the only way into a fresh instance; guessing
  /// "open" would show a door that 403s.
  bool? _registrationOpen;

  /// Which origin [_registrationOpen] describes. Kept beside it so switching
  /// servers cannot show the previous server's answer: the getter below only
  /// trusts it while the two still agree.
  String? _regOrigin;

  bool? get _activeRegistrationOpen =>
      _regOrigin == _activeOrigin ? _registrationOpen : null;

  Future<bool> _probeServer(String origin) async {
    final base = Uri.tryParse(origin);
    if (base == null || base.host.isEmpty) return false;
    try {
      // `/api/ready`, not `/api/health`. The sign-in screen's real question is
      // "can you serve me", not "is your process alive" — an instance whose
      // database is down cannot sign anyone in. And only the readiness probe
      // touches the database, which is exactly what answering
      // `registration_open` requires (a brand-new instance with no users still
      // accepts its first account, however the flag is set).
      final r = await http
          .get(base.resolve('/api/ready'))
          .timeout(const Duration(seconds: 4));
      if (r.statusCode != 200) return false;
      bool? open;
      try {
        final body = jsonDecode(r.body);
        if (body is Map && body['registration_open'] is bool) {
          open = body['registration_open'] as bool;
        }
      } catch (_) {
        // A 200 with a body we cannot read still proves reachability, and a
        // server older than this field simply doesn't send it. Either way the
        // answer is "don't know", never "closed".
      }
      // setState only on the way OUT: this runs inside the pane's probe, and
      // touching parent state on the way in would land during its build.
      if (mounted) {
        setState(() {
          _regOrigin = origin;
          _registrationOpen = open;
        });
      }
      return true;
    } catch (_) {
      return false;
    }
  }

  /// The sign-in screen AS THE SCREEN — not the route.
  ///
  /// Reached whenever the world in effect is a server and there is no session:
  /// on web that is the only state there is, and on desktop it is what you land
  /// in after signing out or after switching to a server you have not signed in
  /// to. No onClose, because nothing is behind it; on desktop the pane's 本地模式
  /// tab (and its 「开始使用」) is the way out, and web has no local world to go to.
  Widget _signInGate(BuildContext context) {
    final desktop = _local.available;
    return Scaffold(
      body: SafeArea(
        child: SignInScreen(
          // Desktop CAN write offline; web cannot, so the feature line differs.
          hero: _signInHero(context, offlineIsReal: desktop),
          pane: desktop
              ? SignInPane(
                  strings: _signInPaneStrings(context),
                  origins: _servers,
                  active: _activeOrigin,
                  onSelect: _setActiveConnection,
                  onEnterLocal: () => _setActiveConnection(kLocalOrigin),
                  onAdd: promptAddServer,
                  onRemove: confirmRemoveServer,
                  probeHealth: _probeServer,
                  authForm: AuthFormCard(
                    strings: _authFormStrings(context),
                    isBusy: _isBusy,
                    errorText: _message,
                    onSubmit: _authenticate,
                    onForgotPassword: _forgotPassword,
                    // Null (not probed yet, or a server too old to say) keeps
                    // the entry — see AuthFormCard.allowRegister for why the
                    // unknown case must not hide it.
                    allowRegister: _activeRegistrationOpen ?? true,
                  ),
                )
              : AuthFormCard(
                  strings: _authFormStrings(context),
                  isBusy: _isBusy,
                  errorText: _message,
                  onSubmit: _authenticate,
                  onForgotPassword: _forgotPassword,
                  allowRegister: _activeRegistrationOpen ?? true,
                ),
        ),
      ),
    );
  }

  AuthFormStrings _authFormStrings(BuildContext context, {String? title}) =>
      AuthFormStrings(
        title: title ?? context.l10n.signInTitle,
        login: context.l10n.loginActionLogin,
        register: context.l10n.loginActionRegister,
        email: context.l10n.loginEmailLabel,
        displayName: context.l10n.accountDisplayName,
        password: context.l10n.loginPasswordLabel,
        forgotPassword: context.l10n.loginForgotPassword,
      );

  /// Offer a fresh confirmation link after a sign-in was refused for an
  /// unconfirmed address. A snackbar ACTION rather than a second dialog: the
  /// person is staring at a form that just failed, and the only useful next step
  /// is one tap away.
  void _offerResendVerification(String email) {
    final l10n = context.l10n;
    final messenger = ScaffoldMessenger.of(context);
    messenger.showSnackBar(
      SnackBar(
        content: Text(l10n.loginEmailNotVerified),
        duration: const Duration(seconds: 8),
        action: SnackBarAction(
          label: l10n.authResendVerification,
          onPressed: () async {
            // Swallow failures on purpose: the endpoint answers 204 for an
            // unknown address, an already-confirmed one and a mail outage alike,
            // so there is nothing here worth distinguishing — and claiming
            // failure would be as misleading as claiming success.
            try {
              await _api.resendVerification(email);
            } catch (_) {}
            messenger.showSnackBar(
              SnackBar(content: Text(l10n.authResendSent)),
            );
          },
        ),
      ),
    );
  }

  /// The forgot-password flow, shared by both platforms' forms. The form hands up
  /// whatever is typed; the copy for "nothing typed" belongs here.
  Future<void> _forgotPassword(String email) async {
    final l10n = context.l10n;
    final messenger = ScaffoldMessenger.of(context);
    if (email.isEmpty) {
      messenger.showSnackBar(
        SnackBar(content: Text(l10n.loginForgotEnterEmail)),
      );
      return;
    }
    try {
      await _api.requestPasswordReset(email);
    } catch (_) {
      // The endpoint answers 204 whether or not the address is registered, so
      // there is nothing to distinguish; a network failure still shouldn't claim
      // the mail went out.
    }
    if (!mounted) return;
    await showDialog<void>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(l10n.loginForgotSentTitle),
        content: Text(l10n.loginForgotSentBody),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: Text(l10n.commonOk),
          ),
        ],
      ),
    );
  }

  /// Pick a picture and make it this account's avatar.
  ///
  /// Returns the avatar URL that is true AFTERWARDS — including when the picker
  /// was dismissed, which is not an error and leaves the old picture in place.
  /// Returning the resulting state rather than an event is what lets Settings
  /// show the change: it is a route, so it never sees the rebuild this triggers
  /// upstream, and reading a widget field back right after the await would read
  /// the frame that has not happened yet. (Found the hard way — the sidebar
  /// updated and the panel you changed it in did not.)
  Future<String?> _changeAvatar() async {
    final session = _requireSession();
    final picked = await pickImage();
    if (picked == null) return _avatarUrlFor(session.user.avatarVersion);
    final version = await _api.setAvatar(
      session.accessToken,
      picked.bytes,
      picked.mime,
    );
    return _applyAvatarVersion(session, version);
  }

  Future<String?> _removeAvatar() async {
    final session = _requireSession();
    await _api.removeAvatar(session.accessToken);
    return _applyAvatarVersion(session, null);
  }

  /// Same copyWith discipline as _updateProfile: rebuilding the session here
  /// would drop the refresh token, and changing your picture would cost you the
  /// ability to renew.
  String? _applyAvatarVersion(AuthSession session, String? version) {
    final updated = session.copyWith(
      user: session.user.withAvatarVersion(version),
    );
    if (mounted) {
      _sessionCtrl.adoptSession(updated);
      _persistSession(updated);
    }
    // Derived from the version we just set, not read back out of state — the
    // answer must be right even when this widget is already gone.
    final url = _avatarUrlFor(version, userId: updated.user.id);
    // The version is a content hash, so setting → removing → setting the SAME
    // picture lands back on the exact URL that 404'd in between, and Flutter's
    // ImageCache holds that failure for the rest of the run: the app would say
    // you have a picture and draw your initial. Evicting here is the one moment
    // anything knows the bytes behind this URL just changed.
    if (url != null) NetworkImage(url).evict();
    return url;
  }

  String? _avatarUrlFor(String? version, {String? userId}) => avatarUrl(
    base: _api.baseUri,
    userId: userId ?? _session?.user.id ?? '',
    version: version,
  );

  Future<void> _changePassword(String current, String next) async {
    final session = _requireSession();
    await _api.changePassword(session.accessToken, current, next);
  }

  Future<void> _deleteAccount(String password) async {
    final session = _requireSession();
    await _api.deleteAccount(session.accessToken, password);
    // Account and everything it owned are gone server-side. Reuse _signOut to
    // tear down the local session and land back on sign-in / the local world;
    // its fire-and-forget logout harmlessly no-ops (the refresh token cascaded
    // away with the account).
    if (mounted) _signOut();
  }

  Future<Map<String, dynamic>> _loadAiSettings() async {
    final session = _requireSession();
    final settings = await _api.getAiSettings(session.accessToken);
    // Keep the Ask-AI affordance in step with what the server actually has.
    // It used to update only on SAVE, so a state reached any other way — a
    // provider switched from another window, a key cleared, a restart — left
    // the button showing or hidden according to a stale answer.
    if (mounted) setState(() => _aiConfigured = _aiReady(settings));
    return settings;
  }

  /// A usable AI needs BOTH a key and a model: with either missing the server
  /// has nothing to call, and an Ask-AI button that appears and then fails is
  /// worse than one that waits.
  bool _aiReady(Map<String, dynamic> settings) =>
      settings['has_key'] == true &&
      (settings['model'] as String? ?? '').trim().isNotEmpty;

  /// This account's past imports, straight from the server. Null-guarded by the
  /// caller in 本地模式 (there is no server to have a history).
  Future<List<ImportHistoryEntry>> _loadImportHistory() async {
    final session = _requireSession();
    return _api.importHistory(session.accessToken);
  }

  /// The provider's live model list. Kept next to [_loadAiSettings] because it
  /// takes the same session and the dialog uses them together.
  Future<Map<String, dynamic>> _listAiModels({
    String? provider,
    String? baseUrl,
    String? apiKey,
  }) async {
    final session = _requireSession();
    return _api.listAiModels(
      session.accessToken,
      provider: provider,
      baseUrl: baseUrl,
      apiKey: apiKey,
    );
  }

  /// Whether the server has a usable AI provider ([_aiReady]). Failure leaves
  /// AI hidden.
  Future<void> _refreshAiConfigured() async {
    final session = _session;
    if (session == null) return;
    try {
      final s = await _api.getAiSettings(session.accessToken);
      // Same rule as every other path — [_aiReady], not a second, looser copy
      // of it. This one said "key OR model", so an instance with a key and no
      // model showed Ask AI here and hid it everywhere else.
      if (mounted) setState(() => _aiConfigured = _aiReady(s));
    } catch (_) {}
  }

  /// Write one provider's config and make it active. Returns the server's view
  /// of THAT provider so the dialog can populate itself from the answer instead
  /// of assuming the write landed exactly as sent.
  Future<Map<String, dynamic>> _saveAiSettings({
    required String providerId,
    String? provider,
    String? baseUrl,
    String? model,
    String? apiKey,
  }) async {
    final session = _requireSession();
    final settings = await _api.updateAiSettings(
      session.accessToken,
      providerId: providerId,
      provider: provider,
      baseUrl: baseUrl,
      model: model,
      apiKey: apiKey,
    );
    // A key alone is not a working AI: without a model the server has nothing
    // to call. Gate the Ask-AI affordance on both, or the button appears and
    // then fails.
    if (mounted) setState(() => _aiConfigured = _aiReady(settings));
    return settings;
  }

  /// Drop one provider's stored config. Same [_aiReady] sync as the writes:
  /// deleting the provider in use leaves nothing to call, and Ask AI has to go
  /// with it rather than wait for the next reload to notice.
  Future<Map<String, dynamic>> _deleteAiProvider(String providerId) async {
    final session = _requireSession();
    final settings = await _api.deleteAiProvider(
      session.accessToken,
      providerId,
    );
    if (mounted) setState(() => _aiConfigured = _aiReady(settings));
    return settings;
  }

  Future<List<Map<String, dynamic>>> _loadTokens() async {
    return _api.listTokens(_requireSession().accessToken);
  }

  Future<Map<String, dynamic>> _createToken(
    String name,
    List<String> scopes,
    int? expiresInDays,
  ) async {
    return _api.createToken(
      _requireSession().accessToken,
      name,
      scopes,
      expiresInDays,
    );
  }

  Future<void> _revokeToken(String id) async {
    await _api.revokeToken(_requireSession().accessToken, id);
  }

  String _titleFromMarkdown(String markdown, String fallback) {
    for (final line in markdown.split('\n')) {
      final trimmed = line.trim();
      final heading = RegExp(r'^#{1,6}\s+(.*)$').firstMatch(trimmed);
      if (heading != null && heading.group(1)!.trim().isNotEmpty) {
        return heading.group(1)!.trim();
      }
      if (trimmed.isNotEmpty) {
        return trimmed.length > 60 ? trimmed.substring(0, 60) : trimmed;
      }
    }
    return fallback;
  }

  Future<void> _aiNewPageFromMarkdown(String markdown) {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      final title = _titleFromMarkdown(markdown, 'Untitled');
      final bootstrap = await _api.importMarkdown(
        session.accessToken,
        workspace.id,
        title,
        markdown,
      );
      setState(() {
        final views = _viewsByWorkspace[workspace.id] ?? const [];
        _sessionCtrl.replaceViews(workspace.id, [..._viewsByWorkspace[workspace.id] ?? const [], bootstrap.view]);
        _selectedView = bootstrap.view;
        _selectedBootstrap = bootstrap;
      });
    });
  }

  /// Import a Markdown file as a new page (title from its H1, else the filename).
  Future<void> _importMarkdownAsPage(String fileName, String markdown) {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      final base = fileName
          .replaceAll(RegExp(r'\.(md|markdown|txt)$', caseSensitive: false), '')
          .trim();
      final title = _titleFromMarkdown(
        markdown,
        base.isEmpty ? 'Imported' : base,
      );
      final bootstrap = await _api.importMarkdown(
        session.accessToken,
        workspace.id,
        title,
        markdown,
      );
      setState(() {
        final views = _viewsByWorkspace[workspace.id] ?? const [];
        _sessionCtrl.replaceViews(workspace.id, [..._viewsByWorkspace[workspace.id] ?? const [], bootstrap.view]);
        _selectedView = bootstrap.view;
        _selectedBootstrap = bootstrap;
      });
    });
  }

  /// Import a workspace archive server-side: one upload, the Rust engine
  /// does everything (unzip, page tree, ordering, Notion adaptation, images
  /// to S3, link rewiring) — see crates/interchange. [notion] forces Notion
  /// adaptation; otherwise the server auto-detects it.
  Future<void> _importWorkspaceZip(
    String fileName,
    Uint8List zipBytes, {
    bool notion = false,
  }) {
    final wsName = ImportController.cleanArchiveName(fileName);
    return _runServerImport(
      zipBytes,
      name: wsName.isEmpty ? 'Imported' : wsName,
      notion: notion,
    );
  }

  /// Import loose files / a picked folder into an EXISTING workspace: pack
  /// them into a STORE ZIP (no compression — it goes straight to our own
  /// backend) and let the server import it.
  Future<void> _importTreeIntoWorkspace(
    Workspace workspace,
    List<ArchiveFile> entries, {
    String? sourceName,
    String? parentViewId,
    String? container,
  }) {
    // [sourceName] (the picked folder's name) names the container the server
    // wraps the import under when [container] is 'wrap' — see
    // ImportMode::IntoContainer. Absent (loose multi-file selection) → the
    // server falls back to "Imported".
    // [container] is the wrap-vs-spill choice: 'spill' drops the top-level
    // entries straight into the destination, 'wrap' nests them under one
    // container; null → server default (auto).
    // [parentViewId] imports UNDER a folder instead of the workspace root.
    return _runServerImport(
      buildStoreZip(entries),
      name: sourceName,
      workspaceId: workspace.id,
      parentViewId: parentViewId,
      container: container,
    );
  }

  /// Upload the archive, then WATCH the import without holding the app.
  ///
  /// Two phases, because only one of them needs the user present. The upload
  /// streams the archive out of this process, so quitting during it loses the
  /// import — that part blocks, and AppFlowy words the same constraint the same
  /// way ("File is uploading. Please do not quit the app"). Once the server has
  /// the bytes it owns the job: it runs in its own task, keyed by a job id, and
  /// nothing about it needs a client.
  ///
  /// The watching used to run inside [_run] too, which held `_isBusy` for the
  /// WHOLE import — a 235-page archive locked the app for as long as the server
  /// took, for no reason except that we were the ones asking how it was going.
  /// Now it polls detached, and the progress row in the sidebar (always on
  /// screen, cancel button and all) is where the import lives.
  ///
  /// The sidebar is deliberately kept as that home instead of moving this to a
  /// settings page: Notion, Outline and Slack all park long imports in
  /// `Settings → Import`, and the reason they do is that progress has to live
  /// somewhere the user can find AGAIN — which the triggering dialog can never
  /// be. The sidebar already satisfies that and is visible the whole time. (An
  /// import HISTORY does belong in settings, but that needs jobs to outlive the
  /// server process, and they are currently in memory only.)
  Future<void> _runServerImport(
    Uint8List zipBytes, {
    String? name,
    bool notion = false,
    String? workspaceId,
    String? parentViewId,
    String? container,
  }) async {
    final l10n = context.l10n;
    String? started;
    await _run(() async {
      final session = _requireSession();
      started = await _api.startWorkspaceImport(
        session.accessToken,
        zipBytes,
        name: name,
        notion: notion,
        workspaceId: workspaceId,
        parentViewId: parentViewId,
        container: container,
        reHostImages: _reHostImages,
      );
    });
    final jobId = started;
    // Upload failed — `_run` has already put the reason on screen.
    if (jobId == null || !mounted) return;
    // Detached on purpose: this is the line that gives the app back. The poll
    // loop, its error reporting and the second image pass live in the
    // controller now; the requireSession placement preserves the old
    // behaviour where a missing session surfaced as `_message`, not a crash.
    _import.rehostEnabled = _reHostImages;
    _import.attach(jobId);
    unawaited(() async {
      try {
        final token = _requireSession().accessToken;
        await _import.watch(token, jobId, workspaceId, l10n.importJobFailed);
      } on ApiException catch (error) {
        if (mounted) setState(() => _message = error.message);
      }
    }());
  }


  Future<void> _aiCurrentFromMarkdown(String markdown) {
    return _run(() async {
      final bootstrap = _selectedBootstrap;
      if (bootstrap == null) {
        throw ApiException(context.l10n.pageOpenFirst);
      }
      final specs = markdownToBlocks(markdown);
      final root = bootstrap.document.rootBlockId;
      var index = bootstrap.childBlocks.length;
      final stamp = DateTime.now().microsecondsSinceEpoch;
      final ops = <Map<String, dynamic>>[];
      for (var i = 0; i < specs.length; i++) {
        ops.add({
          'type': 'insert_block',
          'parent_id': root,
          'index': index,
          'block': {
            'id': 'block_${stamp}_$i',
            'type': specs[i].kind,
            'text': specs[i].text,
            'data': specs[i].data,
            'children': <String>[],
          },
        });
        index++;
      }
      await _applyEditorOperations(ops);
    });
  }

  Future<void> _aiNewWorkspaceFromMarkdown(String markdown) {
    return _run(() async {
      final session = _requireSession();
      final title = _titleFromMarkdown(markdown, 'AI workspace');
      final workspace = await _api.createWorkspace(session.accessToken, title);
      final bootstrap = await _api.importMarkdown(
        session.accessToken,
        workspace.id,
        title,
        markdown,
      );
      _sessionCtrl.addCreated(workspace, [bootstrap.view]);
      setState(() {
        _selectedView = bootstrap.view;
        _selectedBootstrap = bootstrap;
      });
      await _loadSelectedWorkspaceMembers();
    });
  }

  Future<void> _selectView(DocumentView view) {
    // A folder has no document to open (the sidebar routes folder taps to
    // expand/collapse; this guards other callers, e.g. internal page links).
    if (view.objectType == 'folder') return Future.value();
    // Re-tapping the page that is already open and rendered does nothing.
    //
    // It used to re-run the whole bootstrap, and a second tap within the
    // double-click window put two of them in flight at once. The loser hit a
    // disposed session, fell into the offline branch below, got null back
    // (nothing is mirrored for a doc opened online), and blanked the page the
    // user was reading. Switching away and back reloaded it, which is why the
    // content always "came back".
    //
    // Skipping also drops a pointless outbox drain + round trip from every
    // re-tap of the current row, which the sidebar does on any stray click.
    if (!needsBootstrapOnSelect(
      openViewId: _selectedView?.id,
      openViewHasContent: _selectedBootstrap != null,
      targetViewId: view.id,
    )) {
      return Future.value();
    }
    return _run(() async {
      // The editor's pending edits were just flushed (see _navigateToView) into
      // the current doc's cloud session. Drain it — wait for those pushes to be
      // acked/folded server-side — BEFORE loading the next doc, because
      // _reconcileSync will dispose this session right after, and a still
      // in-flight push would be dropped (= lost content). Short timeout so a
      // slow/offline server can't wedge the page switch.
      await _cloudSession?.drainOutbox(timeout: const Duration(seconds: 4));
      final session = _requireSession();
      final workspace = _requireWorkspace();
      DocumentBootstrap? bootstrap;
      var reachedServer = false;
      try {
        bootstrap = await _api.bootstrapDocument(
          session.accessToken,
          workspace.id,
          view.objectId,
        );
        reachedServer = true;
      } on ApiException {
        // The server responded with an error (401/403/404/500 — e.g. the doc was
        // deleted or access was revoked). Surface it; never mask a live-server
        // error with a stale local mirror.
        rethrow;
      } catch (_) {
        // Genuine connectivity failure (SocketException / ClientException /
        // timeout): open the on-device mirror instead so a cached cloud doc still
        // renders (P1c). Null when it was never opened online → select the view
        // with an empty editor pane; the yrs session below seeds/connects when
        // the network returns.
        bootstrap = await _offlineCloudBootstrap(view);
      }
      // One line per page load: what came back, and from where. This is the
      // record that turns "the page went blank" into "the server returned 2
      // blocks while the local copy has 114" — a data problem, not a UI one.
      trace(
        'bootstrap doc=${view.objectId} view=${view.id} '
        'blocks=${(bootstrap?.snapshot.payload['blocks'] as List?)?.length ?? -1} '
        'source=${reachedServer ? 'server' : 'mirror'}',
      );
      final wasShowingThisView =
          _selectedView?.id == view.id && _selectedBootstrap != null;
      // Per-workspace last view (AppFlowy model): switching back to a workspace
      // reopens the page you left there.
      final wsId = _selectedWorkspace?.id;
      if (wsId != null) savePref('lastViewId:$wsId', view.id);
      setState(() {
        _selectedView = view;
        // Never blank a page that is already on screen. `bootstrap` is null
        // when the server was unreachable AND the doc has no on-device mirror;
        // for a doc we are already rendering, keeping what we have beats
        // replacing it with an empty pane. The guard above catches the common
        // case, but two taps can both pass it before either finishes.
        if (mayReplaceBootstrap(
          haveNewBootstrap: bootstrap != null,
          wasShowingSameView: wasShowingThisView,
        )) {
          _selectedBootstrap = bootstrap;
        }
      });
      // Reaching the server means we're back online — restore the real nav if we
      // had fallen back to the offline mirror.
      if (reachedServer) _recoverOnlineNav();
    });
  }

  /// Pick an emoji for [view] and persist it.
  ///
  /// The picker's three-way result carries all the way to the server: dismissing
  /// (null) leaves the icon untouched, `''` clears it, an emoji sets it — which is
  /// why a plain rename can never wipe someone's icon by omission.
  Future<void> _promptSetViewIcon(DocumentView view) async {
    final l10n = context.l10n;
    final picked = await showEmojiPicker(
      context,
      current: view.icon,
      strings: EmojiPickerStrings(
        title: l10n.iconPickerTitle,
        searchHint: l10n.iconPickerSearch,
        removeIcon: l10n.iconPickerRemove,
        noResultsTitle: l10n.iconPickerNoResultTitle,
        noResultsBody: l10n.iconPickerNoResultBody,
        categorySmileys: l10n.iconCategorySmileys,
        categoryPeople: l10n.iconCategoryPeople,
        categoryNature: l10n.iconCategoryNature,
        categoryFood: l10n.iconCategoryFood,
        categoryObjects: l10n.iconCategoryObjects,
        categorySymbols: l10n.iconCategorySymbols,
        categoryFlags: l10n.iconCategoryFlags,
      ),
    );
    if (picked == null || !mounted) return; // dismissed → change nothing
    await _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      final updated = await _api.updateView(
        session.accessToken,
        workspace.id,
        view.id,
        view.name,
        icon: picked,
      );
      setState(() {
        final views = _viewsByWorkspace[workspace.id] ?? const [];
        _sessionCtrl.replaceViews(
          workspace.id,
          (_viewsByWorkspace[workspace.id] ?? const [])
              .map((item) => item.id == updated.id ? updated : item)
              .toList(),
        );
      });
    });
  }

  /// Open a page the home screen listed. Home carries ids (it spans workspaces),
  /// so the view object is looked up here; a stale id — the page was deleted
  /// between render and tap — is simply ignored rather than guessed at.
  void _openViewFromHome(String viewId, {required bool local}) {
    if (local) {
      for (final view in _localViews) {
        if (view.id != viewId) continue;
        // A FOLDER is not a document — opening it as one did nothing at all,
        // which is what the home screen's directory list used to do. Show its
        // contents.
        if (view.isFolder) {
          setState(() => _overviewFolderId = view.id);
          return;
        }
        unawaited(_localSelectView(view));
        return;
      }
      return;
    }
    // Which workspace the hit came from is what this used to throw away by
    // flattening `.values` — and it is exactly what the open needs.
    final workspaceId = workspaceIdOfView(
      viewsByWorkspace: _viewsByWorkspace,
      viewId: viewId,
    );
    if (workspaceId == null) return;
    for (final view in _viewsByWorkspace[workspaceId] ?? const <DocumentView>[]) {
      if (view.id != viewId) continue;
      unawaited(_openAcrossWorkspaces(workspaceId, view));
      return;
    }
  }

  /// Open a page — or reveal a folder — that may live in a DIFFERENT workspace,
  /// switching there first when it does.
  ///
  /// Home lists recents and directories from EVERY workspace (`buildRecents`
  /// says so in as many words), but the open path bootstraps against
  /// `_selectedWorkspace`: `GET /workspaces/{workspace_id}/documents/{id}`
  /// is workspace-scoped, and so is the server's `fetch_document`. Clicking a
  /// recent that belonged to another workspace therefore asked the CURRENT
  /// workspace for a document it does not contain, and got a 404 — the row
  /// looked broken rather than the app looking wrong.
  Future<void> _openAcrossWorkspaces(
    String workspaceId,
    DocumentView view,
  ) async {
    if (_selectedWorkspace?.id != workspaceId) {
      Workspace? target;
      for (final w in _workspaces) {
        if (w.id == workspaceId) {
          target = w;
          break;
        }
      }
      // Not a member any more (or the list is stale): say nothing and do
      // nothing rather than switching to a workspace that will 403.
      if (target == null) return;
      await _selectWorkspace(target);
      // The switch surfaces its own failure through `_run`; if it did not land,
      // opening would fail the same way it used to, so stop here.
      if (!mounted || _selectedWorkspace?.id != workspaceId) return;
    }
    if (view.isFolder) {
      setState(() => _overviewFolderId = view.id);
      return;
    }
    await _selectView(view);
  }

  /// The folder whose contents the overview is showing. Non-null takes precedence
  /// over home; opening a document clears it (see [_selectView] callers).
  String? _overviewFolderId;
  WorkspaceOverviewMode _overviewMode = WorkspaceOverviewMode.cards;

  /// Close the open page so home shows again (the sidebar's Home row). Keeps the
  /// workspace selection — home is a view of the world, not a way out of it.
  void _closeOpenPage({required bool local}) {
    setState(() {
      if (local) {
        _localSelectedView = null;
      } else {
        _selectedView = null;
        _selectedBootstrap = null;
      }
    });
  }

  Future<bool> _renameView(DocumentView view, String name) async {
    final workspaceId =
        workspaceIdOfView(
          viewsByWorkspace: _viewsByWorkspace,
          viewId: view.id,
        ) ??
        _selectedWorkspace?.id;
    var saved = false;
    await _run(() async {
      final session = _requireSession();
      final targetWorkspaceId = workspaceId ?? _requireWorkspace().id;
      final renamed = await _api.updateView(
        session.accessToken,
        targetWorkspaceId,
        view.id,
        name,
      );

      setState(() {
        final views = _viewsByWorkspace[targetWorkspaceId] ?? const [];
        _sessionCtrl.replaceViews(
          targetWorkspaceId,
          (_viewsByWorkspace[targetWorkspaceId] ?? const [])
              .map((item) => item.id == renamed.id ? renamed : item)
              .toList(),
        );

        if (_selectedView?.id == renamed.id) {
          _selectedView = renamed;
        }

        final bootstrap = _selectedBootstrap;
        if (bootstrap != null && bootstrap.view.id == renamed.id) {
          _selectedBootstrap = DocumentBootstrap(
            document: bootstrap.document,
            view: renamed,
            snapshot: bootstrap.snapshot,
          );
        }
      });
      saved = true;
    });
    return saved;
  }

  Future<void> _deleteView(DocumentView view) {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      final views = await _api.deleteView(
        session.accessToken,
        workspace.id,
        view.id,
      );

      setState(() {
        _sessionCtrl.replaceViews(workspace.id, views);
        if (_selectedView?.id == view.id) {
          _selectedView = null;
          _selectedBootstrap = null;
        }
      });
    });
  }

  /// Trash a selection (cloud) in ONE request, then reload the tree.
  ///
  /// Reloads rather than reading a tree back from the response: `batch-trash`
  /// reports what it TOUCHED (ids + a count), which is the honest thing for it
  /// to report and not a page tree. One extra GET for an operation the user
  /// invoked deliberately is a fair trade for not inventing a second shape of
  /// truth about the tree.
  Future<void> _deleteViews(List<String> viewIds) {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      final result = await _api.batchTrashViews(
        token: session.accessToken,
        workspaceId: workspace.id,
        viewIds: viewIds,
      );
      await _loadSelectedWorkspaceViews();
      if (!mounted) return;
      setState(() {
        // The open page may have been anywhere inside a trashed subtree, so the
        // reloaded tree — not the requested ids — decides whether it survived.
        final open = _selectedView?.id;
        final stillThere = (_viewsByWorkspace[workspace.id] ?? const []).any(
          (v) => v.id == open,
        );
        if (open != null && !stillThere) {
          _selectedView = null;
          _selectedBootstrap = null;
        }
      });
      // `skipped` is the server saying it did LESS than asked (already trashed,
      // or gone). Silent here on purpose: both causes mean the row is already
      // where the user wanted it, and the reloaded tree shows exactly that.
      if (result.skipped.isNotEmpty) {
        debugPrint('batch-trash skipped ${result.skipped.length} ids');
      }
    });
  }

  /// Duplicate [view] in place (cloud). The server copies the subtree, shares
  /// blobs, and dedupes the name; we just reload the tree to show the copy.
  Future<void> _cloneView(DocumentView view) {
    final copyName = context.l10n.cloneCopyName(view.name);
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      final report = await _api.cloneView(
        token: session.accessToken,
        workspaceId: workspace.id,
        viewId: view.id,
        name: copyName,
        dryRun: false,
      );
      await _loadSelectedWorkspaceViews();
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text(context.l10n.cloneDone(report.newName))),
      );
    });
  }

  Future<void> _updateRootBlockText(String text) {
    final bootstrap = _selectedBootstrap;
    if (bootstrap == null) {
      return _run(() async {
        throw ApiException(context.l10n.pageSelectFirst);
      });
    }

    return _applySelectedDocumentOperations([
      {
        'type': 'update_block',
        'block_id': bootstrap.document.rootBlockId,
        'text': text,
      },
    ]);
  }

  Future<void> _addBlock(DocumentBlockKind kind, String text) {
    final bootstrap = _selectedBootstrap;
    if (bootstrap == null) {
      return _run(() async {
        throw ApiException(context.l10n.pageSelectFirst);
      });
    }

    return _applySelectedDocumentOperations([
      {
        'type': 'insert_block',
        'parent_id': bootstrap.document.rootBlockId,
        'block': {
          'id': 'block_${DateTime.now().microsecondsSinceEpoch}',
          'type': kind.apiValue,
          'text': text,
          'children': <String>[],
        },
      },
    ]);
  }

  Future<void> _updateBlock(
    DocumentBlock block,
    DocumentBlockKind kind,
    String text,
  ) {
    return _applySelectedDocumentOperations([
      {
        'type': 'update_block',
        'block_id': block.id,
        'kind': kind.apiValue,
        'text': text,
      },
    ]);
  }

  Future<void> _deleteBlock(DocumentBlock block) {
    return _applySelectedDocumentOperations([
      {'type': 'delete_block', 'block_id': block.id},
    ]);
  }

  Future<void> _moveBlock(DocumentBlock block, int targetIndex) {
    final bootstrap = _selectedBootstrap;
    if (bootstrap == null) {
      return _run(() async {
        throw ApiException(context.l10n.pageSelectFirst);
      });
    }

    return _applySelectedDocumentOperations([
      {
        'type': 'move_block',
        'block_id': block.id,
        'parent_id': bootstrap.document.rootBlockId,
        'index': targetIndex,
      },
    ]);
  }

  Future<void> _applySelectedDocumentOperations(
    List<Map<String, dynamic>> operations,
  ) {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      final bootstrap = _selectedBootstrap;
      if (bootstrap == null) {
        throw ApiException(context.l10n.pageSelectFirst);
      }

      final result = await _api.applyDocumentUpdate(
        session.accessToken,
        workspace.id,
        bootstrap.document.id,
        operations,
      );

      setState(() {
        _selectedBootstrap = DocumentBootstrap(
          document: result.document,
          view: bootstrap.view,
          snapshot: result.snapshot,
        );
      });
    });
  }

  /// Apply editor operations without toggling the global busy state, so inline
  /// typing in the block editor stays smooth. Errors surface in the banner.
  /// The ONE editor-op entry point (P3d). Local world → the on-device backend;
  /// cloud world → the CRDT session once it's ready (a mirrored doc is ready
  /// even offline — edits land in the durable append-log outbox, P2b). The
  /// REST fallback below is only reachable in the pre-ready window of a
  /// never-mirrored doc's cold bootstrap (online): offline-with-mirror never
  /// gets here (isReady), and offline-without-mirror has no editor to type in
  /// (P1c shows the empty state) — so it cannot bypass the outbox.
  Future<void> _applyEditorOperations(
    List<Map<String, dynamic>> operations,
  ) async {
    if (_activeIsLocal) {
      await _local.applyOps(operations);
      // The editor owns its in-memory nodes; no bootstrap rebuild needed.
      return;
    }
    final yrs = _cloudSession;
    if (yrs != null && yrs.isReady) {
      yrs.applyLocalOps(operations);
      return;
    }

    final session = _session;
    final workspace = _selectedWorkspace;
    final bootstrap = _selectedBootstrap;
    if (session == null || workspace == null || bootstrap == null) {
      return;
    }

    try {
      final result = await _api.applyDocumentUpdate(
        session.accessToken,
        workspace.id,
        bootstrap.document.id,
        operations,
      );
      if (!mounted ||
          _selectedBootstrap?.document.id != bootstrap.document.id) {
        return;
      }
      setState(() {
        _selectedBootstrap = DocumentBootstrap(
          document: result.document,
          view: _selectedBootstrap!.view,
          snapshot: result.snapshot,
        );
      });
    } catch (error) {
      if (mounted) {
        setState(() => _message = error.toString());
      }
      rethrow;
    }
  }

  // ── Local offline (P2-M3) ──────────────────────────────────────────────────
  // The page tree + documents are on-device. These mirror the cloud callbacks
  // above but route to the LocalOffline facade (SQLite + yrs) instead of _api.

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
    return _localSelectedWorkspace?.id ?? 'local';
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
      while (mounted) {
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
    if (_localReady) return;
    final l10n = context.l10n;
    try {
      await _local.open();
    } catch (error) {
      if (mounted) {
        setState(() {
          _message = l10n.snackLocalStoreOpenFailed('$error');
          // Unblock the shell (P3c gates on _localReady) — the local world
          // shows empty with the error banner; the cloud world still works.
          _localReady = true;
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
    if (mounted) setState(() => _localReady = true);
  }

  Future<void> _localCreateWorkspace(String name) async {
    final title = name.trim().isEmpty
        ? context.l10n.workspaceDefaultName
        : name.trim();
    // Rust mints the id and the position, by the same step-of-ten rule the
    // view tree uses.
    final id = _local.createLocalWorkspace(title);
    if (!mounted) return;
    setState(() {
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
    await _localCreateDocument(context.l10n.pageWelcomeName);
  }

  Future<void> _localSelectWorkspace(Workspace workspace) async {
    savePref('lastWorkspaceId:local', workspace.id);
    setState(() {
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
    if (mounted) {
      setState(() {
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
      if (mounted) {
        setState(() => _message = context.l10n.snackKeepOneLocalWorkspace);
      }
      return;
    }
    if (!mounted) return;
    final wasSelected = _localSelectedWorkspace?.id == workspace.id;
    setState(() {
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
    final title = name.trim().isEmpty ? context.l10n.untitledPage : name.trim();
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
    if (!mounted) return null;
    setState(() {
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
        ? context.l10n.folderNewDefault
        : name.trim();
    // A folder has no document; object_id is an unused placeholder.
    final viewId = _local.createView(
      workspaceId: _localSelectedWorkspace?.id ?? 'local',
      parentId: parentViewId,
      objectId: 'folder_${DateTime.now().microsecondsSinceEpoch}',
      name: title,
      objectType: 'folder',
    );
    if (!mounted) return null;
    setState(_reloadLocalViews);
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
    final l10n = context.l10n;
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
    if (!mounted) return;
    setState(_reloadLocalViews);
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
      context,
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
      if (!mounted) return;
      setState(() => _localSelectedView = view);
      await _showLocalCorruptDialog(view);
      return;
    }
    if (data == null || !mounted) return;
    final loaded = data;
    setState(() {
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
    final l = context.l10n;
    final action = await showDialog<String>(
      context: context,
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
    if (!mounted) return;
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
    await _flushForExit();
    DocVersion toDocVersion(({String id, String? label, int createdAt}) v) =>
        DocVersion(
          id: v.id,
          label: v.label,
          createdAt: DateTime.fromMillisecondsSinceEpoch(
            v.createdAt,
          ).toUtc().toIso8601String(),
        );
    await showDialog<void>(
      context: context,
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
          if (!mounted) return;
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

  // ── §6 本地→云迁移 ──────────────────────────────────────────────────────────
  //
  // In-place mount: a local-offline workspace is *copied* up to a new cloud
  // workspace; the local data is never modified (stays as the offline fallback).
  // Each page is faithfully recreated on the cloud by replaying its block tree
  // as ops onto the cloud doc's root (no meta.root collision — see docs §7.1),
  // and its images are uploaded with their file_ids reconciled sha256 → UUID.

  /// Entry point from the local page menu. Prompts for a cloud account, then
  /// runs the migration. Re-migration is gated by a `migrated:<localWsId>` pref.
  /// Upload a LOCAL workspace to the cloud (P3f §6.1): copy-to-new-cloud-
  /// workspace (AFFiNE-verified shape), reusing the signed-in session (prompt
  /// only when signed out). Afterwards the user chooses delete-or-keep for the
  /// local original (default delete — a kept copy is an independent fork that
  /// never syncs; the old `migrated:` re-run gate is retired in favor of that
  /// explicit choice).
  Future<void> _migrateEntry(WorkspaceEntry entry) async {
    if (kIsWeb || !entry.isLocal) return;
    final localWs = entry.workspace;
    final l10n = context.l10n;
    // Ask BEFORE starting. This creates a cloud workspace, uploads every page
    // and every image blob, then opens a sync session — minutes of work and many
    // requests, and it ends by asking whether to delete the local original. One
    // click on a menu item used to begin all of that with no confirmation at all;
    // the only feedback was the app-wide busy spinner.
    //
    // Not `showDestructiveConfirm`: nothing is destroyed here (the local store is
    // read-only throughout), so a red button would misdescribe it. What warrants
    // the stop is the size and the one-way-ness.
    final go = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(l10n.worldMigrateConfirmTitle),
        content: Text(l10n.worldMigrateConfirmBody(localWs.name)),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: Text(l10n.commonCancel),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: Text(l10n.worldMigrateConfirmAction),
          ),
        ],
      ),
    );
    if (go != true || !mounted) return;
    var session = _session;
    if (session == null) {
      final creds = await _promptCloudAuth(migrateWorkspace: localWs.name);
      if (creds == null || !mounted) return;
      await _run(() async {
        // A registration cannot continue into the migration: there is no session
        // until the address is confirmed. Tell them, and let them come back.
        if (creds.$1 == AuthMode.register) {
          await _api.register(creds.$2);
          if (!mounted) return;
          ScaffoldMessenger.of(
            context,
          ).showSnackBar(SnackBar(content: Text(context.l10n.authVerifySent)));
          return;
        }
        final s = await _api.login(creds.$2);
        _persistSession(s);
        _sessionCtrl.adoptSession(s);
      });
      session = _session;
      if (session == null || !mounted) return;
    }
    var migrated = false;
    await _run(() async {
      final clientId = await _local.deviceClientId();
      if (clientId == null) throw StateError(l10n.worldMigrateNoIdentity);
      final result = await _runWorkspaceMigration(session!, clientId, localWs);
      // Refresh the cloud list so the new workspace appears in the switcher.
      // `session!` here for the same reason as the line above: it is a mutable
      // local captured by this closure, so the CFE refuses to promote it from
      // the null check outside — even though the analyzer accepts it. Only a
      // real `flutter build` reports the difference; analyze and test do not.
      //
      // Hence the ignore: the analyzer calls this `!` unnecessary and it is
      // wrong — removing it breaks `flutter build` while analyze and test stay
      // green. Suppressed HERE, at the one site it is false, so that the
      // release gate can treat warnings as fatal everywhere else (an
      // `invalid_return_type_for_catch_error` warning sat unread long enough to
      // ship a crash on an error path — scripts/release-check.sh).
      // ignore: unnecessary_non_null_assertion
      final workspaces = await _api.listWorkspaces(session!.accessToken);
      if (!mounted) return;
      migrated = true;
      setState(() {
        _sessionCtrl.setWorkspaces(workspaces);
        _message = l10n.worldMigratedMsg(localWs.name, result.docCount);
      });
    });
    if (!migrated || !mounted) return;
    // Post-migration choice (P3 决策④): default delete the local original;
    // keeping it is an explicit escape hatch and creates an independent fork.
    final delete = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(l10n.worldMigrateDoneTitle),
        content: Text(l10n.worldMigrateDoneBody(localWs.name)),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: Text(l10n.worldKeepLocalOriginal),
          ),
          FilledButton(
            onPressed: () => Navigator.of(context).pop(true),
            child: Text(l10n.worldDeleteLocalOriginal),
          ),
        ],
      ),
    );
    if (delete == true && mounted) {
      await _localDeleteWorkspace(localWs);
    }
  }

  /// Detach a CLOUD workspace into an independent local copy (P3f §6.2). The
  /// cloud original stays (and keeps mirroring/syncing); the local fork shares
  /// nothing with it (fresh doc ids). Un-pushed offline edits are included in
  /// the copy AND still push from the mirror on reconnect — no loss either way.
  Future<void> _detachEntry(WorkspaceEntry entry) async {
    if (kIsWeb || entry.isLocal) return;
    final l10n = context.l10n;
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(l10n.worldDetachTitle),
        content: Text(l10n.worldDetachBody(entry.workspace.name)),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: Text(l10n.commonCancel),
          ),
          FilledButton(
            onPressed: () => Navigator.of(context).pop(true),
            child: Text(l10n.worldDetachConfirm),
          ),
        ],
      ),
    );
    if (confirmed != true || !mounted) return;
    final result = _local.detachCloudWorkspace(
      entry.origin,
      entry.workspace.id,
      entry.workspace.name,
    );
    if (result == null) {
      setState(() => _message = l10n.worldDetachStoreUnavailable);
      return;
    }
    setState(() {
      _reloadLocalWorkspaces();
      _message = l10n.worldDetachedMsg(entry.workspace.name, result.docs);
    });
    // The "move" variant, symmetric to 上云's delete-local (#4): once the copy
    // exists locally, an owner may delete the cloud original. Gated to the
    // ACTIVE cloud connection (entry.origin == _api's origin) so the cloud
    // delete can't target the wrong server; anyone else gets copy-only, as
    // before. Delete BEFORE landing in the local copy, while the cloud API/
    // session context is still the active one.
    final canDeleteCloud =
        matchesManageRole(entry.role) &&
        entry.origin == _api.baseUri.toString();
    if (canDeleteCloud && mounted) {
      final delete = await showDialog<bool>(
        context: context,
        builder: (context) => AlertDialog(
          title: Text(l10n.worldDetachDoneTitle),
          content: Text(l10n.worldDetachDoneBody(entry.workspace.name)),
          actions: [
            TextButton(
              onPressed: () => Navigator.of(context).pop(false),
              child: Text(l10n.worldKeepCloudOriginal),
            ),
            FilledButton(
              onPressed: () => Navigator.of(context).pop(true),
              child: Text(l10n.worldDeleteCloudOriginal),
            ),
          ],
        ),
      );
      if (delete == true && mounted) {
        await _deleteWorkspace(entry.workspace);
      }
    }
    if (!mounted) return;
    // Land in the fresh local copy.
    final target = _localWorkspaces
        .where((w) => w.id == result.workspaceId)
        .firstOrNull;
    if (target != null) {
      await _selectEntry(
        WorkspaceEntry(origin: 'local', workspace: target, role: 'owner'),
      );
    }
  }

  /// The migration engine (no UI). Creates a cloud workspace, then per local
  /// page: uploads its blobs (sha256→UUID), creates the cloud doc, and replays
  /// the block tree onto it via a headless [CloudSyncSession]. Local store is
  /// read-only here; the previously-active local doc is restored at the end.
  Future<({String cloudWorkspaceId, int docCount})> _runWorkspaceMigration(
    AuthSession session,
    BigInt clientId,
    Workspace localWs,
  ) async {
    final token = session.accessToken;
    final cloudWs = await _api.createWorkspace(token, localWs.name);

    final views = _local
        .listViews()
        .where((v) => v.workspaceId == localWs.id && !v.trashed)
        .toList();
    final ordered = _orderViewsParentFirst(views);

    final localToCloudView = <String, String>{};
    var docCount = 0;
    try {
      for (final v in ordered) {
        final cloudParent = v.parentId == null
            ? null
            : localToCloudView[v.parentId];

        // A FOLDER migrates as a folder. This used to call createDocument for
        // every view, so every local folder arrived in the cloud as a PAGE —
        // and the moment its first child followed, the server refused the whole
        // migration: "parent_view_id must be a folder — pages cannot contain
        // pages". Nothing about the local tree was wrong, which is why emptying
        // the workspace and re-importing changed nothing: the shape was being
        // created on the way up.
        //
        // The `openDoc == null` skip that sat here did not catch it: a local
        // folder owns a placeholder empty document (see the vault import), so
        // it opened fine and sailed straight into createDocument.
        if (migratesAsFolder(v.objectType)) {
          final folder = await _api.createFolder(
            token,
            cloudWs.id,
            v.name,
            parentViewId: cloudParent,
          );
          localToCloudView[v.id] = folder.id;
          continue;
        }

        final doc = _local.openDoc(v.objectId);
        if (doc == null) continue;
        final created = await _api.createDocument(
          token,
          cloudWs.id,
          v.name,
          parentViewId: cloudParent,
        );
        localToCloudView[v.id] = created.view.id;

        // Upload referenced blobs once each, building the sha256→UUID map.
        final idMap = <String, String>{};
        for (final sha in imageBlobIds(doc.blocks)) {
          final bytes = _local.loadBlob(sha);
          if (bytes == null) continue; // dangling/pruned → leave ref as-is
          final up = await _api.uploadImage(
            token,
            cloudWs.id,
            fileName: 'image',
            mimeType: _sniffImageMime(bytes),
            bytes: bytes,
          );
          idMap[sha] = up.id;
          _local.putBlobAs(
            up.id,
            bytes,
          ); // mirror so the cloud copy renders offline
        }

        // Faithfully replay the local tree onto the cloud doc's root.
        final yrs = CloudSyncSession(
          // Short-lived (one migration), so the token cannot lapse under it.
          uri: () async => documentSocketUri(
            _api.baseUri,
            cloudWs.id,
            created.document.id,
            token,
          ),
          clientId: clientId,
          onReady: (_, _) {},
          onRemoteBlocks: (_) {},
        );
        try {
          yrs.connect();
          await yrs.ready.timeout(const Duration(seconds: 20));
          yrs.applyLocalOps(
            buildMigrationOps(
              blocks: doc.blocks,
              localRootId: doc.rootBlockId,
              cloudRootId: yrs.rootBlockId,
              idMap: idMap,
            ),
          );
          await yrs.drainOutbox();
        } finally {
          yrs.dispose();
        }
        docCount++;
      }
    } finally {
      // Restore the local editor's active doc (migration's openDoc() calls moved
      // it). The local store was never mutated — this just re-points the backend.
      final active = _localSelectedView;
      if (active != null) _local.openDoc(active.objectId);
    }
    return (cloudWorkspaceId: cloudWs.id, docCount: docCount);
  }

  /// Order views so every parent precedes its children (roots first), stable.
  List<ViewData> _orderViewsParentFirst(List<ViewData> views) {
    final byParent = <String?, List<ViewData>>{};
    for (final v in views) {
      (byParent[v.parentId] ??= []).add(v);
    }
    final ids = {for (final v in views) v.id};
    final out = <ViewData>[];
    void emit(String? parent) {
      for (final v in byParent[parent] ?? const <ViewData>[]) {
        out.add(v);
        emit(v.id);
      }
    }

    emit(null);
    // Defensive: surface any view whose parent isn't in this set (orphan) so it
    // still migrates rather than being silently dropped.
    for (final v in views) {
      if (v.parentId != null && !ids.contains(v.parentId)) {
        out.add(v);
        emit(v.id);
      }
    }
    return out;
  }

  /// Sniff an image MIME from magic bytes (local image blocks store only a name,
  /// not a MIME). A wrong-but-decodable type is cosmetic; default to PNG.
  String _sniffImageMime(Uint8List b) {
    if (b.length >= 2 && b[0] == 0x89 && b[1] == 0x50) return 'image/png';
    if (b.length >= 2 && b[0] == 0xFF && b[1] == 0xD8) return 'image/jpeg';
    if (b.length >= 3 && b[0] == 0x47 && b[1] == 0x49 && b[2] == 0x46) {
      return 'image/gif';
    }
    if (b.length >= 12 &&
        b[8] == 0x57 &&
        b[9] == 0x45 &&
        b[10] == 0x42 &&
        b[11] == 0x50) {
      return 'image/webp';
    }
    return 'image/png';
  }

  /// Minimal modal collecting cloud credentials for migration. Returns the auth
  /// mode + form, or null if cancelled.
  /// The cloud sign-in dialog. Two intents share one form:
  ///  - plain sign-in ([migrateWorkspace] == null): just log in / register.
  ///  - sign-in-and-migrate ([migrateWorkspace] set to a local workspace name):
  ///    the caller copies that local workspace to the cloud afterwards.
  /// Only the copy differs — the returned creds and the behavior are identical.
  Future<(AuthMode, AuthFormValue)?> _promptCloudAuth({
    String? migrateWorkspace,
    bool startOnCloud = false,
  }) {
    final migrate = migrateWorkspace != null;
    final l10n = context.l10n;
    // A full-window route, not an AlertDialog. Desktop used to collect these
    // credentials in a small modal while web showed design 01's split screen —
    // the same product looked like two. Now both build SignInScreen with the
    // same hero and the same form; only the copy and the escape hatch differ.
    return Navigator.of(context).push<(AuthMode, AuthFormValue)>(
      MaterialPageRoute(
        fullscreenDialog: true,
        // StatefulBuilder because this is a ROUTE: adding or removing a server
        // calls setState on the shell, which does NOT rebuild a route already on
        // the stack (see dialog_controllers.dart / docs/lessons.md). Without it
        // you would add a server here and the list would not move.
        builder: (routeContext) => StatefulBuilder(
          builder: (routeContext, setLocal) => Scaffold(
            body: SafeArea(
              child: SignInScreen(
                // Desktop CAN write offline, so the feature line drops web's
                // 「桌面端」 qualifier.
                hero: _signInHero(context, offlineIsReal: true),
                // Closeable, unlike the gate: you got here from a world that
                // works, so there is something to go back to — and now it says
                // so, because "there is a whole app behind this" was the part
                // nobody could see.
                onClose: () => Navigator.of(routeContext).pop(),
                closeLabel: l10n.worldBackToApp,
                pane: SignInPane(
                  strings: _signInPaneStrings(context),
                  origins: _servers,
                  active: _activeOrigin,
                  signedInOrigins: _signedInOrigins(),
                  initialCloud: startOnCloud ? true : null,
                  probeHealth: _probeServer,
                  onSelect: (origin) async {
                    await _setActiveConnection(origin);
                    setLocal(() {});
                  },
                  // Already in 本地模式 or going back to it: nothing left to
                  // sign in to, so leave.
                  onEnterLocal: () async {
                    await _setActiveConnection(kLocalOrigin);
                    if (routeContext.mounted) {
                      Navigator.of(routeContext).pop();
                    }
                  },
                  onAdd: () async {
                    await promptAddServer();
                    setLocal(() {});
                  },
                  onRemove: (origin) async {
                    await confirmRemoveServer(origin);
                    if (!routeContext.mounted) return;
                    // Removing the world you pointed at drops the app to
                    // 本地模式 (see _removeServer) — then this screen has
                    // nothing to sign in to.
                    if (_activeOrigin == kLocalOrigin) {
                      Navigator.of(routeContext).pop();
                    } else {
                      setLocal(() {});
                    }
                  },
                  // Do we hold credentials for the server this screen points
                  // at? Then it has nothing to ask for.
                  //
                  // Keyed on STORED credentials, not on `_session`. Those differ
                  // in the case this was first shipped broken for: 本地模式
                  // drops the in-memory session but keeps the credentials, so
                  // `_session != null` answered "signed in to nothing" and the
                  // password form came back (reported 2026-08-12, twice).
                  //
                  // Two shapes, one card. Already IN this world → the button
                  // goes BACK to the app hiding behind this screen. Somewhere
                  // else (本地模式, another server) → it ENTERS this one, which
                  // `_setActiveConnection` does off the stored credentials with
                  // no password.
                  //
                  // Not during a migration: that flow genuinely needs a fresh
                  // sign-in, and its own copy explains why.
                  authForm:
                      worldCardMode(
                            migrating: migrate,
                            storedName: _storedDisplayName(_cloudOrigin),
                            cloudOrigin: _cloudOrigin,
                            activeOrigin: _activeOrigin,
                          ) !=
                          WorldCardMode.signIn
                      ? SignedInCard(
                          strings: SignedInCardStrings(
                            title: l10n.worldAlreadySignedInTitle,
                            body:
                                '${_storedDisplayName(_cloudOrigin)} · '
                                '${serverLabel(_cloudOrigin)}',
                            action: _activeOrigin == _cloudOrigin
                                ? l10n.worldBackToApp
                                : l10n.worldEnterAction,
                            switchHint: _activeOrigin == _cloudOrigin
                                ? l10n.worldAlreadySignedInHint
                                : l10n.worldAlreadySignedInEnterHint,
                          ),
                          onContinue: () async {
                            if (_activeOrigin != _cloudOrigin) {
                              await _setActiveConnection(_cloudOrigin);
                            }
                            if (routeContext.mounted) {
                              Navigator.of(routeContext).pop();
                            }
                          },
                        )
                      : AuthFormCard(
                          allowRegister: _activeRegistrationOpen ?? true,
                          strings: _authFormStrings(
                            context,
                            title: migrate
                                ? l10n.worldMigrateSignInTitle
                                : null,
                          ),
                          note: migrate
                              ? l10n.worldMigrateSignInDesc(migrateWorkspace)
                              : null,
                          actionLabelOverride: migrate
                              ? l10n.worldMigrateAction
                              : null,
                          isBusy: _isBusy,
                          onSubmit: (mode, form) async =>
                              Navigator.of(routeContext).pop((mode, form)),
                          onForgotPassword: _forgotPassword,
                        ),
                ),
              ),
            ),
          ),
        ),
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

  Future<void> _localUpdateRootBlockText(String text) async {
    final root = _localBootstrap?.document.rootBlockId;
    if (root == null) return;
    await _local.applyOps([
      {'type': 'update_block', 'block_id': root, 'text': text},
    ]);
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
    if (!mounted) return true;
    setState(() {
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
    if (!mounted) return;
    setState(() {
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
    if (!mounted) return;
    setState(() {
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
    final copyName = context.l10n.cloneCopyName(view.name);
    final result = _local.cloneView(viewId: view.id, rootName: copyName);
    if (result == null || !mounted) return;
    setState(() => _reloadLocalViews());
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(content: Text(context.l10n.cloneDone(result.newName))),
    );
  }

  Future<void> _localReorderViews(
    String? parentViewId,
    List<DocumentView> ordered,
  ) async {
    // Renumbering and reparenting are one move, and the step-of-ten scheme
    // lives with the create/clone paths rather than being spelled out again.
    _local.reorderViews(parentViewId, [for (final v in ordered) v.id]);
    if (mounted) setState(_reloadLocalViews);
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
    if (mounted) setState(_reloadLocalViews);
  }

  Future<void> _localPurgeView(DocumentView view) async {
    // Permanently remove the page and its subtree from the recycle bin.
    final ids = _local.purgeViewSubtree(view.id).toSet();
    if (!mounted) return;
    setState(() {
      _reloadLocalViews();
      if (_localSelectedView != null && ids.contains(_localSelectedView!.id)) {
        _localSelectedView = null;
        _localBootstrap = null;
      }
    });
  }

  /// Upload image bytes for the editor, returning the new file id + name.
  ///
  /// Online: uploads and returns the cloud file id (UUID), mirroring the bytes
  /// into the on-device CAS so later/offline loads skip the network (§7 read
  /// side). Offline (desktop only): the upload's network call fails, so we land
  /// the bytes in the CAS under their sha256 and return *that* as a placeholder
  /// file_id — the block renders immediately from the CAS — and queue the upload
  /// so `_reconcilePendingUploads` rewrites sha256→UUID once back online (§7
  /// upstream differ). A server-side rejection (auth/size, surfaced as
  /// [ApiException]) is a real error: surfaced, not queued.
  Future<({String fileId, String name})?> _uploadEditorImage(
    Uint8List bytes,
    String fileName,
    String mimeType,
  ) async {
    final session = _session;
    final workspace = _selectedWorkspace;
    if (session == null || workspace == null) return null;
    try {
      final file = await _api.uploadImage(
        session.accessToken,
        workspace.id,
        fileName: fileName,
        mimeType: mimeType,
        bytes: bytes,
      );
      if (!kIsWeb) _local.putBlobAs(file.id, bytes); // mirror for offline reads
      return (fileId: file.id, name: file.name);
    } on ApiException catch (error) {
      // Server reachable but rejected the upload — a genuine failure. Through
      // `_apiMessage`, not `toString`: this is the one path where the size
      // refusals surface, and relaying the raw English defeated the codes they
      // are sent with.
      if (mounted) setState(() => _message = _apiMessage(error));
      return null;
    } catch (error) {
      // Network failure (offline): desktop keeps a sha256 CAS placeholder and
      // queues the upload for reconnect; web has no CAS, so it still fails.
      final docId = _selectedBootstrap?.document.id;
      if (!kIsWeb && docId != null) {
        final sha = _local.putBlob(bytes);
        _enqueuePendingUpload(
          sha: sha,
          workspaceId: workspace.id,
          docId: docId,
          name: fileName,
        );
        return (fileId: sha, name: fileName);
      }
      if (mounted) setState(() => _message = error.toString());
      return null;
    }
  }

  /// Queue an offline image upload (sha256 placeholder) and persist it.
  void _enqueuePendingUpload({
    required String sha,
    required String workspaceId,
    required String docId,
    required String name,
  }) {
    if (_pending.add((
      sha: sha,
      workspaceId: workspaceId,
      docId: docId,
      name: name,
    ))) {
      savePref('pendingBlobUploads', _pending.toJson());
    }
  }

  /// Reconcile any offline-inserted images for [documentId] (must be the active
  /// cloud doc, with a ready session). For each queued sha256: load its CAS
  /// bytes, upload to the cloud, mirror the bytes under the returned UUID, and
  /// rewrite every image block still referencing the sha256 to the UUID via the
  /// live CRDT session — then clear the queue entry. Lazy by design (§7.1): only
  /// the active doc reconciles, so images inserted offline in another doc wait
  /// until that doc is opened online. Best-effort and re-entrancy-guarded; a
  /// still-offline upload leaves the entry queued for the next attempt.
  Future<void> _reconcilePendingUploads(String documentId) async {
    if (kIsWeb || _reconciling) return;
    final session = _session;
    final workspace = _selectedWorkspace;
    final cloud = _cloudSession;
    if (session == null ||
        workspace == null ||
        cloud == null ||
        !cloud.isReady) {
      return;
    }
    final entries = _pending.forDoc(workspace.id, documentId);
    if (entries.isEmpty) return;
    _reconciling = true;
    try {
      var changed = false;
      for (final entry in entries) {
        final bytes = _local.loadBlob(entry.sha);
        if (bytes == null) {
          // Bytes evicted from the CAS — unrecoverable; drop so we don't retry
          // forever.
          if (_pending.remove(workspace.id, documentId, entry.sha))
            changed = true;
          continue;
        }
        String uuid;
        try {
          final file = await _api.uploadImage(
            session.accessToken,
            workspace.id,
            fileName: entry.name,
            mimeType: 'image/png',
            bytes: bytes,
          );
          uuid = file.id;
        } catch (_) {
          // Still offline (or a transient failure): leave queued, stop the pass.
          break;
        }
        // Mirror the bytes under the cloud id so the rewritten block still reads
        // from the local CAS, then rewrite sha256→UUID on the live replica.
        _local.putBlobAs(uuid, bytes);
        final ops = buildImageIdRewriteOps(
          blocks: cloud.allBlocks(),
          fromId: entry.sha,
          toId: uuid,
        );
        if (ops.isNotEmpty) cloud.applyLocalOps(ops);
        if (_pending.remove(workspace.id, documentId, entry.sha))
          changed = true;
      }
      if (changed) {
        savePref('pendingBlobUploads', _pending.toJson());
        // Refresh the editor from the replica so the rewritten file_ids show.
        if (mounted) _applyCloudBlocks(documentId);
      }
    } finally {
      _reconciling = false;
    }
  }

  /// Re-host a pasted image URL server-side, returning the new file id + name.
  ///
  /// Our OWN images short-circuit. Copy can only put a url on the clipboard
  /// (neither HTML nor Markdown has anywhere to keep a `file_id`), so cutting
  /// an image and pasting it back looked exactly like an external link — and
  /// the re-host ladder had the server fetch its own blob back over the public
  /// internet, hash it, and PUT it again, only to land on the same
  /// content-addressed key and get the same `file_id` back from `insert_file`'s
  /// `ON CONFLICT`. Idempotent, but a whole round trip (and, when a CN-hosted
  /// server times out, a second one up the user's own uplink) spent learning
  /// what the url already said.
  Future<({String fileId, String name})?> _importEditorImageUrl(
    String url,
  ) async {
    final session = _session;
    final workspace = _selectedWorkspace;
    if (session == null || workspace == null) return null;
    // THIS workspace only: object keys are `workspaces/<id>/<sha256>` and both
    // the quota and membership are per workspace, so a link from another one
    // must be re-hosted for real rather than aliased across the boundary.
    final own = micaBlobLink(url, apiOrigin(_api.baseUri));
    if (own != null && own.workspaceId == workspace.id) {
      return (fileId: own.fileId, name: own.name);
    }
    try {
      final file = await _api.importImageUrl(
        session.accessToken,
        workspace.id,
        url,
      );
      return (fileId: file.id, name: file.name);
    } catch (_) {
      // Server-side re-host is only the FIRST of two attempts: [_rehostOne] then
      // fetches the bytes from THIS client and uploads them, and this client
      // routinely reaches hosts a CN-hosted server cannot (AppFlowy/medium/imgur
      // 403 the datacenter IP but return 200 here). A failure at this stage is
      // therefore not user-facing — raising a banner here flashed a scary
      // "image couldn't be saved" even when the client fallback then succeeded,
      // and on every silent on-open pass. Stay quiet: a genuine both-attempts
      // failure is reported by the explicit "re-host" menu action's own toast.
      return null;
    }
  }

  /// Map image file ids to their permanent Mica blob links (stable, never
  /// expiring — the endpoint re-signs storage on each request). Used by copy so
  /// pasted images keep displaying anywhere.
  Future<Map<String, String>> _resolveEditorImageUrls(List<String> ids) async {
    final workspace = _selectedWorkspace;
    if (workspace == null || ids.isEmpty) return {};
    final origin = apiOrigin(_api.baseUri);
    return {
      for (final id in ids)
        id: '$origin/api/workspaces/${workspace.id}/files/$id/blob',
    };
  }

  /// Fetch an image's bytes for the canvas. The key is either an external URL
  /// (markdown image) — fetched directly — or a file id, resolved to a fresh
  /// signed URL first.
  Future<Uint8List?> _loadEditorImageBytes(String key) async {
    // External markdown URLs: fetch straight through (not content-addressable).
    if (key.startsWith('http://') || key.startsWith('https://')) {
      try {
        final resp = await sharedHttpClient.get(Uri.parse(key));
        return resp.statusCode == 200 ? resp.bodyBytes : null;
      } catch (_) {
        return null;
      }
    }
    // Cloud file id (§7 "在线查云、离线查本地"): serve from the on-device CAS
    // mirror first — works offline and skips the network round-trip — then fall
    // back to a cloud resolve+download, caching the bytes under the file id so
    // every later load (and any offline session) hits the local copy.
    if (!kIsWeb) {
      final cached = _local.loadBlob(key);
      if (cached != null) return cached;
    }
    final session = _session;
    final workspace = _selectedWorkspace;
    if (session == null || workspace == null) return null;
    try {
      final urls = await _api.resolveFiles(session.accessToken, workspace.id, [
        key,
      ]);
      final url = urls[key];
      if (url == null) return null;
      final resp = await sharedHttpClient.get(Uri.parse(url));
      if (resp.statusCode != 200) return null;
      if (!kIsWeb) _local.putBlobAs(key, resp.bodyBytes);
      return resp.bodyBytes;
    } catch (_) {
      return null;
    }
  }

  /// Open version history for the currently-selected cloud page. Each dialog
  /// callback reads the session token FRESH (so a silent refresh mid-dialog is
  /// picked up) and is scoped to this workspace + document.
  Future<void> _openVersionHistory() async {
    final bootstrap = _selectedBootstrap;
    final workspace = _selectedWorkspace;
    if (bootstrap == null || workspace == null || _session == null) {
      return;
    }
    final wsId = workspace.id;
    final docId = bootstrap.document.id;
    await showDialog<void>(
      context: context,
      builder: (context) => _VersionHistoryDialog(
        onList: () =>
            _api.listVersions(_requireSession().accessToken, wsId, docId),
        onCreate: (name) => _api.createVersion(
          _requireSession().accessToken,
          wsId,
          docId,
          name,
        ),
        onRestore: (versionId) => _api.restoreVersion(
          _requireSession().accessToken,
          wsId,
          docId,
          versionId,
        ),
        onLoadContent: (versionId) => _api.getVersionContent(
          _requireSession().accessToken,
          wsId,
          docId,
          versionId,
        ),
        onLoadImageBytes: _loadEditorImageBytes,
        onResolveImageUrls: _resolveEditorImageUrls,
        authorNames: _versionAuthorNames(wsId),
      ),
    );
  }

  /// User id → display name for the version timeline, or empty when naming
  /// authors would add nothing: a one-person workspace would just repeat your own
  /// name down every row.
  Map<String, String> _versionAuthorNames(String workspaceId) {
    final members = _membersByWorkspace[workspaceId] ?? const [];
    if (members.length < 2) return const {};
    return {
      for (final m in members)
        if (m.displayName.trim().isNotEmpty) m.userId: m.displayName.trim(),
    };
  }

  /// Open the public-share dialog for the selected cloud page. The share URL is
  /// composed from the server origin this client talks to (`/s/{token}`).
  Future<void> _openShare() async {
    final bootstrap = _selectedBootstrap;
    final workspace = _selectedWorkspace;
    if (bootstrap == null || workspace == null || _session == null) {
      return;
    }
    final wsId = workspace.id;
    final docId = bootstrap.document.id;
    final origin = _api.baseUri.origin;
    await showDialog<void>(
      context: context,
      builder: (context) => _ShareDialog(
        onLoad: () => _api.getShare(_requireSession().accessToken, wsId, docId),
        onEnable: () =>
            _api.createShare(_requireSession().accessToken, wsId, docId),
        onDisable: () =>
            _api.deleteShare(_requireSession().accessToken, wsId, docId),
        buildUrl: (token) => '$origin/s/$token',
      ),
    );
  }

  /// Open the move/copy-to-workspace dialog for a sidebar row. The SOURCE is
  /// the workspace whose tree is currently shown; the destination is any OTHER
  /// cloud workspace. [copy] false = move (source soft-deleted after copy),
  /// true = copy (source kept).
  ///
  /// The dialog runs its own dry-run preview and the final transfer; it pops
  /// with the report + chosen destination name so we can refresh + confirm.
  Future<void> _openTransfer(List<DocumentView> views, bool copy) async {
    final source = _selectedWorkspace;
    if (source == null || _session == null || views.isEmpty) return;
    // Destinations: every cloud workspace except the source (you can't transfer
    // a page into the workspace it already lives in).
    final destinations = _workspaces
        .where((w) => w.id != source.id)
        .map((w) => (id: w.id, name: w.name))
        .toList();
    final result = await showDialog<({TransferReport report, String destName})>(
      context: context,
      builder: (context) => _TransferDialog(
        copy: copy,
        destinations: destinations,
        itemCount: views.length,
        // Folders of whichever destination the dialog is showing, flattened
        // depth-first so the dropdown reads like the sidebar does.
        onLoadFolders: (destWorkspaceId) async => flattenedFolderOptions([
          for (final v in await _api.listViews(
            _requireSession().accessToken,
            destWorkspaceId,
          ))
            (
              id: v.id,
              parentId: v.parentViewId,
              name: v.name,
              position: v.position,
              isFolder: v.objectType == 'folder',
            ),
        ]),
        // The dialog supplies (destWorkspaceId, parentViewId, dryRun); we bind
        // source + the selected roots + move/copy here.
        //
        // Always the BATCH endpoint, even for one row: it is the same server
        // core, and keeping a second client path for the single case is how the
        // two drift until only one of them has the fix. One row is a batch of
        // one, and the server's own single route stays for the MCP server.
        onTransfer:
            ({
              required destWorkspaceId,
              required parentViewId,
              required dryRun,
            }) => _api.batchTransferViews(
              token: _requireSession().accessToken,
              workspaceId: source.id,
              viewIds: [for (final v in views) v.id],
              destWorkspaceId: destWorkspaceId,
              parentViewId: parentViewId,
              removeSource: !copy,
              dryRun: dryRun,
            ),
      ),
    );
    if (result == null || !mounted) return; // cancelled
    final report = result.report;
    // A MOVE soft-deletes the source subtree, so the current (source) tree must
    // reload to drop it; a COPY leaves the source tree unchanged but reloading
    // is harmless and keeps one path. The DESTINATION tree isn't refreshed here
    // — we're not viewing it, and switching workspaces would reload it anyway
    // (acceptable for v1; see the dialog's parentViewId note).
    if (!copy) {
      await _run(() async {
        await _loadSelectedWorkspaceViews();
      });
    }
    if (!mounted) return;
    final l10n = context.l10n;
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(
        content: Text(
          copy
              ? l10n.transferCopiedDone(report.documents, result.destName)
              : l10n.transferMovedDone(report.documents, result.destName),
        ),
      ),
    );
  }

  /// Storage for the settings dialog. Deliberately NOT wrapped in `_run`:
  /// that reports failures as a banner, and a storage row that cannot load is
  /// not worth interrupting someone who opened the dialog to rename their
  /// workspace. Null simply omits the row.
  Future<({int used, int quota, int maxUpload})?> _loadWorkspaceUsage(
    Workspace workspace,
  ) async {
    final session = _session;
    if (session == null) return null;
    try {
      final usage = await _api.workspaceUsage(
        session.accessToken,
        workspace.id,
      );
      // Remember the per-file cap for the upload error copy. This call is the
      // only place the client is told it — a presign carries it too, but the
      // presign for an oversized file is precisely the one that fails. Cached
      // rather than fetched on failure so the error path stays synchronous.
      if (usage.maxUpload > 0) _maxUploadBytes = usage.maxUpload;
      return usage;
    } catch (_) {
      return null;
    }
  }

  Future<void> _addWorkspaceMember(String email, WorkspaceRole role) {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      await _api.addWorkspaceMember(
        session.accessToken,
        workspace.id,
        email,
        role.apiValue,
      );
      await _loadSelectedWorkspaceMembers();
    });
  }

  Future<void> _updateWorkspaceMember(
    WorkspaceMember member,
    WorkspaceRole role,
  ) {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      await _api.updateWorkspaceMember(
        session.accessToken,
        workspace.id,
        member.userId,
        role.apiValue,
      );
      await _loadSelectedWorkspaceMembers();
    });
  }

  Future<void> _removeWorkspaceMember(WorkspaceMember member) {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      final members = await _api.removeWorkspaceMember(
        session.accessToken,
        workspace.id,
        member.userId,
      );
      setState(() {
        _membersByWorkspace = {..._membersByWorkspace, workspace.id: members};
      });
    });
  }

  AuthSession _requireSession() {
    final session = _session;
    if (session == null) {
      throw ApiException(context.l10n.accountSignInFirst);
    }
    return session;
  }

  Workspace _requireWorkspace() {
    final workspace = _selectedWorkspace;
    if (workspace == null) {
      throw ApiException(context.l10n.workspaceSelectFirst);
    }
    return workspace;
  }

  Future<void> _loadSelectedWorkspaceMembers() async {
    final session = _session;
    final workspace = _selectedWorkspace;
    if (session == null || workspace == null) {
      return;
    }

    final members = await _api.listWorkspaceMembers(
      session.accessToken,
      workspace.id,
    );
    setState(() {
      _membersByWorkspace = {..._membersByWorkspace, workspace.id: members};
    });
  }

  /// Explicit sign-out: forget the stored credentials, then disconnect.
  void _signOut() {
    // Tell the server first — clearing the local copy would otherwise leave the
    // refresh token live for its full 30 days, so "sign out" would mean nothing
    // to anyone holding a copy of it (a shared machine, a stolen backup).
    // Fire-and-forget: signing out locally must not hinge on the network.
    final refreshToken = _session?.refreshToken ?? '';
    if (refreshToken.isNotEmpty) {
      unawaited(_api.logout(refreshToken).catchError((_) {}));
    }
    _clearPersistedSession();
    _disconnectCloudSession();
    // Land on the sign-in screen, not silently in 本地模式.
    //
    // Teardown puts the app in the local world so it stays usable without an
    // account — that part is right and unchanged. What was wrong is stopping
    // there: signing out then dropped you into a DIFFERENT world you never
    // asked for, and getting back to a login form meant finding
    // 账户菜单 → 切换世界, which nobody does (2026-08-12).
    //
    // This screen dominates the alternative because it offers BOTH ways on:
    // sign in again, or its own 本地模式 tab — one click each, both visible.
    // Landing in 本地模式 offers one, buried in a menu.
    //
    // Web is excluded: it has no local world, and its empty cloud state already
    // renders the sign-in panel in place. Pushing a route there would stack a
    // second copy of the same screen on top of it.
    if (!kIsWeb) unawaited(_promptSignIn(startOnCloud: true));
  }

  /// Tear down the live cloud session/state WITHOUT touching stored
  /// credentials — used by sign-out (after wiping creds) and by switching
  /// cloud servers (which deliberately keeps the old origin's creds, P3c-2).
  void _disconnectCloudSession() {
    _closeAllDocumentSync();
    // The tree-change socket is a cloud thing too; without this it would keep
    // reconnecting with a token that logout just revoked.
    _viewsEvents?.dispose();
    _viewsEvents = null;
    _viewsEventsWorkspaceId = null;
    _sessionCtrl.signOut();
    setState(() {
      _membersByWorkspace = const {};
      _selectedView = null;
      _selectedBootstrap = null;
      _message = null;
      // P3c: signing out collapses the cloud section, it does NOT clear the
      // world — on desktop, land in the local world (its workspaces and the
      // on-device mirrors are untouched). Web has no local world and shows the
      // sign-in panel again via the empty cloud state.
      if (_local.available && !_activeIsLocal) {
        _activeOrigin = 'local';
      }
    });
    if (_local.available) savePref('activeOrigin', _activeOrigin);
  }

  /// Every workspace's page tree as the on-device mirror last saw it.
  ///
  /// Read once at startup so a switch has something to paint immediately. The
  /// mirror is not authoritative and does not pretend to be: the server's tree
  /// replaces it per workspace on arrival, and a failed request still surfaces
  /// (P1c) rather than being masked by this.
  ///
  /// Empty on web without a mirror, and empty before the first successful sync
  /// — both are the honest "we have nothing yet" that the cold path handles.
  Map<String, List<DocumentView>> _warmTreesFromMirror(AuthSession session) {
    final cache = _local.cachedCloudPageTree(_api.baseUri.toString());
    if (cache == null) return const {};
    return rebuildCloudNavFromCache(cache, session.user.id).views;
  }

  /// Show [view]'s body from the on-device replica and hand it to its CRDT
  /// session — the "先渲染后校正" half of a switch.
  ///
  /// Distinct from the offline fallback next door, which stands in for a server
  /// that failed. This runs while ONLINE and does not suppress anything: the
  /// page tree request still goes out, and its errors still surface (P1c). All
  /// it does is stop the editor showing the home pane for the length of a round
  /// trip when a perfectly good local copy exists.
  ///
  /// Silent when there is no mirror (never opened on this device) — then the
  /// REST snapshot is genuinely the first content available, and the home pane
  /// is the honest thing to show until it lands.
  Future<void> _seedBodyFromMirror(DocumentView view) async {
    if (view.objectType == 'folder') return;
    final boot = await _offlineCloudBootstrap(view);
    // The user may have clicked another workspace while the mirror was read.
    if (boot == null || !mounted || _selectedView?.id != view.id) return;
    // The network may have won the race. Its snapshot is fresher than the
    // mirror by definition, so the mirror stands down rather than replacing a
    // body that is already correct — and rather than handing the CRDT session a
    // second, older starting point.
    if (_selectedBootstrap?.document.id == view.objectId) return;
    setState(() => _selectedBootstrap = boot);
    // Ownership of the body transfers here: _reconcileSync opens the yrs
    // session for this document, and everything the server says afterwards
    // arrives through it as a merge.
    _reconcileSync();
  }

  /// The page a workspace was last on, resolved against the tree ALREADY in
  /// memory — null when this workspace has not been opened in this session.
  ///
  /// Deliberately the same rule `_loadSelectedWorkspaceViews` applies once the
  /// real tree arrives (remembered id, else the first openable page), so the
  /// page shown the instant you switch is the page you end up on. A different
  /// rule here would show one page and then swap it for another.
  DocumentView? _rememberedViewOf(String workspaceId) {
    final views = _viewsByWorkspace[workspaceId];
    if (views == null || views.isEmpty) return null;
    final remembered = loadPref('lastViewId:$workspaceId');
    return views.where((view) => view.id == remembered).firstOrNull ??
        firstOpenableView(views);
  }

  Future<void> _loadSelectedWorkspaceViews() async {
    final session = _session;
    final workspace = _selectedWorkspace;
    if (session == null || workspace == null) {
      return;
    }

    // Which page we are about to open is usually already known — it is the one
    // this workspace was last on, and its tree is in memory. Asking for its
    // content only AFTER the tree comes back spends a second round trip
    // establishing something we could have assumed; ask for both at once and
    // throw the guess away if the tree disagrees.
    final guess = _rememberedViewOf(workspace.id);
    Future<DocumentBootstrap>? guessed;
    if (guess != null && guess.objectType != 'folder') {
      guessed = _api.bootstrapDocument(
        session.accessToken,
        workspace.id,
        guess.objectId,
      );
    }

    // Claim a sequence number for THIS request. Anything older that lands later
    // is answering a question we have already replaced.
    final seq = _treeRequests.claim(workspace.id);

    // Ask conditionally, but only while actually holding a tree to fall back
    // on: a 304 answered to a client with nothing would leave the sidebar
    // empty. The tag is stored beside the tree it describes, so a mirror
    // preheated from disk and its tag stay in step across restarts.
    final held = _viewsByWorkspace[workspace.id];
    final answer = await _api.listViewsIfChanged(
      session.accessToken,
      workspace.id,
      etag: (held == null || held.isEmpty)
          ? null
          : loadPref('viewsEtag:${workspace.id}'),
    );
    // Superseded while we were in flight: a newer request for this workspace
    // exists, so this response describes a state that has already been replaced.
    // Writing it would install the older tree AND its older ETag — and the stale
    // ETag is the durable half, because the next cold start sends it, gets a
    // 304, and keeps a tree the server has moved past. The selection block below
    // guards itself separately (`stillCurrent`), but the TREE and its tag had no
    // such guard: a slower load landing last overwrote the newer one.
    if (!_treeRequests.isCurrent(workspace.id, seq)) return;

    final views = answer.views ?? held ?? const <DocumentView>[];
    _commitTree(
      workspaceId: workspace.id,
      views: views,
      etag: answer.views == null ? null : answer.etag,
    );
    SwitchTrace.current?.mark(answer.views == null ? 'tree(304)' : 'tree');
    // Reopen the page last viewed IN THIS workspace (AppFlowy remembers the last
    // view per-workspace). On a restore or a workspace switch `_selectedView` is
    // null, so fall back to the per-workspace saved id; a deleted/absent id just
    // falls through to firstOpenableView below.
    final wantViewId =
        _selectedView?.id ?? loadPref('lastViewId:${workspace.id}');
    final selectedView = views
        .where((view) => view.id == wantViewId)
        .firstOrNull;
    // Auto-open a DOCUMENT (folders have no content to bootstrap — opening one
    // would 404 on its unbacked object_id).
    final viewToOpen = selectedView ?? firstOpenableView(views);
    DocumentBootstrap? bootstrap;
    String? bootstrapError;
    if (viewToOpen != null && viewToOpen.objectType != 'folder') {
      final inFlight = guessed != null && guess?.objectId == viewToOpen.objectId
          ? guessed
          : null;
      if (inFlight == null) guessed?.ignore(); // guessed wrong — drop it quietly
      try {
        bootstrap =
            await (inFlight ??
                _api.bootstrapDocument(
                  session.accessToken,
                  workspace.id,
                  viewToOpen.objectId,
                ));
      } on ApiException catch (error) {
        // The auto-open target answered with an error (404/403 — deleted
        // server-side, access revoked, or an unbacked folder object_id from an
        // older client that didn't guard). listViews already succeeded, so we
        // are demonstrably ONLINE: show the tree with nothing opened rather than
        // letting one bad view cascade into a full offline downgrade at startup.
        //
        // But SAY SO. Swallowing this silently rendered an open page as empty —
        // indistinguishable from "this page has no content", so a server-side
        // fault (e.g. a document whose root block was lost) looked exactly like
        // data loss: pressing refresh appeared to erase the page.
        bootstrap = null;
        bootstrapError = '$error';
      }
    } else {
      // Nothing to open (empty workspace, or a folder was remembered) — the
      // speculative fetch has no taker. Unignored, its failure would surface as
      // an unhandled async error with no user-visible cause.
      guessed?.ignore();
    }

    SwitchTrace.current?.mark('body');
    // Two clicks in quick succession run two of these. The tree is keyed by
    // workspace so a late arrival is harmless there, but the SELECTION is one
    // slot: without this the slower load lands last and opens the workspace the
    // user already switched away from. Nothing guards the entry point — there
    // is no busy latch on the workspace rows — so it has to be caught here.
    final stillCurrent = _selectedWorkspace?.id == workspace.id;
    // The REST snapshot cannot merge — it is the yrs base materialised into
    // plain JSON — so it must not land on a body a CRDT session already holds.
    // See `restSnapshotMayReplaceBody` for what that costs when it does.
    final mayWriteBody =
        viewToOpen == null ||
        restSnapshotMayReplaceBody(
          arrivingDocumentId: viewToOpen.objectId,
          crdtSessionDocumentId: _sync?.documentId,
        );
    setState(() {
      // The tree itself was installed by `_commitTree` (with its ETag, in one
      // step). This block owns the SELECTION and the body only.
      if (stillCurrent) {
        _selectedView = viewToOpen;
        if (mayWriteBody) {
          _selectedBootstrap = bootstrap;
        }
        if (bootstrapError != null) _message = bootstrapError;
      }
    });
    SwitchTrace.current?.mark('paint');
    // A 304 means the mirror ALREADY holds this tree — there is nothing to
    // write, and writing it anyway is the whole cost. Measured on a real
    // account: 2.1s per warm switch, against 130–320ms for everything the user
    // waits on. (Narrowing it to one workspace was not enough on its own: the
    // store's purge step still scans every view of the origin, and that scan is
    // most of the cost. Not doing it at all is both faster and more obviously
    // correct.)
    if (answer.views != null) {
      _cacheCloudPageTree(workspaceId: workspace.id);
    }
    SwitchTrace.current?.mark('mirror-write');
  }

  /// Mirror the cloud page tree (workspace list + per-workspace views) into the
  /// on-device store so a future offline start can still list and navigate cloud
  /// content (P2 option C — Phase 1b/1c). Origin-scoped by server URL, so
  /// switching servers doesn't cross over. P4-2: on web the mirror is
  /// localStorage-backed (LocalOffline web variant). The cloud is authoritative
  /// — this is a clean replace after each successful online load.
  /// Write the page-tree mirror for the ONE workspace whose tree just changed.
  ///
  /// It used to write EVERY workspace held in memory, every time. That was
  /// nearly free while memory only held the two or three workspaces visited in
  /// this session — and stopped being free the moment the mirror was preheated
  /// at startup, because memory then held all of them. Measured on a real
  /// account (35 workspaces) right after that change: a warm switch spent
  /// **2.1 seconds** here, against 130–320ms for everything the user actually
  /// waits on.
  ///
  /// Safe to narrow because `mirrorCloudPageTree` replaces per workspace: the
  /// ones not named here keep their rows.
  ///
  /// [workspaceId] is REQUIRED, and that is the fix for this happening twice.
  /// It was optional, defaulting to "write everything", and the second caller —
  /// creating a folder — simply did not pass it. So the 2.1 s write came back on
  /// every new folder, against a mirror that by then held ~13k views. An
  /// optional parameter whose omission is a performance cliff is a footgun with
  /// a safety catch; required, the compiler asks the question.
  void _cacheCloudPageTree({required String workspaceId}) {
    final origin = _api.baseUri.toString();
    final workspaces = <WorkspaceData>[
      for (final (i, w) in _workspaces.indexed)
        (
          id: w.id,
          name: w.name,
          position: ((i + 1) * 10).toString().padLeft(10, '0'),
          role: w
              .role, // mirror the real role so offline editing knows its rights
        ),
    ];
    final views = <ViewData>[
      for (final e in _viewsByWorkspace.entries)
        if (e.key == workspaceId)
        for (final v in e.value)
          (
            id: v.id,
            workspaceId: e.key,
            parentId: v.parentViewId,
            objectId: v.objectId,
            name: v.name,
            position: v.position,
            trashed: false,
            objectType: v.objectType,
          ),
    ];
    _local.mirrorCloudPageTree(origin, workspaces, views);
  }

  /// The server is unreachable (offline restart / transient outage): fall back to
  /// the on-device page-tree mirror so the user still enters the workspace and
  /// reads cached cloud content (P1c offline read). Reconstructs the workspace
  /// list + views from the mirror, opens the first view's mirrored doc if it was
  /// previously synced, and kicks off [_reconcileSync] (the WS reconnects and the
  /// yrs session seeds from disk immediately; both catch up when the network
  /// returns). Returns true if a cache existed and was applied.
  Future<bool> _applyOfflineCloudNav(AuthSession session) async {
    // The cloud cold-start path never opened the on-device store (only
    // localOffline mode and _setupCloudYrs do), so open it here — otherwise the
    // mirror reads as empty and this fallback silently no-ops. deviceClientId()
    // opens the store if needed and is a no-op once open (null on web, where
    // the mirror needs no FFI store).
    await _local.deviceClientId();
    if (!mounted) return false;
    final cache = _local.cachedCloudPageTree(_api.baseUri.toString());
    if (cache == null || cache.workspaces.isEmpty) return false;
    final rebuilt = rebuildCloudNavFromCache(cache, session.user.id);
    final workspace = rebuilt.workspaces.first;
    final views = rebuilt.views[workspace.id] ?? const <DocumentView>[];
    // Auto-open the first DOCUMENT, never a folder (a folder has no mirrored
    // doc → the editor would sit blank with a folder marked selected, and the
    // online path already skips folders — keep offline consistent).
    final firstView = firstOpenableView(views);
    final bootstrap = firstView == null
        ? null
        : await _offlineCloudBootstrap(firstView);
    if (!mounted) return false;
    _sessionCtrl.restore(session, rebuilt.workspaces, workspace, rebuilt.views);
    setState(() {
      _selectedView = firstView;
      _selectedBootstrap = bootstrap;
      // Mark degraded-nav mode: roles come from the (possibly stale) mirror and
      // ownerId/objectType are defaulted until the server is reachable again
      // (see _recoverOnlineNav). The server re-checks the real role on every
      // push, so a stale mirrored role is a UX gate only, never an authority.
      _offlineNav = true;
    });
    if (bootstrap != null) _reconcileSync();
    // An EMPTY workspace never reaches _reconcileSync (nothing to open), but
    // its sidebar still wants tree-change pings — MCP growing a first page
    // into an empty workspace is this feature's founding case. Idempotent when
    // the line above already ran.
    _reconcileViewsEvents();
    return true;
  }

  /// The server became reachable again after an offline start: refetch the real
  /// workspace list + views so the mirrored (possibly stale) roles and defaulted
  /// metadata are replaced by the authoritative server values — e.g. a role
  /// changed while offline takes effect, and ownerId/objectType become real
  /// again. Idempotent: only the first online contact does the work.
  void _recoverOnlineNav() {
    if (!_offlineNav || !mounted) return;
    _offlineNav = false;
    unawaited(_refreshWorkspaces());
  }

  /// Fired when a cloud session reaches the server (startup or reconnect): come
  /// back online, then push the un-synced outbox of EVERY cloud doc, not just the
  /// active one.
  void _onCloudOnline() {
    _recoverOnlineNav();
    unawaited(_sweepPendingOutboxes());
    // Images inserted offline, on the doc that is open right now. The sweep
    // above only carries TEXT (each doc's append-log); blobs were left to
    // `onReady`, which fires on a cold bootstrap — i.e. only when the document
    // is next OPENED. Reconnecting while sitting on the page therefore restored
    // text and left the pictures pending indefinitely.
    final active = _selectedBootstrap?.document.id;
    if (active != null) unawaited(_reconcilePendingUploads(active));
  }

  /// After coming online, drain the durable outbox of every cloud document that
  /// has un-pushed edits — not only the one currently open.
  ///
  /// The cloud session is a singleton bound to the ACTIVE doc, and each doc's
  /// outbox is only ever read by that doc's own session. So editing pages A/B/C
  /// offline and reconnecting while viewing C pushed only C; A and B's edits sat
  /// in the on-device append-log forever, and other devices kept showing stale
  /// content with no hint (the same gap the image-blob differ closed for images,
  /// never for text). This sweep closes it: for each other cloud doc with a
  /// non-empty outbox, spin up a short-lived headless [CloudSyncSession], let it
  /// flush + ack, and dispose it.
  ///
  /// Desktop only — the cross-doc durable outbox is the FFI append-log; web's
  /// store is per-tab IndexedDB with a single active doc and no enumeration.
  /// Best-effort and bounded: a doc with nothing pending is never even connected;
  /// a fault just leaves the edits durable for the next sweep. Sequential, so a
  /// backlog can't open a burst of sockets.
  Future<void> _sweepPendingOutboxes() async {
    if (kIsWeb || _sweepingOutboxes) return;
    final session = _session;
    final clientId = _deviceClientId;
    if (session == null || clientId == null) return;
    _sweepingOutboxes = true;
    try {
      // Snapshot the cloud (workspace, doc) pairs up front so we don't read
      // widget state across the awaits below. The active doc is drained by its
      // own live session — skip it here.
      final activeDoc = _selectedBootstrap?.document.id;
      final targets = <({String workspaceId, String docId})>[];
      for (final entry in _viewsByWorkspace.entries) {
        for (final v in entry.value) {
          if (v.objectType != 'document' || v.objectId == activeDoc) continue;
          targets.add((workspaceId: entry.key, docId: v.objectId));
        }
      }
      for (final t in targets) {
        if (!mounted) break;
        // Re-check: a doc opened mid-sweep is now the active one, drained live.
        if (t.docId == _selectedBootstrap?.document.id) continue;
        final store = _local.cloudDocStore(t.docId);
        if (store == null) continue;
        if (store.outboxAfter(store.cursor().pushedClock).isEmpty) {
          store.dispose();
          continue;
        }
        await _drainDocOutbox(
          t.workspaceId,
          t.docId,
          store,
          session.accessToken,
          clientId,
        );
      }
    } finally {
      _sweepingOutboxes = false;
    }
  }

  /// Push one non-active cloud doc's outbox through a short-lived headless
  /// session, then dispose it. The session owns [store] (its `dispose` releases
  /// it). Best-effort: any failure leaves the edits durable for a later sweep.
  Future<void> _drainDocOutbox(
    String workspaceId,
    String docId,
    CloudDocStore store,
    String token,
    BigInt clientId,
  ) async {
    final session = CloudSyncSession(
      // Short-lived (drain one outbox, then dispose).
      uri: () async =>
          documentSocketUri(_api.baseUri, workspaceId, docId, token),
      clientId: clientId,
      onReady: (_, _) {},
      onRemoteBlocks: (_) {},
      persistence: store,
    );
    try {
      session.connect();
      await session.drainOutbox();
    } catch (_) {
      // Durable append-log keeps the edits; the next online sweep retries.
    } finally {
      session.dispose(); // also disposes `store` (persistence)
    }
  }

  /// Build a bootstrap for a cloud [view] from its on-device mirror (offline
  /// doc-open, P1c). On desktop the mirrored replica loads synchronously via
  /// FFI (correct `rootBlockId` + blocks before first paint); on web (P4-2) it
  /// hydrates from IndexedDB — async, but awaited before the bootstrap is
  /// applied, so the rootBlockId is equally correct from the start. Null if the
  /// doc was never opened online (nothing mirrored): the tree still lists the
  /// page; opening it shows empty until back online. Uses the cloud doc id as
  /// the record id so [_reconcileSync] wires the same-keyed [CloudSyncSession].
  Future<DocumentBootstrap?> _offlineCloudBootstrap(DocumentView view) async {
    // A folder has no document to open.
    if (view.objectType == 'folder') return null;
    final data =
        _local.openCloudDocMirror(view.objectId) ??
        await openWebDocMirror(_cloudOrigin, view.objectId);
    if (data == null) return null;
    return _localBootstrapFrom(
      view.objectId,
      data.rootBlockId,
      data.blocks,
      view,
    );
  }

  // ── P3b: unified workspace layer ───────────────────────────────────────────
  //
  // One [WorkspaceView] wiring for both worlds. Every prop that used to differ
  // between the cloud shell and the local shell is dispatched here on
  // [_activeIsLocal]; the two build paths now feed the SAME handler set, so
  // P3c can dissolve the mode switch by only changing the dispatch criterion
  // and the shell chrome — not the wiring. Function bodies are the pre-P3b
  // ones, unmodified (mechanical merge).

  /// The origin of the ACTIVE world: `'local'` or the cloud server URL. Both
  /// worlds' state is loaded side by side (P3c); this picks which one the
  /// editor pane + page tree show. Persisted (`activeOrigin`) so a restart
  /// reopens the same world; switched by selecting a workspace entry.
  late String _activeOrigin;

  /// Whether the ACTIVE world is the local one.
  bool get _activeIsLocal => _activeOrigin == 'local';

  /// Select a workspace entry from the unified list: flips the active world to
  /// the entry's origin (persisted), then routes to that world's selector.
  Future<void> _selectEntry(WorkspaceEntry entry) async {
    if (_activeOrigin != entry.origin) {
      setState(() => _activeOrigin = entry.origin);
      savePref('activeOrigin', entry.origin);
    }
    if (entry.isLocal) {
      await _localSelectWorkspace(entry.workspace);
    } else {
      await _selectWorkspace(entry.workspace);
    }
  }

  // Per-entry workspace actions (P3c): the switcher lists BOTH worlds, so row
  // actions must dispatch on the ROW's origin, not the active one.
  Future<void> _renameEntry(WorkspaceEntry e, String name) => e.isLocal
      ? _localRenameWorkspace(e.workspace, name)
      : _renameWorkspace(e.workspace, name);

  Future<void> _deleteEntry(WorkspaceEntry e) => e.isLocal
      ? _localDeleteWorkspace(e.workspace)
      : _deleteWorkspace(e.workspace);

  Future<Uint8List> _exportEntryZip(WorkspaceEntry e) => e.isLocal
      ? Future.value(Uint8List(0)) // parity with the old local-shell stub
      : _exportWorkspaceZip(e.workspace.id);

  /// Persist a new workspace order for ONE world (the switcher only reorders
  /// within the connected world; cloud and local are separate position
  /// spaces). [ordered] is that world's full list in the intended order.
  Future<void> _reorderWorkspaceEntries(List<WorkspaceEntry> ordered) async {
    if (ordered.length < 2) return;
    final ids = [for (final e in ordered) e.workspace.id];
    if (ordered.first.isLocal) {
      _local.reorderWorkspaces(ids);
      if (mounted) setState(_reloadLocalWorkspaces);
    } else {
      await _run(() async {
        final session = _requireSession();
        await _api.reorderWorkspaces(session.accessToken, ids);
        final workspaces = await _api.listWorkspaces(session.accessToken);
        if (mounted) _sessionCtrl.setWorkspaces(workspaces);
      });
    }
  }

  /// Import loose files / a picked folder UNDER [folder] (parent_view_id) in the
  /// active world — the folder-menu counterpart of [_importTreeIntoEntry].
  /// [sourceName] (a picked folder's name) names the wrap container.
  Future<void> _importIntoFolder(
    DocumentView folder,
    List<ArchiveFile> entries, {
    String? sourceName,
    String? container,
  }) async {
    // The tree only ever shows the ACTIVE workspace, so the folder lives in it.
    if (_activeIsLocal) {
      final wsId = _workspaceIdOfView(folder.id);
      final ws = _localWorkspaces.where((w) => w.id == wsId).firstOrNull;
      if (ws == null) return;
      await _localImportVaultTree(
        ws,
        entries,
        sourceName: sourceName,
        parentViewId: folder.id,
        container: container,
      );
    } else {
      final ws = _selectedWorkspace;
      if (ws == null) return;
      await _importTreeIntoWorkspace(
        ws,
        entries,
        sourceName: sourceName,
        parentViewId: folder.id,
        container: container,
      );
    }
  }

  Future<void> _importTreeIntoEntry(
    WorkspaceEntry e,
    List<ArchiveFile> entries, {
    String? sourceName,
    String? container,
  }) => e.isLocal
      ? _localImportVaultTree(
          e.workspace,
          entries,
          sourceName: sourceName,
          container: container,
        )
      : _importTreeIntoWorkspace(
          e.workspace,
          entries,
          sourceName: sourceName,
          container: container,
        );

  /// Sign in to the configured cloud server from the switcher / account UI
  /// (desktop: the login gate is gone — auth is a dialog, P3c §1.3).
  Future<void> _promptSignIn({bool startOnCloud = false}) async {
    final creds = await _promptCloudAuth(startOnCloud: startOnCloud);
    if (creds == null || !mounted) return;
    await _run(() async {
      if (creds.$1 == AuthMode.register) {
        await _api.register(creds.$2);
        if (!mounted) return;
        ScaffoldMessenger.of(
          context,
        ).showSnackBar(SnackBar(content: Text(context.l10n.authVerifySent)));
        return;
      }
      final session = await _api.login(creds.$2);
      final workspaces = await _api.listWorkspaces(session.accessToken);
      _sessionCtrl.signIn(session, workspaces);
      _persistSession(session);
      unawaited(_syncSettingsFromCloud(session));
      unawaited(_refreshAiConfigured());
      await _loadSelectedWorkspaceMembers();
      await _loadSelectedWorkspaceViews();
    });
  }

  /// The unified workspace list (P3): cloud entries (with their roles) followed
  /// by local entries. Derived — the underlying per-world state stays the
  /// source of truth until P3c renders grouped sections from this.
  List<WorkspaceEntry> get _workspaceEntries {
    final cloudOrigin = _api.baseUri.toString();
    return [
      for (final w in _workspaces)
        WorkspaceEntry(origin: cloudOrigin, workspace: w, role: w.role),
      for (final w in _localWorkspaces)
        WorkspaceEntry(origin: 'local', workspace: w, role: 'owner'),
    ];
  }

  /// The selected workspace as a unified entry (null when nothing is selected).
  WorkspaceEntry? get _selectedEntry {
    if (_activeIsLocal) {
      final w = _localSelectedWorkspace;
      return w == null
          ? null
          : WorkspaceEntry(origin: 'local', workspace: w, role: 'owner');
    }
    final w = _selectedWorkspace;
    return w == null
        ? null
        : WorkspaceEntry(
            origin: _api.baseUri.toString(),
            workspace: w,
            role: w.role,
          );
  }

  /// The one [WorkspaceView] instantiation both shells share. Props that
  /// diverge between worlds dispatch on [_activeIsLocal]; identical props are
  /// passed straight through. Capability rule (P3 §2.3): a world without a
  /// feature passes the same stub the old shell passed — P3c turns these into
  /// nullable props that hide the UI.
  /// The current cloud doc's sync phase, driving the quiet header badge. Only
  /// meaningful while a cloud session is live (gated in _unifiedWorkspaceView).
  SyncPhase _syncPhase = SyncPhase.offline;

  /// Comment threads on the open cloud document, as the server resolved them
  /// (each carries offsets against the CURRENT text, or a null anchor when
  /// orphaned). Re-fetched on open — never cached across documents, and never
  /// re-derived locally: the anchor lives in the CRDT and only the server reads
  /// it (docs/comments-plan.md).
  List<CommentThread> _commentThreads = const [];

  /// The thread the reader is currently looking at, drawn stronger than the rest.
  ///
  /// Both halves of this feature were half-built and dead: `CommentPanel` declared
  /// an `onFocusThread` nobody passed (so tapping a quote did nothing), and the
  /// highlight producer hardcoded `active: false` (so the
  /// `editor.commentHighlightActive` token — defined, lerped, given light AND dark
  /// values — could never be painted). They were the same missing idea: there was
  /// no notion of a focused thread for either to refer to.
  String? _focusedCommentThreadId;

  /// Ranges for render.dart to wash: unresolved, still-anchored threads only.
  /// A resolved or orphaned thread must not paint over the text.
  List<CommentHighlight> get _commentHighlights => [
    for (final t in _commentThreads)
      if (t.isHighlightable && t.anchor != null)
        (
          startBlock: t.anchor!.startBlock,
          startOffset: t.anchor!.startOffset,
          // Both ends, as the anchor gives them. This used to pass `startBlock`
          // together with the END block's offset, so a cross-block comment
          // painted a wrong-length run inside its first block — or nothing.
          endBlock: t.anchor!.endBlock,
          endOffset: t.anchor!.endOffset,
          active: t.id == _focusedCommentThreadId,
        ),
  ];

  /// Load (or clear) the open document's comments. Additive by design: if this
  /// fails the document still opens — comments are never allowed to be the reason
  /// someone cannot read their page.
  /// Emphasise one thread's wash, or clear it with a null id.
  ///
  /// Only a state flip — the highlight list is derived, so the repaint follows. A
  /// resolved or orphaned thread is not in that list at all, which is why focusing
  /// one is a no-op rather than a special case here.
  void _focusCommentThread(String? threadId) {
    if (!mounted || _focusedCommentThreadId == threadId) return;
    setState(() => _focusedCommentThreadId = threadId);
  }

  Future<void> _loadComments(String documentId) async {
    final session = _session;
    final workspace = _selectedWorkspace;
    if (session == null || workspace == null || _activeIsLocal) {
      if (mounted) {
        setState(() {
          _commentThreads = const [];
          // A focused id from the previous document would otherwise linger and,
          // on the small chance of an id collision, emphasise the wrong thread.
          _focusedCommentThreadId = null;
        });
      }
      return;
    }
    try {
      final threads = await _api.listComments(
        session.accessToken,
        workspace.id,
        documentId,
      );
      if (!mounted || _selectedBootstrap?.document.id != documentId) return;
      setState(() => _commentThreads = threads);
    } catch (_) {
      if (mounted) {
        setState(() {
          _commentThreads = const [];
          // A focused id from the previous document would otherwise linger and,
          // on the small chance of an id collision, emphasise the wrong thread.
          _focusedCommentThreadId = null;
        });
      }
    }
  }

  /// Re-fetch after any mutation, so the server stays the single source of truth
  /// for anchors and orphan status.
  Future<void> _refreshComments() async {
    final documentId = _selectedBootstrap?.document.id;
    if (documentId != null) await _loadComments(documentId);
  }

  Future<void> _addComment(
    String startBlock,
    int startOffset,
    String endBlock,
    int endOffset,
    String quote,
  ) async {
    final session = _session;
    final workspace = _selectedWorkspace;
    final documentId = _selectedBootstrap?.document.id;
    if (session == null || workspace == null || documentId == null) return;
    final body = await _promptCommentBody();
    if (body == null || body.trim().isEmpty) return;
    try {
      await _api.createCommentThread(
        session.accessToken,
        workspace.id,
        documentId,
        startBlock: startBlock,
        startOffset: startOffset,
        endBlock: endBlock,
        endOffset: endOffset,
        quote: quote,
        body: body.trim(),
      );
      await _refreshComments();
    } catch (error) {
      // The one expected failure: the selection no longer exists (someone edited
      // it away between selecting and submitting). Say that, not the raw error.
      if (!mounted) return;
      final stale = error.toString().contains('cannot anchor');
      setState(
        () => _message = stale
            ? context.l10n.commentAnchorLost
            : error.toString(),
      );
    }
  }

  /// A small composer for the first comment on a range.
  Future<String?> _promptCommentBody() async {
    final l10n = context.l10n;
    final body = await showDialog<String>(
      context: context,
      builder: (context) => DialogTextControllers(
        count: 1,
        builder: (context, fields) => AlertDialog(
          title: Text(l10n.commentAdd),
          content: TextField(
            controller: fields[0],
            autofocus: true,
            minLines: 2,
            maxLines: 6,
            decoration: InputDecoration(
              hintText: l10n.commentPlaceholder,
              border: const OutlineInputBorder(),
            ),
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(context),
              child: Text(l10n.commonCancel),
            ),
            FilledButton(
              onPressed: () => Navigator.pop(context, fields[0].text),
              child: Text(l10n.commentPost),
            ),
          ],
        ),
      ),
    );
    return body;
  }

  Future<void> _replyToComment(String threadId, String body) async {
    final session = _session;
    final workspace = _selectedWorkspace;
    final documentId = _selectedBootstrap?.document.id;
    if (session == null || workspace == null || documentId == null) return;
    await _api.replyToComment(
      session.accessToken,
      workspace.id,
      documentId,
      threadId,
      body,
    );
    await _refreshComments();
  }

  Future<void> _setCommentResolved(String threadId, bool resolved) async {
    final session = _session;
    final workspace = _selectedWorkspace;
    final documentId = _selectedBootstrap?.document.id;
    if (session == null || workspace == null || documentId == null) return;
    await _api.setCommentResolved(
      session.accessToken,
      workspace.id,
      documentId,
      threadId,
      resolved,
    );
    await _refreshComments();
  }

  Future<void> _deleteCommentThread(String threadId) async {
    final session = _session;
    final workspace = _selectedWorkspace;
    final documentId = _selectedBootstrap?.document.id;
    if (session == null || workspace == null || documentId == null) return;
    await _api.deleteCommentThread(
      session.accessToken,
      workspace.id,
      documentId,
      threadId,
    );
    await _refreshComments();
  }

  Widget _unifiedWorkspaceView(AuthSession? session) {
    final local = _activeIsLocal;
    return WorkspaceView(
      session: session,
      // Avatar URLs are composed from this (ui/avatar_url.dart); the account
      // tile and the member list both live under this widget.
      apiBase: _api.baseUri,
      onChangeAvatar: local ? null : _changeAvatar,
      onRemoveAvatar: local ? null : _removeAvatar,
      entries: _workspaceEntries,
      activeOrigin: _activeOrigin,
      // Cloud-only: the badge is meaningless for a local world or with no live
      // cloud session, so it's simply absent there.
      syncPhase: (local || _sync == null) ? null : _syncPhase,
      // Comments are a cloud feature (they live in Postgres beside the document),
      // so the local world gets none and shows no entry point.
      commentThreads: local ? const [] : _commentThreads,
      commentHighlights: local ? const [] : _commentHighlights,
      onAddComment: (local || session == null) ? null : _addComment,
      onReplyComment: _replyToComment,
      onSetCommentResolved: _setCommentResolved,
      onDeleteCommentThread: _deleteCommentThread,
      onFocusCommentThread: _focusCommentThread,
      selectedRef: _selectedEntry?.ref,
      onSelectEntry: _selectEntry,
      onRenameEntry: _renameEntry,
      onDeleteEntry: _deleteEntry,
      onExportEntryZip: _exportEntryZip,
      onImportTreeIntoEntry: _importTreeIntoEntry,
      onImportTreeIntoFolder: _importIntoFolder,
      onReorderWorkspaces: _reorderWorkspaceEntries,
      // Only call a server "Mica Cloud" when this build actually ships one and
      // we are on it; otherwise just name the host we're talking to.
      cloudOriginLabel:
          kDefaultCloudUrl.isNotEmpty &&
              _api.baseUri.host == Uri.parse(kDefaultCloudUrl).host
          ? 'Mica Cloud'
          : _api.baseUri.host,
      onSignIn: session == null ? _promptSignIn : null,
      localAvailable: _local.available,
      isBusy: local ? false : _isBusy,
      onRefresh: local
          ? () => setState(() {
              _reloadLocalWorkspaces();
              _reloadLocalViews();
            })
          : () {
              if (!_isBusy) _refreshWorkspaces();
            },
      // Account-level action: dispatches on the SESSION, not the active world —
      // a signed-in user browsing the local world must still be able to sign
      // out (the tile only shows Sign out when session != null; _signOut is
      // safe regardless).
      onSignOut: () {
        if (!_isBusy) _signOut();
      },
      workspaces: local ? _localWorkspaces : _workspaces,
      selectedWorkspace: local ? _localSelectedWorkspace : _selectedWorkspace,
      members: local || _selectedWorkspace == null
          ? const []
          : _membersByWorkspace[_selectedWorkspace!.id] ?? const [],
      views: local
          ? _localViews
          : _selectedWorkspace == null
          ? const []
          : _viewsByWorkspace[_selectedWorkspace!.id] ?? const [],
      // The local world's tree is read from disk synchronously — it is never
      // "on its way". Only a cloud workspace we hold no tree for at all is.
      treePending:
          !local &&
          _selectedWorkspace != null &&
          !_viewsByWorkspace.containsKey(_selectedWorkspace!.id),
      selectedView: local ? _localSelectedView : _selectedView,
      selectedBootstrap: local ? _localBootstrap : _selectedBootstrap,
      // Cloud only — the local world has no tab model (see _openViewInNewTab).
      tabs: local ? const [] : _tabs,
      activeTabIndex: local ? 0 : _activeTabIndex,
      onSelectTab: local ? null : _selectTab,
      onCloseTab: local ? null : _closeTab,
      onOpenInNewTab: local ? null : _openViewInNewTab,
      onOpenInNewTabById: local ? null : _openViewByIdInNewTab,
      onNewTabPage: local
          ? null
          : () => _createDocumentInNewTab(context.l10n.newPage),
      // Cloud only: the local world has one implicit workspace, so there is
      // nothing for a name search to disambiguate.
      onSelectWorkspaceById: local ? null : _selectWorkspaceById,
      // Cloud only: the local world is a single implicit workspace, so there is
      // nothing to widen the scope to.
      onSearchAllWorkspaces: local ? null : _searchAllWorkspaces,
      onOpenSearchHit: local
          ? null
          : (viewId, workspaceId) =>
                unawaited(_openSearchHit(viewId, workspaceId)),
      searchRecents: buildWorkspaceRecents(
        context,
        workspaceId: local ? _localSelectedWorkspace?.id : _selectedWorkspace?.id,
        workspaceName:
            (local ? _localSelectedWorkspace : _selectedWorkspace)?.name ?? '',
        views: local
            ? _localViews
            : _selectedWorkspace == null
            ? const []
            : _viewsByWorkspace[_selectedWorkspace!.id] ?? const [],
      ),
      presence: local ? const [] : _presence,
      message: _message,
      importProgress: _importProgress,
      // Only offer it while an import is actually in flight.
      onCancelImport: _importJobId == null ? null : _cancelImport,
      onSelectWorkspace: local ? _localSelectWorkspace : _selectWorkspace,
      onCreateWorkspace: local ? _localCreateWorkspace : _createWorkspace,
      onRenameWorkspace: local ? _localRenameWorkspace : _renameWorkspace,
      onDeleteWorkspace: local ? _localDeleteWorkspace : _deleteWorkspace,
      onCreateDocument: local ? _localCreateDocument : _createDocument,
      onCreateChildDocument: local
          ? (parent, name) =>
                _localCreateDocument(name, parentViewId: parent.id)
          : _createChildDocument,
      onCreateFolder: local ? _localCreateFolder : _createFolder,
      onCreateChildFolder: local
          ? (parent, name) => _localCreateFolder(name, parentViewId: parent.id)
          : _createChildFolder,
      // Null in a local workspace: the archive is built server-side, and the
      // menu entry hides itself rather than failing after the click.
      onExportFolderZip: local ? _localExportFolderZip : _exportFolderZip,
      onReorderViews: local ? _localReorderViews : _reorderViews,
      onLoadTrash: local ? _localLoadTrash : _loadTrash,
      onRestoreView: local ? _localRestoreView : _restoreView,
      onPurgeView: local ? _localPurgeView : _purgeView,
      // Cloud only: the on-device store has no bulk purge, so the button is
      // absent rather than broken in 本地模式.
      onPurgeAllTrash: local ? null : _purgeAllTrash,
      onSelectView: local ? _localSelectView : _selectView,
      onRenameView: local ? _localRenameView : _renameView,
      // Cloud-only: the local store's view record has no icon column, so rather
      // than accept a change it would silently drop, the local world simply does
      // not offer the entry (see DocumentListItem.onSetIcon).
      onSetViewIcon: local ? null : _promptSetViewIcon,
      // Home spans every workspace of the ACTIVE world (local and cloud stay
      // separate — see the world switcher), so only the shell can build it. The
      // local world keeps one flat view list rather than a per-workspace map, so
      // it contributes just its selected workspace.
      homePane: buildHomePane(
        context,
        viewsByWorkspace: local
            ? {
                if (_localSelectedWorkspace != null)
                  _localSelectedWorkspace!.id: _localViews,
              }
            : _viewsByWorkspace,
        workspaceNames: {
          for (final w in (local ? _localWorkspaces : _workspaces))
            w.id: w.name,
        },
        onCreatePage: () => unawaited(
          local
              ? _localCreateDocument(context.l10n.newPage)
              : _createDocument(context.l10n.newPage),
        ),
        onOpenView: (viewId) => _openViewFromHome(viewId, local: local),
      ),
      onOpenHome: () {
        setState(() => _overviewFolderId = null);
        _closeOpenPage(local: local);
      },
      // Only built while a folder is being browsed — otherwise home shows.
      overviewPane: _overviewFolderId == null
          ? null
          : buildOverviewPane(
              context,
              views: local
                  ? _localViews
                  : (_viewsByWorkspace[_selectedWorkspace?.id] ?? const []),
              folderId: _overviewFolderId,
              mode: _overviewMode,
              onModeChanged: (mode) => setState(() => _overviewMode = mode),
              onOpen: (pageId) => _openViewFromHome(pageId, local: local),
              onEnterFolder: (id) => setState(() => _overviewFolderId = id),
              onCreatePage: () => unawaited(
                local
                    ? _localCreateDocument(context.l10n.newPage)
                    : _createDocument(context.l10n.newPage),
              ),
            ),
      onDeleteView: local ? _localDeleteView : _deleteView,
      onDeleteViews: local ? _localDeleteViews : _deleteViews,
      onCloneView: local ? _localCloneView : _cloneView,
      onUpdateRootBlockText: local
          ? _localUpdateRootBlockText
          : _updateRootBlockText,
      onAddBlock: local ? (_, _) async {} : _addBlock,
      onUpdateBlock: local ? (_, _, _) async {} : _updateBlock,
      onDeleteBlock: local ? (_) async {} : _deleteBlock,
      onMoveBlock: local ? (_, _) async {} : _moveBlock,
      onApplyOperations:
          _applyEditorOperations, // P3d: one entry, self-dispatches
      onEditorFault: _onEditorFault,
      onUploadImage: local ? _localUploadImage : _uploadEditorImage,
      onImportImageUrl: local ? _localImportImageUrl : _importEditorImageUrl,
      onLoadImageBytes: local ? _localLoadImageBytes : _loadEditorImageBytes,
      onResolveImageUrls: local
          ? _localResolveImageUrls
          : _resolveEditorImageUrls,
      onAiStream: local
          ? (_, {system}) => const Stream<String>.empty()
          : _aiStream,
      onAiNewPage: local ? (_) async {} : _aiNewPageFromMarkdown,
      onAiPreviewEdits: local ? null : _previewAiEdits,
      onAiCurrentPage: local || _selectedBootstrap == null
          ? null
          : _aiCurrentFromMarkdown,
      onAiNewWorkspace: local ? (_) async {} : _aiNewWorkspaceFromMarkdown,
      // Null, not a no-op — absence is what hides the AI tab, the same rule
      // onLoadTokens/onUpdateProfile below already follow. These two were the
      // ones that got missed: AI settings live on the server, so in 本地模式 the
      // tab was a live provider form whose Save went into a function that
      // returned immediately.
      onLoadAiSettings: local ? null : _loadAiSettings,
      onLoadImportHistory: local ? null : _loadImportHistory,
      onListAiModels: local ? null : _listAiModels,
      onSaveAiSettings: local ? null : _saveAiSettings,
      onDeleteAiProvider: local ? null : _deleteAiProvider,
      onLoadTokens: local ? null : _loadTokens,
      onCreateToken: local ? null : _createToken,
      onRevokeToken: local ? null : _revokeToken,
      // Only ever the cloud account: 本地模式 has none, and the Account tab is
      // absent there rather than filled with a fiction. (An earlier attempt put
      // '本地工作区' here — which surfaced as an editable Display name for an
      // account that does not exist, above a Save button that did nothing.)
      userName: _session?.user.displayName ?? '',
      userEmail: _session?.user.email ?? '',
      // Null, not a no-op: absence is what hides the tab. See onLoadTokens.
      onUpdateProfile: local ? null : _updateProfile,
      onChangePassword: local ? null : _changePassword,
      onDeleteAccount: local ? null : _deleteAccount,
      onSwitchWorld: _promptSignIn,
      onRemoveServer: confirmRemoveServer,
      // Assembled here, not stored: the palette comes from the theme above us
      // and the rest from prefs. A stored copy would go stale the moment the
      // theme changed and the canvas would keep painting the old colours.
      appearance: _appearance.withTokens(MicaTheme.of(context)),
      pageWidth: _pageWidth,
      reHostImages: _reHostImages,
      onReHostImagesChanged: (value) {
        setState(() => _reHostImages = value);
        _savePrefs();
      },
      showFormatBar: _showFormatBar,
      onShowFormatBarChanged: (value) {
        setState(() => _showFormatBar = value);
        _savePrefs();
      },
      showPageTitle: _showPageTitle,
      onShowPageTitleChanged: (value) {
        setState(() => _showPageTitle = value);
        _savePrefs();
      },
      showAi: local ? false : _aiEnabled && _aiConfigured,
      // No 本地模式 ternary: the AI tab is absent there (onLoadAiSettings is
      // null), so nothing reads these.
      aiEnabled: _aiEnabled,
      onAiEnabledChanged: (value) {
        setState(() => _aiEnabled = value);
        _savePrefs();
      },
      onAppearanceChanged: (appearance, pageWidth) {
        setState(() {
          _appearance = appearance;
          _pageWidth = pageWidth;
        });
        _savePrefs();
      },
      // Desktop local mode now has a real index (`doc_snapshot.content_text`),
      // so it searches for real. Web local mode still has no on-device store at
      // all, and there `null` is still the honest answer — «this world cannot
      // search» is a different statement from «nothing matched», and the dialog
      // says a different thing for each. A stub returning `const []` used to
      // collapse the two, telling every local user their page was not there.
      onSearch: local ? (kIsWeb ? null : _searchLocalWorkspace) : _searchWorkspace,
      onOpenSearchResult: local ? _openLocalViewById : _openViewById,
      // Both worlds now answer this — `_loadBacklinks` picks the on-device index
      // in 本地模式 and the endpoint in the cloud. It used to be `local ? null`,
      // which hid the panel outright: a locally-linked page looked unlinked.
      onLoadBacklinks: _loadBacklinks,
      onLoadGraph: _loadGraph,
      // Local workspaces have no server to bundle the zip; SAY so rather than
      // handing back Uint8List(0), which the caller happily saved as a 0-byte
      // page.zip and called it a successful export.
      onExportPageZip: local
          ? () async => throw ApiException(context.l10n.exportLocalUnsupported)
          : _exportPageZip,
      // Page export chooses `.md` (no assets) vs `.zip` (bundled images) by
      // content — now in BOTH worlds: local goes through the FFI engine +
      // in-house ZIP writer (see _localExportPage), matching the cloud output.
      onExportPage: local ? _localExportPage : _exportPage,
      onCopyPageMarkdown: local ? _localPageMarkdownText : _pageMarkdownText,
      // HTML export works in BOTH worlds — local goes through the FFI engine
      // (see _localExportPageHtml), so unlike the ZIP it isn't gated off local.
      onExportPageHtml: local ? _localExportPageHtml : _exportPageHtml,
      // PDF is world-agnostic: both worlds produce HTML, then the same
      // WebView2 headless print turns it into PDF bytes (desktop only; web
      // stub returns null and the menu item is hidden there).
      onExportPagePdf: _local.htmlToPdf,
      onImportMarkdown: local ? (_, _) async {} : _importMarkdownAsPage,
      onExportWorkspaceZip: local
          ? (_) async => Uint8List(0)
          : _exportWorkspaceZip,
      onExportAllWorkspaces: local ? null : _exportAllWorkspaces,
      // Cloud only: 本地模式 has no cross-workspace export to describe.
      onLoadExportStats: local ? null : _exportAllStats,
      // Desktop only: web has no on-device store to measure.
      onLoadCacheStats: _local.available
          ? () => _local.cacheStats(mirroredOrigins: _servers)
          : null,
      onClearMirrorCache: _local.available
          ? () => _local.clearMirrorCache(mirroredOrigins: _servers)
          : null,
      onImportWorkspaceZip: local ? null : _importWorkspaceZip,
      onImportWorkspaceTreeInto: local
          ? _localImportVaultTree
          : _importTreeIntoWorkspace,
      // Null, not a no-op: a local workspace has no members at all. The no-op
      // form still rendered and still accepted an email, because local entries
      // carry role 'owner' so canManage was true — you could invite someone and
      // nothing whatsoever happened.
      onAddMember: local ? null : _addWorkspaceMember,
      onLoadWorkspaceUsage: local ? null : _loadWorkspaceUsage,
      onUpdateMember: local ? null : _updateWorkspaceMember,
      onRemoveMember: local ? null : _removeWorkspaceMember,
      onRestoreCheckpoint: local ? _localRollbackDoc : null,
      // Cloud-only: version history lives server-side (local has no history).
      // Both worlds now have real version history — local via the on-device
      // doc_version timeline (FFI), cloud via the yrs endpoints.
      onVersionHistory: local ? _openLocalVersionHistory : _openVersionHistory,
      onShare: local ? null : _openShare,
      // Cross-workspace move/copy — cloud-only (the destination is another
      // cloud workspace; local worlds don't participate). Null hides both
      // row-menu entries in 本地模式.
      onTransfer: local ? null : _openTransfer,
      // P3f: both live on the workspace ROW's menu, dispatching per entry —
      // null on web (no local world / no on-device store).
      onMigrateEntry: _local.available ? _migrateEntry : null,
      onDetachEntry: _local.available ? _detachEntry : null,
      editorEpoch: local ? _localEditorEpoch : 0,
      onCursorChanged: local ? null : _onEditorSelection,
    );
  }

  @override
  Widget build(BuildContext context) {
    final session = _session;
    // Web has no local world: it stays gated on sign-in, as before (P3c §2.6).
    if (!_local.available && session == null) {
      return _signInGate(context);
    }
    // Desktop: the local store backs local workspaces AND the cloud mirrors —
    // wait for it before the shell renders (fast: one SQLite open + list).
    if (_local.available && !_localReady) {
      return const Scaffold(body: Center(child: CircularProgressIndicator()));
    }
    // Desktop pointing at a SERVER with no session. The shell used to render
    // anyway: empty workspace picker, empty page tree, nothing clickable — a
    // whole screen that does nothing, whose only way out was 「登录云端…」 buried
    // in the account menu. Nothing on screen said so. Show the same gate web
    // shows; 本地模式 sits in its world picker, so this is not a dead end.
    if (_local.available && _activeOrigin != kLocalOrigin && session == null) {
      return _signInGate(context);
    }
    return Scaffold(body: SafeArea(child: _unifiedWorkspaceView(session)));
  }
}

/// The currently-mounted editor's debounce flush, registered by the active
/// [WorkspaceView] (via its command hook) so the app-exit path
/// ([_WorkspaceShellState._flushForExit]) can drain the last <=400ms of typing
/// before quitting. Null when no editor is on screen (settings, empty state).
Future<void> Function()? _activeEditorFlush;

