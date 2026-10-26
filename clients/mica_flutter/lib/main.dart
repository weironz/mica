import 'dart:convert';
import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/foundation.dart' show kIsWeb, kReleaseMode;
import 'package:flutter/gestures.dart'
    show
        PointerDeviceKind,
        computeHitSlop,
        kDoubleTapMinTime,
        kDoubleTapSlop,
        kDoubleTapTimeout,
        kPrimaryButton;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_localizations/flutter_localizations.dart';

import 'l10n/app_localizations.dart';
import 'l10n/locale_controller.dart';

import 'cloud/cloud_sync.dart';
import 'cloud/doc_store_platform.dart';
import 'cloud/pending_uploads.dart';
import 'cloud/workspace_migration.dart';
import 'ui/mica_state.dart';
import 'diagnostics.dart';
import 'import/import_controller.dart';
import 'server_registry.dart';
import 'sync/sync_reconcile_controller.dart';
import 'session/session_controller.dart';
import 'ui/inline_rename_controller.dart';
import 'ui/sidebar_tree_controller.dart';
import 'ui/tree_drop_planner.dart';
import 'views/cloud_views_controller.dart';
import 'views/members_controller.dart';
import 'views/invitations_controller.dart';
import 'ui/workspace_invitations.dart';
import 'swallowed.dart';
import 'local/local_offline.dart';
import 'local/local_offline_factory.dart';
import 'editor/word_count.dart';
import 'web/yjs_probe.dart';
import 'editor/clipboard_copy.dart' show copyTextToClipboard;
import 'editor/model.dart' show headingButtonTarget, kMonoFont;
import 'editor/version_diff.dart';
import 'editor/editor.dart';
import 'editor/image_actions.dart';
import 'editor/open_url.dart';
import 'editor/pick_file.dart';
import 'editor/property_panel.dart';
import 'widgets/mica_logo.dart';
import 'editor/pick_image.dart';
import 'ui/auth_form.dart';
import 'ui/autoscroll.dart';
import 'ui/navigation_draggable.dart';
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
import 'ui/home_screen.dart' show HomeDocEntry;
import 'ui/home_pane.dart';
import 'ui/overview_pane.dart';
import 'ui/page_tree_state.dart';
import 'ui/page_tree_cache.dart';
import 'ui/tree_selection.dart';
import 'ui/panel_kit.dart';
import 'ui/rename.dart';
import 'ui/search_data.dart';
import 'ui/settings_sync.dart';
import 'ui/settings_shell.dart';
import 'ui/appearance_preview.dart';
import 'ui/page_title_field.dart';
import 'ui/sign_in_hero.dart';
import 'ui/sign_in_screen.dart';
import 'ui/sign_in_pane.dart';
import 'ui/status_kit.dart';
import 'ui/theme_tokens.dart';
import 'ui/font_warmup.dart';
import 'ui/trash_data.dart';
import 'ui/user_avatar.dart';
import 'ui/update_controls.dart';
import 'ui/version_data.dart';
import 'ui/workspace_overview.dart' show WorkspaceOverviewMode;
import 'local/cache_stats.dart'
    show
        ClearMirrorResult,
        LocalCacheStats,
        cloudCacheOrigin,
        requireCloudCacheOrigin,
        titleFromMarkdown;
import 'doc_tab.dart';
import 'prefs.dart';
import 'theme_controller.dart';
import 'server_list.dart';
import 'updater.dart';
import 'window_setup.dart';
import 'upload/zip_writer.dart';
import 'api/client.dart';
import 'perf/switch_trace.dart';
import 'api/models.dart';
import 'api/doc_socket_uri.dart';
import 'auth/auth_controller.dart';

// The API/data layer lives in lib/api/*.dart; re-export it so existing
// `import 'main.dart'` users (tests, tooling) still see these symbols.
export 'api/client.dart';
export 'api/models.dart';
export 'api/doc_socket_uri.dart';

part 'ui/dialogs.dart';
// _SettingsDialogState split per tab (issue #81) — extensions in these parts
// keep the class's logic one file per settings destination.
part 'ui/dialogs_settings_appearance.dart';
part 'ui/dialogs_settings_account.dart';
part 'ui/dialogs_settings_tokens.dart';
part 'ui/dialogs_settings_data.dart';
part 'ui/dialogs_settings_ai.dart';
part 'ui/dialogs_settings_misc.dart';
part 'ui/widgets.dart';
part 'main_shell.dart';
part 'main_editor_history.dart';
part 'main_workspace_view.dart';
part 'main_workspace_tree_nav.dart';
part 'main_workspace_editor_pane.dart';
part 'main_panels.dart';
part 'main_tree_helpers.dart';
part 'world/world_backend.dart';
part 'world/local_world_backend.dart';

/// Dev convenience: when true, the app signs in automatically on startup so you
/// don't have to log in on every reload. Turn it off for a real login screen:
/// `flutter run --dart-define=MICA_DEV_AUTOLOGIN=false`.
///
/// Never in a release build: no release recipe passes the define, so every
/// shipped app used to try demo/password123 — and then REGISTER it — against
/// any localhost backend, held back only by the server's weak-password list
/// (2026-10-21 review).
const bool kDevAutoLogin =
    !kReleaseMode &&
    bool.fromEnvironment('MICA_DEV_AUTOLOGIN', defaultValue: true);
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
const String kAppVersion = '0.13.68';

final _desktopUpdates = DesktopUpdateController(currentVersion: kAppVersion);

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
      // Attribute HTTP timings to whichever workspace switch is in flight. Null
      // unless tracing is asked for, and the client skips its timing wrapper
      // entirely when it is null.
      apiRequestObserver = SwitchTrace.observer();
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
      warmUpFonts();
      runApp(const MicaApp());
    },
    (error, stack) {
      // Last-resort net for uncaught async errors anywhere in the zone.
      logCrash('Uncaught: $error\n$stack');
    },
  );
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
          theme: MicaTokens.light.toMaterialTheme(),
          darkTheme: MicaTokens.dark_.toMaterialTheme(),
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
