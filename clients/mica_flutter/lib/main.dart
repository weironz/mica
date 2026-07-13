import 'dart:convert';
import 'dart:async';

import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:http/http.dart' as http;

import 'editor/clipboard_copy.dart';
import 'cloud/cloud_sync.dart';
import 'cloud/doc_store_platform.dart';
import 'cloud/pending_uploads.dart';
import 'cloud/workspace_migration.dart';
import 'local/local_offline.dart';
import 'web/yjs_probe.dart';
import 'editor/model.dart' show kMonoFont;
import 'editor/editor.dart';
import 'editor/image_actions.dart';
import 'editor/pick_file.dart';
import 'widgets/mica_logo.dart';
import 'cjk_fonts.dart';
import 'prefs.dart';
import 'updater.dart';
import 'window_setup.dart';
import 'upload/zip_writer.dart';
import 'api/client.dart';
import 'api/models.dart';
import 'api/sync_client.dart';

// The API/data layer lives in lib/api/*.dart; re-export it so existing
// `import 'main.dart'` users (tests, tooling) still see these symbols.
export 'api/client.dart';
export 'api/models.dart';
export 'api/sync_client.dart';

part 'ui/dialogs.dart';
part 'ui/widgets.dart';

/// Dev convenience: when true, the app signs in automatically on startup so you
/// don't have to log in on every reload. Turn it off for a real login screen:
/// `flutter run --dart-define=MICA_DEV_AUTOLOGIN=false`.
const bool kDevAutoLogin = bool.fromEnvironment(
  'MICA_DEV_AUTOLOGIN',
  defaultValue: true,
);
const String kDevEmail = String.fromEnvironment(
  'MICA_DEV_EMAIL',
  defaultValue: 'demo@mica.dev',
);
const String kDevPassword = String.fromEnvironment(
  'MICA_DEV_PASSWORD',
  defaultValue: 'password123',
);

/// The official hosted Mica instance (see docs/deploy.md).
const String kMicaCloudUrl = 'https://mica.cloudcele.com';

/// App version, shown in the About dialog. Keep in sync with `pubspec.yaml`
/// (`version:`) and `crates/api-server/Cargo.toml` on each release.
const String kAppVersion = '0.4.0';

/// The `exp` (expiry) claim of a JWT as a UTC time, or null if it can't be
/// parsed. Used to cheaply reject an expired persisted token on startup before
/// hitting the network.
DateTime? jwtExpiry(String token) {
  try {
    final parts = token.split('.');
    if (parts.length != 3) return null;
    var payload = parts[1].replaceAll('-', '+').replaceAll('_', '/');
    payload = payload.padRight((payload.length + 3) ~/ 4 * 4, '=');
    final map =
        jsonDecode(utf8.decode(base64.decode(payload))) as Map<String, dynamic>;
    final exp = map['exp'];
    if (exp is! int) return null;
    return DateTime.fromMillisecondsSinceEpoch(exp * 1000, isUtc: true);
  } catch (_) {
    return null;
  }
}

/// One-time migration (P3c-2) of the legacy world-switch prefs
/// (`serverMode`/`serverUrl`, the pre-P3 ServerMode model) into the dissolved
/// model: which cloud server is configured ([cloudOrigin]) and which world
/// starts active ([activeOrigin], `'local'` or the cloud origin). Pure — no
/// I/O — so it is unit-testable. Semantics carried over from the old resolve():
/// legacy `cloud`/`self` fold into a URL; a desktop fresh install is
/// local-first UNLESS the user had signed in / set a URL before (they were
/// online users — don't strand them); web is always cloud-active.
@visibleForTesting
({String cloudOrigin, String activeOrigin}) resolveLegacyCloudSetup({
  required String? savedMode,
  required String savedUrl,
  required String authToken,
  required bool isWeb,
}) {
  // Everything downstream keys off Dart-normalized URL strings
  // (_api.baseUri.toString(): lowercased host, default ports stripped) — so the
  // migration MUST emit the same normal form, or a legacy raw URL (mixed-case
  // host, explicit :443) would file the auth token under a key nothing ever
  // reads again (= silent sign-out on the first Settings save).
  String normalize(String url) => Uri.tryParse(url)?.toString() ?? url;
  final onlineUrl = normalize(
    savedUrl.isEmpty ? ApiClient.defaultBaseUri().toString() : savedUrl,
  );
  switch (savedMode) {
    case 'local':
      // Keep the user's configured server (they just weren't ACTIVE on it) so
      // any stale token files under the RIGHT origin — hardcoding Mica Cloud
      // here would send a self-hosted token to the wrong server on restore.
      return (
        cloudOrigin: savedUrl.isEmpty ? kMicaCloudUrl : onlineUrl,
        activeOrigin: 'local',
      );
    case 'cloud': // legacy: the fixed Mica Cloud preset is now just a URL
      return (cloudOrigin: kMicaCloudUrl, activeOrigin: kMicaCloudUrl);
    case 'online':
    case 'self': // legacy self-hosted → same thing, keep its URL
      return (cloudOrigin: onlineUrl, activeOrigin: onlineUrl);
    default:
      final usedOnlineBefore = authToken.isNotEmpty || savedUrl.isNotEmpty;
      return (isWeb || usedOnlineBefore)
          ? (cloudOrigin: onlineUrl, activeOrigin: onlineUrl)
          : (cloudOrigin: kMicaCloudUrl, activeOrigin: 'local');
  }
}

void main() async {
  WidgetsFlutterBinding.ensureInitialized();
  // Desktop: restore window size/position + enforce a min size before the first
  // frame (no-op on web/mobile). Awaited so the window is ready before runApp.
  await initDesktopWindow();
  // Suppress the browser's native right-click menu so the editor can show its
  // own (e.g. image actions) on web.
  if (kIsWeb) BrowserContextMenu.disableContextMenu();
  // Web: register the yjs CRDT self-test hook (no-op off web). P2-M4 W1.
  registerYjsSelfTest();
  _warmUpFonts();
  runApp(const MicaApp());
}

/// Flutter Web doesn't bundle CJK fonts — the engine downloads a Noto fallback
/// on first use, which makes the custom-painted editor briefly show ".notdef"
/// boxes. Kick that download off at startup (during login/loading) so the font
/// is cached before any document renders.
void _warmUpFonts() {
  const samples = ['中文字体预热示例 ABCabc 0123 ，。！', '繁體字預熱 測試'];
  for (final text in samples) {
    final painter = TextPainter(
      text: TextSpan(text: text, style: const TextStyle(fontSize: 16)),
      textDirection: TextDirection.ltr,
    )..layout();
    painter.dispose();
  }
  // Also warm the icon font used by the editor's painted toolbars.
  for (final icon in [Icons.content_copy, Icons.wrap_text, Icons.add]) {
    final painter = TextPainter(
      text: TextSpan(
        text: String.fromCharCode(icon.codePoint),
        style: TextStyle(
          fontFamily: icon.fontFamily,
          package: icon.fontPackage,
          fontSize: 16,
        ),
      ),
      textDirection: TextDirection.ltr,
    )..layout();
    painter.dispose();
  }
}

class MicaApp extends StatelessWidget {
  const MicaApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Mica',
      debugShowCheckedModeBanner: false,
      theme: ThemeData(
        colorScheme: ColorScheme.fromSeed(
          seedColor: const Color(0xFF2563EB),
          brightness: Brightness.light,
        ),
        scaffoldBackgroundColor: const Color(0xFFF8FAFC),
        useMaterial3: true,
        // Crisp system CJK fonts on desktop (Windows 微软雅黑 / macOS 苹方 /
        // Linux Noto CJK); the bundled font is the tail + web's only option.
        fontFamilyFallback: cjkFontFallback,
      ),
      home: const WorkspaceShell(),
    );
  }
}

class WorkspaceShell extends StatefulWidget {
  const WorkspaceShell({super.key});

  @override
  State<WorkspaceShell> createState() => _WorkspaceShellState();
}

class _WorkspaceShellState extends State<WorkspaceShell> {
  final ApiClient _api = ApiClient();

  /// Which backend we talk to (cloud / self-hosted / local-offline). Loaded
  /// from prefs in [_loadPrefs] and applied to [_api] before any request.
  /// The configured cloud server's origin URL (P3c-2). Always set (defaults to
  /// Mica Cloud); whether the user is signed in to it is a separate question
  /// (the per-origin auth prefs). There is no "mode" anymore — the local world
  /// always exists alongside.
  late String _cloudOrigin;

  AuthSession? _session;
  List<Workspace> _workspaces = const [];
  Map<String, List<WorkspaceMember>> _membersByWorkspace = const {};
  Map<String, List<DocumentView>> _viewsByWorkspace = const {};
  Workspace? _selectedWorkspace;
  DocumentView? _selectedView;
  DocumentBootstrap? _selectedBootstrap;
  String? _selectedMarkdown;
  String? _message;
  bool _isBusy = false;
  // True while the cloud nav was rebuilt from the on-device mirror because the
  // server was unreachable (P1c). Roles are forced read-only until the server is
  // reached again; [_recoverOnlineNav] then refetches the authoritative nav.
  bool _offlineNav = false;

  // Editor appearance (in-memory; applied live to the editor).
  EditorAppearance _appearance = const EditorAppearance();
  double _pageWidth = 1160;
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

  DocumentSyncClient? _sync;
  List<PresenceUser> _presence = const [];
  Timer? _syncRefetchTimer;

