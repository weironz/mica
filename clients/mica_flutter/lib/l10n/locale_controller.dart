/// The app's UI language, as a single app-wide [ValueNotifier]. `null` = follow
/// the system (Flutter resolves the OS locale against supportedLocales); a
/// concrete [Locale] is the user's explicit override. MicaApp listens and
/// rebuilds MaterialApp with `locale:`; Settings writes through [setLanguage].
///
/// Persisted under `uiLanguage` as a choice token ('system' | 'zh' | 'en') so
/// the stored value stays stable even if we later add locale variants.
library;

import 'package:flutter/widgets.dart';

import '../prefs.dart';
import 'app_localizations.dart';

/// `context.l10n.someKey` — terser than `AppLocalizations.of(context).someKey`.
extension AppLocalizationsX on BuildContext {
  AppLocalizations get l10n => AppLocalizations.of(this);
}

/// Look up strings WITHOUT a BuildContext — for the desktop tray menu / window
/// close listener, which live outside the widget tree. Resolves the user's
/// override, else the OS locale, falling back to the first supported locale.
AppLocalizations get l10nNoContext {
  final override = localeController.value;
  if (override != null) return lookupAppLocalizations(override);
  final system = WidgetsBinding.instance.platformDispatcher.locale;
  final matched = kSupportedLocales.any((l) => l.languageCode == system.languageCode)
      ? Locale(system.languageCode)
      : kSupportedLocales.first;
  return lookupAppLocalizations(matched);
}

const String _prefKey = 'uiLanguage';

/// Choice tokens (also the persisted values). Kept as constants so Settings and
/// the loader agree.
const String kLangSystem = 'system';
const String kLangChinese = 'zh';
const String kLangEnglish = 'en';

/// The locales the app ships translations for. Order matters: the first is the
/// fallback when the system locale matches none.
const List<Locale> kSupportedLocales = [Locale('en'), Locale('zh')];

/// null = follow system. Seeded by [loadPersistedLocale] before runApp.
final ValueNotifier<Locale?> localeController = ValueNotifier<Locale?>(null);

/// Load the persisted choice into [localeController]. Call in main() before
/// runApp so the first frame already renders in the chosen language.
void loadPersistedLocale() {
  localeController.value = _localeFor(loadPref(_prefKey));
}

/// The current choice token ('system' | 'zh' | 'en') — for the Settings selector.
String get currentLanguageChoice {
  final locale = localeController.value;
  if (locale == null) return kLangSystem;
  return locale.languageCode == kLangChinese ? kLangChinese : kLangEnglish;
}

/// Apply and persist a language choice. [kLangSystem] clears the override so the
/// app follows the OS language again.
void setLanguage(String choice) {
  localeController.value = _localeFor(choice);
  savePref(_prefKey, choice);
}

Locale? _localeFor(String? choice) {
  switch (choice) {
    case kLangChinese:
      return const Locale('zh');
    case kLangEnglish:
      return const Locale('en');
    default:
      return null; // 'system' or unset → follow the OS
  }
}
