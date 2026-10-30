// The tab strip's keyboard (ui/tab_shortcuts.dart): the wrap-around index
// and which keys are bound. Ctrl+W must vanish when there is only one tab —
// the strip refuses to close it, and a bound key that does nothing reads as
// broken.
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/tab_shortcuts.dart';

void main() {
  test('next and previous wrap around', () {
    expect(nextTabIndex(0, 3), 1);
    expect(nextTabIndex(2, 3), 0);
    expect(nextTabIndex(0, 3, backwards: true), 2);
    expect(nextTabIndex(1, 3, backwards: true), 0);
    expect(nextTabIndex(0, 1), 0);
    expect(nextTabIndex(0, 0), 0);
  });

  bool bound(Map<ShortcutActivator, VoidCallback> map, LogicalKeyboardKey key,
          {bool control = false, bool meta = false, bool shift = false}) =>
      map.keys.whereType<SingleActivator>().any(
            (a) =>
                a.trigger == key &&
                a.control == control &&
                a.meta == meta &&
                a.shift == shift,
          );

  test('close is bound only when a tab may be closed', () {
    void noop() {}
    final many = tabShortcutBindings(
      newTab: noop,
      close: noop,
      next: noop,
      previous: noop,
    );
    final one = tabShortcutBindings(
      newTab: noop,
      close: null,
      next: noop,
      previous: noop,
    );
    expect(bound(many, LogicalKeyboardKey.keyW, control: true), isTrue);
    expect(bound(many, LogicalKeyboardKey.keyW, meta: true), isTrue);
    expect(bound(one, LogicalKeyboardKey.keyW, control: true), isFalse);
    for (final map in [many, one]) {
      expect(bound(map, LogicalKeyboardKey.keyT, control: true), isTrue);
      expect(bound(map, LogicalKeyboardKey.keyT, meta: true), isTrue);
      expect(bound(map, LogicalKeyboardKey.tab, control: true), isTrue);
      expect(
        bound(map, LogicalKeyboardKey.tab, control: true, shift: true),
        isTrue,
      );
      // Cmd+Tab is the macOS app switcher; binding it would be dead code.
      expect(bound(map, LogicalKeyboardKey.tab, meta: true), isFalse);
    }
  });
}
