// Which palette is in effect, and where that choice is remembered.
//
// A library-level notifier, the same shape as `localeController`: the choice has
// to be readable ABOVE `MaterialApp` (that is where `theme` / `darkTheme` are
// set) while being written from Settings, which lives far below it. Threading it
// through every widget in between would be a lot of plumbing for one enum.
//
// Resolving "system" is deliberately NOT done here. `MaterialApp` already knows
// the platform brightness and already picks between `theme` and `darkTheme`;
// asking it afterwards (`Theme.of(context).brightness`) is one source of truth.
// Computing it a second time from `PlatformDispatcher` would be a second answer
// that can disagree — typically for one frame after the OS flips.

import 'package:flutter/material.dart';

import 'prefs.dart';
import 'ui/theme_tokens.dart';

/// The user's choice, not the resulting brightness.
enum MicaThemeMode {
  /// Follow the OS.
  system,
  light,
  dark;

  ThemeMode get material => switch (this) {
    MicaThemeMode.system => ThemeMode.system,
    MicaThemeMode.light => ThemeMode.light,
    MicaThemeMode.dark => ThemeMode.dark,
  };
}

/// The pref value for [mode]. Stable strings, not indices: reordering the enum
/// must not silently turn everyone's "dark" into "light".
String themeModePref(MicaThemeMode mode) => mode.name;

/// Read a stored pref back.
///
/// Anything unrecognised — absent, empty, an old spelling, junk — reads as
/// [MicaThemeMode.system], because following the OS is the one answer that is
/// never wrong for a user who never chose.
MicaThemeMode parseThemeMode(String? raw) => switch (raw) {
  'light' => MicaThemeMode.light,
  'dark' => MicaThemeMode.dark,
  _ => MicaThemeMode.system,
};

/// The palette for an already-resolved brightness.
MicaTokens tokensForBrightness(Brightness brightness) =>
    brightness == Brightness.dark ? MicaTokens.dark_ : MicaTokens.light;

/// The live choice. Seeded by [loadPersistedThemeMode] before runApp, then set
/// from Settings; `MicaApp` listens.
final ValueNotifier<MicaThemeMode> themeModeController =
    ValueNotifier<MicaThemeMode>(MicaThemeMode.system);

/// Load the persisted choice into [themeModeController].
///
/// **Call this in main() before runApp**, the same way the locale is seeded.
/// Assigning it any later does not work, and fails in a way that looks like
/// nothing at all: the shell used to do it from `initState`, which runs while
/// `MaterialApp` is already building the first frame. A notifier assigned then
/// cannot rebuild an ancestor that has finished building — so the app came up in
/// the DEFAULT palette while Settings correctly showed 「深色」 ticked, and only
/// touching the setting (a real change, so a real notification) fixed it. Two
/// readings of the same notifier, one frame apart, disagreeing.
void loadPersistedThemeMode() {
  themeModeController.value = parseThemeMode(loadPref('themeMode'));
}
