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
}
