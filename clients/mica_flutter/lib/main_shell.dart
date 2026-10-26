part of 'main.dart';

class _WorkspaceShellState extends MicaState<WorkspaceShell> {
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

  AuthSession? get _session => _sessionCtrl.session;

  /// The auth domain's session half — credential persistence keyed by origin,
  /// renewal/expiry lifecycle, profile refresh, the sign-in core and the
  /// server readiness probe — see auth/auth_controller.dart (god-class sixth
  /// step). The legacy fields below ([_registrationOpen], [_regOrigin]) are
  /// synced view-model copies for the build helpers; the session itself lives
  /// in [_sessionCtrl] and every credential write goes through the
  /// controller.
  late final AuthController _auth;

  /// The session/workspace hub — the four fields every domain used to touch
  /// directly, now owned by [SessionController] (god-class third step; see
  /// docs/reviews/code-optimization/OPTIMIZATION_PLAN_ROUND3.md §7). The four legacy
  /// names below are plain passthrough getters so the build helpers read them
  /// unchanged; every mutation goes through a controller transition and the
  /// listener only triggers a rebuild.
  late final SessionController _sessionCtrl;
  List<Workspace> get _workspaces => _sessionCtrl.workspaces;

  /// The workspace members map — see views/members_controller.dart. The name
  /// below is a passthrough getter for the build helpers.
  late final MembersController _membersCtrl;

  /// Workspace invitations (views/invitations_controller.dart).
  late final InvitationsController _invites = InvitationsController(
    api: _api,
    token: () => _sessionCtrl.session?.accessToken,
    onJoined: (w) =>
        _refreshWorkspaces().then((_) => _selectWorkspaceById(w.id)),
  );
  Map<String, List<WorkspaceMember>> get _membersByWorkspace =>
      _membersCtrl.membersByWorkspace;
  Map<String, List<DocumentView>> get _viewsByWorkspace =>
      _sessionCtrl.viewsByWorkspace;

  /// The cloud page-tree domain (writes, loads, mirror) — see
  /// views/cloud_views_controller.dart (god-class fifth step). The tree
  /// itself lives in [_sessionCtrl]; this owns the fetch bookkeeping.
  late final CloudViewsController _viewsCtrl;
  Workspace? get _selectedWorkspace => _sessionCtrl.selectedWorkspace;

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
  /// docs/reviews/code-optimization/OPTIMIZATION_PLAN_ROUND3.md. [_importProgress]
  /// and [_importJobId] below are plain passthrough getters, so the sidebar
  /// row and its cancel button keep reading them unchanged; the controller's
  /// listener only triggers a rebuild.
  late final ImportController _import;
  ({int done, int total})? get _importProgress => _import.progress;

  /// The running import's job id, so the progress row can ask it to stop. Null
  /// whenever no import is in flight.
  String? get _importJobId => _import.jobId;
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

  // The ACTIVE tab's sync session (see `_cloudSession` below); background tabs
  // keep their own in [DocTab.cloudSession], reached only by the cap. Writes
  // go through the sync controller, which holds the tab list.
  List<PresenceUser> _presence = const [];

  /// The sync domain — the reconcile loop that keeps the document sockets
  /// tracking the open tab, the tree-change socket, the cross-document outbox
  /// sweep and the sync-paused banner — see sync/sync_reconcile_controller.dart
  /// (god-class seventh step). The sockets themselves live on the tabs; the
  /// controller reaches them through the tab list it is handed.
  late final SyncReconcileController _syncCtrl;

  // §7 upstream blob differ (desktop only): images inserted while offline land in
  // the on-device CAS under a sha256 placeholder file_id and queue here. When the
  // doc is next open online, `_reconcilePendingUploads` uploads the bytes, learns
  // the cloud UUID, and rewrites the block's file_id sha256→UUID. Persisted in
  // prefs so the intent survives a restart; `_reconciling` guards re-entrancy.
  PendingUploads _pending = PendingUploads();
  bool _reconciling = false;

  // --- Cloud yrs CRDT sync (P2-M4.5c, desktop only) ---
  // When the server speaks the yrs sync protocol (M4.4+), the desktop cloud
  // editor edits a CRDT replica instead of POSTing block ops: edits push yrs
  // diffs, remote updates merge + reconcile. It activates only once bootstrap
  // succeeds (`isReady`), so against an older server the app falls back to the
  // op/REST path transparently. The same session now also carries presence
  // (folded in from the old standalone socket, which never reconnected).
  CloudSyncSession? get _cloudSession => _activeTab.cloudSession;

  /// Bumped on every tab activation; each tab records the value it saw. See
  /// [DocTab.lastActivated].
  int _activationTick = 0;

  // --- Local offline (P2-M3) ---
  // A single implicit local workspace + synthetic identity; the page tree and
  // documents live entirely on-device (SQLite via the LocalOffline facade).
  // The LOCAL WORLD's page tree/selection/bootstrap/epoch and every local
  // operation live on [_localBackend] (world/local_world_backend.dart); the
  // store itself stays here because it also backs the cloud mirrors.
  final LocalOffline _local = createLocalOffline();
  bool _localReady = false;

