/// Desktop window setup (selected for non-web via window_setup.dart):
/// enforce a minimum size, remember the last window size/position in prefs, and
/// decide what the window's X button does.
library;

import 'dart:io' show exit;

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:tray_manager/tray_manager.dart';
import 'package:window_manager/window_manager.dart';

import 'l10n/app_localizations.dart';
import 'l10n/locale_controller.dart';
import 'prefs.dart';

const Size _minSize = Size(860, 600);
const Size _defaultSize = Size(1280, 800);

/// A fast, synchronous local-durability flush the app installs (from the root
/// state) so quitting can hard-`exit(0)` without losing debounced edits. Quit
/// goes `exit(0)` rather than `windowManager.destroy()` because the graceful
/// engine teardown blocks for seconds waiting on plugin/thread/socket shutdown
/// (notably the cloud WebSocket close handshake, which stalls to a TCP timeout
/// on a flaky connection) — that is the "几秒卡顿" on close. exit(0) is instant;
/// this hook makes it safe by persisting local state first. Cloud unacked edits
/// already sit in the local outbox and resend next launch, so dropping the
/// socket loses nothing.
void Function()? appExitFlush;

/// Persist local state, then terminate the process immediately. The single quit
/// path for both the X button and the tray "退出".
Never _quitNow() {
  try {
    appExitFlush?.call();
  } catch (_) {
    // A flush failure must not wedge the quit — exit regardless.
  }
  exit(0);
}

bool get _isDesktop =>
    defaultTargetPlatform == TargetPlatform.windows ||
    defaultTargetPlatform == TargetPlatform.linux ||
    defaultTargetPlatform == TargetPlatform.macOS;

// ── Close behaviour ─────────────────────────────────────────────────────────
// What the window's X does. Persisted under `closeBehavior`; [kCloseAsk] is the
// default so the first X asks instead of guessing (quitting an editor someone
// meant to minimise loses their place).

/// Ask on the next close, and remember what they pick.
const kCloseAsk = 'ask';
const kCloseQuit = 'quit';
const kCloseMinimize = 'minimize';
const kCloseTray = 'tray';

String loadCloseBehavior() => loadPref('closeBehavior') ?? kCloseAsk;
void saveCloseBehavior(String v) => savePref('closeBehavior', v);

/// Whether this platform can put an icon in the tray.
///
/// Windows only, deliberately. `tray_manager` on Linux needs
/// `libayatana-appindicator3` at build AND run time (it aborts at startup when
/// the lib is missing, even if you never call it), fails to compile on Debian
/// 13 / recent Ubuntu where the deprecated `app_indicator_new` meets `-Werror`,
/// and on GNOME needs a user-installed extension before the icon appears at all.
/// Any of those turns "hide to tray" into "the window is gone and there is no
/// way back". macOS additionally needs an AppDelegate change to survive its last
/// window closing. Neither ships from CI today (the release workflow builds the
/// Flutter desktop app on windows-latest only), so neither is exercised.
bool get trayIsSupported =>
    !kIsWeb && defaultTargetPlatform == TargetPlatform.windows;

/// The navigator used to ask the close question. It lives here because the
/// close listener has no BuildContext of its own; main.dart hands it to
/// MaterialApp.
final GlobalKey<NavigatorState> appNavigatorKey = GlobalKey<NavigatorState>();

bool _trayReady = false;

/// Restore saved bounds (or center at a default), enforce the minimum size, and
/// start persisting bounds on resize/move. No-op on mobile, where
/// `window_manager` is unsupported (the import still compiles; calls are gated).
Future<void> initDesktopWindow() async {
  if (!_isDesktop) return;
  await windowManager.ensureInitialized();

  final saved = _loadBounds();
  final options = WindowOptions(
    size: saved.size ?? _defaultSize,
    center: saved.position == null,
    minimumSize: _minSize,
    title: 'Mica',
    titleBarStyle: TitleBarStyle.normal,
  );
  await windowManager.waitUntilReadyToShow(options, () async {
    final pos = saved.position;
    if (pos != null) {
      await windowManager.setBounds(null, position: pos);
    }
    // Maximized is part of where the window was, and it is the one part the
    // rect above deliberately does NOT carry: _save refuses to record
    // maximized geometry, so that un-maximizing has somewhere sane to go. The
    // cost was that anyone who works maximized got a small window every single
    // launch, forever.
    //
    // Before show(), not after: the Windows side posts SC_MAXIMIZE rather than
    // resizing inline, so showing first means showing the small window and
    // then watching it snap.
    if (saved.maximized) await windowManager.maximize();
    await windowManager.show();
    await windowManager.focus();
  });

  // Take over the X button. Without this the window is destroyed before any of
  // the branching below gets a say. NOTE this also makes windowManager.close()
  // re-enter onWindowClose — only destroy() actually quits from here on.
  await windowManager.setPreventClose(true);
  windowManager.addListener(_BoundsPersister());
  windowManager.addListener(_CloseHandler());

  // Register the tray up front when it's the standing choice, so the icon is
  // already there when the user hits X — and so a failure is discovered now,
  // while the window is still visible, rather than after we hide it.
  if (loadCloseBehavior() == kCloseTray) await ensureTray();
}

