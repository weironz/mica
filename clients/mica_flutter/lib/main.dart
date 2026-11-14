import 'ui/typography.dart';
import 'ui/api_error_message.dart';
import 'ui/deferred_load.dart';
import 'ui/editor_projection.dart';
import 'session/import_notification_actions.dart';
import 'session/bootstrap_config.dart';
export 'session/bootstrap_config.dart'
    show
        kDefaultCloudUrl,
        resolveLegacyCloudSetup,
        kDevAutoLogin,
        kDevEmail,
        kDevPassword;
import 'dart:convert';
import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_localizations/flutter_localizations.dart';

import 'l10n/app_localizations.dart';
import 'l10n/locale_controller.dart';

import 'cloud/cloud_sync.dart';
import 'cloud/doc_store_platform.dart';
import 'cloud/pending_uploads.dart';
import 'cloud/workspace_migration.dart';
import 'ui/export_format.dart';
import 'ui/settings_dialog.dart';
import 'ui/transfer_dialog.dart';
import 'ui/search_dialog.dart';
import 'ui/graph_dialog.dart' deferred as graph_ui;
import 'ui/ai_dialog.dart' deferred as ai_ui;
import 'ui/recycle_bin_dialog.dart';
import 'ui/share_dialog.dart';
import 'ui/version_history_dialog.dart' deferred as history_ui;
import 'ui/mica_state.dart';
import 'diagnostics.dart';
import 'import/import_controller.dart';
import 'server_registry.dart';
import 'sync/sync_reconcile_controller.dart';
import 'session/session_controller.dart';
import 'session/shell_preferences_controller.dart';
import 'ui/inline_rename_controller.dart';
import 'ui/sidebar_tree_controller.dart';
import 'ui/tree_drop_planner.dart';
import 'views/cloud_views_controller.dart';
import 'views/members_controller.dart';
import 'views/invitations_controller.dart';
import 'ui/workspace_invitations.dart';
import 'local/local_offline.dart';
import 'local/local_offline_factory.dart';
import 'editor/word_count.dart';
import 'web/web_probe.dart';
import 'markdown/markdown_codec.dart';
import 'editor/clipboard_copy.dart' show copyTextToClipboard;
import 'editor/model.dart' show headingButtonTarget, kMonoFont;
import 'editor/editor.dart';
import 'editor/block_marquee.dart';
import 'editor/image_actions.dart';
import 'editor/pick_file.dart';
import 'editor/property_panel.dart';
import 'editor/pick_image.dart';
import 'ui/auth_form.dart';
import 'ui/autoscroll.dart';
import 'ui/navigation_draggable.dart';
import 'ui/avatar_url.dart';
import 'ui/comment_panel.dart';
import 'ui/doc_tab_strip.dart';
import 'ui/export_shape.dart';
import 'doc_tab_persistence.dart';
import 'ui/tab_shortcuts.dart';
import 'ui/narrow_tab_switcher.dart';
import 'import/local_zip_import.dart';
import 'ui/destructive_confirm.dart';
import 'ui/dialog_controllers.dart';
import 'ui/emoji_picker.dart';
import 'ui/format_bytes.dart';
import 'ui/home_data.dart' show RelativeTimeStrings, countPages;
import 'ui/home_screen.dart' show HomeDocEntry;
import 'ui/home_pane.dart';
import 'ui/overview_pane.dart';
import 'ui/page_tree_state.dart';
import 'ui/page_tree_cache.dart';
import 'ui/tree_selection.dart';
import 'ui/rename.dart';
import 'ui/search_data.dart';
import 'ui/page_title_field.dart';
import 'ui/sign_in_hero.dart';
import 'ui/sign_in_screen.dart';
import 'ui/sign_in_pane.dart';
import 'ui/status_kit.dart';
import 'ui/theme_tokens.dart';
import 'ui/font_warmup.dart';
import 'ui/user_avatar.dart';
import 'ui/update_controls.dart';
import 'ui/workspace_overview.dart' show WorkspaceOverviewMode;
import 'local/cache_stats.dart'
    show
        ClearMirrorResult,
        LocalCacheStats,
        cloudCacheOrigin,
        requireCloudCacheOrigin;
import 'import/import_title.dart' show titleFromMarkdown;
import 'doc_tab.dart';
import 'main_tree_helpers.dart';
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

export 'ui/search_dialog.dart' show SearchResultTile;
export 'ui/ai_prompts.dart' show kAiPageEditSystem;
// State extensions and world backends still share this library. Dialogs and
// preferences synchronize through independent libraries with explicit inputs.
import 'ui/shell_widgets.dart';
export 'ui/shell_widgets.dart';
import 'ui/workspace_panels.dart';
part 'main_shell.dart';
part 'main_editor_history.dart';
part 'main_workspace_view.dart';
part 'main_workspace_tree_nav.dart';
part 'main_workspace_editor_pane.dart';
part 'world/world_backend.dart';
part 'world/local_world_backend.dart';

/// App version, shown in the About dialog. Keep in sync with `pubspec.yaml`
/// (`version:`) and `crates/api-server/Cargo.toml` on each release.
const String kAppVersion = '0.14.6';

final _desktopUpdates = DesktopUpdateController(currentVersion: kAppVersion);

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
      // Restore desktop window geometry before the first frame (no-op on web/mobile).
      await initDesktopWindow();
      // Seed the UI language and the palette from the persisted choices (prefs are
      // file/localStorage backed and loaded synchronously) before the first frame
      // renders. Both MUST happen here rather than from a widget's initState —
      // see loadPersistedThemeMode for what that looked like when it didn't.
      loadPersistedLocale();
      loadPersistedThemeMode();
      // Suppress the browser menu so the editor can show its own image actions.
      if (kIsWeb) BrowserContextMenu.disableContextMenu();
      registerWebProbes(); // browser harness hooks; no-op off web
      // index.html started both engines; sync builds replicas synchronously.
      await Future.wait([initMarkdownCodec(), initDocEngine()]);
      await warmUpFonts();
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