  // --- Cloud yrs CRDT sync (P2-M4.5c, desktop only) ---
  // When the server speaks the yrs sync protocol (M4.4+), the desktop cloud
  // editor edits a CRDT replica instead of POSTing block ops: edits push yrs
  // diffs, remote updates merge + reconcile. It activates only once bootstrap
  // succeeds (`isReady`), so against an older server the app falls back to the
  // op/REST path transparently. `DocumentSyncClient` stays up for presence.
  CloudSyncSession? _cloudSession;
  BigInt? _deviceClientId;

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
    _loadPrefs();
    // P3c: both worlds live side by side — the local store always opens (it
    // backs local workspaces AND the cloud mirrors), and a cloud session is
    // restored when one was persisted. The active world comes from the
    // persisted `activeOrigin` (written by the P3c-2 legacy migration in
    // _loadPrefs on first run).
    _activeOrigin = loadPref('activeOrigin') ??
        (_local.available ? 'local' : _cloudOrigin);
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
  Future<void> _connectCloudServer(String url) async {
    final parsed = Uri.tryParse(url.trim());
    if (parsed == null || !parsed.hasScheme || parsed.host.isEmpty) return;
    final normalized = parsed.toString();
    if (normalized == _cloudOrigin) return;
    _disconnectCloudSession(); // keep the old origin's stored credentials
    setState(() => _cloudOrigin = normalized);
    savePref('cloudOrigin', normalized);
    _api.baseUri = parsed;
    await _restoreSession(); // the new origin may already have stored creds
    if (mounted && _session != null && _activeOrigin != _cloudOrigin) {
      setState(() => _activeOrigin = _cloudOrigin);
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
    _cloudOrigin = cloudOrigin;
    final base = Uri.tryParse(_cloudOrigin);
    if (base != null) {
      _api.baseUri = base;
    }
    final fontScale = double.tryParse(loadPref('fontScale') ?? '');
    final fontFamily = loadPref('fontFamily');
    _appearance = EditorAppearance(
      fontScale: (fontScale ?? 1.0).clamp(0.85, 1.4),
      fontFamily: (fontFamily == null || fontFamily.isEmpty)
          ? null
          : fontFamily,
    );
    // Ceiling matches the Settings slider's max (the editor measures the real
    // realizable width and clamps the thumb; this just keeps a wide saved value
    // from being trimmed on reload).
    _pageWidth = (double.tryParse(loadPref('pageWidth') ?? '') ?? 1160).clamp(
      640,
      2400,
    );
    _reHostImages = loadPref('reHostImages') != 'false';
    _showFormatBar = loadPref('showFormatBar') == 'true';
    _showPageTitle = loadPref('showPageTitle') != 'false';
    _aiEnabled = loadPref('aiEnabled') == 'true';
    if (!kIsWeb) _pending = PendingUploads.fromJson(loadPref('pendingBlobUploads'));
  }

  void _savePrefs() {
    savePref('fontScale', _appearance.fontScale.toString());
    savePref('fontFamily', _appearance.fontFamily ?? '');
    savePref('pageWidth', _pageWidth.toString());
    savePref('reHostImages', _reHostImages.toString());
    savePref('showFormatBar', _showFormatBar.toString());
    savePref('showPageTitle', _showPageTitle.toString());
    savePref('aiEnabled', _aiEnabled.toString());
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
  void _persistSession(AuthSession session) {
    savePref('authToken:$_cloudOrigin', session.accessToken);
    savePref('authUser:$_cloudOrigin', jsonEncode(session.user.toJson()));
  }

  void _clearPersistedSession() {
    savePref('authToken:$_cloudOrigin', '');
    savePref('authUser:$_cloudOrigin', '');
    // Also clear the legacy single-key copies so an explicit sign-out can't be
    // resurrected by a future migration re-run.
    savePref('authToken', '');
    savePref('authUser', '');
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
    final token = loadPref('authToken:$_cloudOrigin');
    final userJson = loadPref('authUser:$_cloudOrigin');
    if (token == null ||
        token.isEmpty ||
        userJson == null ||
        userJson.isEmpty) {
      return;
    }
    final exp = jwtExpiry(token);
    if (exp != null && !exp.isAfter(DateTime.now().toUtc())) {
      _clearPersistedSession();
      _fallBackToLocalWorld();
      return;
    }
    final AuthSession session;
    try {
      session = AuthSession(
        accessToken: token,
        user: User.fromJson(jsonDecode(userJson) as Map<String, dynamic>),
      );
    } catch (_) {
      _clearPersistedSession();
      return;
    }
    try {
      final workspaces = await _api.listWorkspaces(token);
      if (!mounted) return;
      setState(() {
        _session = session;
        _workspaces = workspaces;
        _selectedWorkspace = workspaces.firstOrNull;
      });
      unawaited(_refreshAiConfigured());
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
      if (error.toString().toLowerCase().contains('unauthorized')) {
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

  Future<void> _run(Future<void> Function() action) async {
    setState(() {
      _isBusy = true;
      _message = null;
    });

    try {
      await action();
      _reconcileSync();
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

  /// Open, switch, or close the document WebSocket so it always tracks the
  /// currently selected document.
  void _reconcileSync() {
    final session = _session;
    final workspace = _selectedWorkspace;
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
        if (mounted) {
          setState(() => _presence = users);
        }
      },
    );
    _sync = sync;
    setState(() => _presence = const []);
    sync.connect();

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
    if (_selectedBootstrap?.document.id != documentId || _sync?.documentId != documentId) {
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
      uri: documentSocketUri(
        _api.baseUri,
        workspace.id,
        documentId,
        session.accessToken,
      ),
      clientId: clientId,
      onReady: (_, _) {
        _clearSyncBanner(); // B3: a fresh bootstrap means sync recovered.
        _applyCloudBlocks(documentId);
        // Now online for this doc: drain any images inserted offline (§7.1).
        unawaited(_reconcilePendingUploads(documentId));
      },
      onRemoteBlocks: (_) => _applyCloudBlocks(documentId),
      onFault: (reason, count) => _onCloudSyncFault(documentId, reason, count),
      // Reaching the server means we're back online — leave the P1c offline-nav
      // fallback (refetch the authoritative workspace list / real roles).
      onServerConnected: _recoverOnlineNav,
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
      _selectedMarkdown = null;
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
    debugPrint('[cloud-sync] integrity fault ($reason) on $documentId — #$count');
    // The session auto-heals (capped re-bootstrap) for the first few consecutive
    // faults; past that it's genuinely stuck (B3). Surface it once so the user
    // knows edits may not be reaching the cloud — with a one-tap retry — instead
    // of failing silently (red line #1).
    if (count <= 3 || _syncBannerShown || !mounted) return;
    _syncBannerShown = true;
    ScaffoldMessenger.maybeOf(context)?.showMaterialBanner(
      MaterialBanner(
        content: const Text(
          '云同步已暂停，最近的编辑可能还没保存到云端。请重试或刷新页面。',
        ),
        leading: const Icon(Icons.cloud_off_outlined),
        actions: [
          TextButton(onPressed: _retryCloudSync, child: const Text('重试')),
          TextButton(onPressed: _clearSyncBanner, child: const Text('忽略')),
        ],
      ),
    );
  }

  /// Clear the sync-paused banner (B3) — recovery succeeded, doc switched, or the
  /// user dismissed it.
  void _clearSyncBanner() {
    if (!_syncBannerShown) return;
    _syncBannerShown = false;
    if (mounted) ScaffoldMessenger.maybeOf(context)?.hideCurrentMaterialBanner();
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
        _selectedMarkdown = null;
      });
    } catch (_) {
      // Transient sync refetch failures are non-fatal; the next update retries.
    }
  }

  @override
  void dispose() {
    _closeDocumentSync();
    super.dispose();
  }

  Future<void> _authenticate(AuthMode mode, AuthFormValue form) {
    return _run(() async {
      final session = mode == AuthMode.register
          ? await _api.register(form)
          : await _api.login(form);
      final workspaces = await _api.listWorkspaces(session.accessToken);

      setState(() {
        _session = session;
        _workspaces = workspaces;
        _selectedWorkspace = workspaces.firstOrNull;
      });
      _persistSession(session);

      unawaited(_refreshAiConfigured());
      await _loadSelectedWorkspaceMembers();
      await _loadSelectedWorkspaceViews();
    });
  }

  Future<void> _refreshWorkspaces() {
    return _run(() async {
      final session = _requireSession();
      final workspaces = await _api.listWorkspaces(session.accessToken);
      setState(() {
        _workspaces = workspaces;
        _selectedWorkspace = _selectedWorkspace == null
            ? workspaces.firstOrNull
            : workspaces
                  .where((workspace) => workspace.id == _selectedWorkspace!.id)
                  .firstOrNull;
      });

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
      setState(() {
        _workspaces = [..._workspaces, workspace];
        _selectedWorkspace = workspace;
        _viewsByWorkspace = {
          ..._viewsByWorkspace,
          workspace.id: [created.view],
        };
        _selectedView = created.view;
        _selectedBootstrap = bootstrap;
        _selectedMarkdown = null;
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
      setState(() {
        _workspaces = _workspaces
            .map((item) => item.id == renamed.id ? renamed : item)
            .toList();
        _selectedWorkspace = renamed;
      });
    });
  }

  Future<void> _deleteWorkspace(Workspace workspace) {
    return _run(() async {
      final session = _requireSession();
      await _api.deleteWorkspace(session.accessToken, workspace.id);
      final remaining = _workspaces
          .where((item) => item.id != workspace.id)
          .toList();
      final wasSelected = _selectedWorkspace?.id == workspace.id;
      setState(() {
        _workspaces = remaining;
        _viewsByWorkspace = {..._viewsByWorkspace}..remove(workspace.id);
        if (wasSelected) {
          _selectedWorkspace = remaining.isNotEmpty ? remaining.first : null;
          _selectedView = null;
          _selectedBootstrap = null;
          _selectedMarkdown = null;
        }
      });
      if (wasSelected && _selectedWorkspace != null) {
        await _loadSelectedWorkspaceMembers();
        await _loadSelectedWorkspaceViews();
      }
    });
  }

  Future<void> _selectWorkspace(Workspace workspace) {
    return _run(() async {
      setState(() {
        _selectedWorkspace = workspace;
        _selectedView = null;
        _selectedBootstrap = null;
        _selectedMarkdown = null;
      });
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
        await _loadSelectedWorkspaceMembers();
        await _loadSelectedWorkspaceViews();
      } on ApiException {
        rethrow;
      } catch (_) {
        await _openWorkspaceFromMirror(workspace);
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
    final bootstrap =
        firstView == null ? null : await _offlineCloudBootstrap(firstView);
    if (!mounted) return;
    setState(() {
      _viewsByWorkspace = {..._viewsByWorkspace, workspace.id: views};
      _membersByWorkspace = {..._membersByWorkspace, workspace.id: const []};
      _selectedView = firstView;
      _selectedBootstrap = bootstrap;
      _selectedMarkdown = null;
      _offlineNav = true;
    });
    if (bootstrap != null) _reconcileSync();
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
        _viewsByWorkspace = {
          ..._viewsByWorkspace,
          workspace.id: [...views, created.view],
        };
        _selectedView = created.view;
        _selectedBootstrap = bootstrap;
        _selectedMarkdown = null;
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
        _viewsByWorkspace = {
          ..._viewsByWorkspace,
          workspace.id: [...views, view],
        };
      });
      newId = view.id;
      _cacheCloudPageTree();
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
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      String pad(int n) => n.toString().padLeft(10, '0');

      final moved = <DocumentView>[];
      for (var i = 0; i < orderedSiblings.length; i++) {
        final desired = pad((i + 1) * 10);
        final view = orderedSiblings[i];
        if (view.position != desired || view.parentViewId != parentViewId) {
          moved.add(
            await _api.moveView(
              session.accessToken,
              workspace.id,
              view.id,
              parentViewId: parentViewId,
              position: desired,
            ),
          );
        }
      }
      if (moved.isEmpty) {
        return;
      }
      setState(() {
        final views = [...?_viewsByWorkspace[workspace.id]];
        for (final m in moved) {
          final idx = views.indexWhere((v) => v.id == m.id);
          if (idx >= 0) {
            views[idx] = m;
          }
        }
        _viewsByWorkspace = {..._viewsByWorkspace, workspace.id: views};
      });
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
        _viewsByWorkspace = {..._viewsByWorkspace, workspace.id: views};
      });
    });
  }

  Future<void> _purgeView(DocumentView view) {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      await _api.purgeView(session.accessToken, workspace.id, view.id);
    });
  }

  // ---------------------------------------------------------------------------
  // AI
  // ---------------------------------------------------------------------------

  Stream<String> _aiStream(String prompt, {String? system}) {
    final session = _session;
    if (session == null) {
      return Stream<String>.error(const ApiException('Not signed in.'));
    }
    return _api.aiStream(session.accessToken, prompt, system: system);
  }

  Future<String> _exportPageMarkdown() async {
    final session = _requireSession();
    final workspace = _requireWorkspace();
    final bootstrap = _selectedBootstrap;
    if (bootstrap == null) {
      throw const ApiException('Open a page first.');
    }
    return _api.exportMarkdown(
      session.accessToken,
      workspace.id,
      bootstrap.document.id,
    );
  }

  Future<Uint8List> _exportPageZip() async {
    final session = _requireSession();
    final workspace = _requireWorkspace();
    final bootstrap = _selectedBootstrap;
    if (bootstrap == null) {
      throw const ApiException('Open a page first.');
    }
    return _api.exportDocumentZip(
      session.accessToken,
      workspace.id,
      bootstrap.document.id,
    );
  }

  Future<String> _exportWorkspaceMarkdown() async {
    final session = _requireSession();
    final workspace = _requireWorkspace();
    return _api.exportWorkspaceMarkdown(session.accessToken, workspace.id);
  }

  Future<Uint8List> _exportWorkspaceZip(String workspaceId) async {
    final session = _requireSession();
    return _api.exportWorkspaceZip(session.accessToken, workspaceId);
  }

  Future<String> _exportAllMarkdown() async {
    final session = _requireSession();
    return _api.exportAllMarkdown(session.accessToken);
  }

  Future<List<SearchResult>> _searchWorkspace(String query) async {
    final session = _session;
    final workspace = _selectedWorkspace;
    if (session == null || workspace == null) return const [];
    return _api.searchWorkspace(session.accessToken, workspace.id, query);
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

  Future<void> _updateProfile(String displayName) async {
    final session = _requireSession();
    final user = await _api.updateMe(session.accessToken, displayName);
    if (mounted) {
      final updated = AuthSession(accessToken: session.accessToken, user: user);
      setState(() => _session = updated);
      _persistSession(updated); // keep the saved display name fresh for restart
    }
  }

  Future<void> _changePassword(String current, String next) async {
    final session = _requireSession();
    await _api.changePassword(session.accessToken, current, next);
  }

  Future<Map<String, dynamic>> _loadAiSettings() async {
    final session = _requireSession();
    return _api.getAiSettings(session.accessToken);
  }

  /// Whether an AI provider is configured server-side (an API key, or a
  /// model for keyless local providers). Failure leaves AI hidden.
  Future<void> _refreshAiConfigured() async {
    final session = _session;
    if (session == null) return;
    try {
      final s = await _api.getAiSettings(session.accessToken);
      if (mounted) {
        setState(() {
          _aiConfigured =
              s['has_key'] == true || (s['model'] as String? ?? '').isNotEmpty;
        });
      }
    } catch (_) {}
  }

  Future<void> _saveAiSettings({
    required String provider,
    required String baseUrl,
    required String model,
    String? apiKey,
  }) async {
    final session = _requireSession();
    await _api.updateAiSettings(
      session.accessToken,
      provider: provider,
      baseUrl: baseUrl,
      model: model,
      apiKey: apiKey,
    );
    if (mounted) setState(() => _aiConfigured = true);
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
        _viewsByWorkspace = {
          ..._viewsByWorkspace,
          workspace.id: [...views, bootstrap.view],
        };
        _selectedView = bootstrap.view;
        _selectedBootstrap = bootstrap;
        _selectedMarkdown = null;
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
        _viewsByWorkspace = {
          ..._viewsByWorkspace,
          workspace.id: [...views, bootstrap.view],
        };
        _selectedView = bootstrap.view;
        _selectedBootstrap = bootstrap;
        _selectedMarkdown = null;
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
    final wsName = _cleanArchiveName(fileName);
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
    List<ArchiveFile> entries,
  ) {
    return _runServerImport(buildStoreZip(entries), workspaceId: workspace.id);
  }

  /// Upload the archive, poll the import job, then refresh and open the
  /// resulting workspace.
  Future<void> _runServerImport(
    Uint8List zipBytes, {
    String? name,
    bool notion = false,
    String? workspaceId,
  }) {
    return _run(() async {
      final session = _requireSession();
      final jobId = await _api.startWorkspaceImport(
        session.accessToken,
        zipBytes,
        name: name,
        notion: notion,
        workspaceId: workspaceId,
      );
      ImportJobStatus job;
      while (true) {
        job = await _api.importJobStatus(session.accessToken, jobId);
        if (job.status != 'running') break;
        await Future<void>.delayed(const Duration(milliseconds: 600));
      }
      if (job.status == 'error') {
        throw ApiException(job.error ?? 'import failed');
      }
      final workspaces = await _api.listWorkspaces(session.accessToken);
      if (mounted) setState(() => _workspaces = workspaces);
      final targetId = job.workspaceId ?? workspaceId;
      for (final w in workspaces) {
        if (w.id == targetId) {
          await _selectWorkspace(w);
          break;
        }
      }
    });
  }

  /// Workspace name from an archive filename: drop the extension and the
  /// ID noise Notion adds — `Export-<uuid>.zip` (suffix) and
  /// `<uuid>_Export.zip` (prefix) both clean up to "Export".
  String _cleanArchiveName(String fileName) {
    const hex32 = r'[0-9a-fA-F]{32}';
    const uuid =
        r'[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}'
        r'-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}';
    var name = fileName
        .replaceAll(RegExp(r'\.zip$', caseSensitive: false), '')
        .trim();
    name = name
        .replaceFirst(RegExp('[ \\-_]+($hex32|$uuid)\$'), '')
        .replaceFirst(RegExp('^($hex32|$uuid)[ \\-_]+'), '');
    return name;
  }

  Future<void> _aiCurrentFromMarkdown(String markdown) {
    return _run(() async {
      final bootstrap = _selectedBootstrap;
      if (bootstrap == null) {
        throw const ApiException('Open a page first.');
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
      setState(() {
        _workspaces = [..._workspaces, workspace];
        _selectedWorkspace = workspace;
        _viewsByWorkspace = {
          ..._viewsByWorkspace,
          workspace.id: [bootstrap.view],
        };
        _selectedView = bootstrap.view;
        _selectedBootstrap = bootstrap;
        _selectedMarkdown = null;
      });
      await _loadSelectedWorkspaceMembers();
    });
  }

  Future<void> _selectView(DocumentView view) {
    // A folder has no document to open (the sidebar routes folder taps to
    // expand/collapse; this guards other callers, e.g. internal page links).
    if (view.objectType == 'folder') return Future.value();
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
      setState(() {
        _selectedView = view;
        _selectedBootstrap = bootstrap;
        _selectedMarkdown = null;
      });
      // Reaching the server means we're back online — restore the real nav if we
      // had fallen back to the offline mirror.
      if (reachedServer) _recoverOnlineNav();
    });
  }

  Future<void> _renameView(DocumentView view, String name) {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      final renamed = await _api.updateView(
        session.accessToken,
        workspace.id,
        view.id,
        name,
      );

      setState(() {
        final views = _viewsByWorkspace[workspace.id] ?? const [];
        _viewsByWorkspace = {
          ..._viewsByWorkspace,
          workspace.id: views
              .map((item) => item.id == renamed.id ? renamed : item)
              .toList(),
        };

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
    });
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
        _viewsByWorkspace = {..._viewsByWorkspace, workspace.id: views};
        if (_selectedView?.id == view.id) {
          _selectedView = null;
          _selectedBootstrap = null;
          _selectedMarkdown = null;
        }
      });
    });
  }

  Future<void> _updateRootBlockText(String text) {
    final bootstrap = _selectedBootstrap;
    if (bootstrap == null) {
      return _run(() async {
        throw const ApiException('Select a page first.');
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
        throw const ApiException('Select a page first.');
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
        throw const ApiException('Select a page first.');
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
        throw const ApiException('Select a page first.');
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
        _selectedMarkdown = null;
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
        _selectedMarkdown = null;
      });
    } catch (error) {
      if (mounted) {
        setState(() => _message = error.toString());
      }
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

  Workspace _workspaceFromData(WorkspaceData w) => Workspace(
    id: w.id,
    name: w.name,
    ownerId: 'local',
    role: 'owner',
  );

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
    final selId = _localSelectedWorkspace?.id;
    _localSelectedWorkspace = _localWorkspaces.firstWhere(
      (w) => w.id == selId,
      orElse: () => _localWorkspaces.first,
    );
  }

  String _nextWorkspacePosition() {
    var max = 0;
    for (final w in _local.listWorkspaces()) {
      final n = int.tryParse(w.position) ?? 0;
      if (n > max) max = n;
    }
    return (max + 10).toString().padLeft(10, '0');
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
  String _nextLocalPosition(String? parentViewId) {
    var max = 0;
    for (final v in _localViews) {
      if (v.parentViewId == parentViewId) {
        final n = int.tryParse(v.position) ?? 0;
        if (n > max) max = n;
      }
    }
    return (max + 10).toString().padLeft(10, '0');
  }

  /// Open the on-device store, load the page tree, and select (or seed) a page.
  Future<void> _initLocalOffline() async {
    if (_localReady) return;
    try {
      await _local.open();
    } catch (error) {
      if (mounted) {
        setState(() {
          _message = '本地存储打开失败: $error';
          // Unblock the shell (P3c gates on _localReady) — the local world
          // shows empty with the error banner; the cloud world still works.
          _localReady = true;
        });
      }
      return;
    }
    _reloadLocalWorkspaces();
    _reloadLocalViews();
    if (_localViews.isEmpty) {
      await _localCreateDocument('欢迎');
    } else {
      // Open the first document, skipping folders (a folder has no doc to open;
      // _localSelectView would early-return, leaving the editor blank).
      final firstDoc = firstOpenableView(_localViews);
      if (firstDoc != null) await _localSelectView(firstDoc);
    }
    if (mounted) setState(() => _localReady = true);
  }

  Future<void> _localCreateWorkspace(String name) async {
    final title = name.trim().isEmpty ? '工作区' : name.trim();
    final id = 'ws_${DateTime.now().microsecondsSinceEpoch}';
    _local.saveWorkspace(
      (id: id, name: title, position: _nextWorkspacePosition(), role: 'owner'),
    );
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
    await _localCreateDocument('欢迎');
  }

  Future<void> _localSelectWorkspace(Workspace workspace) async {
    setState(() {
      _localSelectedWorkspace = workspace;
      _reloadLocalViews();
      _localSelectedView = null;
      _localBootstrap = null;
    });
    final firstDoc = firstOpenableView(_localViews);
    if (firstDoc != null) await _localSelectView(firstDoc);
  }

  Future<void> _localRenameWorkspace(Workspace workspace, String name) async {
    final title = name.trim().isEmpty ? workspace.name : name.trim();
    final pos = _local
        .listWorkspaces()
        .firstWhere(
          (w) => w.id == workspace.id,
          orElse: () =>
              (id: workspace.id, name: title, position: '0000000010', role: 'owner'),
        )
        .position;
    _local.saveWorkspace(
      (id: workspace.id, name: title, position: pos, role: 'owner'),
    );
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
    // Keep at least one workspace on the device.
    if (_localWorkspaces.length <= 1) {
      setState(() => _message = '至少保留一个本地工作区。');
      return;
    }
    _local.deleteWorkspace(workspace.id);
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

  Future<String?> _localCreateDocument(String name, {String? parentViewId}) async {
    final title = name.trim().isEmpty ? kUntitledPage : name.trim();
    final created = _local.newDoc();
    final viewId = 'view_${DateTime.now().microsecondsSinceEpoch}';
    final position = _nextLocalPosition(parentViewId);
    final data = (
      id: viewId,
      workspaceId: _localSelectedWorkspace?.id ?? 'local',
      parentId: parentViewId,
      objectId: created.docId,
      name: title,
      position: position,
      trashed: false,
      objectType: 'document',
    );
    _local.saveView(data);
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
  Future<String?> _localCreateFolder(String name, {String? parentViewId}) async {
    final title = name.trim().isEmpty ? '新文件夹' : name.trim();
    final viewId = 'view_${DateTime.now().microsecondsSinceEpoch}';
    _local.saveView((
      id: viewId,
      workspaceId: _localSelectedWorkspace?.id ?? 'local',
      parentId: parentViewId,
      // A folder has no document; object_id is an unused placeholder.
      objectId: 'folder_$viewId',
      name: title,
      position: _nextLocalPosition(parentViewId),
      trashed: false,
      objectType: 'folder',
    ));
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
    List<ArchiveFile> entries,
  ) async {
    final files = <({String path, List<int> bytes})>[
      for (final f in entries) (path: f.name, bytes: f.bytes),
    ];
    final result = await _local.importVaultTree(files, workspace.id);
    if (!mounted) return;
    setState(_reloadLocalViews);
    final parts = <String>[
      if (result.docs > 0) '${result.docs} 篇笔记',
      if (result.folders > 0) '${result.folders} 个文件夹',
    ];
    final msg = parts.isEmpty
        ? (result.errors.isEmpty
              ? '没找到 Markdown 文件'
              : '导入失败:${result.errors.first}')
        : '已导入 ${parts.join('、')}'
              '${result.errors.isNotEmpty ? '(${result.errors.length} 个跳过)' : ''}';
    ScaffoldMessenger.maybeOf(
      context,
    )?.showSnackBar(SnackBar(content: Text(msg)));
  }

  Future<void> _localSelectView(DocumentView view) async {
    if (view.objectType == 'folder') return; // a folder has no document to open
    final data = _local.openDoc(view.objectId);
    if (data == null || !mounted) return;
    setState(() {
      _localSelectedView = view;
      _localBootstrap = _localBootstrapFrom(
        view.objectId,
        data.rootBlockId,
        data.blocks,
        view,
      );
    });
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
    var session = _session;
    if (session == null) {
      final creds = await _promptCloudAuth(migrateWorkspace: localWs.name);
      if (creds == null || !mounted) return;
      await _run(() async {
        final s = creds.$1 == AuthMode.register
            ? await _api.register(creds.$2)
            : await _api.login(creds.$2);
        _persistSession(s);
        setState(() => _session = s);
      });
      session = _session;
      if (session == null || !mounted) return;
    }
    var migrated = false;
    await _run(() async {
      final clientId = await _local.deviceClientId();
      if (clientId == null) throw StateError('本地身份不可用,无法迁移');
      final result = await _runWorkspaceMigration(session!, clientId, localWs);
      // Refresh the cloud list so the new workspace appears in the switcher.
      final workspaces = await _api.listWorkspaces(session.accessToken);
      if (!mounted) return;
      migrated = true;
      setState(() {
        _workspaces = workspaces;
        _message = '已把 “${localWs.name}” 上云(${result.docCount} 页)。';
      });
    });
    if (!migrated || !mounted) return;
    // Post-migration choice (P3 决策④): default delete the local original;
    // keeping it is an explicit escape hatch and creates an independent fork.
    final delete = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('上云完成'),
        content: Text(
          '“${localWs.name}” 已复制到云端。本地原件现在是独立副本,'
          '不会再和云端同步——保留它会出现两个同名但内容会分叉的工作区。',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: const Text('保留本地原件'),
          ),
          FilledButton(
            onPressed: () => Navigator.of(context).pop(true),
            child: const Text('删除本地原件'),
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
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('转为本地工作区'),
        content: Text(
          '把 “${entry.workspace.name}” 复制为一个新的本地工作区?'
          '云端原工作区保持不变;两者从此独立,不再互相同步。',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: const Text('取消'),
          ),
          FilledButton(
            onPressed: () => Navigator.of(context).pop(true),
            child: const Text('复制为本地'),
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
      setState(() => _message = '本地存储不可用,无法转为本地。');
      return;
    }
    setState(() {
      _reloadLocalWorkspaces();
      _message =
          '已把 “${entry.workspace.name}” 复制为本地工作区(${result.docs} 页有内容)。';
    });
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
        final doc = _local.openDoc(v.objectId);
        if (doc == null) continue;
        final cloudParent =
            v.parentId == null ? null : localToCloudView[v.parentId];
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
          _local.putBlobAs(up.id, bytes); // mirror so the cloud copy renders offline
        }

        // Faithfully replay the local tree onto the cloud doc's root.
        final yrs = CloudSyncSession(
          uri: documentSocketUri(
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
          yrs.applyLocalOps(buildMigrationOps(
            blocks: doc.blocks,
            localRootId: doc.rootBlockId,
            cloudRootId: yrs.rootBlockId,
            idMap: idMap,
          ));
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
  Future<(AuthMode, AuthFormValue)?> _promptCloudAuth({String? migrateWorkspace}) {
    final migrate = migrateWorkspace != null;
    final email = TextEditingController();
    final name = TextEditingController();
    final pass = TextEditingController();
    var mode = AuthMode.login;
    return showDialog<(AuthMode, AuthFormValue)?>(
      context: context,
      builder: (ctx) => StatefulBuilder(
        builder: (ctx, setLocal) => AlertDialog(
          title: Text(migrate ? '连接云端账号并迁移' : '登录云端账号'),
          content: SingleChildScrollView(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Text(
                  migrate
                      ? '把本地工作区 “$migrateWorkspace” 复制到云端(本地数据保留)。'
                      : '登录后即可访问你的云端工作区(本地数据不受影响)。',
                  style: Theme.of(ctx).textTheme.bodySmall,
                ),
                const SizedBox(height: 12),
                SegmentedButton<AuthMode>(
                  segments: const [
                    ButtonSegment(value: AuthMode.login, label: Text('登录')),
                    ButtonSegment(value: AuthMode.register, label: Text('注册')),
                  ],
                  selected: {mode},
                  onSelectionChanged: (s) => setLocal(() => mode = s.first),
                ),
                const SizedBox(height: 12),
                TextField(
                  controller: email,
                  keyboardType: TextInputType.emailAddress,
                  decoration: const InputDecoration(labelText: '邮箱'),
                ),
                if (mode == AuthMode.register)
                  TextField(
                    controller: name,
                    decoration: const InputDecoration(labelText: '显示名'),
                  ),
                TextField(
                  controller: pass,
                  obscureText: true,
                  decoration: const InputDecoration(labelText: '密码'),
                ),
              ],
            ),
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.of(ctx).pop(null),
              child: const Text('取消'),
            ),
            FilledButton(
              onPressed: () => Navigator.of(ctx).pop((
                mode,
                AuthFormValue(
                  email: email.text.trim(),
                  displayName: name.text.trim(),
                  password: pass.text,
                ),
              )),
              child: Text(migrate ? '迁移' : '登录'),
            ),
          ],
        ),
      ),
    ).whenComplete(() {
      email.dispose();
      name.dispose();
      pass.dispose();
    });
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
  Future<({String fileId, String name})?> _localImportImageUrl(String url) async {
    try {
      final resp = await http.get(Uri.parse(url));
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
        final resp = await http.get(Uri.parse(key));
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

  Future<void> _localRenameView(DocumentView view, String name) async {
    final title = name.trim().isEmpty ? 'Untitled' : name.trim();
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
    if (!mounted) return;
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
  }

  /// A view and all its descendants from the on-device set (incl. trashed rows),
  /// so local delete/restore/purge cascade the whole subtree like the server's
  /// recursive-CTE handlers. Without this, trashing a folder orphans its
  /// children: deep descendants vanish from the sidebar (the orphan fallback in
  /// _visibleDocumentTree only lifts direct children) until the parent returns.
  List<ViewData> _localSubtree(String rootId) {
    final all = _local.listViews();
    final ids = collectSubtreeIds(
      all.map((v) => (id: v.id, parentId: v.parentId)),
      rootId,
    );
    return [
      for (final v in all)
        if (ids.contains(v.id)) v,
    ];
  }

  Future<void> _localDeleteView(DocumentView view) async {
    // Soft-delete the page AND its whole subtree (folders carry children).
    final subtree = _localSubtree(view.id);
    final ids = {for (final v in subtree) v.id};
    for (final v in subtree) {
      _local.saveView((
        id: v.id,
        workspaceId: v.workspaceId,
        parentId: v.parentId,
        objectId: v.objectId,
        name: v.name,
        position: v.position,
        trashed: true,
        objectType: v.objectType,
      ));
    }
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

  Future<void> _localReorderViews(
    String? parentViewId,
    List<DocumentView> ordered,
  ) async {
    for (var i = 0; i < ordered.length; i++) {
      final v = ordered[i];
      final position = ((i + 1) * 10).toString().padLeft(10, '0');
      if (v.position != position || v.parentViewId != parentViewId) {
        _local.saveView((
          id: v.id,
          workspaceId: _workspaceIdOfView(v.id),
          parentId: parentViewId,
          objectId: v.objectId,
          name: v.name,
          position: position,
          trashed: false,
          objectType: v.objectType,
        ));
      }
    }
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
    // Restore the page and the subtree that was trashed with it.
    for (final v in _localSubtree(view.id)) {
      _local.saveView((
        id: v.id,
        workspaceId: v.workspaceId,
        parentId: v.parentId,
        objectId: v.objectId,
        name: v.name,
        position: v.position,
        trashed: false,
        objectType: v.objectType,
      ));
    }
    // Mirror the server's restore_view: if the restored root's parent is no
    // longer an active view, lift it to the top level so it isn't an orphan.
    final active = {
      for (final v in _local.listViews())
        if (!v.trashed) v.id,
    };
    final root = _local.listViews().where((v) => v.id == view.id).firstOrNull;
    if (root != null && root.parentId != null && !active.contains(root.parentId)) {
      _local.saveView((
        id: root.id,
        workspaceId: root.workspaceId,
        parentId: null,
        objectId: root.objectId,
        name: root.name,
        position: root.position,
        trashed: false,
        objectType: root.objectType,
      ));
    }
    if (mounted) setState(_reloadLocalViews);
  }

  Future<void> _localPurgeView(DocumentView view) async {
    // Permanently remove the page and its subtree from the recycle bin.
    final subtree = _localSubtree(view.id);
    final ids = {for (final v in subtree) v.id};
    for (final v in subtree) {
      _local.purgeView(v.id, v.objectId);
    }
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
      // Server reachable but rejected the upload — a genuine failure.
      if (mounted) setState(() => _message = error.toString());
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
    if (_pending.add((sha: sha, workspaceId: workspaceId, docId: docId, name: name))) {
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
    if (session == null || workspace == null || cloud == null || !cloud.isReady) {
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
          if (_pending.remove(workspace.id, documentId, entry.sha)) changed = true;
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
        if (_pending.remove(workspace.id, documentId, entry.sha)) changed = true;
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
  Future<({String fileId, String name})?> _importEditorImageUrl(
    String url,
  ) async {
    final session = _session;
    final workspace = _selectedWorkspace;
    if (session == null || workspace == null) return null;
    try {
      final file = await _api.importImageUrl(
        session.accessToken,
        workspace.id,
        url,
      );
      return (fileId: file.id, name: file.name);
    } catch (error) {
      if (mounted) setState(() => _message = error.toString());
      return null;
    }
  }

  /// Map image file ids to their permanent Mica blob links (stable, never
  /// expiring — the endpoint re-signs storage on each request). Used by copy so
  /// pasted images keep displaying anywhere.
  Future<Map<String, String>> _resolveEditorImageUrls(List<String> ids) async {
    final workspace = _selectedWorkspace;
    if (workspace == null || ids.isEmpty) return {};
    final base = _api.baseUri;
    final origin = '${base.scheme}://${base.host}:${base.port}';
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
        final resp = await http.get(Uri.parse(key));
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
      final urls = await _api.resolveFiles(
        session.accessToken,
        workspace.id,
        [key],
      );
      final url = urls[key];
      if (url == null) return null;
      final resp = await http.get(Uri.parse(url));
      if (resp.statusCode != 200) return null;
      if (!kIsWeb) _local.putBlobAs(key, resp.bodyBytes);
      return resp.bodyBytes;
    } catch (_) {
      return null;
    }
  }

  Future<void> _exportSelectedMarkdown() {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      final bootstrap = _selectedBootstrap;
      if (bootstrap == null) {
        throw const ApiException('Select a page first.');
      }

      final markdown = await _api.exportMarkdown(
        session.accessToken,
        workspace.id,
        bootstrap.document.id,
      );

      setState(() {
        _selectedMarkdown = markdown;
      });
    });
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
      throw const ApiException('Sign in first.');
    }
    return session;
  }

  Workspace _requireWorkspace() {
    final workspace = _selectedWorkspace;
    if (workspace == null) {
      throw const ApiException('Select a workspace first.');
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
    _clearPersistedSession();
    _disconnectCloudSession();
  }

  /// Tear down the live cloud session/state WITHOUT touching stored
  /// credentials — used by sign-out (after wiping creds) and by switching
  /// cloud servers (which deliberately keeps the old origin's creds, P3c-2).
  void _disconnectCloudSession() {
    _closeDocumentSync();
    setState(() {
      _session = null;
      _workspaces = const [];
      _membersByWorkspace = const {};
      _viewsByWorkspace = const {};
      _selectedWorkspace = null;
      _selectedView = null;
      _selectedBootstrap = null;
      _selectedMarkdown = null;
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

  Future<void> _loadSelectedWorkspaceViews() async {
    final session = _session;
    final workspace = _selectedWorkspace;
    if (session == null || workspace == null) {
      return;
    }

    final views = await _api.listViews(session.accessToken, workspace.id);
    final selectedView = views
        .where((view) => view.id == _selectedView?.id)
        .firstOrNull;
    // Auto-open a DOCUMENT (folders have no content to bootstrap — opening one
    // would 404 on its unbacked object_id).
    final viewToOpen = selectedView ?? firstOpenableView(views);
    DocumentBootstrap? bootstrap;
    if (viewToOpen != null && viewToOpen.objectType != 'folder') {
      try {
        bootstrap = await _api.bootstrapDocument(
          session.accessToken,
          workspace.id,
          viewToOpen.objectId,
        );
      } on ApiException {
        // The auto-open target answered with an error (404/403 — deleted
        // server-side, access revoked, or an unbacked folder object_id from an
        // older client that didn't guard). listViews already succeeded, so we
        // are demonstrably ONLINE: show the tree with nothing opened rather than
        // letting one bad view cascade into a full offline downgrade at startup.
        bootstrap = null;
      }
    }

    setState(() {
      _viewsByWorkspace = {..._viewsByWorkspace, workspace.id: views};
      _selectedView = viewToOpen;
      _selectedBootstrap = bootstrap;
      _selectedMarkdown = null;
    });
    _cacheCloudPageTree();
  }

  /// Mirror the cloud page tree (workspace list + per-workspace views) into the
  /// on-device store so a future offline start can still list and navigate cloud
  /// content (P2 option C — Phase 1b/1c). Origin-scoped by server URL, so
  /// switching servers doesn't cross over. P4-2: on web the mirror is
  /// localStorage-backed (LocalOffline web variant). The cloud is authoritative
  /// — this is a clean replace after each successful online load.
  void _cacheCloudPageTree() {
    final origin = _api.baseUri.toString();
    final workspaces = <WorkspaceData>[
      for (final (i, w) in _workspaces.indexed)
        (
          id: w.id,
          name: w.name,
          position: ((i + 1) * 10).toString().padLeft(10, '0'),
          role: w.role, // mirror the real role so offline editing knows its rights
        ),
    ];
    final views = <ViewData>[
      for (final e in _viewsByWorkspace.entries)
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
    final bootstrap =
        firstView == null ? null : await _offlineCloudBootstrap(firstView);
    if (!mounted) return false;
    setState(() {
      _session = session;
      _workspaces = rebuilt.workspaces;
      _selectedWorkspace = workspace;
      _viewsByWorkspace = rebuilt.views;
      _selectedView = firstView;
      _selectedBootstrap = bootstrap;
      _selectedMarkdown = null;
      // Mark degraded-nav mode: roles come from the (possibly stale) mirror and
      // ownerId/objectType are defaulted until the server is reachable again
      // (see _recoverOnlineNav). The server re-checks the real role on every
      // push, so a stale mirrored role is a UX gate only, never an authority.
      _offlineNav = true;
    });
    if (bootstrap != null) _reconcileSync();
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
    final data = _local.openCloudDocMirror(view.objectId) ??
        await openWebDocMirror(_cloudOrigin, view.objectId);
    if (data == null) return null;
    return _localBootstrapFrom(view.objectId, data.rootBlockId, data.blocks, view);
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

  /// Switch the whole switcher to a world (the Level-1 world toggle). Lands on
  /// that world's current/first workspace; switching to a signed-out cloud world
  /// prompts sign-in first, then lands there. In-place — both worlds are
  /// first-class and long-lived, so no restart (unlike AppFlowy's anon→cloud).
  Future<void> _switchWorld(bool local) async {
    if (local == _activeIsLocal) return; // already in that world
    if (local) {
      final target = _localSelectedWorkspace ?? _localWorkspaces.firstOrNull;
      if (target == null) return; // local always seeds one, but be safe
      await _selectEntry(
        WorkspaceEntry(origin: 'local', workspace: target, role: 'owner'),
      );
      return;
    }
    // Cloud world needs a signed-in session.
    if (_session == null) {
      await _promptSignIn();
      if (!mounted || _session == null) return; // cancelled / failed
    }
    final cloudOrigin = _api.baseUri.toString();
    final target = _selectedWorkspace ?? _workspaces.firstOrNull;
    if (target == null) {
      // Signed in but no cloud workspaces yet — flip world so the empty-cloud
      // state (create / import) shows instead of the local tree.
      if (_activeOrigin != cloudOrigin) {
        setState(() => _activeOrigin = cloudOrigin);
        savePref('activeOrigin', cloudOrigin);
      }
      return;
    }
    await _selectEntry(
      WorkspaceEntry(origin: cloudOrigin, workspace: target, role: target.role),
    );
  }

  // Per-entry workspace actions (P3c): the switcher lists BOTH worlds, so row
  // actions must dispatch on the ROW's origin, not the active one.
  Future<void> _renameEntry(WorkspaceEntry e, String name) => e.isLocal
      ? _localRenameWorkspace(e.workspace, name)
      : _renameWorkspace(e.workspace, name);

  Future<void> _deleteEntry(WorkspaceEntry e) =>
      e.isLocal ? _localDeleteWorkspace(e.workspace) : _deleteWorkspace(e.workspace);

  Future<Uint8List> _exportEntryZip(WorkspaceEntry e) => e.isLocal
      ? Future.value(Uint8List(0)) // parity with the old local-shell stub
      : _exportWorkspaceZip(e.workspace.id);

  Future<void> _importTreeIntoEntry(
    WorkspaceEntry e,
    List<ArchiveFile> entries,
  ) => e.isLocal
      ? _localImportVaultTree(e.workspace, entries)
      : _importTreeIntoWorkspace(e.workspace, entries);

  /// Create a workspace of the chosen kind (P3c unified create dialog), then
  /// make its world active — each impl already selects the new workspace inside
  /// its own world, so without the flip a user creating into the OTHER world
  /// would see nothing happen.
  Future<void> _createWorkspaceTyped(String name, {required bool local}) async {
    await (local ? _localCreateWorkspace(name) : _createWorkspace(name));
    final origin = local ? 'local' : _api.baseUri.toString();
    if (_activeOrigin != origin && mounted) {
      setState(() => _activeOrigin = origin);
      savePref('activeOrigin', origin);
    }
  }

  /// Sign in to the configured cloud server from the switcher / account UI
  /// (desktop: the login gate is gone — auth is a dialog, P3c §1.3).
  Future<void> _promptSignIn() async {
    final creds = await _promptCloudAuth();
    if (creds == null || !mounted) return;
    await _run(() async {
      final session = creds.$1 == AuthMode.register
          ? await _api.register(creds.$2)
          : await _api.login(creds.$2);
      final workspaces = await _api.listWorkspaces(session.accessToken);
      setState(() {
        _session = session;
        _workspaces = workspaces;
        _selectedWorkspace = workspaces.firstOrNull;
      });
      _persistSession(session);
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
  Widget _unifiedWorkspaceView(AuthSession? session) {
    final local = _activeIsLocal;
    return WorkspaceView(
      session: session,
      entries: _workspaceEntries,
      activeIsLocal: local,
      onSwitchWorld: _switchWorld,
      selectedRef: _selectedEntry?.ref,
      onSelectEntry: _selectEntry,
      onRenameEntry: _renameEntry,
      onDeleteEntry: _deleteEntry,
      onExportEntryZip: _exportEntryZip,
      onImportTreeIntoEntry: _importTreeIntoEntry,
      onCreateWorkspaceTyped: _createWorkspaceTyped,
      cloudOriginLabel:
          _api.baseUri.host == Uri.parse(kMicaCloudUrl).host
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
      selectedView: local ? _localSelectedView : _selectedView,
      selectedBootstrap: local ? _localBootstrap : _selectedBootstrap,
      selectedMarkdown: local ? null : _selectedMarkdown,
      presence: local ? const [] : _presence,
      message: _message,
      onSelectWorkspace: local ? _localSelectWorkspace : _selectWorkspace,
      onCreateWorkspace: local ? _localCreateWorkspace : _createWorkspace,
      onRenameWorkspace: local ? _localRenameWorkspace : _renameWorkspace,
      onDeleteWorkspace: local ? _localDeleteWorkspace : _deleteWorkspace,
      onCreateDocument: local ? _localCreateDocument : _createDocument,
      onCreateChildDocument: local
          ? (parent, name) => _localCreateDocument(name, parentViewId: parent.id)
          : _createChildDocument,
      onCreateFolder: local ? _localCreateFolder : _createFolder,
      onCreateChildFolder: local
          ? (parent, name) => _localCreateFolder(name, parentViewId: parent.id)
          : _createChildFolder,
      onReorderViews: local ? _localReorderViews : _reorderViews,
      onLoadTrash: local ? _localLoadTrash : _loadTrash,
      onRestoreView: local ? _localRestoreView : _restoreView,
      onPurgeView: local ? _localPurgeView : _purgeView,
      onSelectView: local ? _localSelectView : _selectView,
      onRenameView: local ? _localRenameView : _renameView,
      onDeleteView: local ? _localDeleteView : _deleteView,
      onUpdateRootBlockText: local
          ? _localUpdateRootBlockText
          : _updateRootBlockText,
      onAddBlock: local ? (_, _) async {} : _addBlock,
      onUpdateBlock: local ? (_, _, _) async {} : _updateBlock,
      onDeleteBlock: local ? (_) async {} : _deleteBlock,
      onMoveBlock: local ? (_, _) async {} : _moveBlock,
      onApplyOperations: _applyEditorOperations, // P3d: one entry, self-dispatches
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
      onAiCurrentPage: local || _selectedBootstrap == null
          ? null
          : _aiCurrentFromMarkdown,
      onAiNewWorkspace: local ? (_) async {} : _aiNewWorkspaceFromMarkdown,
      onLoadAiSettings: local ? () async => const {} : _loadAiSettings,
      onSaveAiSettings: local
          ? ({
              required String provider,
              required String baseUrl,
              required String model,
              String? apiKey,
            }) async {}
          : _saveAiSettings,
      onLoadTokens: local ? null : _loadTokens,
      onCreateToken: local ? null : _createToken,
      onRevokeToken: local ? null : _revokeToken,
      userName: _session?.user.displayName ?? (local ? '本地' : ''),
      userEmail: _session?.user.email ?? '',
      onUpdateProfile: local ? (_) async {} : _updateProfile,
      onChangePassword: local ? (_, _) async {} : _changePassword,
      cloudOrigin: _cloudOrigin,
      onConnectCloud: _connectCloudServer,
      appearance: _appearance,
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
      aiEnabled: local ? false : _aiEnabled,
      onAiEnabledChanged: local
          ? (_) {}
          : (value) {
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
      onSearch: local ? (_) async => const <SearchResult>[] : _searchWorkspace,
      onOpenSearchResult: local ? (_) async {} : _openViewById,
      onExportPageMarkdown: local ? () async => '' : _exportPageMarkdown,
      onExportPageZip: local ? () async => Uint8List(0) : _exportPageZip,
      onImportMarkdown: local ? (_, _) async {} : _importMarkdownAsPage,
      onExportWorkspaceMarkdown: local
          ? () async => ''
          : _exportWorkspaceMarkdown,
      onExportWorkspaceZip: local
          ? (_) async => Uint8List(0)
          : _exportWorkspaceZip,
      onImportWorkspaceZip: local
          ? (_, _, {bool notion = false}) async {}
          : _importWorkspaceZip,
      onImportWorkspaceTreeInto: local
          ? _localImportVaultTree
          : _importTreeIntoWorkspace,
      onExportAllMarkdown: local ? () async => '' : _exportAllMarkdown,
      onExportMarkdown: local ? () async {} : _exportSelectedMarkdown,
      onAddMember: local ? (_, _) async {} : _addWorkspaceMember,
      onUpdateMember: local ? (_, _) async {} : _updateWorkspaceMember,
      onRemoveMember: local ? (_) async {} : _removeWorkspaceMember,
      onRestoreCheckpoint: local ? _localRollbackDoc : null,
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
      return Scaffold(
        body: SafeArea(
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              SizedBox(
                width: 360,
                child: SidePanel(
                  session: session,
                  isBusy: _isBusy,
                  onAuthenticate: _authenticate,
                  onCreateWorkspace: _createWorkspace,
                ),
              ),
              const VerticalDivider(width: 1),
              const Expanded(
                child: EmptyState(
                  icon: Icons.description_outlined,
                  title: 'Mica',
                  detail: 'Sign in to open your workspace.',
                ),
              ),
            ],
          ),
        ),
      );
    }
    // Desktop: the local store backs local workspaces AND the cloud mirrors —
    // wait for it before the shell renders (fast: one SQLite open + list).
    if (_local.available && !_localReady) {
      return const Scaffold(body: Center(child: CircularProgressIndicator()));
    }
    return Scaffold(body: SafeArea(child: _unifiedWorkspaceView(session)));
  }

}

class SidePanel extends StatefulWidget {
  const SidePanel({
    required this.session,
    required this.isBusy,
    required this.onAuthenticate,
    required this.onCreateWorkspace,
    super.key,
  });

  final AuthSession? session;
  final bool isBusy;
  final Future<void> Function(AuthMode mode, AuthFormValue form) onAuthenticate;
  final Future<void> Function(String name) onCreateWorkspace;

  @override
  State<SidePanel> createState() => _SidePanelState();
}

class _SidePanelState extends State<SidePanel> {
  AuthMode _mode = AuthMode.login;
  final _email = TextEditingController();
  final _displayName = TextEditingController();
  final _password = TextEditingController();
  final _workspaceName = TextEditingController();

  @override
  void dispose() {
    _email.dispose();
    _displayName.dispose();
    _password.dispose();
    _workspaceName.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final session = widget.session;

    return ColoredBox(
      color: Colors.white,
      child: Padding(
        padding: const EdgeInsets.all(20),
        child: session == null ? _authForm(context) : _workspaceForm(context),
      ),
    );
  }

  Widget _authForm(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text('Account', style: Theme.of(context).textTheme.titleLarge),
        const SizedBox(height: 16),
        SegmentedButton<AuthMode>(
          segments: const [
            ButtonSegment(
              value: AuthMode.login,
              icon: Icon(Icons.login),
              label: Text('Login'),
            ),
            ButtonSegment(
              value: AuthMode.register,
              icon: Icon(Icons.person_add),
              label: Text('Register'),
            ),
          ],
          selected: {_mode},
          onSelectionChanged: widget.isBusy
              ? null
              : (selection) {
                  setState(() {
                    _mode = selection.single;
                  });
                },
        ),
        const SizedBox(height: 18),
        TextField(
          controller: _email,
          enabled: !widget.isBusy,
          keyboardType: TextInputType.emailAddress,
          decoration: const InputDecoration(
            labelText: 'Email',
            prefixIcon: Icon(Icons.alternate_email),
            border: OutlineInputBorder(),
          ),
        ),
        const SizedBox(height: 12),
        if (_mode == AuthMode.register) ...[
          TextField(
            controller: _displayName,
            enabled: !widget.isBusy,
            decoration: const InputDecoration(
              labelText: 'Display name',
              prefixIcon: Icon(Icons.badge),
              border: OutlineInputBorder(),
            ),
          ),
          const SizedBox(height: 12),
        ],
        TextField(
          controller: _password,
          enabled: !widget.isBusy,
          obscureText: true,
          decoration: const InputDecoration(
            labelText: 'Password',
            prefixIcon: Icon(Icons.lock),
            border: OutlineInputBorder(),
          ),
        ),
        const SizedBox(height: 16),
        FilledButton.icon(
          onPressed: widget.isBusy ? null : _submitAuth,
          icon: Icon(
            _mode == AuthMode.register ? Icons.person_add : Icons.login,
          ),
          label: Text(_mode == AuthMode.register ? 'Register' : 'Login'),
        ),
      ],
    );
  }

  Widget _workspaceForm(BuildContext context) {
    final session = widget.session!;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text('Workspace', style: Theme.of(context).textTheme.titleLarge),
        const SizedBox(height: 4),
        Text(
          session.user.email,
          style: Theme.of(context).textTheme.bodySmall,
          overflow: TextOverflow.ellipsis,
        ),
        const SizedBox(height: 20),
        TextField(
          controller: _workspaceName,
          enabled: !widget.isBusy,
          decoration: const InputDecoration(
            labelText: 'Workspace name',
            prefixIcon: Icon(Icons.workspaces),
            border: OutlineInputBorder(),
          ),
        ),
        const SizedBox(height: 12),
        FilledButton.icon(
          onPressed: widget.isBusy ? null : _submitWorkspace,
          icon: const Icon(Icons.add),
          label: const Text('Create'),
        ),
      ],
    );
  }

  void _submitAuth() {
    widget.onAuthenticate(
      _mode,
      AuthFormValue(
        email: _email.text,
        displayName: _displayName.text,
        password: _password.text,
      ),
    );
  }

  void _submitWorkspace() {
    widget.onCreateWorkspace(_workspaceName.text);
    _workspaceName.clear();
  }
}

class WorkspaceView extends StatefulWidget {
  const WorkspaceView({
    required this.session,
    required this.entries,
    required this.activeIsLocal,
    required this.onSwitchWorld,
    required this.selectedRef,
    required this.onSelectEntry,
    required this.onRenameEntry,
    required this.onDeleteEntry,
    required this.onExportEntryZip,
    required this.onImportTreeIntoEntry,
    required this.onCreateWorkspaceTyped,
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
    required this.selectedView,
    required this.selectedBootstrap,
    required this.selectedMarkdown,
    required this.presence,
    required this.message,
    required this.onSelectWorkspace,
    required this.onCreateWorkspace,
    required this.onRenameWorkspace,
    required this.onDeleteWorkspace,
    required this.onCreateDocument,
    required this.onCreateChildDocument,
    required this.onCreateFolder,
    required this.onCreateChildFolder,
    required this.onReorderViews,
    required this.onLoadTrash,
    required this.onRestoreView,
    required this.onPurgeView,
    required this.onSelectView,
    required this.onRenameView,
    required this.onDeleteView,
    required this.onUpdateRootBlockText,
    required this.onAddBlock,
    required this.onUpdateBlock,
    required this.onDeleteBlock,
    required this.onMoveBlock,
    required this.onApplyOperations,
    required this.onUploadImage,
    required this.onImportImageUrl,
    required this.onLoadImageBytes,
    required this.onResolveImageUrls,
    required this.onAiStream,
    required this.onAiNewPage,
    required this.onAiCurrentPage,
    required this.onAiNewWorkspace,
    required this.onLoadAiSettings,
    required this.onSaveAiSettings,
    this.onLoadTokens,
    this.onCreateToken,
    this.onRevokeToken,
    required this.userName,
    required this.userEmail,
    required this.onUpdateProfile,
    required this.onChangePassword,
    required this.cloudOrigin,
    required this.onConnectCloud,
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
    required this.onSearch,
    required this.onOpenSearchResult,
    required this.onExportPageMarkdown,
    required this.onExportPageZip,
    required this.onImportMarkdown,
    required this.onExportWorkspaceMarkdown,
    required this.onExportWorkspaceZip,
    required this.onImportWorkspaceZip,
    required this.onImportWorkspaceTreeInto,
    required this.onExportAllMarkdown,
    required this.onExportMarkdown,
    required this.onAddMember,
    required this.onUpdateMember,
    required this.onRemoveMember,
    this.onRestoreCheckpoint,
    this.onMigrateEntry,
    this.onDetachEntry,
    this.onCursorChanged,
    this.editorEpoch = 0,
    super.key,
  });

  final AuthSession? session;

  /// The unified workspace list (P3c): local + cloud entries, grouped by
  /// origin in the switcher. Row actions dispatch on the ROW's entry.
  final List<WorkspaceEntry> entries;

  /// Whether the ACTIVE world is local — the Level-1 world toggle reflects it,
  /// and the switcher shows only this world's workspaces (one world at a time).
  final bool activeIsLocal;

  /// Switch the whole switcher to a world (true = local). Signed-out cloud
  /// prompts sign-in first. In-place; no restart.
  final Future<void> Function(bool local) onSwitchWorld;
  final WorkspaceRef? selectedRef;
  final Future<void> Function(WorkspaceEntry entry) onSelectEntry;
  final Future<void> Function(WorkspaceEntry entry, String name) onRenameEntry;
  final Future<void> Function(WorkspaceEntry entry) onDeleteEntry;
  final Future<Uint8List> Function(WorkspaceEntry entry) onExportEntryZip;
  final Future<void> Function(WorkspaceEntry entry, List<ArchiveFile> entries)
  onImportTreeIntoEntry;

  /// Create a workspace of the chosen kind (`local: true` = on-device).
  final Future<void> Function(String name, {required bool local})
  onCreateWorkspaceTyped;

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
  final DocumentView? selectedView;
  final DocumentBootstrap? selectedBootstrap;
  final String? selectedMarkdown;
  final List<PresenceUser> presence;
  final String? message;
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
  final Future<void> Function(String? parentViewId, List<DocumentView> ordered)
  onReorderViews;
  final Future<List<DocumentView>> Function() onLoadTrash;
  final Future<void> Function(DocumentView view) onRestoreView;
  final Future<void> Function(DocumentView view) onPurgeView;
  final Future<void> Function(DocumentView view) onSelectView;
  final Future<void> Function(DocumentView view, String name) onRenameView;
  final Future<void> Function(DocumentView view) onDeleteView;
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
  final Future<void> Function(String markdown)? onAiCurrentPage;
  final Future<void> Function(String markdown) onAiNewWorkspace;
  final Future<Map<String, dynamic>> Function() onLoadAiSettings;
  final Future<void> Function({
    required String provider,
    required String baseUrl,
    required String model,
    String? apiKey,
  })
  onSaveAiSettings;
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
  final Future<void> Function(String displayName) onUpdateProfile;
  final Future<void> Function(String current, String next) onChangePassword;
  /// The configured cloud server's origin URL (P3c-2) — Settings shows it and
  /// [onConnectCloud] switches it.
  final String cloudOrigin;
  final Future<void> Function(String url) onConnectCloud;
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
  final Future<List<SearchResult>> Function(String query) onSearch;
  final Future<void> Function(String viewId) onOpenSearchResult;
  final Future<String> Function() onExportPageMarkdown;
  final Future<Uint8List> Function() onExportPageZip;
  final Future<void> Function(String fileName, String markdown)
  onImportMarkdown;
  final Future<String> Function() onExportWorkspaceMarkdown;
  final Future<Uint8List> Function(String workspaceId) onExportWorkspaceZip;
  final Future<void> Function(String fileName, Uint8List bytes, {bool notion})
  onImportWorkspaceZip;
  final Future<void> Function(Workspace workspace, List<ArchiveFile> entries)
  onImportWorkspaceTreeInto;
  final Future<String> Function() onExportAllMarkdown;
  final Future<void> Function() onExportMarkdown;
  final Future<void> Function(String email, WorkspaceRole role) onAddMember;
  final Future<void> Function(WorkspaceMember member, WorkspaceRole role)
  onUpdateMember;
  final Future<void> Function(WorkspaceMember member) onRemoveMember;

  /// Restore the open document to its last on-device checkpoint (local mode
  /// only — null elsewhere, which hides the menu item).
  final Future<void> Function()? onRestoreCheckpoint;

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

  @override
  State<WorkspaceView> createState() => _WorkspaceViewState();
}

class _WorkspaceViewState extends State<WorkspaceView> {
  final _rename = TextEditingController();
  final _memberEmail = TextEditingController();
  final _pageTitle = TextEditingController();
  final FocusNode _editorFocus = FocusNode(debugLabel: 'MicaEditorBody');
  final FocusNode _pageTitleFocus = FocusNode(debugLabel: 'PageTitle');
  Timer? _pageTitleSaveTimer;
  // Persisted per-workspace: which nodes are EXPANDED. Absent = collapsed (the
  // default). The tree opens collapsed and remembers what the user expanded;
  // navigating to / creating a nested page reveals its ancestors.
  final Set<String> _expandedViewIds = {};
  // The view whose sidebar name is in inline-rename (edit) mode — null = none.
  // Set right after creating a page/folder (so the user types its name
  // immediately, no dialog) or from the row's "重命名" action; the matching row
  // renders a focused TextField instead of the name Text.
  String? _renamingViewId;
  // True only while a page is being dragged in the tree. The drop zones overlay
  // each row, so they are mounted only during a drag — otherwise they would
  // intercept ordinary taps on the page rows.
  bool _draggingTree = false;
  // Pointer is over the navigation sidebar — reveals the tree's expand
  // toggles (AppFlowy-style: they live in their own slim column, opacity 0
  // at rest so the page icons keep one aligned column).
  bool _navHovered = false;
  WorkspaceRole _memberRole = WorkspaceRole.editor;
  bool _toolsExpanded = false;
  bool _navCollapsed = false;
  // Pane widths, drag-resizable via the splitters (long page names need room).
  double _navWidth = 280;
  double _toolsWidth = 300;
  // Once the user drags the nav splitter we stop auto-fitting and honor their
  // width (persisted). Until then, the sidebar fits itself to the longest page
  // name on each workspace switch (min/max clamped) — the "auto width" ask.
  bool _navWidthManual = false;
  static const double _navWidthMin = 220;
  static const double _navWidthMax = 480;
  final EditorScrollHook _scrollHook = EditorScrollHook();
  // The realizable page-column width (editor pane minus the scroll padding),
  // measured live so the Settings "Page width" slider maxes at full-bleed
  // instead of offering px values the window can't show.
  double _editorAvailWidth = 1160;
  final GlobalKey _editorSurfaceKey = GlobalKey();
  final EditorCommandHook _commandHook = EditorCommandHook();
  // Live document outline (TOC). The editor republishes headings on every edit;
  // the outline panel listens, so it tracks typing instead of only navigation.
  final EditorOutlineHook _outlineHook = EditorOutlineHook();
  // In-page find (Ctrl+F). The editor owns the find bar; the app-level shortcut
  // opens it through this hook even when focus isn't in the editor.
  final EditorFindHook _findHook = EditorFindHook();

  @override
  void initState() {
    super.initState();
    // Backfill from the initial selection so the very first frame shows the page
    // name (and workspace rename field) instead of an empty "Untitled" hint —
    // didUpdateWidget only fires on later changes, so without this the title
    // looks blank until the next page switch.
    final name = widget.selectedBootstrap?.view.name ?? '';
    // Show the placeholder (not solid text) for an untitled page.
    _pageTitle.text = isUntitledPageName(name) ? '' : name;
    final workspace = widget.selectedWorkspace;
    if (workspace != null) _rename.text = workspace.name;
    // Restore this workspace's remembered expand state (tree opens collapsed by
    // default; reveal the initially-selected page's ancestors so it shows).
    _loadExpanded();
    final sel = widget.selectedBootstrap?.view.id;
    if (sel != null) _revealAncestors(sel);
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
      if (rowWidth > widest) widest = rowWidth;
    }
    _navWidth = widest.clamp(_navWidthMin, _navWidthMax);
  }

  @override
  void didUpdateWidget(covariant WorkspaceView oldWidget) {
    super.didUpdateWidget(oldWidget);
    final selected = widget.selectedWorkspace;
    final wsChanged =
        selected != null && selected.id != oldWidget.selectedWorkspace?.id;
    if (wsChanged) {
      _rename.text = selected.name;
      _loadExpanded(); // restore this workspace's remembered expand state
    }
    // Re-fit the sidebar to content when the tree could have changed shape:
    // switched workspace, added/removed a page, or renamed the open page.
    // (No-op once the width is user-controlled.)
    if (wsChanged ||
        widget.views.length != oldWidget.views.length ||
        widget.selectedBootstrap?.view.name !=
            oldWidget.selectedBootstrap?.view.name) {
      _fitNavWidthToContent();
    }

    final bootstrap = widget.selectedBootstrap;
    final idChanged =
        bootstrap?.view.id != oldWidget.selectedBootstrap?.view.id;
    // New page open → reseed the outline from its snapshot immediately (the new
    // editor's live publish is one frame away; this avoids a stale-headings flash).
    if (idChanged) _seedOutline();
    if (idChanged ||
        bootstrap?.view.name != oldWidget.selectedBootstrap?.view.name) {
      // Skip the no-op echo of our own rename: assigning .text resets the
      // selection, which the web engine renders as select-all — one
      // backspace in the title would select the whole name after the
      // debounced save round-tripped. An untitled page renders empty so its
      // placeholder shows instead of solid text.
      final name = bootstrap?.view.name ?? '';
      final display = isUntitledPageName(name) ? '' : name;
      if (_pageTitle.text != display) _pageTitle.text = display;
    }
    // Opening a fresh/untitled page with the title shown: land the caret in the
    // title so you can name it right away (instead of on the body), matching the
    // "Untitled is a placeholder" model.
    if (idChanged &&
        widget.showPageTitle &&
        bootstrap != null &&
        bootstrap.view.objectType == 'document' &&
        isUntitledPageName(bootstrap.view.name)) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!mounted) return;
        _pageTitleFocus.requestFocus();
        _pageTitle.selection = const TextSelection.collapsed(offset: 0);
      });
    }
    // Reveal the opened page in the sidebar: expand its ancestor chain so a
    // nested selection isn't hidden under collapsed parents (and the expansion
    // is remembered). Also fire when the view set first populates — on a cold
    // start the selection can be set before widget.views arrives, so the
    // initState reveal would have walked an empty tree.
    final viewsChanged = widget.views.length != oldWidget.views.length;
    if ((idChanged || viewsChanged) &&
        bootstrap != null &&
        _revealAncestors(bootstrap.view.id)) {
      _saveExpanded();
    }
  }

  @override
  void dispose() {
    _rename.dispose();
    _memberEmail.dispose();
    _pageTitle.dispose();
    _editorFocus.dispose();
    _pageTitleFocus.dispose();
    _pageTitleSaveTimer?.cancel();
    _outlineHook.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    // P3c: no sign-in gate — a null session just means no cloud account is
    // attached; local workspaces work regardless, and the switcher's cloud
    // section offers sign-in.
    return CallbackShortcuts(
      bindings: _appShortcuts(),
      child: Row(
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
              _navWidth =
                  (_navWidth + dx).clamp(_navWidthMin, _navWidthMax);
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
    ),
    );
  }

  /// App-level keyboard shortcuts (desktop feel): new page, search, settings.
  /// The editor handles its own editing shortcuts (Ctrl+B/I/Z/…) and returns the
  /// rest unhandled, so these fire when a key bubbles past it. Both Control
  /// (Win/Linux) and Meta (macOS) variants are bound.
  Map<ShortcutActivator, VoidCallback> _appShortcuts() {
    void newPage() =>
        _createThenRename(() => widget.onCreateDocument(kUntitledPage));
    return <ShortcutActivator, VoidCallback>{
      const SingleActivator(LogicalKeyboardKey.keyN, control: true): newPage,
      const SingleActivator(LogicalKeyboardKey.keyN, meta: true): newPage,
      // Ctrl/Cmd+F → in-page find within the open document; Ctrl/Cmd+Shift+F →
      // the workspace-wide search (what plain Ctrl+F used to do).
      const SingleActivator(LogicalKeyboardKey.keyF, control: true):
          _openInPageFind,
      const SingleActivator(LogicalKeyboardKey.keyF, meta: true):
          _openInPageFind,
      const SingleActivator(LogicalKeyboardKey.keyF, control: true, shift: true):
          _openSearch,
      const SingleActivator(LogicalKeyboardKey.keyF, meta: true, shift: true):
          _openSearch,
      // Ctrl+, is the convention (works on English layouts / macOS Cmd+,), but a
      // Chinese IME grabs Ctrl+,/Ctrl+. at the OS level (punctuation toggle), so
      // it won't reach the app while such an IME is active. Settings is also
      // reachable from the menu.
      const SingleActivator(LogicalKeyboardKey.comma, control: true):
          _openSettings,
      const SingleActivator(LogicalKeyboardKey.comma, meta: true): _openSettings,
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

  Widget _navigationPane(BuildContext context) {
    final canEdit = matchesEditRole(widget.selectedWorkspace?.role);

    return ColoredBox(
      color: Colors.white,
      child: MouseRegion(
        onEnter: (_) => setState(() => _navHovered = true),
        onExit: (_) => setState(() => _navHovered = false),
        child: Padding(
          padding: const EdgeInsets.all(16),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              // App header — top-left, grouped with the sidebar (no global AppBar).
              Row(
                children: [
                  const MicaLogo(size: 24),
                  const SizedBox(width: 8),
                  const Text(
                    'Mica',
                    style: TextStyle(fontSize: 20, fontWeight: FontWeight.w700),
                  ),
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
                    tooltip: 'Collapse sidebar',
                    visualDensity: VisualDensity.compact,
                    onPressed: () => setState(() => _navCollapsed = true),
                    icon: const Icon(
                      Icons.keyboard_double_arrow_left,
                      size: 20,
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 14),
              Row(
                crossAxisAlignment: CrossAxisAlignment.center,
                children: [
                  Expanded(
                    child: _WorkspaceSelector(
                      entries: widget.entries,
                      activeIsLocal: widget.activeIsLocal,
                      selectedRef: widget.selectedRef,
                      cloudEmail: widget.session?.user.email,
                      onSignIn: widget.onSignIn,
                      onSelect: widget.onSelectEntry,
                      onRename: _promptRenameWorkspace,
                      onDelete: _confirmDeleteWorkspace,
                      onExport: _exportWorkspaceFile,
                      onCreate: _promptCreateWorkspace,
                      onImport: (notion) =>
                          _importWorkspaceFile(notion: notion),
                      onImportFilesInto: _importFilesIntoWorkspace,
                      onImportFolderInto: _importFolderIntoWorkspace,
                      onMigrate: widget.onMigrateEntry,
                      onDetach: widget.onDetachEntry,
                    ),
                  ),
                  const SizedBox(width: 4),
                  IconButton(
                    tooltip: 'Workspace settings',
                    visualDensity: VisualDensity.compact,
                    onPressed: widget.selectedWorkspace == null
                        ? null
                        : _openWorkspaceSettingsDialog,
                    icon: const Icon(Icons.tune, size: 20),
                  ),
                ],
              ),
              if (widget.message != null) ...[
                const SizedBox(height: 12),
                ErrorBanner(widget.message!),
              ],
              const SizedBox(height: 12),
              _searchBox(context),
              // Slim, label-free action strip above the tree — the tree itself
              // is the section, it doesn't need a name.
              const SizedBox(height: 8),
              Row(
                mainAxisAlignment: MainAxisAlignment.end,
                children: [
                  IconButton(
                    tooltip: 'Refresh',
                    visualDensity: VisualDensity.compact,
                    onPressed: widget.onRefresh,
                    icon: const Icon(Icons.refresh, size: 20),
                  ),
                  if (canEdit) ...[
                    IconButton(
                      tooltip: 'Recycle bin',
                      visualDensity: VisualDensity.compact,
                      onPressed: _openRecycleBin,
                      icon: const Icon(Icons.delete_outline, size: 20),
                    ),
                    PopupMenuButton<String>(
                      tooltip: '新建',
                      position: PopupMenuPosition.under,
                      onSelected: (v) {
                        if (v == 'page') {
                          _createThenRename(
                            () => widget.onCreateDocument(kUntitledPage),
                          );
                        } else {
                          _createThenRename(
                            () => widget.onCreateFolder('新文件夹'),
                          );
                        }
                      },
                      itemBuilder: (_) => const [
                        PopupMenuItem(
                          value: 'page',
                          child: _MenuRow(icon: Icons.add, label: '新建页面'),
                        ),
                        PopupMenuItem(
                          value: 'folder',
                          child: _MenuRow(
                            icon: Icons.create_new_folder_outlined,
                            label: '新建文件夹',
                          ),
                        ),
                      ],
                      icon: const Icon(Icons.note_add_outlined, size: 20),
                    ),
                  ],
                ],
              ),
              const SizedBox(height: 4),
              Expanded(child: _pageTree(context, canEdit)),
              const Divider(height: 24),
              // AI entry points exist only when the feature is enabled in
              // Settings AND a provider is configured.
              if (widget.showAi) ...[
                SizedBox(
                  width: double.infinity,
                  child: FilledButton.tonalIcon(
                    onPressed: _openAiDialog,
                    icon: const Icon(Icons.auto_awesome, size: 18),
                    label: const Text('Ask AI'),
                  ),
                ),
                const SizedBox(height: 12),
              ],
              _accountTile(context),
            ],
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
          color: const Color(0xFFF1F5F9),
          borderRadius: BorderRadius.circular(8),
        ),
        child: Row(
          children: [
            Icon(
              Icons.search,
              size: 18,
              color: enabled
                  ? const Color(0xFF64748B)
                  : const Color(0xFFCBD5E1),
            ),
            const SizedBox(width: 8),
            Text(
              'Search…',
              style: TextStyle(
                color: enabled
                    ? const Color(0xFF64748B)
                    : const Color(0xFFCBD5E1),
              ),
            ),
          ],
        ),
      ),
    );
  }

  /// The sidebar collapsed to a slim rail: just the logo and an expand button.
  Widget _collapsedNavRail(BuildContext context) {
    return ColoredBox(
      color: Colors.white,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 16),
        child: Column(
          children: [
            const MicaLogo(size: 24),
            const SizedBox(height: 12),
            IconButton(
              tooltip: 'Expand sidebar',
              visualDensity: VisualDensity.compact,
              onPressed: () => setState(() => _navCollapsed = false),
              icon: const Icon(Icons.keyboard_double_arrow_right, size: 20),
            ),
          ],
        ),
      ),
    );
  }

  /// Account row pinned to the bottom of the left sidebar. Tapping opens a menu
  /// with Settings and (when a cloud account is attached) Sign out, or Sign in.
  Widget _accountTile(BuildContext context) {
    final user = widget.session?.user;
    final name = user?.displayName.isNotEmpty == true
        ? user!.displayName
        : (user?.email ?? '本地');
    final initial = name.isNotEmpty ? name.substring(0, 1).toUpperCase() : '?';
    return PopupMenuButton<String>(
      tooltip: 'Account',
      position: PopupMenuPosition.under,
      // Match the sidebar content width (280 panel − 16 padding each side).
      constraints: const BoxConstraints(minWidth: 248, maxWidth: 248),
      onSelected: (value) {
        switch (value) {
          case 'settings':
            _openSettings();
          case 'signout':
            widget.onSignOut();
          case 'signin':
            widget.onSignIn?.call();
        }
      },
      itemBuilder: (context) => [
        const PopupMenuItem(
          value: 'settings',
          child: ListTile(
            dense: true,
            contentPadding: EdgeInsets.zero,
            leading: Icon(Icons.settings_outlined),
            title: Text('Settings'),
          ),
        ),
        if (widget.session != null)
          const PopupMenuItem(
            value: 'signout',
            child: ListTile(
              dense: true,
              contentPadding: EdgeInsets.zero,
              leading: Icon(Icons.logout),
              title: Text('Sign out'),
            ),
          )
        else if (widget.onSignIn != null)
          const PopupMenuItem(
            value: 'signin',
            child: ListTile(
              dense: true,
              contentPadding: EdgeInsets.zero,
              leading: Icon(Icons.login),
              title: Text('登录云端'),
            ),
          ),
      ],
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: 6, horizontal: 4),
        child: Row(
          children: [
            CircleAvatar(
              radius: 16,
              backgroundColor: const Color(0xFFE2E8F0),
              child: Text(
                initial,
                style: const TextStyle(
                  fontSize: 14,
                  fontWeight: FontWeight.w600,
                  color: Color(0xFF334155),
                ),
              ),
            ),
            const SizedBox(width: 10),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                mainAxisSize: MainAxisSize.min,
                children: [
                  Text(
                    name,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(
                      fontSize: 14,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                  if (user?.email != null)
                    Text(
                      user!.email,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: const TextStyle(
                        fontSize: 12,
                        color: Color(0xFF64748B),
                      ),
                    ),
                ],
              ),
            ),
            const Icon(Icons.more_vert, size: 18, color: Color(0xFF94A3B8)),
          ],
        ),
      ),
    );
  }

  Widget _pageTree(BuildContext context, bool canEdit) {
    if (widget.selectedWorkspace == null) {
      return const EmptyState(
        icon: Icons.ads_click,
        title: 'Select workspace',
        detail: 'Pages appear after selecting a workspace.',
      );
    }

    if (widget.views.isEmpty) {
      return const EmptyState(
        icon: Icons.note_add,
        title: 'No pages',
        detail: 'Create a page to start writing.',
      );
    }

    return ListView(
      children: _visibleDocumentTree().map((item) {
        final row = DocumentListItem(
          key: ValueKey(item.view.id),
          view: item.view,
          depth: item.depth,
          hasChildren: item.hasChildren,
          revealToggle: _navHovered,
          isCollapsed: !_expandedViewIds.contains(item.view.id),
          isSelected: item.view.id == widget.selectedView?.id,
          canEdit: canEdit,
          isRenaming: item.view.id == _renamingViewId,
          onToggle: () => _toggleViewExpand(item.view),
          onPressed: () => _navigateToView(item.view),
          onCreateChild: () {
            setState(() => _expandForChildOf(item.view.id)); // reveal the new child
            _createThenRename(
              () => widget.onCreateChildDocument(item.view, kUntitledPage),
            );
          },
          onCreateChildFolder: () {
            setState(() => _expandForChildOf(item.view.id));
            _createThenRename(
              () => widget.onCreateChildFolder(item.view, '新文件夹'),
            );
          },
          onRename: () => _beginRename(item.view),
          onRenameSubmit: (name) => _commitRename(item.view, name),
          onRenameCancel: _cancelRename,
          onDelete: () => widget.onDeleteView(item.view),
        );
        if (!canEdit) {
          return Padding(padding: const EdgeInsets.only(bottom: 8), child: row);
        }
        return _draggableTreeRow(item.view, row);
      }).toList(),
    );
  }

  /// Wrap a page row so it can be dragged to reorder among its siblings (its
  /// subtree follows, since children render under their parent). Press and
  /// move to start dragging — Draggable fires as soon as the pointer clears
  /// touch slop, while a motionless click still opens the page. (Long-press
  /// felt broken with a mouse: moving during the 500ms hold cancels it.)
  /// The top/bottom half of each sibling row is a drop slot (before/after).
  Widget _draggableTreeRow(DocumentView view, Widget row) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 8),
      child: Draggable<DocumentView>(
        data: view,
        dragAnchorStrategy: pointerDragAnchorStrategy,
        onDragStarted: () => setState(() => _draggingTree = true),
        onDragEnd: (_) => setState(() => _draggingTree = false),
        onDraggableCanceled: (_, _) => setState(() => _draggingTree = false),
        onDragCompleted: () => setState(() => _draggingTree = false),
        feedback: Material(
          elevation: 6,
          borderRadius: BorderRadius.circular(6),
          child: Container(
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
            constraints: const BoxConstraints(maxWidth: 260),
            decoration: BoxDecoration(
              color: Colors.white,
              borderRadius: BorderRadius.circular(6),
            ),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                const Icon(
                  Icons.description_outlined,
                  size: 18,
                  color: Color(0xFF2563EB),
                ),
                const SizedBox(width: 8),
                Flexible(
                  child: Text(view.name, overflow: TextOverflow.ellipsis),
                ),
              ],
            ),
          ),
        ),
        childWhenDragging: Opacity(opacity: 0.4, child: row),
        child: Stack(
          children: [
            row,
            // Drop zones exist only during a drag, so they never block taps.
            if (_draggingTree)
              Positioned.fill(
                child: Column(
                  children: [
                    Expanded(flex: 3, child: _dropSlot(view, _DropMode.before)),
                    Expanded(flex: 4, child: _dropSlot(view, _DropMode.into)),
                    Expanded(flex: 3, child: _dropSlot(view, _DropMode.after)),
                  ],
                ),
              ),
          ],
        ),
      ),
    );
  }

  Widget _dropSlot(DocumentView target, _DropMode mode) {
    return DragTarget<DocumentView>(
      // The whole zone must be droppable; DragTarget defaults to deferToChild,
      // which would limit the hit area to the thin indicator. These overlays
      // only exist during a drag, so opaque is safe (no taps to intercept).
      hitTestBehavior: HitTestBehavior.opaque,
      onWillAcceptWithDetails: (details) =>
          !_isSelfOrDescendant(target.id, details.data.id) &&
          _parentAllowsChildren(_dropParentId(target, mode)),
      onAcceptWithDetails: (details) => _handleDrop(details.data, target, mode),
      builder: (context, candidate, rejected) {
        final active = candidate.isNotEmpty;
        if (mode == _DropMode.into) {
          // Nesting: highlight the whole target row.
          return Container(
            decoration: BoxDecoration(
              borderRadius: BorderRadius.circular(6),
              border: Border.all(
                color: active ? const Color(0xFF2563EB) : Colors.transparent,
                width: 2,
              ),
              color: active ? const Color(0x142563EB) : Colors.transparent,
            ),
          );
        }
        return Align(
          alignment: mode == _DropMode.before
              ? Alignment.topCenter
              : Alignment.bottomCenter,
          child: Container(
            height: 2,
            color: active ? const Color(0xFF2563EB) : Colors.transparent,
          ),
        );
      },
    );
  }

  /// True when [candidateId] is [rootId] itself or anywhere inside its subtree —
  /// dropping there would create a cycle, so it must be rejected.
  bool _isSelfOrDescendant(String candidateId, String rootId) {
    if (candidateId == rootId) {
      return true;
    }
    final parents = {for (final v in widget.views) v.id: v.parentViewId};
    String? cursor = candidateId;
    final seen = <String>{};
    while (cursor != null && seen.add(cursor)) {
      final parent = parents[cursor];
      if (parent == rootId) {
        return true;
      }
      cursor = parent;
    }
    return false;
  }

  /// The view a drop would reparent the dragged item under (null = workspace
  /// root): `into` nests under the target; before/after makes it the target's
  /// sibling, i.e. under the target's own parent.
  String? _dropParentId(DocumentView target, _DropMode mode) =>
      mode == _DropMode.into ? target.id : target.parentViewId;

  bool _parentAllowsChildren(String? parentId) =>
      canNestUnder(widget.views, parentId);

  void _handleDrop(DocumentView dragged, DocumentView target, _DropMode mode) {
    if (mode == _DropMode.into) {
      final children =
          widget.views
              .where((v) => v.parentViewId == target.id && v.id != dragged.id)
              .toList()
            ..sort((a, b) => a.position.compareTo(b.position));
      children.add(dragged);
      setState(() => _expandForChildOf(target.id)); // reveal the drop target
      widget.onReorderViews(target.id, children);
      return;
    }

    final parentId = target.parentViewId;
    final siblings =
        widget.views
            .where((v) => v.parentViewId == parentId && v.id != dragged.id)
            .toList()
          ..sort((a, b) => a.position.compareTo(b.position));
    final targetIndex = siblings.indexWhere((v) => v.id == target.id);
    if (targetIndex < 0) {
      return;
    }
    siblings.insert(
      mode == _DropMode.before ? targetIndex : targetIndex + 1,
      dragged,
    );
    widget.onReorderViews(parentId, siblings);
  }

  List<({DocumentView view, int depth, bool hasChildren})>
  _visibleDocumentTree() {
    final childrenByParent = <String?, List<DocumentView>>{};
    for (final view in widget.views) {
      childrenByParent.putIfAbsent(view.parentViewId, () => []).add(view);
    }

    for (final children in childrenByParent.values) {
      children.sort((left, right) => left.position.compareTo(right.position));
    }

    final ordered = <({DocumentView view, int depth, bool hasChildren})>[];
    final visited = <String>{};

    void appendChildren(String? parentId, int depth) {
      final children = childrenByParent[parentId] ?? const <DocumentView>[];
      for (final child in children) {
        if (!visited.add(child.id)) {
          continue;
        }

        final hasChildren = (childrenByParent[child.id] ?? const []).isNotEmpty;
        ordered.add((view: child, depth: depth, hasChildren: hasChildren));
        if (_expandedViewIds.contains(child.id)) {
          appendChildren(child.id, depth + 1);
        }
      }
    }

    appendChildren(null, 0);
    // Surface genuine orphans (parent missing) at the top level. A node that is
    // unvisited only because an ancestor is collapsed must stay hidden — not be
    // dumped at depth 0 (which made new children of a collapsed parent appear as
    // siblings of it).
    final viewIds = {for (final view in widget.views) view.id};
    for (final view in widget.views) {
      if (visited.contains(view.id)) {
        continue;
      }
      final parentId = view.parentViewId;
      if (parentId != null && viewIds.contains(parentId)) {
        continue; // hidden under a collapsed ancestor
      }
      if (visited.add(view.id)) {
        final hasChildren = (childrenByParent[view.id] ?? const []).isNotEmpty;
        ordered.add((view: view, depth: 0, hasChildren: hasChildren));
      }
    }

    return ordered;
  }

  void _toggleViewExpand(DocumentView view) {
    setState(() {
      if (!_expandedViewIds.add(view.id)) {
        _expandedViewIds.remove(view.id);
      }
      _saveExpanded();
    });
  }

  /// Pref key for the active workspace's expanded set. Per-workspace so each
  /// remembers its own shape (node ids are only unique within a workspace).
  String? get _expandedPrefKey {
    final wsId = widget.selectedWorkspace?.id;
    return wsId == null ? null : 'sidebar.expandedIds.$wsId';
  }

  /// Load the persisted expanded set for the active workspace. Absent/garbage →
  /// empty (all collapsed). No stale filter here — [widget.views] may not be
  /// loaded yet on first build, and stale ids are harmless (a deleted node never
  /// renders); pruning happens on save when the tree is populated.
  void _loadExpanded() {
    _expandedViewIds.clear();
    final key = _expandedPrefKey;
    if (key == null) return;
    final raw = loadPref(key);
    if (raw == null || raw.isEmpty) return;
    try {
      _expandedViewIds.addAll((jsonDecode(raw) as List).cast<String>());
    } catch (_) {
      // corrupt value → treat as none expanded
    }
  }

  void _saveExpanded() {
    final key = _expandedPrefKey;
    if (key == null) return;
    // Prune ids of deleted nodes now that the tree is loaded, so the blob can't
    // grow forever (mirrors AppFlowy's remove-on-collapse without a delete hook).
    if (widget.views.isNotEmpty) {
      final live = {for (final v in widget.views) v.id};
      _expandedViewIds.removeWhere((id) => !live.contains(id));
    }
    savePref(key, jsonEncode(_expandedViewIds.toList()));
  }

  /// Expand every ANCESTOR of [id] so a (possibly nested) node is revealed in
  /// the sidebar — used on navigate/create so the active/new page is never
  /// hidden under a collapsed parent (AppFlowy/Notion "reveal current page").
  /// The node itself is not expanded (that would show ITS children). Returns
  /// whether anything changed.
  bool _revealAncestors(String id) {
    var changed = false;
    for (final a in ancestorIds(widget.views, id)) {
      if (_expandedViewIds.add(a)) changed = true;
    }
    return changed;
  }

  /// Expand [id] itself (so a freshly-created/dropped child under it is visible)
  /// plus its ancestor chain, and persist. Call inside setState.
  void _expandForChildOf(String id) {
    _expandedViewIds.add(id);
    _revealAncestors(id);
    _saveExpanded();
  }

  void _schedulePageTitleSave() {
    _pageTitleSaveTimer?.cancel();
    _pageTitleSaveTimer = Timer(const Duration(milliseconds: 700), () {
      final bootstrap = widget.selectedBootstrap;
      final title = _pageTitle.text.trim();
      if (bootstrap == null || title.isEmpty || title == bootstrap.view.name) {
        return;
      }

      widget.onRenameView(bootstrap.view, title);
    });
  }

  Widget _editorPane(BuildContext context) {
    final workspace = widget.selectedWorkspace;
    if (workspace == null) {
      return const EmptyState(
        icon: Icons.ads_click,
        title: 'Select a workspace',
        detail: 'Choose a workspace and open a page from the document tree.',
      );
    }

    final bootstrap = widget.selectedBootstrap;
    if (bootstrap == null) {
      return const EmptyState(
        icon: Icons.description_outlined,
        title: 'Select a page',
        detail: 'Open a page from the document tree to edit it.',
      );
    }

    final canEdit = matchesEditRole(workspace.role);

    return ColoredBox(
      color: Colors.white,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (widget.showFormatBar && canEdit) _formatBar(context),
          Expanded(
            child: LayoutBuilder(
              builder: (context, constraints) {
                // Track the realizable column width (pane minus the 28px scroll
                // padding on each side) so the page-width slider's full travel
                // maps to achievable widths. Plain assignment — no setState, so
                // no rebuild loop; read lazily when Settings opens.
                if (constraints.maxWidth.isFinite) {
                  _editorAvailWidth = (constraints.maxWidth - 56).clamp(
                    768.0,
                    2400.0,
                  );
                }
                return _editorScroll(context, canEdit, bootstrap);
              },
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
  Widget _formatBar(BuildContext context) {
    Widget btn(IconData icon, String tip, VoidCallback onTap) {
      return Tooltip(
        message: tip,
        waitDuration: const Duration(milliseconds: 500),
        child: InkWell(
          borderRadius: BorderRadius.circular(6),
          onTap: onTap,
          child: Padding(
            padding: const EdgeInsets.all(6),
            child: Icon(icon, size: 18, color: const Color(0xFF475569)),
          ),
        ),
      );
    }

    Widget divider() => Container(
      width: 1,
      height: 18,
      margin: const EdgeInsets.symmetric(horizontal: 4),
      color: const Color(0xFFE2E8F0),
    );

    final h = _commandHook;
    return Container(
      decoration: const BoxDecoration(
        color: Colors.white,
        border: Border(bottom: BorderSide(color: Color(0xFFE2E8F0))),
      ),
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 3),
      child: SingleChildScrollView(
        scrollDirection: Axis.horizontal,
        child: Row(
          children: [
            btn(Icons.undo, 'Undo (Ctrl+Z)', h.undo),
            btn(Icons.redo, 'Redo (Ctrl+Y)', h.redo),
            divider(),
            btn(Icons.notes, 'Text', () => h.setBlock('paragraph')),
            btn(
              Icons.looks_one_outlined,
              'Heading 1',
              () => h.setBlock('heading', {'level': 1}),
            ),
            btn(
              Icons.looks_two_outlined,
              'Heading 2',
              () => h.setBlock('heading', {'level': 2}),
            ),
            btn(
              Icons.looks_3_outlined,
              'Heading 3',
              () => h.setBlock('heading', {'level': 3}),
            ),
            divider(),
            btn(Icons.format_bold, 'Bold (Ctrl+B)', () => h.toggleMark('bold')),
            btn(
              Icons.format_italic,
              'Italic (Ctrl+I)',
              () => h.toggleMark('italic'),
            ),
            btn(
              Icons.format_strikethrough,
              'Strikethrough',
              () => h.toggleMark('strike'),
            ),
            btn(Icons.code, 'Inline code (Ctrl+E)', () => h.toggleMark('code')),
            btn(Icons.link, 'Link (Ctrl+K)', h.editLink),
            divider(),
            btn(
              Icons.format_list_bulleted,
              'Bulleted list',
              () => h.setBlock('bulleted_list'),
            ),
            btn(
              Icons.format_list_numbered,
              'Numbered list',
              () => h.setBlock('numbered_list'),
            ),
            btn(
              Icons.check_box_outlined,
              'To-do list',
              () => h.setBlock('todo', {'checked': false}),
            ),
            btn(Icons.format_quote, 'Quote', () => h.setBlock('quote')),
            btn(Icons.terminal, 'Code block', () => h.setBlock('code_block')),
            divider(),
            btn(Icons.horizontal_rule, 'Divider', () => h.insert('divider')),
            btn(Icons.grid_on, 'Table', () => h.insert('table')),
            btn(Icons.image_outlined, 'Image', () => h.insert('image')),
          ],
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
      child: SingleChildScrollView(
        padding: const EdgeInsets.symmetric(horizontal: 28, vertical: 28),
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
                          child: Focus(
                            // Intercepts keys bubbling from the title field:
                            // ArrowDown moves into the first body line.
                            canRequestFocus: false,
                            skipTraversal: true,
                            onKeyEvent: (node, event) {
                              if (event is KeyDownEvent &&
                                  event.logicalKey ==
                                      LogicalKeyboardKey.arrowDown) {
                                _commandHook.focusFirstLine();
                                return KeyEventResult.handled;
                              }
                              return KeyEventResult.ignored;
                            },
                            child: TextField(
                              controller: _pageTitle,
                              focusNode: _pageTitleFocus,
                              style: Theme.of(context).textTheme.headlineMedium,
                              textInputAction: TextInputAction.next,
                              onChanged: (_) => _schedulePageTitleSave(),
                              // Enter in the title: the text after the caret
                              // (or nothing) becomes a NEW first body line,
                              // pushing the body down. onEditingComplete (not
                              // onSubmitted) — it REPLACES the default
                              // TextInputAction.next finalize, which would
                              // otherwise nextFocus() away from the editor.
                              onEditingComplete: _titleEnter,
                              decoration: const InputDecoration(
                                hintText: kUntitledPage,
                                border: InputBorder.none,
                              ),
                            ),
                          ),
                        ),
                      PopupMenuButton<String>(
                        tooltip: 'Page menu',
                        icon: const Icon(Icons.expand_more),
                        onSelected: _onPageMenu,
                        itemBuilder: (context) => [
                          const PopupMenuItem(
                            value: 'export-md',
                            child: ListTile(
                              dense: true,
                              contentPadding: EdgeInsets.zero,
                              leading: Icon(Icons.download_outlined),
                              title: Text('Export Markdown'),
                            ),
                          ),
                          const PopupMenuItem(
                            value: 'export-zip',
                            child: ListTile(
                              dense: true,
                              contentPadding: EdgeInsets.zero,
                              leading: Icon(Icons.folder_zip_outlined),
                              title: Text('Export ZIP (with images)'),
                            ),
                          ),
                          const PopupMenuDivider(),
                          const PopupMenuItem(
                            value: 'import-md',
                            child: ListTile(
                              dense: true,
                              contentPadding: EdgeInsets.zero,
                              leading: Icon(Icons.upload_file_outlined),
                              title: Text('Import Markdown…'),
                            ),
                          ),
                          if (widget.onRestoreCheckpoint != null) ...[
                            const PopupMenuDivider(),
                            const PopupMenuItem(
                              value: 'restore-checkpoint',
                              child: ListTile(
                                dense: true,
                                contentPadding: EdgeInsets.zero,
                                leading: Icon(Icons.restore_outlined),
                                title: Text('Restore last checkpoint'),
                              ),
                            ),
                          ],
                        ],
                      ),
                      const SizedBox(width: 4),
                      IconButton(
                        tooltip: _toolsExpanded
                            ? 'Hide side panel'
                            : 'Show side panel',
                        onPressed: () =>
                            setState(() => _toolsExpanded = !_toolsExpanded),
                        icon: Icon(
                          _toolsExpanded
                              ? Icons.chevron_right
                              : Icons.chevron_left,
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
                Padding(
                  key: _editorSurfaceKey,
                  padding: const EdgeInsets.only(top: 4),
                  child: MicaEditor(
                    key: ValueKey('${bootstrap.document.id}#${widget.editorEpoch}'),
                    focusNode: _editorFocus,
                    rootBlockId: bootstrap.document.rootBlockId,
                    nodes: [
                      for (final b in bootstrap.childBlocks)
                        EditorNode(
                          id: b.id,
                          kind: b.kind,
                          text: b.text,
                          data: Map<String, dynamic>.from(b.data),
                        ),
                    ],
                    version: bootstrap.snapshot.versionSeq,
                    canEdit: canEdit,
                    onSelectionChanged: widget.onCursorChanged,
                    remoteCursors: [
                      for (final p in widget.presence)
                        if (p.hasCursor)
                          (
                            blockId: p.cursorBlockId!,
                            offset: p.cursorOffset!,
                            color: p.color,
                            label: p.name,
                          ),
                    ],
                    onApplyOperations: widget.onApplyOperations,
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
                    onExitTop: () {
                      if (!widget.showPageTitle) return;
                      _pageTitleFocus.requestFocus();
                      // The web TextField select-alls when focused
                      // programmatically; that happens in the focus
                      // microtask, so queue ours right behind it — the
                      // caret is collapsed before the next frame paints
                      // (a post-frame callback here flashed the
                      // selection for one frame).
                      Future.microtask(() {
                        if (!mounted) return;
                        _pageTitle.selection = TextSelection.collapsed(
                          offset: _pageTitle.text.length,
                        );
                      });
                    },
                    appearance: widget.appearance,
                    onOpenPage: _openPageLink,
                    pageLinks: () => [
                      for (final v in widget.views)
                        if (v.objectType == 'document')
                          PageLinkTarget(id: v.id, title: v.name),
                    ],
                  ),
                ),
                if (widget.selectedMarkdown != null) ...[
                  const SizedBox(height: 28),
                  Text(
                    'Markdown',
                    style: Theme.of(context).textTheme.titleLarge,
                  ),
                  const SizedBox(height: 12),
                  DecoratedBox(
                    decoration: BoxDecoration(
                      color: Colors.white,
                      borderRadius: BorderRadius.circular(8),
                      border: Border.all(color: const Color(0xFFE2E8F0)),
                    ),
                    child: Padding(
                      padding: const EdgeInsets.all(16),
                      child: SelectableText(widget.selectedMarkdown!),
                    ),
                  ),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }

  /// Tappable outline entries for the current page's headings (no section
  /// header). Tapping scrolls the editor to that heading. Fed the LIVE heading
  /// list from [_outlineHook] (republished by the editor on every edit), so it
  /// tracks typing — not just navigation.
  List<Widget> _pageOutlineItems(
    BuildContext context,
    List<OutlineEntry> headings,
  ) {
    return [
      for (final h in headings)
        if (h.text.trim().isNotEmpty)
          InkWell(
            onTap: () => _scrollHook.scrollToBlock(h.id),
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
                  color: const Color(0xFF334155),
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
  Widget _workspaceTools(BuildContext context) {
    return ListenableBuilder(
      listenable: _outlineHook,
      builder: (context, _) {
        final outline = _pageOutlineItems(context, _outlineHook.headings);
        return ColoredBox(
          color: Colors.white,
          child: outline.isEmpty
              ? const EmptyState(
                  icon: Icons.toc,
                  title: 'Outline',
                  detail: 'Headings in this page appear here.',
                )
              : SingleChildScrollView(
                  padding: const EdgeInsets.all(16),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Row(
                        children: [
                          const Icon(Icons.toc, size: 18),
                          const SizedBox(width: 8),
                          Text(
                            'Outline',
                            style: Theme.of(context).textTheme.titleLarge,
                          ),
                        ],
                      ),
                      const SizedBox(height: 8),
                      ...outline,
                    ],
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
    await showDialog<void>(
      context: context,
      builder: (dialogContext) => StatefulBuilder(
        builder: (dialogContext, setLocal) {
          final ws = widget.selectedWorkspace ?? workspace;
          final canManage = matchesManageRole(ws.role);
          final members = widget.members;
          return AlertDialog(
            title: const Text('Workspace settings'),
            content: SizedBox(
              width: 440,
              child: SingleChildScrollView(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    DetailRow(label: 'Role', value: ws.role),
                    DetailRow(label: 'ID', value: ws.id),
                    const SizedBox(height: 16),
                    TextField(
                      controller: _rename,
                      decoration: const InputDecoration(
                        labelText: 'Rename workspace',
                        prefixIcon: Icon(Icons.edit),
                        border: OutlineInputBorder(),
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
                        label: const Text('Save'),
                      ),
                    ),
                    const SizedBox(height: 22),
                    Row(
                      children: [
                        const Icon(Icons.group, size: 18),
                        const SizedBox(width: 8),
                        Text(
                          'Members',
                          style: Theme.of(dialogContext).textTheme.titleMedium,
                        ),
                      ],
                    ),
                    const SizedBox(height: 12),
                    if (canManage) ...[
                      _addMemberForm(setLocal),
                      const SizedBox(height: 14),
                    ],
                    if (members.isEmpty)
                      const Text(
                        'No members loaded.',
                        style: TextStyle(color: Color(0xFF94A3B8)),
                      )
                    else
                      for (final member in members)
                        Padding(
                          padding: const EdgeInsets.only(bottom: 8),
                          child: MemberListItem(
                            member: member,
                            canManage: canManage,
                            canRemove: member.role != 'owner',
                            onRoleChanged: (role) async {
                              await widget.onUpdateMember(member, role);
                              setLocal(() {});
                            },
                            onRemove: () async {
                              await widget.onRemoveMember(member);
                              setLocal(() {});
                            },
                          ),
                        ),
                  ],
                ),
              ),
            ),
            actions: [
              TextButton(
                onPressed: () => Navigator.of(dialogContext).pop(),
                child: const Text('Close'),
              ),
            ],
          );
        },
      ),
    );
  }

  /// Add-member form for the settings dialog. [setLocal] rebuilds the dialog
  /// (role dropdown selection + the refreshed member list after an add).
  Widget _addMemberForm(void Function(void Function()) setLocal) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        TextField(
          controller: _memberEmail,
          keyboardType: TextInputType.emailAddress,
          decoration: const InputDecoration(
            labelText: 'Member email',
            prefixIcon: Icon(Icons.alternate_email),
            border: OutlineInputBorder(),
          ),
        ),
        const SizedBox(height: 10),
        DropdownButtonFormField<WorkspaceRole>(
          initialValue: _memberRole,
          decoration: const InputDecoration(
            labelText: 'Role',
            border: OutlineInputBorder(),
          ),
          items: WorkspaceRole.values
              .map(
                (role) =>
                    DropdownMenuItem(value: role, child: Text(role.apiValue)),
              )
              .toList(),
          onChanged: (role) {
            if (role == null) return;
            setLocal(() => _memberRole = role);
          },
        ),
        const SizedBox(height: 10),
        FilledButton.icon(
          onPressed: () async {
            await widget.onAddMember(_memberEmail.text, _memberRole);
            _memberEmail.clear();
            setLocal(() {});
          },
          icon: const Icon(Icons.person_add),
          label: const Text('Add'),
        ),
      ],
    );
  }

  Future<void> _promptCreateWorkspace() async {
    final controller = TextEditingController();
    // P3c unified create: pick the kind here. Signed-in defaults to cloud
    // (signing in expresses the collaboration intent); otherwise local. Web
    // has no local world, so the choice collapses to cloud.
    final canCloud = widget.session != null;
    final canLocal = widget.localAvailable;
    var makeLocal = canLocal && !canCloud;
    if (!canLocal) makeLocal = false;
    final result = await showDialog<({String name, bool local})>(
      context: context,
      builder: (context) {
        return StatefulBuilder(
          builder: (context, setDialogState) => AlertDialog(
            title: const Text('New workspace'),
            content: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                TextField(
                  controller: controller,
                  autofocus: true,
                  decoration: const InputDecoration(
                    labelText: 'Workspace name',
                    border: OutlineInputBorder(),
                  ),
                  onSubmitted: (value) => Navigator.of(
                    context,
                  ).pop((name: value, local: makeLocal)),
                ),
                if (canLocal) ...[
                  const SizedBox(height: 14),
                  SegmentedButton<bool>(
                    segments: [
                      const ButtonSegment(
                        value: true,
                        icon: Icon(Icons.computer_outlined),
                        label: Text('本地'),
                      ),
                      ButtonSegment(
                        value: false,
                        icon: const Icon(Icons.cloud_outlined),
                        label: const Text('云端'),
                        enabled: canCloud,
                      ),
                    ],
                    selected: {makeLocal},
                    onSelectionChanged: (sel) =>
                        setDialogState(() => makeLocal = sel.first),
                  ),
                  if (!canCloud) ...[
                    const SizedBox(height: 6),
                    const Text(
                      '登录后可创建云端工作区。',
                      style: TextStyle(
                        fontSize: 12,
                        color: Color(0xFF94A3B8),
                      ),
                    ),
                  ],
                ],
              ],
            ),
            actions: [
              TextButton(
                onPressed: () => Navigator.of(context).pop(),
                child: const Text('Cancel'),
              ),
              FilledButton(
                onPressed: () => Navigator.of(
                  context,
                ).pop((name: controller.text, local: makeLocal)),
                child: const Text('Create'),
              ),
            ],
          ),
        );
      },
    );
    controller.dispose();

    final trimmed = result?.name.trim() ?? '';
    if (trimmed.isNotEmpty && result != null) {
      await widget.onCreateWorkspaceTyped(trimmed, local: result.local);
    }
  }

  Future<void> _promptRenameWorkspace(WorkspaceEntry entry) async {
    final workspace = entry.workspace;
    final controller = TextEditingController(text: workspace.name);
    final name = await showDialog<String>(
      context: context,
      builder: (context) {
        return AlertDialog(
          title: const Text('Rename workspace'),
          content: TextField(
            controller: controller,
            autofocus: true,
            decoration: const InputDecoration(
              labelText: 'Workspace name',
              border: OutlineInputBorder(),
            ),
            onSubmitted: (value) => Navigator.of(context).pop(value),
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.of(context).pop(),
              child: const Text('Cancel'),
            ),
            FilledButton.icon(
              onPressed: () => Navigator.of(context).pop(controller.text),
              icon: const Icon(Icons.save),
              label: const Text('Save'),
            ),
          ],
        );
      },
    );
    controller.dispose();

    final trimmed = name?.trim() ?? '';
    if (trimmed.isNotEmpty && trimmed != workspace.name) {
      await widget.onRenameEntry(entry, trimmed);
    }
  }

  // ── Inline rename (no dialog) ──────────────────────────────────────────────
  // The sidebar row renders a focused TextField for `_renamingViewId`; these
  // drive it. Renaming starts from the row's "重命名" action or automatically
  // right after creating a page/folder (see `_createThenRename`).

  void _beginRename(DocumentView view) {
    setState(() => _renamingViewId = view.id);
  }

  void _cancelRename() {
    if (_renamingViewId == null) return;
    setState(() => _renamingViewId = null);
  }

  Future<void> _commitRename(DocumentView view, String name) async {
    final trimmed = name.trim();
    if (_renamingViewId != null) setState(() => _renamingViewId = null);
    // Empty or unchanged → keep the current name (server rejects empty names).
    if (trimmed.isEmpty || trimmed == view.name) return;
    await widget.onRenameView(view, trimmed);
  }

  /// Create a page/folder via [create], then drop its new sidebar row straight
  /// into inline-rename so the user just types the name — no naming dialog.
  Future<void> _createThenRename(Future<String?> Function() create) async {
    final id = await create();
    if (id != null && mounted) setState(() => _renamingViewId = id);
  }

  Future<void> _confirmDeleteWorkspace(WorkspaceEntry entry) async {
    final workspace = entry.workspace;
    final shouldDelete = await showDialog<bool>(
      context: context,
      builder: (context) {
        return AlertDialog(
          title: const Text('Delete workspace'),
          content: Text(
            'Delete "${workspace.name}" and all of its pages? '
            'This cannot be undone.',
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.of(context).pop(false),
              child: const Text('Cancel'),
            ),
            FilledButton.icon(
              style: FilledButton.styleFrom(
                backgroundColor: const Color(0xFFDC2626),
              ),
              onPressed: () => Navigator.of(context).pop(true),
              icon: const Icon(Icons.delete_outline),
              label: const Text('Delete'),
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
      ),
    );
  }

  /// Page title menu: Markdown export/import + ZIP export.
  Future<void> _onPageMenu(String value) async {
    switch (value) {
      case 'export-md':
        await _downloadPageMarkdown();
      case 'export-zip':
        await _onExport('page-zip');
      case 'import-md':
        await _importMarkdownFile();
      case 'restore-checkpoint':
        final restore = widget.onRestoreCheckpoint;
        if (restore == null) return;
        final ok = await showDialog<bool>(
          context: context,
          builder: (context) => AlertDialog(
            title: const Text('Restore last checkpoint?'),
            content: const Text(
              'This reverts the page to its last on-device checkpoint and '
              'discards changes made since. This cannot be undone.',
            ),
            actions: [
              TextButton(
                onPressed: () => Navigator.pop(context, false),
                child: const Text('Cancel'),
              ),
              FilledButton(
                onPressed: () => Navigator.pop(context, true),
                child: const Text('Restore'),
              ),
            ],
          ),
        );
        if (ok == true) await restore();
    }
  }

  Future<void> _downloadPageMarkdown() async {
    try {
      final markdown = await widget.onExportPageMarkdown();
      final title = _pageTitle.text.trim().isEmpty
          ? 'page'
          : _pageTitle.text.trim();
      downloadImage(
        Uint8List.fromList(utf8.encode(markdown)),
        '$title.md',
        'text/markdown',
      );
    } catch (error) {
      if (mounted) {
        ScaffoldMessenger.of(
          context,
        ).showSnackBar(SnackBar(content: Text('Export failed: $error')));
      }
    }
  }

  Future<void> _importMarkdownFile() async {
    final picked = await pickTextFile();
    if (picked == null || !mounted) return;
    await widget.onImportMarkdown(picked.name, picked.text);
  }

  /// Download a whole workspace as a Markdown ZIP (page-tree folders + assets).
  Future<void> _exportWorkspaceFile(WorkspaceEntry entry) async {
    try {
      final bytes = await widget.onExportEntryZip(entry);
      final name = entry.workspace.name.trim().isEmpty
          ? 'workspace'
          : entry.workspace.name.trim();
      downloadImage(bytes, '$name.zip', 'application/zip');
    } catch (error) {
      if (mounted) {
        ScaffoldMessenger.of(
          context,
        ).showSnackBar(SnackBar(content: Text('Export failed: $error')));
      }
    }
  }

  Future<void> _onExport(String kind) async {
    if (kind == 'page-zip') {
      try {
        final bytes = await widget.onExportPageZip();
        downloadImage(bytes, 'page.zip', 'application/zip');
      } catch (error) {
        if (mounted) {
          ScaffoldMessenger.of(
            context,
          ).showSnackBar(SnackBar(content: Text('Export failed: $error')));
        }
      }
      return;
    }
    final (title, future) = switch (kind) {
      'page' => ('Export current page', widget.onExportPageMarkdown()),
      'workspace' => ('Export workspace', widget.onExportWorkspaceMarkdown()),
      _ => ('Export all workspaces', widget.onExportAllMarkdown()),
    };
    String markdown;
    try {
      markdown = await future;
    } catch (error) {
      if (mounted) {
        ScaffoldMessenger.of(
          context,
        ).showSnackBar(SnackBar(content: Text('Export failed: $error')));
      }
      return;
    }
    if (!mounted) return;
    await showDialog<void>(
      context: context,
      builder: (context) => _ExportDialog(title: title, markdown: markdown),
    );
  }

  void _openSearch() {
    showDialog<void>(
      context: context,
      builder: (context) => _SearchDialog(
        onSearch: widget.onSearch,
        onOpen: (viewId) {
          Navigator.of(context).pop();
          widget.onOpenSearchResult(viewId);
        },
      ),
    );
  }

  void _openSettings() {
    showDialog<void>(
      context: context,
      builder: (context) => _SettingsDialog(
        onLoadAiSettings: widget.onLoadAiSettings,
        onSaveAiSettings: widget.onSaveAiSettings,
        onLoadTokens: widget.onLoadTokens,
        onCreateToken: widget.onCreateToken,
        onRevokeToken: widget.onRevokeToken,
        userName: widget.userName,
        userEmail: widget.userEmail,
        onUpdateProfile: widget.onUpdateProfile,
        onChangePassword: widget.onChangePassword,
        cloudOrigin: widget.cloudOrigin,
        onConnectCloud: widget.onConnectCloud,
        activeIsLocal: widget.activeIsLocal,
        onSwitchWorld: widget.onSwitchWorld,
        localAvailable: widget.localAvailable,
        appearance: widget.appearance,
        pageWidth: widget.pageWidth,
        maxPageWidth: _editorAvailWidth,
        reHostImages: widget.reHostImages,
        onReHostImagesChanged: widget.onReHostImagesChanged,
        showFormatBar: widget.showFormatBar,
        showPageTitle: widget.showPageTitle,
        onShowPageTitleChanged: widget.onShowPageTitleChanged,
        aiEnabled: widget.aiEnabled,
        onAiEnabledChanged: widget.onAiEnabledChanged,
        onShowFormatBarChanged: widget.onShowFormatBarChanged,
        onAppearanceChanged: widget.onAppearanceChanged,
        onImportWorkspace: () => _importWorkspaceFile(fromSettings: true),
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
    await _commandHook.flush();
    if (!mounted) return;
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
    final picked = await pickImportFile(zipOnly: true);
    if (picked == null || !mounted) return;
    if (fromSettings) {
      Navigator.of(context).pop(); // close settings before the import flow runs
    }
    await widget.onImportWorkspaceZip(
      picked.name,
      picked.bytes,
      notion: notion,
    );
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
  Future<void> _importFolderIntoWorkspace(WorkspaceEntry entry) async {
    final picked = await pickImportFolder();
    if (picked.isEmpty || !mounted) return;
    final entries = <ArchiveFile>[];
    for (final f in picked) {
      // The picker includes the chosen folder itself as the first segment —
      // drop it so the folder's contents land at the workspace root.
      final parts = f.path.split('/');
      entries.add(
        ArchiveFile(
          parts.length > 1 ? parts.sublist(1).join('/') : f.path,
          f.bytes,
        ),
      );
    }
    await widget.onImportTreeIntoEntry(entry, entries);
  }

  void _openAiDialog() {
    showDialog<void>(
      context: context,
      builder: (context) => _AiDialog(
        canEdit: matchesEditRole(widget.selectedWorkspace?.role),
        hasWorkspace: widget.selectedWorkspace != null,
        onStream: widget.onAiStream,
        onNewPage: widget.onAiNewPage,
        onCurrentPage: widget.onAiCurrentPage,
        onNewWorkspace: widget.onAiNewWorkspace,
      ),
    );
  }
}
