import 'dart:convert';
import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:http/http.dart' as http;

import 'l10n/app_localizations.dart';
import 'l10n/locale_controller.dart';

import 'cloud/cloud_sync.dart';
import 'cloud/doc_store_platform.dart';
import 'cloud/pending_uploads.dart';
import 'cloud/workspace_migration.dart';
import 'diagnostics.dart';
import 'swallowed.dart';
import 'local/local_offline.dart';
import 'editor/word_count.dart';
import 'web/yjs_probe.dart';
import 'editor/clipboard_copy.dart' show copyTextToClipboard;
import 'editor/model.dart' show kMonoFont;
import 'editor/editor.dart';
import 'editor/image_actions.dart';
import 'editor/open_url.dart';
import 'editor/pick_file.dart';
import 'editor/property_panel.dart';
import 'widgets/mica_logo.dart';
import 'editor/pick_image.dart';
import 'ui/auth_form.dart';
import 'ui/autoscroll.dart';
import 'ui/avatar_url.dart';
import 'ui/comment_panel.dart';
import 'ui/copy_button.dart';
import 'ui/doc_tab_strip.dart';
import 'ui/destructive_confirm.dart';
import 'ui/dialog_controllers.dart';
import 'ui/emoji_picker.dart';
import 'ui/format_bytes.dart';
import 'ui/home_data.dart' show RelativeTimeStrings, countPages, relativeMeta;
import 'ui/page_graph_view.dart';
import 'ui/home_pane.dart';
import 'ui/overview_pane.dart';
import 'ui/panel_kit.dart';
import 'ui/rename.dart';
import 'ui/search_data.dart';
import 'ui/sign_in_hero.dart';
import 'ui/sign_in_screen.dart';
import 'ui/sign_in_pane.dart';
import 'ui/status_kit.dart';
import 'ui/theme_tokens.dart';
import 'ui/trash_data.dart';
import 'ui/user_avatar.dart';
import 'ui/version_data.dart';
import 'ui/workspace_overview.dart' show WorkspaceOverviewMode;
import 'local/cache_stats.dart' show LocalCacheStats;
import 'cjk_fonts.dart';
import 'doc_tab.dart';
import 'prefs.dart';
import 'theme_controller.dart';
import 'server_list.dart';
import 'updater.dart';
import 'window_setup.dart';
import 'upload/zip_writer.dart';
import 'api/client.dart';
import 'api/models.dart';
import 'api/profile_watch.dart';
import 'api/session_refresher.dart';
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

/// The cloud server a fresh install starts with — EMPTY on purpose.
///
/// A shipped binary must not carry anyone's server address: baking one in means
/// every downloaded client points at that host on first launch and can sign up
/// against it, which is somebody's private deployment, not a default. So a new
/// install starts in local mode with no server configured, and the user adds
/// one through "添加服务器…".
///
/// Anyone running a hosted flavour can bake their own in at build time with
/// `--dart-define=MICA_CLOUD_URL=https://…`; the public release sets nothing.
const String kDefaultCloudUrl = String.fromEnvironment('MICA_CLOUD_URL');

/// App version, shown in the About dialog. Keep in sync with `pubspec.yaml`
/// (`version:`) and `crates/api-server/Cargo.toml` on each release.
const String kAppVersion = '0.13.17';

/// Editor page (content column) width, as 11 discrete steps like AppFlowy —
/// `680 · 800 · … · 1880` (min + max + 10 divisions, step 120). The DEFAULT and
/// the Settings "reset" both go to [kPageWidthDefault] = 800px, the readable
/// column width the note-app field converges on (Obsidian 700, Notion ~708,
/// AFFiNE ~720-800) — NOT AppFlowy's reset-to-full-width. See research 2026-07.
const double kPageWidthMin = 680;
const double kPageWidthMax = 1880;
const double kPageWidthDefault = 800;
const int kPageWidthDivisions = 10;

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
        cloudOrigin: savedUrl.isEmpty ? kDefaultCloudUrl : onlineUrl,
        activeOrigin: 'local',
      );
    case 'cloud':
      // Legacy: a fixed "Mica Cloud" preset that carried no URL of its own.
      // There is no built-in host to resolve it to any more, so fall back to
      // whatever URL the install had (empty on a build with no
      // MICA_CLOUD_URL) and start local. Such a user re-adds their server once
      // — better than shipping someone's address to everyone to avoid it.
      return kDefaultCloudUrl.isEmpty
          ? (
              cloudOrigin: savedUrl.isEmpty ? '' : onlineUrl,
              activeOrigin: 'local',
            )
          : (cloudOrigin: kDefaultCloudUrl, activeOrigin: kDefaultCloudUrl);
    case 'online':
    case 'self': // legacy self-hosted → same thing, keep its URL
      return (cloudOrigin: onlineUrl, activeOrigin: onlineUrl);
    default:
      final usedOnlineBefore = authToken.isNotEmpty || savedUrl.isNotEmpty;
      return (isWeb || usedOnlineBefore)
          ? (cloudOrigin: onlineUrl, activeOrigin: onlineUrl)
          // Fresh desktop install: local-first, and NO server preconfigured.
          : (cloudOrigin: kDefaultCloudUrl, activeOrigin: 'local');
  }
}

