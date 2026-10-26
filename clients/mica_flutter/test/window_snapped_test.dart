// Guards the FFI signature of `isWindowSnapped`, which runs on EVERY window
// move and resize.
//
// A wrong native signature does not throw — it corrupts the stack and takes the
// process down, so the try/catch inside the function cannot save it. Calling it
// here is what catches that: if the typedefs drift from user32's real ABI, this
// test crashes the runner instead of shipping a build that dies whenever the
// user drags their window.
//
// What it deliberately does NOT assert: that a snapped window returns true.
// That needs a real snapped HWND, which a headless test cannot produce.
// (Verified out-of-band on Windows 11: IsWindowArranged is True for half- and
// quarter-snap, False floating and False maximized — the distinction the
// bounds persister relies on, since a snapped window reports SW_SHOWNORMAL.)
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/window_snapped_win.dart';

void main() {
  test('isWindowSnapped is callable and total', () {
    expect(isWindowSnapped(), isA<bool>());
    // Repeat: the lookup allocates and frees a UTF-16 string each call, and
    // this runs per move event — a leak or double-free surfaces under repetition.
    for (var i = 0; i < 200; i++) {
      isWindowSnapped();
    }
  });

  test('off-Windows it is false, never an exception', () {
    if (Platform.isWindows) return;
    expect(
      isWindowSnapped(),
      isFalse,
      reason:
          'the persister calls this unconditionally on desktop; on '
          'macOS/Linux it must degrade to "not snapped", not throw',
    );
  });

  group('snap decision uses only this process window', () {
    int Function(int) enumeration(List<int> windows) => (after) {
      final index = windows.indexOf(after) + 1;
      return index < windows.length ? windows[index] : 0;
    };

    test(
      'skips a snapped foreign Flutter window before our floating window',
      () {
        if (!Platform.isWindows) return;
        final checked = <int>[];
        expect(
          isWindowSnapped(
            nextOfClass: enumeration([100, 200]),
            ownerPid: (h) => h == 200 ? pid : pid + 1,
            arranged: (h) {
              checked.add(h);
              return h == 100;
            },
          ),
          isFalse,
        );
        expect(checked, [
          200,
        ], reason: 'only our HWND may reach IsWindowArranged');
      },
    );

    test('our snapped window wins over a floating foreign window', () {
      if (!Platform.isWindows) return;
      expect(
        isWindowSnapped(
          nextOfClass: enumeration([100, 200]),
          ownerPid: (h) => h == 200 ? pid : pid + 1,
          arranged: (h) => h == 200,
        ),
        isTrue,
      );
    });

    test('only foreign or missing windows safely return false', () {
      if (!Platform.isWindows) return;
      for (final windows in [
        <int>[],
        [100],
      ]) {
        expect(
          isWindowSnapped(
            nextOfClass: enumeration(windows),
            ownerPid: (_) => pid + 1,
            arranged: (_) => throw StateError('foreign HWND was inspected'),
          ),
          isFalse,
        );
      }
    });
  });
}