  /// The two worlds behind the shell (world/world_backend.dart). [_activeBackend]
  /// is the one [_unifiedWorkspaceView] wires itself to — the interface is what
  /// replaced the ~80 `local ? ... : ...` ternaries that wiring used to carry.
  late final _LocalWorldBackend _localBackend = _LocalWorldBackend(this);
  late final _CloudWorldBackend _cloudBackend = _CloudWorldBackend(this);
  WorldBackend get _activeBackend =>
      _activeIsLocal ? _localBackend : _cloudBackend;

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
    _registry.addListener(
      _syncRegistryState,
    ); // after loadPrefs: setState-in-initState throws
    _servers = _registry.servers;
    _cloudOrigin = _registry.cloudOrigin;
    _sessionCtrl = SessionController()..addListener(_syncSessionState);
    _initAuthController();
    _initViewsController();
    _initMembersController();
    _initImportController();
    _initSyncController();
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
      WidgetsBinding.instance.addPostFrameCallback(
        (_) => _localBackend._initLocalOffline(),
      );
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
      onBeforeRemove: () async {
        if (_activeOrigin != origin) return true;
        await _cloudSession?.drainOutbox(timeout: const Duration(seconds: 4));
        if (!mounted) return false;
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
    final base = resolveApiBase(pinned: pinned, saved: cloudOrigin);
    if (base != null) {
      _api.baseUri = base;
    }
    // The palette is NOT seeded here. It used to be, and that is exactly one
    // frame too late: this runs from initState, inside MaterialApp's first
    // build, where notifying an ancestor cannot rebuild it. main() does it
    // before runApp instead.
    _loadAppearancePrefs();
    if (!kIsWeb) {
      _pending = PendingUploads.fromJson(loadPref('pendingBlobUploads'));
    }
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

  // Session persistence / renewal / profile live in [AuthController].

  /// Losing the cloud credentials (expired/revoked token) leaves a stale cloud
  /// activeOrigin behind — fall back to the local world (desktop) so a restart
  /// doesn't keep opening an empty cloud pane (matches _signOut's semantics).
  void _fallBackToLocalWorld() {
    if (!_local.available || _activeIsLocal) return;
    setState(() => _activeOrigin = 'local');
    savePref('activeOrigin', _activeOrigin);
  }

  /// Restore a persisted cloud/self-hosted session on startup so the user isn't
  /// forced to re-login every launch. Cheaply rejects an expired token by its
  /// JWT `exp`, then validates the rest by loading workspaces: a 401 (revoked /
  /// server JWT-secret changed) drops the token; a transient network error keeps
  /// it (this launch shows login, the next retries).
  Future<void> _restoreSession() async {
    if (_session != null) return;
    // Every write below is keyed by the origin at WRITE time. A server switch
    // (or a manual sign-in) during an await used to persist this server's
    // token under the other's key and send it there (2026-10-21 review U2).
    final origin = _cloudOrigin;
    final startGeneration = _sessionCtrl.generation;
    bool stale() => !mounted || _cloudOrigin != origin || _session != null;
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
        if (stale()) return;
        _auth.persistSession(recovered);
        token = recovered.accessToken;
      } on ApiException catch (error) {
        // The cookie is gone or burnt — a real end of session, but only for
        // the sign-in that asked (issue #147: error branches owe `stale()` too).
        if (error.isUnauthorized && !stale()) {
          _auth.clearPersistedSession();
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
      _auth.clearPersistedSession();
      return;
    }

    // The access token died while the app was closed — which, at a 24h TTL, is
    // most mornings. With a refresh token that is a non-event; without one (an
    // older server, or 30 days idle) the sign-in really is over.
    final exp = jwtExpiry(token);
    if (exp != null && !exp.isAfter(DateTime.now().toUtc())) {
      if (refreshToken.isEmpty) {
        _auth.clearPersistedSession();
        _fallBackToLocalWorld();
        return;
      }
      try {
        // Through the controller's refresher, not _api directly: it holds the
        // single-flight latch, and startup restore can overlap with a _run()
        // the user just triggered. Two refreshes of one token is what the
        // server reads as theft — we would burn the session we came here to
        // rescue.
        session = await _auth.refresher.ensureFresh(session) ?? session;
        if (stale()) return;
        _auth.persistSession(session);
      } on ApiException catch (error) {
        // Only the sign-in that asked may lose its credentials (issue #147).
        if (error.isUnauthorized && !stale()) {
          _auth.clearPersistedSession();
          _fallBackToLocalWorld();
        }
        // Anything else (server down) keeps the session for the next launch.
        return;
      } catch (_) {
        return; // offline: try again next launch, don't destroy credentials
      }
    }
    var installed = false;
    try {
      // `session.accessToken`, not the `token` we loaded: a refresh above
      // replaced it, and spending the dead one here would 401 and throw the
      // freshly-renewed sign-in away.
      final workspaces = await _api.listWorkspaces(session.accessToken);
      if (stale()) return;
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
      _sessionCtrl.restore(session, workspaces, restoredWs, {
        ..._warmTreesFromMirror(session),
        ..._viewsByWorkspace,
      });
      installed = true;
      unawaited(_syncSettingsFromCloud(session));
      unawaited(_refreshAiConfigured());
      // The session we just restored carries the profile as it was at LAST
      // login — it came off disk, which is why a restart was not what fixed a
      // stale avatar. This is the startup counterpart to the _run() call.
      unawaited(_auth.refreshProfile());
      await _loadSelectedWorkspaceMembers();
      await _loadSelectedWorkspaceViews();
      // The startup restore isn't a _run() action, so nothing else wires the
      // doc it just auto-opened: without this, the WS sync session (presence +
      // yrs CRDT + local-first mirror) only starts on the user's FIRST click —
      // typing before that silently rode the REST fallback with no mirror.
      _reconcileSync();
    } catch (error) {
      // Sign-in/out during the awaits retired this restore (issue #147): a
      // generation check, since the restore installs a session mid-flight.
      final expected = installed ? startGeneration + 1 : startGeneration;
      if (!mounted || _cloudOrigin != origin ||
          _sessionCtrl.generation != expected) {
        return; // never clear the NEW sign-in's credentials
      }
      // Revoked/invalid token → drop it; desktop falls back to the local world.
      // Keyed on the status, not on the message text: sniffing for the word
      // "unauthorized" both missed 401s worded differently and would fire on a
      // 400 that merely mentioned the word.
      if (error is ApiException && error.isUnauthorized) {
        _auth.clearPersistedSession();
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
      'file_too_large' =>
        _maxUploadBytes > 0
            ? context.l10n.uploadFileTooLargeWithLimit(
                formatBytesBinary(_maxUploadBytes),
              )
            : context.l10n.uploadFileTooLarge,
      'workspace_quota_exceeded' => context.l10n.uploadWorkspaceFull,
      'owner_quota_exceeded' => context.l10n.uploadOwnerFull,
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
      // Not fire-and-forget-and-swallow: the auth controller's ensure-fresh
      // handles its own failures (401 ends the session, anything else keeps it).
      final expiry = _session?.expiresAt;
      final expired =
          expiry != null &&
          !expiry.isAfter(
            DateTime.now().toUtc().add(const Duration(seconds: 30)),
          );
      if (expired) {
        await _auth.ensureFreshSession();
      } else {
        unawaited(_auth.ensureFreshSession());
      }
      await action();
      _reconcileSync();
      // Unawaited: the action is done and the user is waiting on nothing here.
      // This is the app's most frequent "we just talked to the server anyway"
      // moment, which is what makes a profile changed elsewhere show up without
      // anything polling on a timer.
      unawaited(_auth.refreshProfile());
    } on ApiException catch (error) {
      // A 401 that survived the renewal above: revoked, or the server's JWT
      // secret rotated. The session is unusable — say so instead of parroting
      // the server's bare `unauthorized`, which told the user nothing and left
      // the local mirror rendering a half-dead page that merely looked broken.
      if (error.isUnauthorized && _session != null) {
        _auth.endExpiredSession();
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
  /// Commit the mounted editor's debounced (≤400ms) typing to the document it
  /// was typed into, BEFORE anything changes which document is active.
  ///
  /// Editor ops are routed by the shell's ACTIVE state (cloud session / local
  /// doc), and the editor's own dispose does not flush — so a tab switch used to
  /// unmount the editor with the last keystrokes still debounced, and they were
  /// never committed (2026-10-21 review P1-2). The sidebar path already awaited
  /// this; every other way of changing the active document now does too.
  Future<void> _flushActiveEditor() async {
    debugProbeShellFlush?.call();
    try {
      await _activeEditorFlush?.call();
    } catch (_) {}
  }

  /// Flush, then open a fresh tab and run [load] into it.
  Future<void> _inNewTab(Future<void> Function() load) async {
    await _flushActiveEditor();
    if (!mounted) return;
    _pushTab();
    await load();
  }

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
    unawaited(_inNewTab(() => _openViewById(viewId)));
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
  /// Nothing is revealed in the sidebar. `SidebarTreeController.revealInTree`
  /// on an id that is not in the current tree PARKS the request until that
  /// workspace's tree loads, so revealing here would make the sidebar jump at
  /// some unrelated later moment.
  Future<void> _openElsewhereInBackgroundTab(
    String viewId,
    String workspaceId,
  ) async {
    final session = _session;
    if (session == null) return;

    // The tree of a workspace this device has never opened is not in memory —
    // the controller fetches it (one request) and files it; the tree is what
    // the tab needs anyway the moment you click it. An ApiException reaches
    // the message bus through the controller's onError hook.
    final view = await _viewsCtrl.ensureView(
      session.accessToken,
      workspaceId,
      viewId,
    );
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
      setState(() => _message = error.message);
      // Through _closeTab: the user may have clicked into this tab while it
      // loaded, and a bare remove left _activeTabIndex past the end (RangeError)
      // and the tab's socket live (2026-10-21 review U3).
      final i = _tabs.indexOf(tab);
      if (i >= 0) await _closeTab(i);
    }
  }

  /// Create a page and open it in a new tab. The `+` menu's first entry.
  ///
  /// No parent: the sidebar's placement rules follow where the POINTER is, and
  /// the `+` sits in the tab strip, which is nowhere in the tree. Root is the
  /// only honest answer — guessing the last-focused folder would put pages
  /// somewhere the user never pointed at.
  void _createDocumentInNewTab(String name) {
    unawaited(_inNewTab(() => _createDocument(name)));
  }

  /// Open [view] in a NEW tab and switch to it. The sidebar row's context menu.
  void _openViewInNewTab(DocumentView view) {
    // The load runs AFTER the new tab is active: `_selectView` skips the load
    // when the target is already open, and it tests that against the ACTIVE
    // tab. A fresh tab has a null view, so the same page opening in a second
    // tab is correctly treated as "not open here" and loads.
    unawaited(_inNewTab(() => _selectView(view)));
  }

  Future<void> _selectTab(int index) async {
    if (index < 0 || index >= _tabs.length || index == _activeTabIndex) return;
    await _flushActiveEditor();
    if (!mounted || index >= _tabs.length || index == _activeTabIndex) return;
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
  Future<void> _closeTab(int index) async {
    if (_tabs.length <= 1 || index < 0 || index >= _tabs.length) return;
    // Only the active tab has a mounted editor with typing to commit.
    if (index == _activeTabIndex) {
      final closing = _tabs[index];
      await _flushActiveEditor();
      if (!mounted) return;
      index = _tabs.indexOf(closing);
      if (_tabs.length <= 1 || index < 0) return;
    }
    // BEFORE the removal, and unconditionally: once the tab is out of `_tabs`
    // nothing reaches its sockets, and `_reconcileSync` below only ever touches
    // the ACTIVE tab — so a closed background tab would keep a live WebSocket
    // and an undrained replica for the rest of the session.
    _syncCtrl.parkTab(_tabs[index]);
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

  // The tree-change socket for the workspace whose SIDEBAR is showing lives in
  // the sync controller now (it owns the socket + the workspace id it watches).

  /// Re-ask for [workspaceId]'s tree because the server said it changed.
  ///
  /// The ETag turns a ping about something this client already knows (its own
  /// action's refetch, a rename it just made) into a 304. Errors are swallowed:
  /// this runs off a background bell, there is nothing for a user to act on,
  /// and the next bell — or the manual refresh button, which stays — retries.
  Future<void> _refreshTreeFromServer(String workspaceId) async {
    final session = _session;
    if (session == null) return;
    await _viewsCtrl.refreshTree(session.accessToken, workspaceId);
  }

  /// Open, switch, or close the document sockets so they always track the
  /// open document — the controller's reconcile, dispatched from here (the
  /// hooks it fires land back in this State).
  void _reconcileSync() => _syncCtrl.reconcile();

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
    _syncCtrl.onEditorSelection(blockId, offset);
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

  /// The editor's op pipeline failed to commit a batch (red line #1: never lose
  /// an edit without a trace) — routed through the controller so a failing
  /// cloud session reuses the "sync paused" banner+retry.
  void _onEditorFault(Object error, int count) =>
      _syncCtrl.onEditorFault(error, count);

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
    // Owned here (initState): detach AND dispose, or the notifier leaks.
    // themeMode/locale are app-level singletons, detached only.
    _registry.removeListener(_syncRegistryState);
    _sessionCtrl.removeListener(_syncSessionState);
    _auth.removeListener(_syncAuthState);
    _membersCtrl.removeListener(_syncMembersState);
    _import.removeListener(_syncImportState);
    _registry.dispose();
    _sessionCtrl.dispose();
    _auth.dispose();
    _viewsCtrl.dispose();
    _membersCtrl.dispose();
    _import.dispose();
    _invites.dispose();
    themeModeController.removeListener(_schedulePushSettings);
    localeController.removeListener(_schedulePushSettings);
    _settingsPushTimer?.cancel();
    _syncCtrl.closeAllDocumentSync();
    _syncCtrl.dispose();
    super.dispose();
  }

  void _syncRegistryState() {
    if (!mounted) return;
    setState(() {
      _servers = _registry.servers;
      _cloudOrigin = _registry.cloudOrigin;
    });
  }

  /// The session hub changed: rebuild — the field names above are passthrough
  /// getters, so there is nothing to copy. The one side effect lives on the
  /// WORKSPACE LIST edge: the invitee inbox rides on every new list (startup,
  /// sign-in, refresh; sign-out clears it). No push — a list refresh is the
  /// moment.
  void _syncSessionState() {
    if (!mounted) return;
    final workspaces = _sessionCtrl.workspaces;
    if (!identical(workspaces, _lastWorkspaces)) {
      _lastWorkspaces = workspaces;
      unawaited(_invites.refreshReceived());
    }
    setState(() {});
  }

  /// The last workspace list the session listener saw, so the invitee-inbox
  /// refresh above can still tell a NEW list from a same-list notification.
  List<Workspace> _lastWorkspaces = const [];

  /// The auth controller's probe answers land here — the only state it
  /// publishes as a notifier (the session itself arrives through
  /// [_syncSessionState], via the hub it adopts into).
  void _syncAuthState() {
    if (!mounted) return;
    setState(() {
      _registrationOpen = _auth.registrationOpen;
      _regOrigin = _auth.regOrigin;
    });
  }

  /// Wire the auth controller (god-class sixth step): every remote call and
  /// side effect is a closure over this State's api/prefs helpers; user-visible
  /// outcomes come back as hooks implemented here. The controller never
  /// touches a context, and the credential keys follow the registry's live
  /// origin — [ServerRegistry.connect] rebinds both in one step.
  void _initAuthController() {
    _auth = AuthController(
      session: _sessionCtrl,
      origin: () => _registry.cloudOrigin,
      servers: () => _registry.servers,
      readPref: loadPref,
      savePref: savePref,
      isMounted: () => mounted,
      register: _api.register,
      login: _api.login,
      listWorkspaces: _api.listWorkspaces,
      fetchMe: _api.fetchMe,
      refreshSession: _api.refreshSession,
      removeAvatar: _api.removeAvatar,
      httpGet: (uri) => sharedHttpClient.get(uri),
      avatarUrlFor: (version, {userId}) =>
          _avatarUrlFor(version, userId: userId),
      evictImage: (url) => NetworkImage(url).evict(),
    );
    _auth.onSessionExpired = _onSessionExpired;
    _auth
      ..registrationOpen = _registrationOpen
      ..regOrigin = _regOrigin;
    _auth.addListener(_syncAuthState);
  }

  /// The sign-in expired and was dropped by the auth controller: the half of
  /// `_endExpiredSession` that needs a context — the banner, and the fallback
  /// to the local world.
  void _onSessionExpired() {
    setState(() => _message = context.l10n.snackSessionExpired);
    _fallBackToLocalWorld();
  }

  /// Wire the cloud views controller (god-class fifth step): every remote
  /// call is a closure over this State's api/session helpers, and every
  /// user-facing or tab-state outcome comes back as a hook implemented here.
  /// The controller itself never touches a context.
  void _initViewsController() {
    _viewsCtrl = CloudViewsController(
      session: _sessionCtrl,
      createDocument: (token, workspaceId, name, {parentViewId}) => _api
          .createDocument(token, workspaceId, name, parentViewId: parentViewId),
      bootstrapDocument: (token, workspaceId, documentId) =>
          _api.bootstrapDocument(token, workspaceId, documentId),
      createFolder: (token, workspaceId, name, {parentViewId}) => _api
          .createFolder(token, workspaceId, name, parentViewId: parentViewId),
      updateView: (token, workspaceId, viewId, name, {icon}) =>
          _api.updateView(token, workspaceId, viewId, name, icon: icon),
      deleteView: _api.deleteView,
      batchTrashViews: _api.batchTrashViews,
      cloneView: _api.cloneView,
      reorderViews: _api.reorderViews,
      restoreView: _api.restoreView,
      listViewsIfChanged: _api.listViewsIfChanged,
      readPref: loadPref,
      savePref: savePref,
      mirrorPageTree: (origin, workspaces, views) =>
          _local.mirrorCloudPageTree(origin, workspaces, views),
      origin: () => requireCloudCacheOrigin(
        _api.baseUri.toString(),
        _session?.user.id,
      ),
      isMounted: () => mounted,
      openViewId: () => _selectedView?.id,
      crdtSessionDocumentId: () => _cloudSession?.documentId,
    );
    _viewsCtrl.onError = (message) {
      if (mounted) setState(() => _message = message);
    };
    _viewsCtrl.onOpenPageGone = () {
      // The reloaded tree decided the open page was inside a trashed subtree.
      setState(() {
        _selectedView = null;
        _selectedBootstrap = null;
      });
    };
    _viewsCtrl.onCloned = (report) {
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text(context.l10n.cloneDone(report.newName))),
      );
    };
    _viewsCtrl.onSelection = (selection) {
      if (!mounted) return;
      setState(() {
        // The tree itself was installed by the controller's commit step (with
        // its ETag, in one transaction). This block owns the SELECTION and
        // the body only.
        if (selection.stillCurrent) {
          _selectedView = selection.view;
          if (selection.writeBody) {
            _selectedBootstrap = selection.bootstrap;
          }
          if (selection.bootstrapError != null) {
            _message = selection.bootstrapError;
          }
        }
      });
    };
  }

  /// Wire the members controller; the legacy field name is a passthrough
  /// getter, so the listener only triggers a rebuild.
  void _initMembersController() {
    _membersCtrl = MembersController(
      listMembers: _api.listWorkspaceMembers,
      updateMember: _api.updateWorkspaceMember,
      removeMember: _api.removeWorkspaceMember,
    )..addListener(_syncMembersState);
  }

  /// The members map changed: rebuild — the share sheet and the
  /// version-author projection read it through the getter unchanged.
  void _syncMembersState() {
    if (!mounted) return;
    setState(() {});
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
      rehostImage:
          (
            token,
            workspaceId,
            documentId, {
            required blockId,
            required fileName,
            required bytes,
          }) => _api.rehostImage(
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

  /// Import progress / job id changed: rebuild — the sidebar row and its
  /// cancel button read them through the getters unchanged.
  void _syncImportState() {
    if (!mounted) return;
    setState(() {});
  }

  /// Wire the sync controller (god-class seventh step): the reconcile loop,
  /// the socket teardown family, the outbox sweep, the fault banner and the
  /// remote-seq debounce all live there; this State contributes the shell
  /// facts (tabs, sign-in, base URI, the on-device stores) as closures, and
  /// implements the user-facing hooks (banner, presence row, editor body,
  /// bootstrap refetch). The controller never touches a context.
  void _initSyncController() {
    _syncCtrl = SyncReconcileController(
      session: _sessionCtrl,
      tabs: () => _tabs,
      activeTab: () => _activeTab,
      workspaceForActiveTab: () => _workspaceForTab(_activeTab),
      activeDocumentId: () => _selectedBootstrap?.document.id,
      activeBootstrap: () => _selectedBootstrap,
      baseUri: () => _api.baseUri,
      activeIsLocal: () => _activeIsLocal,
      isMounted: () => mounted,
      isWeb: () => kIsWeb,
      deviceClientId: () => _local.deviceClientId(),
      cloudDocStore: (docId) => _local.cloudDocStore(
        requireCloudCacheOrigin(_api.baseUri.toString(), _session?.user.id),
        docId,
      ),
      openWebDocStore: (docId) =>
          openWebDocStore(requireCloudCacheOrigin(_cloudOrigin, _session?.user.id), docId),
      ensureFreshSession: () => _auth.ensureFreshSession(),
      readPref: loadPref,
      savePref: savePref,
      refreshTree: (workspaceId) => _refreshTreeFromServer(workspaceId),
      applyCloudBlocks: _applyCloudBlocks,
      loadComments: _loadComments,
      onCloudOnline: _onCloudOnline,
      onReadyReconcileUploads: (documentId) =>
          _reconcilePendingUploads(documentId),
      // Active doc only; the badge value itself is derived on read (issue #160).
      onSyncPhase: (_, _) {
        if (mounted) setState(() {});
      },
      onFaultBanner: () {
        if (!mounted) return;
        final l10n = context.l10n;
        ScaffoldMessenger.maybeOf(context)?.showMaterialBanner(
          MaterialBanner(
            content: Text(l10n.snackCloudSyncPaused),
            leading: const Icon(Icons.cloud_off_outlined),
            actions: [
              TextButton(
                onPressed: _syncCtrl.retryCloudSync,
                child: Text(l10n.commonRetry),
              ),
              TextButton(
                onPressed: _syncCtrl.clearSyncBanner,
                child: Text(l10n.snackDismiss),
              ),
            ],
          ),
        );
      },
      onHideFaultBanner: () {
        if (mounted) {
          ScaffoldMessenger.maybeOf(context)?.hideCurrentMaterialBanner();
        }
      },
      onRefreshBootstrap: (documentId) =>
          unawaited(_refreshSelectedBootstrap(documentId)),
      onPresence: (documentId, users) {
        // The controller's guard (documentId == activeDocumentId) has already
        // filtered the background tabs' rosters; this is the row write.
        setState(() => _presence = users);
      },
      presenceIsEmpty: () => _presence.isEmpty,
      onPresenceCleared: () {
        setState(() => _presence = const []);
      },
    );
  }

  /// The sign-in form's flow — a thin adapter over [AuthController.authenticate]
  /// (god-class sixth step): the controller owns the sequence and the refusal
  /// KINDS; this side translates them into localized messages, shows the
  /// register snackbar, and runs the post-sign-in loads the same _run() wrapper
  /// always did.
  Future<void> _authenticate(AuthMode mode, AuthFormValue form) {
    return _run(() async {
      final outcome = await _auth.authenticate(mode, form);
      if (!mounted) return;
      switch (outcome) {
        case AuthRegistered(:final verified):
          // Registering no longer signs you in: the address has to be confirmed
          // first, so there is no session to take. The FIRST account on an empty
          // instance is verified server-side on the spot, so telling it to go
          // check its email is two lies in one sentence: nothing was sent, and
          // nothing needs clicking.
          ScaffoldMessenger.of(context).showSnackBar(
            SnackBar(
              content: Text(
                verified
                    ? context.l10n.authFirstAccountReady
                    : context.l10n.authVerifySent,
              ),
            ),
          );
        case AuthSignedIn(:final session):
          unawaited(_syncSettingsFromCloud(session));
          unawaited(_refreshAiConfigured());
          await _loadSelectedWorkspaceMembers();
          await _loadSelectedWorkspaceViews();
        case AuthRefused(:final refusal):
          switch (refusal) {
            case AuthRefusal.badCredentials:
              throw ApiException(context.l10n.loginBadCredentials);
            case AuthRefusal.emailTaken:
              throw ApiException(context.l10n.loginEmailTaken);
            case AuthRefusal.emailNotVerified:
              if (mounted) _offerResendVerification(form.email);
              throw ApiException(context.l10n.loginEmailNotVerified);
          }
      }
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
      // Leaving the open page: commit its typing first (see _flushActiveEditor).
      if (!keepOpenPage) await _flushActiveEditor();
      if (!mounted) return;
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
    final userId = _session?.user.id;
    if (userId == null || userId.isEmpty) return;
    final cache = _local.cachedCloudPageTree(
      cloudCacheOrigin(_api.baseUri.toString(), userId),
    );
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
    // Members are unknowable offline — file the empty list through the
    // controller BEFORE the setState so the sync copy below lands with it.
    _membersCtrl.replace(workspace.id, const []);
    setState(() {
      _sessionCtrl.replaceViews(workspace.id, views);
      _selectedView = firstView;
      _selectedBootstrap = bootstrap;
      _offlineNav = true;
    });
    if (bootstrap != null) _reconcileSync();
    // An EMPTY workspace never reaches _reconcileSync (nothing to open), but
    // its sidebar still wants tree-change pings — MCP growing a first page
    // into an empty workspace is this feature's founding case. Idempotent when
    // the line above already ran.
    _syncCtrl.reconcileViewsEvents();
  }

  /// Returns the new view's id (null on failure) so the caller can drop it into
  /// inline-rename — the sidebar name becomes editable immediately, no dialog.
  Future<String?> _createDocument(String name, {String? parentViewId}) async {
    String? newId;
    await _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      final created = await _viewsCtrl.createDocument(
        session.accessToken,
        workspace.id,
        name,
        parentViewId: parentViewId,
      );
      setState(() {
        _selectedView = created.view;
        _selectedBootstrap = created.bootstrap;
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
  /// user creates pages under it. Mirrored offline via the controller's
  /// mirror step (one workspace only — see its doc).
  Future<String?> _createFolder(String name, {String? parentViewId}) async {
    String? newId;
    await _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      newId = await _viewsCtrl.createFolder(
        session.accessToken,
        workspace.id,
        name,
        parentViewId: parentViewId,
      );
    });
    return newId;
  }

  Future<String?> _createChildFolder(DocumentView parent, String name) {
    return _createFolder(name, parentViewId: parent.id);
  }

  /// Persist a new sibling order: assign evenly spaced, zero-padded positions to
  /// [orderedSiblings] (all sharing [parentViewId]) and push the ones that
  /// changed. Ordering is per-parent, so renumbering one group is self-contained.
  /// The optimistic write and the step-of-ten rule live in the controller (see
  /// CloudViewsController.reorderViews for the history: the per-row move loop
  /// and the lag it caused).
  Future<void> _reorderViews(
    String? parentViewId,
    List<DocumentView> orderedSiblings,
  ) {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      await _viewsCtrl.reorderViews(
        session.accessToken,
        workspace.id,
        parentViewId,
        orderedSiblings,
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
      await _viewsCtrl.restoreView(session.accessToken, workspace.id, view.id);
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
    // The LOCAL world tracks its open page on the local backend, NOT the
    // cloud/shared `_selectedBootstrap` (the editor renders the local
    // bootstrap too — see the `bootstrap` build site in _unifiedWorkspaceView).
    // Reading `_selectedBootstrap` here exported a stale, unrelated doc.
    final bootstrap = _localBackend.bootstrap;
    if (bootstrap == null) {
      throw ApiException(context.l10n.pageOpenFirst);
    }
    final name = title.trim().isEmpty
        ? context.l10n.untitledPage
        : title.trim();
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
    final bootstrap = _localBackend.bootstrap;
    if (bootstrap == null) {
      throw ApiException(context.l10n.pageOpenFirst);
    }
    final base = title.trim().isEmpty
        ? context.l10n.untitledPage
        : title.trim();
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
  /// Reads the LOCAL world's bootstrap off the local backend, NOT the cloud
  /// `_selectedBootstrap` — the same trap [_localExportPageHtml] documents,
  /// where reading the wrong one silently exported an unrelated, stale document.
  Future<String> _localPageMarkdownText() async {
    final bootstrap = _localBackend.bootstrap;
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
    final wsId = _localBackend._workspaceIdOfView(folder.id);
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

  // Local-world search lives on the local backend now (same store, same
  // `_localWsId` — world/local_world_backend.dart); the backlinks/graph
  // loaders below still run here because they answer for the ACTIVE world,
  // either of them.

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

  // Opening a LOCAL search hit is the local backend's job (it owns the local
  // page tree selection) — `WorldBackend.searchOps.openResult`.

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
    if (_activeIsLocal) return _local.graphLocal(_localBackend._localWsId);
    final session = _session;
    final workspace = _selectedWorkspace;
    if (session == null || workspace == null) return PageGraph.empty;
    return _api.workspaceGraph(session.accessToken, workspace.id);
  }

  Future<List<Backlink>> _loadBacklinks(String viewId) async {
    if (_activeIsLocal) {
      return _local.backlinksLocal(_localBackend._localWsId, viewId);
    }
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
      _auth.persistSession(
        updated,
      ); // keep the saved display name fresh for restart
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

  /// Whether the server at [_regOrigin] accepts new registrations, as of the
  /// last probe — synced view-model copies of [AuthController.registrationOpen]
  /// / [AuthController.regOrigin] (god-class sixth step). Null — including "we
  /// asked a server too old to answer" — means DON'T KNOW, and the registration
  /// entry then stays exactly as it was. Guessing "closed" would hide the only
  /// way into a fresh instance; guessing "open" would show a door that 403s.
  bool? _registrationOpen;

  /// Which origin [_registrationOpen] describes. Kept beside it so switching
  /// servers cannot show the previous server's answer: the getter below only
  /// trusts it while the two still agree.
  String? _regOrigin;

  bool? get _activeRegistrationOpen =>
      _regOrigin == _activeOrigin ? _registrationOpen : null;

  Future<bool> _probeServer(String origin) => _auth.probeServer(origin);

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
    final origin = _cloudOrigin;
    final picked = await pickImage();
    if (!_auth.canApplyAvatar(session, origin)) return null;
    if (picked == null) return _avatarUrlFor(session.user.avatarVersion);
    final version = await _api.setAvatar(
      session.accessToken,
      picked.bytes,
      picked.mime,
    );
    return _auth.applyAvatarVersion(session, version, requestOrigin: origin);
  }

  Future<String?> _removeAvatar() => _auth.removeAvatar(_requireSession());

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
    String? providerId,
    String? baseUrl,
    String? apiKey,
  }) async => _api.listAiModels(
    _requireSession().accessToken,
    provider: provider,
    providerId: providerId,
    baseUrl: baseUrl,
    apiKey: apiKey,
  );

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

  String _titleFromMarkdown(String markdown, String fallback) =>
      titleFromMarkdown(markdown, fallback);

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
        _sessionCtrl.replaceViews(workspace.id, [
          ..._viewsByWorkspace[workspace.id] ?? const [],
          bootstrap.view,
        ]);
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
        _sessionCtrl.replaceViews(workspace.id, [
          ..._viewsByWorkspace[workspace.id] ?? const [],
          bootstrap.view,
        ]);
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
      // The tab this load is FOR. The user can switch tabs or click another page
      // while it is in flight; writing the result to whichever tab is active at
      // the end put this page into the wrong one (2026-10-21 review U1).
      final tab = _activeTab;
      final seq = ++tab.navSeq;
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
      if (!mounted || !_tabs.contains(tab) || tab.navSeq != seq) return;
      final wasShowingThisView =
          tab.view?.id == view.id && tab.bootstrap != null;
      // Per-workspace last view (AppFlowy model): switching back to a workspace
      // reopens the page you left there.
      final wsId = _selectedWorkspace?.id;
      if (wsId != null) savePref('lastViewId:$wsId', view.id);
      setState(() {
        tab.view = view;
        // Never blank a page that is already on screen. `bootstrap` is null
        // when the server was unreachable AND the doc has no on-device mirror;
        // for a doc we are already rendering, keeping what we have beats
        // replacing it with an empty pane. The guard above catches the common
        // case, but two taps can both pass it before either finishes.
        if (mayReplaceBootstrap(
          haveNewBootstrap: bootstrap != null,
          wasShowingSameView: wasShowingThisView,
        )) {
          tab.bootstrap = bootstrap;
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
      await _viewsCtrl.setViewIcon(
        session.accessToken,
        workspace.id,
        view,
        picked,
      );
    });
  }

  // The flagged `_openViewFromHome(viewId, local:)` is gone — opening a
  // home-listed page (and the folder-reveal branch) now lives on the two world
  // backends as `ViewOps.openViewFromHome` (world_backend.dart /
  // local_world_backend.dart). The LOCAL half dispatches to
  // `_overviewFolderId` through the shell handle it holds already.

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

  // Closing the open page (the sidebar's Home row) moved onto the world
  // backends too — `ViewOps.closeOpenPage`. What stays on the shell is the
  // `_overviewFolderId` reset the caller still owns.

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
      final renamed = await _viewsCtrl.renameView(
        session.accessToken,
        targetWorkspaceId,
        view.id,
        name,
      );

      setState(() {
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
      await _viewsCtrl.deleteView(session.accessToken, workspace.id, view.id);
      if (!mounted) return;
      setState(() {
        if (_selectedView?.id == view.id) {
          _selectedView = null;
          _selectedBootstrap = null;
        }
      });
    });
  }

  /// Trash a selection (cloud) in ONE request, then reload the tree. The batch
  /// call, the reload and the "did the open page survive" decision live in the
  /// controller (CloudViewsController.deleteViews); the tab-close comes back
  /// through its onOpenPageGone hook.
  Future<void> _deleteViews(List<String> viewIds) {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      await _viewsCtrl.deleteViews(
        token: session.accessToken,
        workspaceId: workspace.id,
        viewIds: viewIds,
      );
    });
  }

  /// Duplicate [view] in place (cloud). The server copies the subtree, shares
  /// blobs, and dedupes the name; we just reload the tree to show the copy.
  /// The snackbar fires through the controller's onCloned hook.
  Future<void> _cloneView(DocumentView view) {
    final copyName = context.l10n.cloneCopyName(view.name);
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      await _viewsCtrl.cloneView(
        token: session.accessToken,
        workspaceId: workspace.id,
        viewId: view.id,
        name: copyName,
      );
    });
  }

  /// Editor writes stay on the local-origin CRDT, including cold bootstrap.
  Future<void> _applyEditorOperations(
    List<Map<String, dynamic>> operations, {
    bool trackHistory = true,
  }) async {
    if (_activeIsLocal) {
      await _local.applyOps(operations, trackHistory: trackHistory);
      return;
    }
    final documentId = _selectedBootstrap?.document.id;
    if (documentId == null) throw StateError('No active editor document');
    final replica = await _syncCtrl.readyForDocument(documentId);
    if (!mounted ||
        _activeIsLocal ||
        _selectedBootstrap?.document.id != documentId) {
      throw StateError('Editor document changed');
    }
    replica.applyLocalOps(operations, trackHistory: trackHistory);
  }

  // ── Local offline (P2-M3) ── now lives on the local backend ────────────────
  // The local world's page tree, selection, bootstrap, editor epoch and every
  // local operation moved to world/local_world_backend.dart (`_LocalWorldBackend`,
  // reached from here as `_localBackend` / `_activeBackend`), verbatim. What
  // stays below is the cross-world migration engine (§6), which reads the local
  // store too.

  // (_localSelectView / _showLocalCorruptDialog moved to the local backend.)

  // (_localRollbackDoc moved to the local backend too.)

  // (_openLocalVersionHistory moved to the local backend as well — the local
  // half of `WorldBackend.versionHistory`.)

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
        _auth.persistSession(s);
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
      await _localBackend._localDeleteWorkspace(localWs);
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
      requireCloudCacheOrigin(entry.origin, _session?.user.id),
      entry.workspace.id,
      entry.workspace.name,
    );
    if (result == null) {
      setState(() => _message = l10n.worldDetachStoreUnavailable);
      return;
    }
    setState(() {
      _localBackend._reloadLocalWorkspaces();
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
    final target = _localBackend._localWorkspaces
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
    final ordered = orderViewsParentFirst(
      views,
      idOf: (view) => view.id,
      parentOf: (view) => view.parentId,
    );

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
            mimeType: sniffImageMime(bytes),
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
      final active = _localBackend.selectedView;
      if (active != null) _local.openDoc(active.objectId);
    }
    return (cloudWorkspaceId: cloudWs.id, docCount: docCount);
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
        // the stack (see dialog_controllers.dart / docs/dev/lessons.md). Without it
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
                  signedInOrigins: _auth.signedInOrigins(),
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
                            storedName: _auth.storedDisplayName(_cloudOrigin),
                            cloudOrigin: _cloudOrigin,
                            activeOrigin: _activeOrigin,
                          ) !=
                          WorldCardMode.signIn
                      ? SignedInCard(
                          strings: SignedInCardStrings(
                            title: l10n.worldAlreadySignedInTitle,
                            body:
                                '${_auth.storedDisplayName(_cloudOrigin)} · '
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

  // (The local half of the image pipeline — upload / re-host / load / resolve —
  // moved to the local backend as `ImageOps`; so did the local view/rename ops
  // below, as `ViewOps`.)

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
          if (_pending.remove(workspace.id, documentId, entry.sha)) {
            changed = true;
          }
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
        if (_pending.remove(workspace.id, documentId, entry.sha)) {
          changed = true;
        }
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

  Future<void> _updateWorkspaceMember(
    WorkspaceMember member,
    WorkspaceRole role,
  ) {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      await _membersCtrl.update(
        session.accessToken,
        workspace.id,
        member.userId,
        role.apiValue,
      );
    });
  }

  Future<void> _removeWorkspaceMember(WorkspaceMember member) {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      await _membersCtrl.remove(
        session.accessToken,
        workspace.id,
        member.userId,
      );
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
    await _membersCtrl.load(session.accessToken, workspace.id);
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
    _auth.clearPersistedSession();
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
    _syncCtrl.closeAllDocumentSync();
    // The tree-change socket is a cloud thing too; without this it would keep
    // reconnecting with a token that logout just revoked.
    _syncCtrl.disposeViewsEvents();
    _sessionCtrl.signOut();
    _membersCtrl.clear();
    setState(() {
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
    final cache = _local.cachedCloudPageTree(
      requireCloudCacheOrigin(_api.baseUri.toString(), session.user.id),
    );
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
    return _viewsCtrl.rememberedViewOf(workspaceId);
  }

  /// The workspace-switch tree load — tree + ETag + auto-open, now in the
  /// controller (see CloudViewsController.loadWorkspaceViews for the ordering
  /// rules this preserves: the speculative bootstrap that races the tree, the
  /// 304 discipline, the superseded-response guards, and the
  /// stillCurrent/writeBody selection gate). The selection outcome lands back
  /// through the controller's onSelection hook, wired in
  /// [_initViewsController].
  Future<void> _loadSelectedWorkspaceViews() async {
    final session = _session;
    final workspace = _selectedWorkspace;
    if (session == null || workspace == null) {
      return;
    }
    await _viewsCtrl.loadWorkspaceViews(session.accessToken, workspace.id);
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
    final cache = _local.cachedCloudPageTree(
      requireCloudCacheOrigin(_api.baseUri.toString(), session.user.id),
    );
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
    _syncCtrl.reconcileViewsEvents();
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

  /// After coming online, drain the durable outbox of every cloud document
  /// that has un-pushed edits — not only the one currently open. The sweep
  /// loop lives in the sync controller (it owns the guard flag and reads the
  /// hub's trees); this State-level adapter keeps the call sites unchanged.
  Future<void> _sweepPendingOutboxes() => _syncCtrl.sweepPendingOutboxes();

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
    final origin = requireCloudCacheOrigin(
      _api.baseUri.toString(),
      _session?.user.id,
    );
    final data =
        _local.openCloudDocMirror(origin, view.objectId) ??
        await openWebDocMirror(origin, view.objectId);
    if (data == null) return null;
    return _localBackend._localBootstrapFrom(
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
      await _localBackend._localSelectWorkspace(entry.workspace);
    } else {
      await _selectWorkspace(entry.workspace);
    }
  }

  // Per-entry workspace actions (P3c): the switcher lists BOTH worlds, so row
  // actions must dispatch on the ROW's origin, not the active one.
  Future<void> _renameEntry(WorkspaceEntry e, String name) => e.isLocal
      ? _localBackend._localRenameWorkspace(e.workspace, name)
      : _renameWorkspace(e.workspace, name);

  Future<void> _deleteEntry(WorkspaceEntry e) => e.isLocal
      ? _localBackend._localDeleteWorkspace(e.workspace)
      : _deleteWorkspace(e.workspace);

  Future<Uint8List> _exportEntryZip(WorkspaceEntry e) => e.isLocal
      ? _localBackend._localExportWorkspaceZip(e.workspace.id)
      : _exportWorkspaceZip(e.workspace.id);

  /// Persist a new workspace order for ONE world (the switcher only reorders
  /// within the connected world; cloud and local are separate position
  /// spaces). [ordered] is that world's full list in the intended order.
  Future<void> _reorderWorkspaceEntries(List<WorkspaceEntry> ordered) async {
    if (ordered.length < 2) return;
    final ids = [for (final e in ordered) e.workspace.id];
    if (ordered.first.isLocal) {
      _local.reorderWorkspaces(ids);
      if (mounted) setState(_localBackend._reloadLocalWorkspaces);
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
      final wsId = _localBackend._workspaceIdOfView(folder.id);
      final ws = _localBackend._localWorkspaces
          .where((w) => w.id == wsId)
          .firstOrNull;
      if (ws == null) return;
      await _localBackend._localImportVaultTree(
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
      ? _localBackend._localImportVaultTree(
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
      final session = await _auth.signIn(creds.$2);
      if (!mounted) return;
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
      for (final w in _localBackend.workspaces)
        WorkspaceEntry(origin: 'local', workspace: w, role: 'owner'),
    ];
  }

  /// The selected workspace as a unified entry (null when nothing is selected).
  WorkspaceEntry? get _selectedEntry {
    if (_activeIsLocal) {
      final w = _localBackend.selectedWorkspace;
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
  /// meaningful while a cloud session is live (gated in _unifiedWorkspaceView);
  /// derived from the ACTIVE document's last phase, re-read on tab switches
  /// (issue #160).
  SyncPhase get _syncPhase =>
      _syncCtrl.phaseOf(_selectedBootstrap?.document.id) ?? SyncPhase.offline;

  /// Comment threads on the open cloud document, as the server resolved them
  /// (each carries offsets against the CURRENT text, or a null anchor when
  /// orphaned). Re-fetched on open — never cached across documents, and never
  /// re-derived locally: the anchor lives in the CRDT and only the server reads
  /// it (docs/features/comments-plan.md).
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
    // One WorldBackend for the ACTIVE world — local or cloud. Every prop that
    // used to be an inline `local ? ... : ...` dispatch now reads through it
    // (world/world_backend.dart; the interface's doc comments kept each
    // polarity's rationale). What stays here is genuinely cross-world: the
    // session/account/spinner, prefs, entries (which span both worlds), and
    // the §6 migrate/detach engine.
    final b = _activeBackend;
    final viewOps = b.viewOps;
    return WorkspaceView(
      session: session,
      // Avatar URLs are composed from this (ui/avatar_url.dart); the account
      // tile and the member list both live under this widget.
      apiBase: _api.baseUri,
      onChangeAvatar: b.changeAvatar,
      onRemoveAvatar: b.removeAvatar,
      entries: _workspaceEntries,
      activeOrigin: _activeOrigin,
      syncPhase: b.syncPhase,
      commentThreads: b.commentThreads,
      commentHighlights: b.commentHighlights,
      // Comments are a cloud feature — the local backend's addComment is null,
      // and with no session at all even the cloud world hides the entry.
      onAddComment: session == null ? null : b.addComment,
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
      isBusy: b.isBusy,
      onRefresh: b.onRefresh,
      // Account-level action: dispatches on the SESSION, not the active world —
      // a signed-in user browsing the local world must still be able to sign
      // out (the tile only shows Sign out when session != null; _signOut is
      // safe regardless).
      onSignOut: () {
        if (!_isBusy) _signOut();
      },
      workspaces: b.workspaces,
      selectedWorkspace: b.selectedWorkspace,
      members: b.members,
      views: b.views,
      treePending: b.treePending,
      selectedView: b.selectedView,
      selectedBootstrap: b.bootstrap,
      // Cloud only — the local world has no tab model (the backend's tab
      // callbacks are null there, hiding the strip).
      tabs: b.tabs,
      activeTabIndex: b.activeTabIndex,
      onSelectTab: b.selectTab,
      onCloseTab: b.closeTab,
      onOpenInNewTab: b.openViewInNewTab,
      onOpenInNewTabById: b.openViewInNewTabById,
      onNewTabPage: b.newTabPage,
      onSelectWorkspaceById: b.searchOps.selectWorkspaceById,
      onSearchAllWorkspaces: b.searchOps.searchAll,
      onOpenSearchHit: b.searchOps.openHit,
      searchRecents: buildWorkspaceRecents(
        context,
        workspaceId: b.selectedWorkspace?.id,
        workspaceName: b.selectedWorkspace?.name ?? '',
        views: b.views,
      ),
      presence: b.presence,
      message: _message,
      importProgress: _importProgress,
      // Only offer it while an import is actually in flight.
      onCancelImport: _importJobId == null ? null : _cancelImport,
      onSelectWorkspace: b.workspaceOps.select,
      onCreateWorkspace: b.workspaceOps.create,
      onRenameWorkspace: b.workspaceOps.rename,
      onDeleteWorkspace: b.workspaceOps.delete,
      onCreateDocument: viewOps.createDocument,
      onCreateChildDocument: viewOps.createChildDocument,
      onCreateFolder: viewOps.createFolder,
      onCreateChildFolder: viewOps.createChildFolder,
      onExportFolderZip: viewOps.exportFolderZip,
      onReorderViews: viewOps.reorder,
      onLoadTrash: viewOps.loadTrash,
      onRestoreView: viewOps.restore,
      onPurgeView: viewOps.purge,
      onPurgeAllTrash: viewOps.purgeAll,
      onSelectView: viewOps.select,
      onRenameView: viewOps.rename,
      onSetViewIcon: viewOps.setIcon,
      // Home spans every workspace of the ACTIVE world (local and cloud stay
      // separate — see the world switcher), so only the shell can build it. The
      // local world keeps one flat view list rather than a per-workspace map,
      // so it contributes just its selected workspace (see backends'
      // viewsByWorkspace).
      homePane: buildHomePane(
        context,
        viewsByWorkspace: b.viewsByWorkspace,
        workspaceNames: {for (final w in b.workspaces) w.id: w.name},
        onCreatePage: () =>
            unawaited(viewOps.createDocument(context.l10n.newPage)),
        onOpenView: viewOps.openViewFromHome,
      ),
      onOpenHome: () {
        setState(() => _overviewFolderId = null);
        viewOps.closeOpenPage();
      },
      // Only built while a folder is being browsed — otherwise home shows.
      overviewPane: _overviewFolderId == null
          ? null
          : buildOverviewPane(
              context,
              views: b.views,
              folderId: _overviewFolderId,
              mode: _overviewMode,
              onModeChanged: (mode) => setState(() => _overviewMode = mode),
              onOpen: viewOps.openViewFromHome,
              onEnterFolder: (id) => setState(() => _overviewFolderId = id),
              onCreatePage: () =>
                  unawaited(viewOps.createDocument(context.l10n.newPage)),
            ),
      onDeleteView: viewOps.delete,
      onDeleteViews: viewOps.bulkDelete,
      onCloneView: viewOps.clone,
      onApplyOperations:
          _applyEditorOperations, // P3d: one entry, self-dispatches
      onApplyHistory: _editorHistoryForDocument(),
      onApplyMaintenanceOperations: (ops) =>
          _applyEditorOperations(ops, trackHistory: false),
      onEditorFault: _onEditorFault,
      onUploadImage: b.imageOps.upload,
      onImportImageUrl: b.imageOps.importUrl,
      onLoadImageBytes: b.imageOps.loadBytes,
      onResolveImageUrls: b.imageOps.resolveUrls,
      onAiStream: b.aiOps.stream,
      onAiNewPage: b.aiOps.newPage,
      onAiPreviewEdits: b.aiOps.previewEdits,
      onAiCurrentPage: b.aiOps.currentPage,
      onAiNewWorkspace: b.aiOps.newWorkspace,
      // Null, not a no-op — absence is what hides the AI tab, the same rule
      // onLoadTokens/onUpdateProfile below already follow: AI settings live on
      // the server, so the local world's callbacks stay null and the Settings
      // AI tab renders nothing instead of a form whose Save went nowhere.
      onLoadAiSettings: b.loadAiSettings,
      onLoadImportHistory: b.loadImportHistory,
      onListAiModels: b.listAiModels,
      onSaveAiSettings: b.saveAiSettings,
      onDeleteAiProvider: b.deleteAiProvider,
      onLoadTokens: b.loadTokens,
      onCreateToken: b.createToken,
      onRevokeToken: b.revokeToken,
      // Only ever the cloud account: 本地模式 has none, and the Account tab is
      // absent there rather than filled with a fiction. (An earlier attempt put
      // '本地工作区' here — which surfaced as an editable Display name for an
      // account that does not exist, above a Save button that did nothing.)
      userName: _session?.user.displayName ?? '',
      userEmail: _session?.user.email ?? '',
      // Null, not a no-op: absence is what hides the tab. See onLoadTokens.
      onUpdateProfile: b.updateProfile,
      onChangePassword: b.changePassword,
      onDeleteAccount: b.deleteAccount,
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
      showAi: b.showAi,
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
      // The null-both-ways polarity (web local cannot search, desktop local
      // searches the on-device index, cloud searches the server) lives on
      // `SearchOps.search` — «cannot search» vs «nothing matched» stays
      // distinguishable there.
      onSearch: b.searchOps.search,
      onOpenSearchResult: b.searchOps.openResult,
      // Both worlds now answer this — `_loadBacklinks` picks the on-device index
      // in 本地模式 and the endpoint in the cloud. It used to be `local ? null`,
      // which hid the panel outright: a locally-linked page looked unlinked.
      onLoadBacklinks: _loadBacklinks,
      onLoadGraph: _loadGraph,
      onExportPage: b.exportOps.page,
      onCopyPageMarkdown: b.exportOps.copyMarkdown,
      onExportPageHtml: b.exportOps.pageHtml,
      // PDF is world-agnostic: both worlds produce HTML, then the same
      // WebView2 headless print turns it into PDF bytes (desktop only; web
      // stub returns null and the menu item is hidden there).
      onExportPagePdf: _local.htmlToPdf,
      onImportMarkdown: b.exportOps.importMarkdown,
      onExportWorkspaceZip: b.exportOps.workspaceZip,
      onExportAllWorkspaces: b.exportAllWorkspaces,
      onLoadExportStats: b.exportStats,
      // Desktop only: web has no on-device store to measure.
      onLoadCacheStats: _local.available && _session != null
          ? () => _local.cacheStats(
              servers: _servers,
              accountId: _session!.user.id,
            )
          : null,
      onClearMirrorCache: _local.available && _session != null
          ? () => _local.clearMirrorCache(
              servers: _servers,
              accountId: _session!.user.id,
            )
          : null,
      onLoadQuarantinedCaches: _local.available
          ? () async => _local.quarantinedWorkspaces(servers: _servers)
          : null,
      onExportQuarantinedCache: _local.available
          ? (origin, workspaceId) async =>
              _local.exportWorkspaceZip(workspaceId, origin: origin)
          : null,
      onImportWorkspaceZip: b.importWorkspaceZip,
      onImportWorkspaceTreeInto: b.exportOps.importTree,
      // Null, not a no-op: a local workspace has no members at all. The no-op
      // form still rendered and still accepted an email, because local entries
      // carry role 'owner' so canManage was true — you could invite someone and
      // nothing whatsoever happened.
      invitations: b.invitations,
      onLoadWorkspaceUsage: b.loadWorkspaceUsage,
      onUpdateMember: b.updateMember,
      onRemoveMember: b.removeMember,
      onRestoreCheckpoint: b.restoreCheckpoint,
      // Both worlds now have real version history — local via the on-device
      // doc_version timeline (FFI), cloud via the yrs endpoints.
      onVersionHistory: b.versionHistory,
      onShare: b.share,
      // Cross-workspace move/copy — cloud-only (the destination is another
      // cloud workspace; local worlds don't participate). Null hides both
      // row-menu entries in 本地模式.
      onTransfer: b.transfer,
      // P3f: both live on the workspace ROW's menu, dispatching per entry —
      // null on web (no local world / no on-device store).
      onMigrateEntry: _local.available ? _migrateEntry : null,
      onDetachEntry: _local.available ? _detachEntry : null,
      editorEpoch: b.editorEpoch,
      onCursorChanged: b.cursorChanged,
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

/// Test-only probe, fired at the head of every
/// [_WorkspaceShellState._flushActiveEditor] call: the flush-before-switch
/// ordering (2026-10-21 review P1-2) is otherwise invisible to widget tests,
/// which cannot reach the shell's private state. Tests assign it, then clear
/// it in tearDown.
@visibleForTesting
void Function()? debugProbeShellFlush;
