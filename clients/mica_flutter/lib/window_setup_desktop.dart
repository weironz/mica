/// Desktop window setup (selected for non-web via window_setup.dart):
/// enforce a minimum size and remember the last window size/position in prefs.
library;

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:window_manager/window_manager.dart';

import 'prefs.dart';

const Size _minSize = Size(860, 600);
const Size _defaultSize = Size(1280, 800);

bool get _isDesktop =>
    defaultTargetPlatform == TargetPlatform.windows ||
    defaultTargetPlatform == TargetPlatform.linux ||
    defaultTargetPlatform == TargetPlatform.macOS;

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

  windowManager.addListener(_BoundsPersister());
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
