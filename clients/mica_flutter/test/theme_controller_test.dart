import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/theme_controller.dart';

/// The pref is the only part of this that can silently ruin someone's setup, so
/// that is what the tests are about.
void main() {
  test('the pref stores the NAME, so reordering the enum is harmless', () {
    // An index would mean a future enum insertion turns everyone's "dark" into
    // something else on upgrade.
    expect(themeModePref(MicaThemeMode.system), 'system');
    expect(themeModePref(MicaThemeMode.light), 'light');
    expect(themeModePref(MicaThemeMode.dark), 'dark');
  });

  test('every mode round-trips through the pref', () {
    for (final mode in MicaThemeMode.values) {
      expect(parseThemeMode(themeModePref(mode)), mode);
    }
  });

  test('anything unrecognised follows the system', () {
    // Absent (first run), empty, an old spelling, junk — "follow the OS" is the
    // only answer that is never wrong for someone who never chose.
    for (final raw in [null, '', 'System', 'DARK', 'auto', 'true', '2']) {
      expect(parseThemeMode(raw), MicaThemeMode.system, reason: 'raw=$raw');
    }
  });

  test('the mode maps onto Flutter ThemeMode, which resolves "system"', () {
    expect(MicaThemeMode.system.material, ThemeMode.system);
    expect(MicaThemeMode.light.material, ThemeMode.light);
    expect(MicaThemeMode.dark.material, ThemeMode.dark);
  });

  test('a resolved brightness picks the matching palette', () {
    expect(tokensForBrightness(Brightness.dark).dark, isTrue);
    expect(tokensForBrightness(Brightness.light).dark, isFalse);
  });

  test('main() seeds the palette BEFORE runApp, like the locale', () {
    // The bug this pins had nothing wrong with any function above: every one of
    // them returned the right answer. The palette was simply assigned one frame
    // too late — from the shell's initState, which runs inside MaterialApp's
    // first build, where notifying an ancestor cannot rebuild it. The app opened
    // in the default palette while Settings showed 「深色」 ticked, and only
    // touching the setting fixed it. Desktop reproduced it every launch; the web
    // build happened to win the same race, so no unit test over these functions
    // could have caught it — the invariant is an ORDERING in main(), so that is
    // what gets asserted.
    final source = File('lib/main.dart').readAsStringSync();
    final seeded = source.indexOf('loadPersistedThemeMode()');
    final started = source.indexOf('runApp(');

    expect(
      seeded,
      isNonNegative,
      reason: 'main() must seed the palette at all',
    );
    expect(started, isNonNegative);
    expect(
      seeded,
      lessThan(started),
      reason: 'seeding after runApp is the bug: the first frame is already out',
    );
  });
}
