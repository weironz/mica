/// Web stub: there is no native window to size, position, or close — the tab's
/// close button belongs to the browser and no app code can intercept it. These
/// mirror window_setup_desktop.dart's surface so main.dart and Settings compile
/// unchanged; the close-behavior setting is simply never offered here
/// ([trayIsSupported] is false and Settings hides the section on web).
library;

import 'package:flutter/widgets.dart';

const kCloseAsk = 'ask';
const kCloseQuit = 'quit';
const kCloseMinimize = 'minimize';
const kCloseTray = 'tray';

Future<void> initDesktopWindow() async {}

String loadCloseBehavior() => kCloseQuit;
void saveCloseBehavior(String v) {}
Future<void> applyCloseBehavior(String behavior) async {}
Future<bool> ensureTray() async => false;

bool get trayIsSupported => false;

/// Unused on web (the browser owns tab close; no exit(0) path), but main.dart
/// assigns it unconditionally — mirror the desktop surface so it compiles.
void Function()? appExitFlush;

/// Unused on web, but MaterialApp still takes it — one key, one code path.
final GlobalKey<NavigatorState> appNavigatorKey = GlobalKey<NavigatorState>();
