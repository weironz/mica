// Keyboard for the tab strip: Ctrl/Cmd+T, Ctrl/Cmd+W, Ctrl+Tab, Ctrl+Shift+Tab.
//
// The bindings follow the browser and the Notion / AppFlowy desktop apps, so
// they need no learning. Two choices worth knowing:
//
// * Ctrl+T opens the page picker in new-tab mode rather than creating a page.
//   A browser's new tab asks where to go; creating a page already has Ctrl+N,
//   and the picker is the same one the strip's `+` menu offers.
// * Ctrl+Tab stays Ctrl on macOS too. Cmd+Tab is the system app switcher and
//   never reaches the app; Notion and Safari use Ctrl+Tab there as well.
//
// On the web the browser keeps all four for its own tabs, so these only act in
// the desktop app. Registering them on the web is harmless: the keys never
// arrive.
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

/// The tab after (or before, with [backwards]) [active], wrapping around.
int nextTabIndex(int active, int count, {bool backwards = false}) {
  if (count <= 0) return 0;
  final step = backwards ? count - 1 : 1;
  return (active + step) % count;
}

/// The shortcut map for the tab strip. [close] is only bound when the strip
/// allows closing (the last tab cannot be closed, so Ctrl+W must not pretend).
Map<ShortcutActivator, VoidCallback> tabShortcutBindings({
  required VoidCallback newTab,
  required VoidCallback? close,
  required VoidCallback next,
  required VoidCallback previous,
}) => {
  const SingleActivator(LogicalKeyboardKey.keyT, control: true): newTab,
  const SingleActivator(LogicalKeyboardKey.keyT, meta: true): newTab,
  if (close != null) ...{
    const SingleActivator(LogicalKeyboardKey.keyW, control: true): close,
    const SingleActivator(LogicalKeyboardKey.keyW, meta: true): close,
  },
  const SingleActivator(LogicalKeyboardKey.tab, control: true): next,
  const SingleActivator(LogicalKeyboardKey.tab, control: true, shift: true):
      previous,
};