void main() {
  // Run the whole app inside a guarded zone so an uncaught async error can't
  // vanish without a trace. Binding init + runApp MUST share this zone (Flutter
  // asserts they match), so they live inside the callback. Every fault sink here
  // routes to the diagnostics crash log, which is deliberately NOT gated by the
  // "诊断" toggle (a crash can't be armed for in advance) and is a no-op on web.
  runZonedGuarded(
    () async {
      WidgetsFlutterBinding.ensureInitialized();
      // Framework errors (build/layout/paint/gesture): log then keep the default
      // console dump so nothing that worked before is lost.
      final priorOnError = FlutterError.onError;
      FlutterError.onError = (details) {
        logCrash(
          'FlutterError: ${details.exceptionAsString()}\n${details.stack}',
        );
        priorOnError?.call(details);
      };
      // Errors the engine dispatches outside the zone (some platform callbacks).
      // Return false → still propagate to the zone/default handler (console).
      WidgetsBinding.instance.platformDispatcher.onError = (error, stack) {
        logCrash('PlatformError: $error\n$stack');
        return false;
      };
      // Desktop: restore window size/position + enforce a min size before the first
      // frame (no-op on web/mobile). Awaited so the window is ready before runApp.
      await initDesktopWindow();
      // Seed the UI language and the palette from the persisted choices (prefs are
      // file/localStorage backed and loaded synchronously) before the first frame
      // renders. Both MUST happen here rather than from a widget's initState —
      // see loadPersistedThemeMode for what that looked like when it didn't.
      loadPersistedLocale();
      loadPersistedThemeMode();
      // Suppress the browser's native right-click menu so the editor can show its
      // own (e.g. image actions) on web.
      if (kIsWeb) BrowserContextMenu.disableContextMenu();
      // Web: register the yjs CRDT self-test hook (no-op off web). P2-M4 W1.
      registerYjsSelfTest();
      _warmUpFonts();
      runApp(const MicaApp());
    },
    (error, stack) {
      // Last-resort net for uncaught async errors anywhere in the zone.
      logCrash('Uncaught: $error\n$stack');
    },
  );
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
    // Rebuild MaterialApp when the UI language changes (Settings → localeController).
    // `locale: null` means follow the system, resolved against supportedLocales.
    return ValueListenableBuilder<Locale?>(
      valueListenable: localeController,
      builder: (context, locale, _) => ValueListenableBuilder<MicaThemeMode>(
        valueListenable: themeModeController,
        builder: (context, themeMode, _) => MaterialApp(
          title: 'Mica',
          // The window's close listener lives outside the widget tree and has no
          // context of its own — this is how it reaches a Navigator to ask whether
          // X should quit or minimise. No-op on web.
          navigatorKey: appNavigatorKey,
          debugShowCheckedModeBanner: false,
          locale: locale,
          localizationsDelegates: const [
            AppLocalizations.delegate,
            GlobalMaterialLocalizations.delegate,
            GlobalWidgetsLocalizations.delegate,
            GlobalCupertinoLocalizations.delegate,
          ],
          supportedLocales: kSupportedLocales,
          // Both palettes go in and `themeMode` decides, so following the OS is
          // Flutter's job rather than a brightness we resolve ourselves.
          theme: MicaTokens.light.toMaterialTheme().copyWith(
            // Crisp system CJK fonts on desktop (Windows 微软雅黑 / macOS 苹方 /
            // Linux Noto CJK); the bundled font is the tail + web's only option.
            textTheme: ThemeData(fontFamilyFallback: cjkFontFallback).textTheme,
          ),
          darkTheme: MicaTokens.dark_.toMaterialTheme().copyWith(
            textTheme: ThemeData(
              brightness: Brightness.dark,
              fontFamilyFallback: cjkFontFallback,
            ).textTheme,
          ),
          themeMode: themeMode.material,
          // Inside the builder, `Theme.of` reports which of the two Flutter
          // actually picked — so the tokens the rest of the app reads can never
          // disagree with the Material theme around them.
          builder: (context, child) => MicaTheme(
            tokens: tokensForBrightness(Theme.of(context).brightness),
            child: child ?? const SizedBox.shrink(),
          ),
          home: const WorkspaceShell(),
        ),
      ),
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

  AuthSession? _session;
  List<Workspace> _workspaces = const [];
  Map<String, List<WorkspaceMember>> _membersByWorkspace = const {};
  Map<String, List<DocumentView>> _viewsByWorkspace = const {};
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
  set _selectedBootstrap(DocumentBootstrap? value) =>
      _activeTab.bootstrap = value;

  String? _selectedMarkdown;
  String? _message;
  bool _isBusy = false;

  /// Pages finished / pages planned for a running cloud import, straight from
  /// the job status. Null when no import is in flight — [_isBusy] alone can't
  /// carry this, since it's true for every operation and says nothing about size.
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
    _loadPrefs();
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
  List<String> _loadServers(String seed) =>
      knownServers(rawPref: loadPref('servers'), seed: seed);

  void _saveServers() => savePref('servers', jsonEncode(_servers));

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
    final parsed = Uri.tryParse(url.trim());
    if (parsed == null || !parsed.hasScheme || parsed.host.isEmpty) {
      return context.l10n.serverInvalidUrl;
    }
    final normalized = parsed.toString();
    if (_servers.contains(normalized)) {
      return context.l10n.serverAlreadyAdded;
    }
    setState(() => _servers = [..._servers, normalized]);
    _saveServers();
    return null;
  }

  /// Forget a server completely: its credentials AND its on-device mirror.
  ///
  /// "删除就删除干净,反正 server 云端还有数据在" — true for anything that
  /// reached the server, which is why we push first: [drainOutbox] gives the
  /// pending edits their last chance while we still have a session. What can't
  /// be pushed (offline, server gone) only ever existed here, so the caller
  /// must have said so out loud before we get here.
  Future<void> _removeServer(String origin) async {
    if (!_servers.contains(origin)) return;
    if (_activeOrigin == origin) {
      await _cloudSession?.drainOutbox(timeout: const Duration(seconds: 4));
      await _setActiveConnection('local');
      if (!mounted) return;
    }
    setState(() => _servers = _servers.where((s) => s != origin).toList());
    _saveServers();
    // Credentials, then the mirror. Both are keyed by origin, so this is exact
    // — no other server's data can be caught by it.
    savePref('authToken:$origin', '');
    savePref('authUser:$origin', '');
    savePref('refreshToken:$origin', '');
    _local.forgetOrigin(origin);
    // The app must always point at a server that exists. Removing the LAST one
    // leaves none configured — fall back to the build default (empty in public
    // builds), and never invent a server to replace the one just deleted.
    if (_cloudOrigin == origin) {
      final next = _servers.firstOrNull ?? kDefaultCloudUrl;
      if (next.isNotEmpty && !_servers.contains(next)) {
        setState(() => _servers = [next]);
        _saveServers();
      }
      setState(() => _cloudOrigin = next);
      savePref('cloudOrigin', next);
      final base = next.isEmpty ? null : Uri.tryParse(next);
      if (base != null) _api.baseUri = base;
    }
  }

  Future<void> _connectCloudServer(String url) async {
    final parsed = Uri.tryParse(url.trim());
    if (parsed == null || !parsed.hasScheme || parsed.host.isEmpty) return;
    final normalized = parsed.toString();
    if (normalized == _cloudOrigin) {
      // Nothing to rebind — which never meant nothing to do. 本地模式 drops the
      // session but leaves _cloudOrigin pointing here, so coming back lands on
      // this branch with credentials sitting in prefs and no session to show
      // for them. Returning early here is what made cloud → 本地模式 → cloud
      // ask for a password.
      if (_session == null) await _restoreSession();
      return;
    }
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
    _servers = _loadServers(cloudOrigin);
    final base = Uri.tryParse(_cloudOrigin);
    if (base != null) {
      _api.baseUri = base;
    }
    // The palette is NOT seeded here. It used to be, and that is exactly one
    // frame too late: this runs from initState, inside MaterialApp's first
    // build, where notifying an ancestor cannot rebuild it. main() does it
    // before runApp instead.
    final fontScale = double.tryParse(loadPref('fontScale') ?? '');
    final fontFamily = loadPref('fontFamily');
    _appearance = EditorAppearance(
      fontScale: (fontScale ?? 1.0).clamp(0.85, 1.4),
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
    if (!kIsWeb)
      _pending = PendingUploads.fromJson(loadPref('pendingBlobUploads'));
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
    // Dev auto-login is a best-effort convenience for a running local backend.
    // When none is up (`just app` with no `just dev`), both attempts fail
    // with a raw ClientException that _run parks in the banner — a scary red
    // toast for what is a no-op. No backend just means "stay signed out in the
    // local world", so clear it. (Nothing else runs before this on startup, so
    // the only message that can be here is the one these two attempts set.)
    if (mounted && _session == null && _message != null) {
      setState(() => _message = null);
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
    try {
      final user = await _profileWatch.poll(session);
      // Re-read the session: the poll awaited a round trip, and a sign-out or a
      // token rotation in that window would make `session` the wrong thing to
      // build on — copyWith on the stale one would resurrect a dead token.
      final current = _session;
      if (user == null || current == null || !mounted) return;
      final updated = current.copyWith(user: user);
      setState(() => _session = updated);
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
    try {
      final next = await _refresher.ensureFresh(session);
      if (next == null || !mounted) return;
      setState(() => _session = next);
      // Persist immediately: the refresh token just rotated, and the copy on
      // disk is now burnt. Losing this write costs the sign-in at next launch.
      _persistSession(next);
    } on ApiException catch (error) {
      // The server refused the refresh token: 30 days idle, revoked, or its
      // family burnt by reuse detection. Nothing to recover — say so, rather
      // than firing doomed requests and leaving the user at `unauthorized`.
      if (error.isUnauthorized) _endExpiredSession();
    } catch (_) {
      // Offline / server down: keep the session. The token may well still be
      // good, and local-first means there is work to do without a network.
    }
  }

  /// The sign-in is over and cannot be renewed. Drop it and say so plainly.
  void _endExpiredSession() {
    if (!mounted || _session == null) return;
    _clearPersistedSession();
    setState(() {
      _session = null;
      _message = context.l10n.snackSessionExpired;
    });
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
      setState(() {
        _session = session;
        _workspaces = workspaces;
        _selectedWorkspace = restoredWs;
      });
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
      _ => error.toString(),
    };
  }

  Future<void> _run(Future<void> Function() action) async {
    setState(() {
      _isBusy = true;
      _message = null;
    });

    try {
      // Renew first if the token is about to lapse. Proactive rather than
      // retry-on-401 on purpose: [action] is not generally idempotent (it may
      // create a document), so replaying one after a refresh could duplicate
      // work. Renewing beforehand means it never gets refused in the first place.
      await _ensureFreshSession();
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

  /// Open [view] in a NEW tab and switch to it.
  ///
  /// The tab is created empty and filled by the caller's normal open path, so
  /// "open in new tab" and "open here" load a page exactly the same way — the
  /// only difference is which [DocTab] is active while it loads.
  void _openViewInNewTab(DocumentView view) {
    setState(() {
      _tabs.add(DocTab()..lastActivated = ++_activationTick);
      _activeTabIndex = _tabs.length - 1;
    });
    // Deliberately AFTER the new tab is active: `_selectView` skips the load
    // when the target is already open, and it tests that against the ACTIVE
    // tab. A fresh tab has a null view, so the same page opening in a second
    // tab is correctly treated as "not open here" and loads.
    unawaited(_selectView(view));
  }

  void _selectTab(int index) {
    if (index < 0 || index >= _tabs.length || index == _activeTabIndex) return;
    setState(() {
      _activeTabIndex = index;
      _activeTab.lastActivated = ++_activationTick;
    });
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
        _selectedMarkdown = null;
      });
    } catch (_) {
      // Transient sync refetch failures are non-fatal; the next update retries.
    }
  }

  @override
  void dispose() {
    _closeAllDocumentSync();
    super.dispose();
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
    savePref('lastWorkspaceId', workspace.id);
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
    final bootstrap = firstView == null
        ? null
        : await _offlineCloudBootstrap(firstView);
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
        _viewsByWorkspace = {..._viewsByWorkspace, optimisticWorkspace.id: views};
      });
    }

    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();

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

  /// Ask the running import to stop. It halts at the next page boundary and
  /// keeps what it already wrote — see the server's `cancel_requested`.
  Future<void> _cancelImport() async {
    final jobId = _importJobId;
    final session = _session;
    if (jobId == null || session == null) return;
    try {
      await _api.cancelImportJob(session.accessToken, jobId);
    } catch (_) {
      // The poll loop is what reports the outcome; a failed cancel just means
      // the import keeps going, which the progress row already shows.
    }
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
    return _api.exportMarkdown(
      session.accessToken,
      workspace.id,
      bootstrap.document.id,
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
    final markdown = _local.exportDocMarkdownText(bootstrap.document.id);
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
      setState(() => _session = updated);
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
      setState(() => _session = updated);
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

  /// Upload the archive, poll the import job, then refresh and open the
  /// resulting workspace.
  Future<void> _runServerImport(
    Uint8List zipBytes, {
    String? name,
    bool notion = false,
    String? workspaceId,
    String? parentViewId,
    String? container,
  }) {
    final l10n = context.l10n;
    return _run(() async {
      final session = _requireSession();
      final jobId = await _api.startWorkspaceImport(
        session.accessToken,
        zipBytes,
        name: name,
        notion: notion,
        workspaceId: workspaceId,
        parentViewId: parentViewId,
        container: container,
        reHostImages: _reHostImages,
      );
      if (mounted) setState(() => _importJobId = jobId);
      ImportJobStatus job;
      while (true) {
        job = await _api.importJobStatus(session.accessToken, jobId);
        // The server has been reporting done/total all along and the client
        // parsed both, then dropped them on the floor — a 96-page import showed
        // one indeterminate spinner for minutes with no sign it was moving.
        if (mounted && job.total > 0) {
          setState(() => _importProgress = (done: job.done, total: job.total));
        }
        if (job.status != 'running') break;
        await Future<void>.delayed(const Duration(milliseconds: 600));
      }
      if (job.status == 'error') {
        throw ApiException(job.error ?? l10n.importJobFailed);
      }
      // Stopped on request. Say how much did land — the import is NOT rolled
      // back, and treating this like a no-op would leave the user believing
      // their workspace is untouched when it now holds part of the archive.
      if (job.status == 'cancelled') {
        if (mounted) {
          ScaffoldMessenger.maybeOf(context)?.showSnackBar(
            SnackBar(content: Text(l10n.importCancelled(job.done))),
          );
        }
        final workspaces = await _api.listWorkspaces(session.accessToken);
        if (mounted) setState(() => _workspaces = workspaces);
        return;
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
      // The local import path has always confirmed itself with a snackbar; the
      // cloud path said nothing at all, so a successful import was
      // indistinguishable from one that quietly did nothing.
      if (mounted && job.done > 0) {
        final skipped = job.skippedTotal;
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
      }
    }).whenComplete(() {
      if (mounted) {
        setState(() {
          _importProgress = null;
          _importJobId = null;
        });
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
          _selectedMarkdown = null;
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
        _viewsByWorkspace = {
          ..._viewsByWorkspace,
          workspace.id: views
              .map((item) => item.id == updated.id ? updated : item)
              .toList(),
        };
      });
    });
  }

  /// Open a page the home screen listed. Home carries ids (it spans workspaces),
  /// so the view object is looked up here; a stale id — the page was deleted
  /// between render and tap — is simply ignored rather than guessed at.
  void _openViewFromHome(String viewId, {required bool local}) {
    final candidates = local
        ? _localViews
        : _viewsByWorkspace.values.expand((views) => views);
    for (final view in candidates) {
      if (view.id != viewId) continue;
      // A FOLDER is not a document — opening it as one did nothing at all, which
      // is what the home screen's directory list used to do. Show its contents.
      if (view.isFolder) {
        setState(() => _overviewFolderId = view.id);
        return;
      }
      unawaited(local ? _localSelectView(view) : _selectView(view));
      return;
    }
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
        _selectedMarkdown = null;
      }
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
        setState(() => _session = s);
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
      final workspaces = await _api.listWorkspaces(session.accessToken);
      if (!mounted) return;
      migrated = true;
      setState(() {
        _workspaces = workspaces;
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
        final doc = _local.openDoc(v.objectId);
        if (doc == null) continue;
        final cloudParent = v.parentId == null
            ? null
            : localToCloudView[v.parentId];
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
                // works, so there is something to go back to.
                onClose: () => Navigator.of(routeContext).pop(),
                pane: SignInPane(
                  strings: _signInPaneStrings(context),
                  origins: _servers,
                  active: _activeOrigin,
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
                  authForm: AuthFormCard(
                    allowRegister: _activeRegistrationOpen ?? true,
                    strings: _authFormStrings(
                      context,
                      title: migrate ? l10n.worldMigrateSignInTitle : null,
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
      final urls = await _api.resolveFiles(session.accessToken, workspace.id, [
        key,
      ]);
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
        throw ApiException(context.l10n.pageSelectFirst);
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
  Future<void> _openTransfer(DocumentView view, bool copy) async {
    final source = _selectedWorkspace;
    if (source == null || _session == null) return;
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
        // The dialog supplies (destWorkspaceId, dryRun); we bind source + view
        // + move/copy here. v1 always targets the destination ROOT
        // (parentViewId: null) — no destination-folder picker yet.
        onTransfer: ({required destWorkspaceId, required dryRun}) =>
            _api.transferView(
              token: _requireSession().accessToken,
              workspaceId: source.id,
              viewId: view.id,
              destWorkspaceId: destWorkspaceId,
              parentViewId: null,
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
  Future<({int used, int quota})?> _loadWorkspaceUsage(Workspace workspace) async {
    final session = _session;
    if (session == null) return null;
    try {
      return await _api.workspaceUsage(session.accessToken, workspace.id);
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
  }

  /// Tear down the live cloud session/state WITHOUT touching stored
  /// credentials — used by sign-out (after wiping creds) and by switching
  /// cloud servers (which deliberately keeps the old origin's creds, P3c-2).
  void _disconnectCloudSession() {
    _closeAllDocumentSync();
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
      try {
        bootstrap = await _api.bootstrapDocument(
          session.accessToken,
          workspace.id,
          viewToOpen.objectId,
        );
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
    }

    setState(() {
      _viewsByWorkspace = {..._viewsByWorkspace, workspace.id: views};
      _selectedView = viewToOpen;
      _selectedBootstrap = bootstrap;
      _selectedMarkdown = null;
      if (bootstrapError != null) _message = bootstrapError;
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
          role: w
              .role, // mirror the real role so offline editing knows its rights
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
    final bootstrap = firstView == null
        ? null
        : await _offlineCloudBootstrap(firstView);
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
        if (mounted) setState(() => _workspaces = workspaces);
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
  Future<void> _promptSignIn() async {
    final creds = await _promptCloudAuth();
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
      selectedView: local ? _localSelectedView : _selectedView,
      selectedBootstrap: local ? _localBootstrap : _selectedBootstrap,
      // Cloud only — the local world has no tab model (see _openViewInNewTab).
      tabs: local ? const [] : _tabs,
      activeTabIndex: local ? 0 : _activeTabIndex,
      onSelectTab: local ? null : _selectTab,
      onCloseTab: local ? null : _closeTab,
      onOpenInNewTab: local ? null : _openViewInNewTab,
      selectedMarkdown: local ? null : _selectedMarkdown,
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
      onSaveAiSettings: local ? null : _saveAiSettings,
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
      onImportWorkspaceZip: local
          ? (_, _, {bool notion = false}) async {}
          : _importWorkspaceZip,
      onImportWorkspaceTreeInto: local
          ? _localImportVaultTree
          : _importTreeIntoWorkspace,
      onExportMarkdown: local ? () async {} : _exportSelectedMarkdown,
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

/// The rename dialog, as a widget that OWNS its controller.
///
/// It must own it: disposing a controller right after `showDialog` returns —
/// the obvious spelling, and what the workspace dialogs above still do — throws
/// in debug. The pop only *starts* the exit transition, and that transition
/// rebuilds the field, which re-`addListener`s the controller you just killed
/// (`ChangeNotifier.debugAssertNotDisposed`, seen as a red screen on cancel).
/// State.dispose runs after the route is really gone, so it is the safe hook.
class _RenameDialog extends StatefulWidget {
  const _RenameDialog({required this.initialName});

  final String initialName;

  @override
  State<_RenameDialog> createState() => _RenameDialogState();
}

class _RenameDialogState extends State<_RenameDialog> {
  late final TextEditingController _ctrl =
      TextEditingController(text: widget.initialName)
        ..selection = TextSelection(
          baseOffset: 0,
          extentOffset: widget.initialName.length,
        );

  @override
  void dispose() {
    _ctrl.dispose();
    super.dispose();
  }

  void _submit() => Navigator.of(context).pop(_ctrl.text);

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    return AlertDialog(
      title: Text(l10n.commonRename),
      content: SizedBox(
        // Wide enough for a real page name; the sidebar row it replaces is
        // ~180px and is pinned to the screen edge, which is the whole reason
        // this is a dialog.
        width: 400,
        child: TextField(
          controller: _ctrl,
          autofocus: true,
          decoration: InputDecoration(
            labelText: l10n.renameNameLabel,
            border: const OutlineInputBorder(),
          ),
          onSubmitted: (_) => _submit(),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l10n.commonCancel),
        ),
        FilledButton.icon(
          onPressed: _submit,
          icon: const Icon(Icons.save),
          label: Text(l10n.commonSave),
        ),
      ],
    );
  }
}

/// Reverse-reference panel under a cloud page: the pages that link TO it.
///
/// Lazy: fetches on mount and whenever [viewId] changes, so opening a page is
/// never blocked on the backlink scan (it lands a frame or two later). Renders
/// NOTHING while loading, on error, or when empty — a page with no backlinks
/// shows no panel at all, matching the "empty ⇒ hide" spec. Only when there are
/// results does the "N backlinks" header + tappable source list appear; a tap
/// reuses the same view-open path search results use ([onOpen]).
class _BacklinksPanel extends StatefulWidget {
  const _BacklinksPanel({
    required this.viewId,
    required this.load,
    required this.onOpen,
    super.key,
  });

  final String viewId;
  final Future<List<Backlink>> Function(String viewId) load;
  final Future<void> Function(String viewId) onOpen;

  @override
  State<_BacklinksPanel> createState() => _BacklinksPanelState();
}

class _BacklinksPanelState extends State<_BacklinksPanel> {
  List<Backlink> _links = const [];

  @override
  void initState() {
    super.initState();
    _fetch();
  }

  @override
  void didUpdateWidget(_BacklinksPanel old) {
    super.didUpdateWidget(old);
    // A ValueKey on viewId remounts this widget per page, so this rarely fires;
    // kept as a belt-and-braces refresh if the key strategy ever changes.
    if (old.viewId != widget.viewId) {
      setState(() => _links = const []);
      _fetch();
    }
  }

  Future<void> _fetch() async {
    final viewId = widget.viewId;
    try {
      final links = await widget.load(viewId);
      // Guard against a late response after the user navigated away.
      if (!mounted || widget.viewId != viewId) return;
      setState(() => _links = links);
    } catch (_) {
      // Backlinks are a best-effort convenience; a failed scan just shows no
      // panel rather than surfacing an error over the page.
      if (!mounted || widget.viewId != viewId) return;
      setState(() => _links = const []);
    }
  }

  @override
  Widget build(BuildContext context) {
    if (_links.isEmpty) return const SizedBox.shrink();
    return Padding(
      padding: const EdgeInsets.only(top: 28),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Icon(
                Icons.link,
                size: 16,
                color: MicaTheme.of(context).text.muted,
              ),
              const SizedBox(width: 6),
              Text(
                context.l10n.backlinksHeading(_links.length),
                style: TextStyle(
                  fontSize: 13,
                  fontWeight: FontWeight.w600,
                  color: MicaTheme.of(context).text.muted,
                ),
              ),
            ],
          ),
          const SizedBox(height: 6),
          for (final link in _links)
            InkWell(
              onTap: () => widget.onOpen(link.viewId),
              borderRadius: BorderRadius.circular(6),
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 6),
                child: Row(
                  children: [
                    Icon(
                      Icons.description_outlined,
                      size: 16,
                      color: MicaTheme.of(context).text.muted,
                    ),
                    const SizedBox(width: 8),
                    Expanded(
                      child: Text(
                        link.title.isEmpty ? 'Untitled' : link.title,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontSize: 14,
                          color: MicaTheme.of(context).text.primary,
                        ),
                      ),
                    ),
                  ],
                ),
              ),
            ),
        ],
      ),
    );
  }
}

/// The comments entry in the doc header: an outline bubble, with the count of
/// still-open threads when there are any. Quiet when the document has none.
/// The sidebar's Home row. Its own widget so the pane's build method stays a list
/// of parts rather than another nested block of decoration.
class _HomeNavRow extends StatelessWidget {
  const _HomeNavRow({
    required this.label,
    required this.selected,
    required this.onTap,
  });

  final String label;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return InkWell(
      onTap: onTap,
      borderRadius: BorderRadius.circular(8),
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 9),
        decoration: BoxDecoration(
          color: selected ? MicaTheme.of(context).accent.wash : null,
          borderRadius: BorderRadius.circular(8),
        ),
        child: Row(
          children: [
            Icon(
              Icons.home_outlined,
              size: 18,
              color: selected
                  ? MicaTheme.of(context).accent.primary
                  : MicaTheme.of(context).text.muted,
            ),
            const SizedBox(width: 10),
            Text(
              label,
              style: TextStyle(
                fontSize: 14,
                fontWeight: FontWeight.w600,
                color: selected
                    ? MicaTheme.of(context).accent.hover
                    : MicaTheme.of(context).text.primary,
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// The muted words·chars readout. Lives in the shell now, pinned to the pane's
/// bottom-right; it used to be drawn by the editor at the end of the CONTENT,
/// which on a short page put it in the middle of the screen with nothing around
/// it. `IgnorePointer` so it never eats a click meant for the page beneath.
class _WordCountBadge extends StatelessWidget {
  const _WordCountBadge({required this.counts});

  final DocCounts counts;

  @override
  Widget build(BuildContext context) {
    return IgnorePointer(
      child: DecoratedBox(
        decoration: BoxDecoration(
          color: MicaTheme.of(context).surface.raised.withValues(alpha: 0.95),
          borderRadius: BorderRadius.circular(6),
          border: Border.all(color: const Color(0x14000000)),
        ),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
          child: Text(
            context.l10n.editorWordCount(counts.words, counts.chars),
            style: TextStyle(
              fontSize: 12,
              height: 1.0,
              color: MicaTheme.of(context).text.muted,
            ),
          ),
        ),
      ),
    );
  }
}

/// The right sidebar's tabs.
enum _ToolsTab { outline, comments }

/// One tab in the right sidebar's header. Icon + label + optional count.
class _ToolsTabButton extends StatelessWidget {
  const _ToolsTabButton({
    required this.icon,
    required this.label,
    required this.active,
    required this.onTap,
    this.badge = 0,
  });

  final IconData icon;
  final String label;
  final bool active;
  final VoidCallback onTap;

  /// Unresolved count, shown on the tab so you do not have to open it to learn
  /// there is something in it. Zero draws nothing.
  final int badge;

  @override
  Widget build(BuildContext context) {
    final tokens = MicaTheme.of(context);
    final color = active ? tokens.accent.primary : tokens.text.muted;
    return InkWell(
      onTap: onTap,
      borderRadius: BorderRadius.circular(6),
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 5),
        decoration: BoxDecoration(
          color: active ? tokens.accent.wash : null,
          borderRadius: BorderRadius.circular(6),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(icon, size: 15, color: color),
            const SizedBox(width: 6),
            Text(
              label,
              style: TextStyle(
                fontSize: 12,
                fontWeight: active ? FontWeight.w600 : FontWeight.w400,
                color: color,
              ),
            ),
            if (badge > 0) ...[
              const SizedBox(width: 5),
              Text(
                '$badge',
                style: TextStyle(fontSize: 11, color: tokens.text.faint),
              ),
            ],
          ],
        ),
      ),
    );
  }
}

class _CommentsButton extends StatelessWidget {
  const _CommentsButton({
    required this.openCount,
    required this.onTap,
    this.active = false,
  });

  final int openCount;
  final VoidCallback onTap;

  /// The rail is docked open. Shown because the button is now a TOGGLE, not a
  /// "show me" action — without the state it is the only control on the row
  /// whose effect you cannot see from the control itself.
  final bool active;

  @override
  Widget build(BuildContext context) {
    return Tooltip(
      message: context.l10n.commentsTitle,
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(4),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 4),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(
                Icons.chat_bubble_outline,
                size: 15,
                color: active
                    ? MicaTheme.of(context).accent.primary
                    : MicaTheme.of(context).text.muted,
              ),
              if (openCount > 0) ...[
                const SizedBox(width: 3),
                Text(
                  '$openCount',
                  style: TextStyle(
                    fontSize: 11,
                    color: MicaTheme.of(context).text.muted,
                  ),
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

/// The quiet sync-status affordance in the doc header. Calibrated minimal (see
/// sync_status.dart + the SiYuan/AFFiNE research): NOTHING when synced, a faint
/// slow spinner while syncing, a muted cloud-off glyph when offline. It earns
/// attention only when edits are not yet safely on the server — easy to miss
/// when healthy, by design.
class _SyncBadge extends StatelessWidget {
  const _SyncBadge(this.phase);

  final SyncPhase phase;

  @override
  Widget build(BuildContext context) {
    switch (phase) {
      case SyncPhase.synced:
        return const SizedBox.shrink();
      case SyncPhase.syncing:
        return Padding(
          padding: const EdgeInsets.symmetric(horizontal: 6),
          child: SizedBox(
            width: 13,
            height: 13,
            child: CircularProgressIndicator(
              strokeWidth: 1.6,
              color: MicaTheme.of(context).text.faint,
            ),
          ),
        );
      case SyncPhase.offline:
        return Padding(
          padding: const EdgeInsets.symmetric(horizontal: 6),
          child: Tooltip(
            message: context.l10n.syncOffline,
            child: Icon(
              Icons.cloud_off_outlined,
              size: 15,
              color: MicaTheme.of(context).text.muted,
            ),
          ),
        );
    }
  }
}

/// The page's path, workspace first, one name per segment.
///
/// This is the single source for BOTH things that show a path: "copy path"
/// joins it with `/`, and [PageBreadcrumb] renders the same list left to right.
/// They were computed separately once, and drifted — the clipboard led with the
/// workspace while the breadcrumb started one level below it, so you could not
/// tell what the button next to it was about to copy. Nothing failed when they
/// disagreed; only a human reading both at once could notice. Hence one
/// function, and `test/page_path_test.dart` pinning the two against each other.
///
/// Returns null when there is no open page (or no workspace) — the caller says
/// "open a page first" rather than showing half a path.
///
/// Names are NOT escaped: a `/` inside a page name is ambiguous here on
/// purpose. The path is for a human to read and for an agent to resolve by
/// searching the last segment and checking the chain with `parent_view_id`, not
/// by splitting on a delimiter.
List<String>? pagePathSegments({
  required String? workspaceName,
  required DocumentView? view,
  required List<DocumentView> views,
}) {
  if (workspaceName == null || view == null) return null;
  return [
    ...ancestorPathSegments(
      workspaceName: workspaceName,
      parentViewId: view.parentViewId,
      views: views,
      startedAt: view.id,
    ),
    view.name,
  ];
}

/// Where something LIVES: workspace first, then each folder down to (and
/// including) [parentViewId]. The thing itself is not in here.
///
/// Split out of [pagePathSegments] for the search results list, which shows the
/// trail beside a hit's name. Walking the tree a second time there would put two
/// implementations of "the path" back in the codebase — which is the exact shape
/// of the bug this pair exists to prevent.
///
/// Returns empty (never a placeholder) when there is nothing to say: a page at
/// the workspace root with no workspace name has no trail, and the caller draws
/// nothing rather than a lone separator.
List<String> ancestorPathSegments({
  required String? workspaceName,
  required String? parentViewId,
  required List<DocumentView> views,
  String? startedAt,
}) {
  final byId = {for (final v in views) v.id: v};
  final folders = <String>[];
  // Same `seen` guard as _revealFolder: the tree forbids cycles, but walking it
  // should not be the thing that hangs if one ever appears.
  final seen = <String>{?startedAt};
  var cursor = parentViewId;
  while (cursor != null && seen.add(cursor)) {
    final parent = byId[cursor];
    if (parent == null) break; // not loaded — stop rather than invent a gap
    folders.add(parent.name);
    cursor = parent.parentViewId;
  }
  return [
    if (workspaceName != null && workspaceName.trim().isNotEmpty) workspaceName,
    ...folders.reversed,
  ];
}

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
    required this.selectedView,
    required this.selectedBootstrap,
    this.tabs = const [],
    this.activeTabIndex = 0,
    this.onSelectTab,
    this.onCloseTab,
    this.onOpenInNewTab,
    required this.selectedMarkdown,
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
    required this.onCloneView,
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
    required this.onExportMarkdown,
    this.onAddMember,
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
  final String? selectedMarkdown;
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
  final Future<void> Function(DocumentView view, String name) onRenameView;

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

  /// Duplicate a view's subtree in place (cloud or local). Always provided —
  /// clone works in both, unlike onTransfer.
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

  /// Null in 本地模式 — AI settings live on the server, so there is no provider
  /// to configure and Settings drops the tab. Same rule as [onLoadTokens].
  final Future<Map<String, dynamic>> Function()? onLoadAiSettings;
  final Future<void> Function({
    required String provider,
    required String baseUrl,
    required String model,
    String? apiKey,
  })?
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

  final Future<void> Function(String fileName, Uint8List bytes, {bool notion})
  onImportWorkspaceZip;
  final Future<void> Function(Workspace workspace, List<ArchiveFile> entries)
  onImportWorkspaceTreeInto;
  final Future<void> Function() onExportMarkdown;

  /// All three are null in 本地模式, where membership does not exist. Absent
  /// rather than inert: see the member section in
  /// [_WorkspaceViewState._openWorkspaceSettingsDialog].
  final Future<void> Function(String email, WorkspaceRole role)? onAddMember;

  /// Storage this workspace occupies + the limit in force. Null in 本地模式:
  /// there is no server to ask and no quota to be near.
  final Future<({int used, int quota})?> Function(Workspace workspace)?
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
  final Future<void> Function(DocumentView view, bool copy)? onTransfer;

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
  final _memberEmail = TextEditingController();
  final _pageTitle = TextEditingController();
  final FocusNode _editorFocus = FocusNode(debugLabel: 'MicaEditorBody');
  final FocusNode _pageTitleFocus = FocusNode(debugLabel: 'PageTitle');
  Timer? _pageTitleSaveTimer;
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
  // The view whose sidebar name is in inline-rename (edit) mode — null = none.
  // Set right after creating a page/folder (so the user types its name
  // immediately, no dialog) or from the row's "重命名" action; the matching row
  // renders a focused TextField instead of the name Text.
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
  final GlobalKey _treeListKey = GlobalKey();
  Timer? _autoScrollTimer;
  double _autoScrollVelocity = 0;
  // Pointer is over the navigation sidebar — reveals the tree's expand
  // toggles (AppFlowy-style: they live in their own slim column, opacity 0
  // at rest so the page icons keep one aligned column).
  bool _navHovered = false;
  WorkspaceRole _memberRole = WorkspaceRole.editor;
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
    // Expose this editor's debounce flush to the app-exit path so quitting right
    // after typing commits the last <=400ms instead of dropping it.
    _activeEditorFlush = _boundExitFlush;
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
    // Keep the sidebar's "located" node in step with the open doc: opening a
    // page (link / auto-open / navigate) refocuses it, so the single row
    // highlight follows and a stale folder focus doesn't linger.
    if (widget.selectedView?.id != oldWidget.selectedView?.id) {
      _focusedNavId = widget.selectedView?.id;
      _rootFocused = false; // opening a doc re-locates it, off the root
    }
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
    _activeBlockHook.dispose();
    _stopAutoScroll();
    _treeScroll.dispose();
    // Only clear the exit-flush slot if it still points at ours (a remounting
    // sibling may have already claimed it).
    if (identical(_activeEditorFlush, _boundExitFlush))
      _activeEditorFlush = null;
    super.dispose();
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
      // Clicking OUTSIDE the sidebar releases the located row — the other half
      // of the rule whose first half is the blank-space tap inside it. Moving
      // the mouse away is not enough: you have to act somewhere else.
      //
      // A Listener, not a GestureDetector: this must not enter the gesture
      // arena at all. Competing there would put it up against every tap in the
      // editor — including text selection and drags — and losing or winning
      // either one would change behaviour that has nothing to do with the
      // sidebar. onPointerDown only observes.
      //
      // `_navHovered` is the test for "inside", reusing the MouseRegion that is
      // already tracking exactly that rather than measuring the pane's rect a
      // second time and letting the two disagree.
      child: Listener(
        behavior: HitTestBehavior.translucent,
        onPointerDown: (_) {
          if (!_navHovered) _focusRoot();
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
    final title = widget.selectedBootstrap?.view.name ?? 'Mica';
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
                        onImport: (notion) =>
                            _importWorkspaceFile(notion: notion),
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
                      onPressed: widget.onLoadGraph == null ||
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
    if (widget.selectedWorkspace == null) {
      return EmptyState(
        icon: Icons.ads_click,
        title: context.l10n.workspaceRowSelectWorkspace,
        detail: context.l10n.sidebarSelectWorkspaceDetail,
      );
    }

    if (widget.views.isEmpty) {
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
    // reach this. Opaque so the empty space below the last row is hittable.
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: _focusRoot,
      child: ListView(
        key: _treeListKey,
        controller: _treeScroll,
        children: _visibleDocumentTree().map((item) {
          final row = DocumentListItem(
            key: ValueKey(item.view.id),
            view: item.view,
            depth: item.depth,
            hasChildren: item.hasChildren,
            revealToggle: _navHovered,
            isCollapsed: !_expandedViewIds.contains(item.view.id),
            isSelected: item.view.id == activeId,
            canEdit: canEdit,
            isRenaming: item.view.id == _renamingViewId,
            onToggle: () => _toggleViewExpand(item.view),
            onPressed: () => _navigateToView(item.view),
            onCreateChild: () => _createInFolder(item.view, folder: false),
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
                : () => widget.onTransfer!(item.view, false),
            onTransferCopy: widget.onTransfer == null
                ? null
                : () => widget.onTransfer!(item.view, true),
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
              padding: const EdgeInsets.only(bottom: 8),
              child: row,
            );
          }
          return _draggableTreeRow(item.view, row);
        }).toList(),
      ),
    );
  }

  /// Tapping the tree's blank area deselects the located node: nothing is
  /// highlighted and the top New-page/New-folder buttons create at the
  /// workspace root (the only way to reach the root while a doc stays open).
  void _focusRoot() {
    if (_rootFocused && _focusedNavId == null) return;
    setState(() {
      _rootFocused = true;
      _focusedNavId = null;
    });
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
        onDragUpdate: (d) => _updateTreeAutoScroll(d.globalPosition),
        onDragEnd: (_) => _endTreeDrag(),
        onDraggableCanceled: (_, _) => _endTreeDrag(),
        onDragCompleted: _endTreeDrag,
        feedback: Material(
          elevation: 6,
          borderRadius: BorderRadius.circular(6),
          child: Container(
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
            constraints: const BoxConstraints(maxWidth: 260),
            decoration: BoxDecoration(
              color: MicaTheme.of(context).surface.overlay,
              borderRadius: BorderRadius.circular(6),
            ),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Icon(
                  Icons.description_outlined,
                  size: 18,
                  color: MicaTheme.of(context).accent.primary,
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

  /// While dragging, scroll the tree when the pointer nears its top/bottom edge
  /// so items off-screen become reachable. Called on every drag move with the
  /// pointer's GLOBAL position.
  void _updateTreeAutoScroll(Offset globalPointer) {
    final box = _treeListKey.currentContext?.findRenderObject() as RenderBox?;
    if (box == null || !_treeScroll.hasClients) {
      _stopAutoScroll();
      return;
    }
    final velocity = edgeAutoScrollVelocity(
      box.globalToLocal(globalPointer).dy,
      box.size.height,
    );
    if (velocity == 0) {
      _stopAutoScroll();
      return;
    }
    _autoScrollVelocity = velocity;
    // A single ticking timer reads the latest velocity each frame; onDragUpdate
    // only fires while the pointer MOVES, but the pointer often rests inside the
    // edge zone, so the timer — not the callback — must drive the scroll.
    _autoScrollTimer ??= Timer.periodic(const Duration(milliseconds: 16), (_) {
      if (!_treeScroll.hasClients) {
        _stopAutoScroll();
        return;
      }
      final pos = _treeScroll.position;
      final next = (pos.pixels + _autoScrollVelocity).clamp(
        pos.minScrollExtent,
        pos.maxScrollExtent,
      );
      if (next == pos.pixels) {
        return; // already at an end — keep the timer for when the pointer moves
      }
      _treeScroll.jumpTo(next);
    });
  }

  void _stopAutoScroll() {
    _autoScrollTimer?.cancel();
    _autoScrollTimer = null;
  }

  void _endTreeDrag() {
    _stopAutoScroll();
    setState(() => _draggingTree = false);
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
                color: active
                    ? MicaTheme.of(context).accent.primary
                    : Colors.transparent,
                width: 2,
              ),
              color: active
                  ? MicaTheme.of(context).accent.primary.withValues(alpha: 0.08)
                  : Colors.transparent,
            ),
          );
        }
        return Align(
          alignment: mode == _DropMode.before
              ? Alignment.topCenter
              : Alignment.bottomCenter,
          child: Container(
            height: 2,
            color: active
                ? MicaTheme.of(context).accent.primary
                : Colors.transparent,
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
      // Tapping a folder both toggles its subtree AND "locates" it — folders
      // never become `selectedView`, so this is how a folder becomes the target
      // for the top New-page/New-folder buttons (and gets the row highlight).
      _focusedNavId = view.id;
      _rootFocused = false; // locating a folder takes us off the root

      if (!_expandedViewIds.add(view.id)) {
        _expandedViewIds.remove(view.id);
      }
      _saveExpanded();
    });
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
  void _revealFolder(String viewId) {
    final byId = {for (final view in widget.views) view.id: view};
    // Not in this workspace's tree (a stale result, or it was moved away): do
    // nothing rather than focus an id that renders nowhere.
    if (!byId.containsKey(viewId)) return;
    setState(() {
      _focusedNavId = viewId;
      _rootFocused = false;
      _expandedViewIds.add(viewId);
      // Every ancestor too, or the row stays hidden inside a collapsed parent
      // and "locating" it shows the user nothing. The `seen` guard is
      // belt-and-braces — the page tree forbids cycles (folder-only parents,
      // enforced server-side and by a DB trigger) — but this walk should not be
      // the thing that hangs if one ever appears.
      final seen = <String>{viewId};
      var cursor = byId[viewId]?.parentViewId;
      while (cursor != null && seen.add(cursor)) {
        _expandedViewIds.add(cursor);
        cursor = byId[cursor]?.parentViewId;
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
      if (bootstrap == null) return;
      final title = renamedTo(_pageTitle.text, bootstrap.view.name);
      if (title == null) return;

      widget.onRenameView(bootstrap.view, title);
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
    if (onSelectTab == null || onCloseTab == null || widget.tabs.length < 2) {
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
        ),
        Expanded(child: body),
      ],
    );
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
            child: Row(
              children: [
                // The path shares the top row now. A FIXED budget, not a
                // flexible one: the toolbar beside it is a strip of fixed-size
                // click targets, so the width it gets must not depend on how
                // deep the current page happens to be. Inside this budget the
                // path collapses in the middle, and beyond that it scrolls
                // horizontally (it always could) — it never pushes.
                if (widget.selectedView != null)
                  ConstrainedBox(
                    constraints: const BoxConstraints(maxWidth: 260),
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
                Expanded(
                  child: (widget.showFormatBar && canEdit)
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
                  onPressed: () => setState(
                    () => _toolsExpanded = !_toolsExpanded,
                  ),
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
                        _toolsExpanded &&
                        _toolsTab == _ToolsTab.comments,
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
                  onTap: () => setState(
                    () => _showProperties = !_showProperties,
                  ),
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
        // Align the buttons with the page's centered text column (not the pane
        // edge): same horizontal padding + max width + left gutter as the body.
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 28),
          child: Center(
            child: ConstrainedBox(
              constraints: BoxConstraints(maxWidth: widget.pageWidth),
              child: Padding(
                padding: const EdgeInsets.only(left: EditorTheme.gutter),
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
                      btn(
                        Icons.looks_one_outlined,
                        context.l10n.pageFormatHeading1,
                        () => h.setBlock('heading', {'level': 1}),
                        active: onHeading(1),
                      ),
                      btn(
                        Icons.looks_two_outlined,
                        context.l10n.pageFormatHeading2,
                        () => h.setBlock('heading', {'level': 2}),
                        active: onHeading(2),
                      ),
                      btn(
                        Icons.looks_3_outlined,
                        context.l10n.pageFormatHeading3,
                        () => h.setBlock('heading', {'level': 3}),
                        active: onHeading(3),
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
      child: SingleChildScrollView(
        // Top padding moved to the pinned breadcrumb above; keeping it here too
        // would open a gap between the pinned row and the page it belongs to.
        padding: const EdgeInsets.fromLTRB(28, 4, 28, 28),
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
                              decoration: InputDecoration(
                                hintText: context.l10n.untitledPage,
                                border: InputBorder.none,
                              ),
                            ),
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
                // docs/page-properties.md.
                if (_showProperties)
                  PropertyPanel(
                    frontMatter: bootstrap.rootFrontMatter,
                    canEdit: canEdit,
                    // Clicking a tag opens workspace search for `#tag` — the
                    // precise token the index stores for list/tag values, so it
                    // finds pages that CARRY the tag, not ones that merely
                    // mention the word in their body (M2).
                    onOpenTag: (v) => _openSearch('#$v'),
                    onCommit: (fm) {
                      final data = Map<String, dynamic>.from(
                        bootstrap.rootData,
                      );
                      if (fm.isEmpty) {
                        data.remove('front_matter');
                      } else {
                        data['front_matter'] = fm;
                      }
                      return widget.onApplyOperations([
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
                      color: MicaTheme.of(context).surface.base,
                      borderRadius: BorderRadius.circular(8),
                      border: Border.all(
                        color: MicaTheme.of(context).border.normal,
                      ),
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
                    onTap: () =>
                        setState(() => _toolsTab = _ToolsTab.outline),
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
                    onTap: () =>
                        setState(() => _toolsTab = _ToolsTab.comments),
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
                    children: [
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
              matchesManageRole(ws.role) && widget.onAddMember != null;
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
                    if (widget.onAddMember == null)
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
                        _addMemberForm(setLocal),
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

  /// Add-member form for the settings dialog. [setLocal] rebuilds the dialog
  /// (role dropdown selection + the refreshed member list after an add).
  Widget _addMemberForm(void Function(void Function()) setLocal) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        TextField(
          controller: _memberEmail,
          keyboardType: TextInputType.emailAddress,
          decoration: InputDecoration(
            labelText: context.l10n.workspaceMemberEmail,
            prefixIcon: const Icon(Icons.alternate_email),
            border: const OutlineInputBorder(),
          ),
        ),
        const SizedBox(height: 10),
        DropdownButtonFormField<WorkspaceRole>(
          initialValue: _memberRole,
          decoration: InputDecoration(
            labelText: context.l10n.widgetRoleLabel,
            border: const OutlineInputBorder(),
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
            // Non-null: this form only renders when canManage, which requires it.
            await widget.onAddMember!(_memberEmail.text, _memberRole);
            _memberEmail.clear();
            setLocal(() {});
          },
          icon: const Icon(Icons.person_add),
          label: Text(context.l10n.workspaceMemberAdd),
        ),
      ],
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
    // The located row can sit inside a collapsed parent — switching workspaces
    // re-opens its doc but restores the remembered (possibly all-collapsed)
    // expand state. Reveal it so the row being renamed is actually on screen
    // behind the dialog.
    setState(() {
      if (_revealAncestors(view.id)) _saveExpanded();
    });
    _promptRenameView(view);
  }

  void _cancelRename() {
    if (_renamingViewId == null) return;
    setState(() => _renamingViewId = null);
  }

  Future<void> _commitRename(DocumentView view, String name) async {
    if (_renamingViewId != null) setState(() => _renamingViewId = null);
    // Empty or unchanged → keep the current name; see [renamedTo].
    final next = renamedTo(name, view.name);
    if (next == null) return;
    await widget.onRenameView(view, next);
  }

  /// Create a page/folder via [create], then drop its new sidebar row straight
  /// into inline-rename so the user just types the name — no naming dialog.
  Future<void> _createThenRename(Future<String?> Function() create) async {
    final id = await create();
    if (id != null && mounted) setState(() => _renamingViewId = id);
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

  void _openSearch([String? initialQuery]) {
    showDialog<void>(
      context: context,
      builder: (context) => _SearchDialog(
        onSearch: widget.onSearch,
        views: widget.views,
        workspaceName: widget.selectedWorkspace?.name,
        initialQuery: initialQuery,
        onOpen: (viewId) {
          Navigator.of(context).pop();
          widget.onOpenSearchResult(viewId);
        },
        onReveal: (viewId) {
          Navigator.of(context).pop();
          _revealFolder(viewId);
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
        onSaveAiSettings: widget.onSaveAiSettings,
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
        onImportWorkspace: () => _importWorkspaceFile(fromSettings: true),
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
    if (_rootFocused || _focusedNavId != view.id) {
      setState(() {
        _focusedNavId = view.id;
        _rootFocused = false;
      });
    }
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