/// Put Mica in the tray. Returns whether the icon is actually up — the caller
/// MUST NOT hide the window unless this is true, since the icon is the only way
/// back. Never throws: a tray that fails to register is a downgrade, not a crash.
Future<bool> ensureTray() async {
  if (!trayIsSupported) return false;
  if (_trayReady) return true;
  try {
    await trayManager.setIcon('assets/tray_icon.ico');
    await trayManager.setToolTip('Mica');
    final l = l10nNoContext;
    await trayManager.setContextMenu(
      Menu(
        items: [
          MenuItem(key: 'show', label: l.trayShow),
          MenuItem.separator(),
          MenuItem(key: 'exit', label: l.trayExit),
        ],
      ),
    );
    trayManager.addListener(_TrayHandler());
    _trayReady = true;
    return true;
  } catch (e) {
    debugPrint('tray unavailable, will minimize instead: $e');
    return false;
  }
}

/// Do what [behavior] says. Split out so Settings can preview a choice and the
/// close listener can reuse it.
Future<void> applyCloseBehavior(String behavior) async {
  switch (behavior) {
    case kCloseMinimize:
      await windowManager.minimize();
    case kCloseTray:
      // Fall back to a plain minimize if the tray didn't come up: hiding a
      // window whose only restore path is a missing icon strands the user.
      if (await ensureTray()) {
        await windowManager.hide();
      } else {
        await windowManager.minimize();
      }
    default:
      // Quit. exit(0), not windowManager.destroy(): destroy() runs the engine's
      // graceful teardown, which blocks for seconds on plugin/socket shutdown
      // (the "几秒卡顿"). _quitNow() persists local state first, then exits now.
      _quitNow();
  }
}

class _TrayHandler with TrayListener {
  @override
  void onTrayIconMouseDown() => windowManager.show();

  @override
  void onTrayIconRightMouseDown() => trayManager.popUpContextMenu();

  @override
  void onTrayMenuItemClick(MenuItem menuItem) {
    switch (menuItem.key) {
      case 'show':
        windowManager.show();
        windowManager.focus();
      case 'exit':
        _quitNow();
    }
  }
}

class _CloseHandler with WindowListener {
  bool _asking = false;

  @override
  Future<void> onWindowClose() async {
    final saved = loadCloseBehavior();
    if (saved != kCloseAsk) {
      await applyCloseBehavior(saved);
      return;
    }
    // Don't stack dialogs if X is clicked again while the question is up.
    if (_asking) return;
    _asking = true;
    final choice = await _askCloseBehavior();
    _asking = false;
    // Dismissed (Esc / clicked away) = "I didn't mean to close" → stay open.
    if (choice == null) return;
    await applyCloseBehavior(choice);
  }
}

/// The first-close question. Returns the chosen behaviour, or null to stay open.
/// Whatever they pick becomes the standing answer (Settings can change it).
Future<String?> _askCloseBehavior() async {
  final context = appNavigatorKey.currentContext;
  // No navigator (closed before first frame) → the safe reading of X is "quit".
  if (context == null) return kCloseQuit;
  final l = AppLocalizations.of(context);

  var remember = true;
  final choice = await showDialog<String>(
    context: context,
    builder: (context) => StatefulBuilder(
      builder: (context, setState) => AlertDialog(
        title: Text(l.closeWindowTitle),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(l.closeWindowPrompt),
            const SizedBox(height: 8),
            CheckboxListTile(
              value: remember,
              onChanged: (v) => setState(() => remember = v ?? true),
              contentPadding: EdgeInsets.zero,
              controlAffinity: ListTileControlAffinity.leading,
              title: Text(l.closeRemember),
            ),
          ],
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: Text(l.closeCancel),
          ),
          TextButton(
            onPressed: () => Navigator.pop(context, kCloseMinimize),
            child: Text(l.closeMinimizeTaskbar),
          ),
          if (trayIsSupported)
            TextButton(
              onPressed: () => Navigator.pop(context, kCloseTray),
              child: Text(l.closeMinimizeTray),
            ),
          FilledButton(
            onPressed: () => Navigator.pop(context, kCloseQuit),
            child: Text(l.closeQuit),
          ),
        ],
      ),
    ),
  );
  if (choice != null && remember) saveCloseBehavior(choice);
  return choice;
}

({Size? size, Offset? position, bool maximized}) _loadBounds() {
  final w = double.tryParse(loadPref('windowWidth') ?? '');
  final h = double.tryParse(loadPref('windowHeight') ?? '');
  final x = double.tryParse(loadPref('windowX') ?? '');
  final y = double.tryParse(loadPref('windowY') ?? '');
  final size =
      (w != null && h != null && w >= _minSize.width && h >= _minSize.height)
      ? Size(w, h)
      : null;
  final position = (x != null && y != null) ? Offset(x, y) : null;
  return (
    size: size,
    position: position,
    maximized: loadPref('windowMaximized') == 'true',
  );
}

/// Persists the window rect after the user finishes a resize or move. Skips
/// maximized/minimized/fullscreen states so those don't become the saved
/// "restore" geometry.
class _BoundsPersister with WindowListener {
  @override
  void onWindowResized() => _save();

  @override
  void onWindowMoved() => _save();

  /// Maximized rides on its own key precisely because [_save] refuses to touch
  /// the rect while maximized — the two facts are separate, and the window
  /// needs both to come back the way it left.
  @override
  void onWindowMaximize() => savePref('windowMaximized', 'true');

  @override
  void onWindowUnmaximize() => savePref('windowMaximized', 'false');

  Future<void> _save() async {
    if (await windowManager.isMaximized() ||
        await windowManager.isMinimized() ||
        await windowManager.isFullScreen()) {
      return;
    }
    final b = await windowManager.getBounds();
    savePref('windowX', b.left.toStringAsFixed(0));
    savePref('windowY', b.top.toStringAsFixed(0));
    savePref('windowWidth', b.width.toStringAsFixed(0));
    savePref('windowHeight', b.height.toStringAsFixed(0));
  }
}
