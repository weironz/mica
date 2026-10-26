// Browser-only regression harness for #141. Compile with `dart compile js`,
// load beside its generated JS in an isolated browser profile, and inspect
// body[data-result]. This exercises the production web preference API and real
// localStorage without booting the Flutter renderer.
import 'package:mica_flutter/prefs_web.dart';
import 'package:web/web.dart' as web;

void check(bool condition, String message) {
  if (!condition) throw StateError(message);
}

void main() {
  final storage = web.window.localStorage;
  try {
    storage.clear();
    const secrets = [
      'mica.authToken',
      'mica.authToken:https://a.example',
      'mica.authToken:https://b.example',
      'mica.refreshToken:https://a.example',
      'mica.refreshToken:https://b.example',
    ];
    for (final key in secrets) {
      storage.setItem(key, 'obsolete plaintext token');
    }
    const ordinary = {
      'mica.cloudOrigin': 'https://a.example',
      'mica.themeMode': 'dark',
      'mica.authUser:https://a.example': 'will',
      'other.authToken': 'theirs',
      'mica.authTokenBackup': 'ordinary preference',
      'mica.refreshToken': 'outside the secret-key rule',
    };
    ordinary.forEach((key, value) => storage.setItem(key, value));
    check(loadPref('cloudOrigin') == 'https://a.example', 'origin lost');
    for (final key in secrets) {
      check(storage.getItem(key) == null, '$key was not purged');
    }
    ordinary.forEach((key, value) {
      check(storage.getItem(key) == value, '$key was changed');
    });
    check(loadPref('authToken') == null, 'legacy token entered memory');
    savePref('authToken:https://a.example', 'live-session');
    for (var i = 0; i < 3; i++) {
      check(loadPref('cloudOrigin') == 'https://a.example', 'origin lost');
      check(
        loadPref('authToken:https://a.example') == 'live-session',
        'current session lost',
      );
      check(
        storage.getItem('mica.authToken:https://a.example') == null,
        'current token persisted',
      );
    }
    web.document.body!.textContent =
        'PASS: legacy secrets purged; prefs and session preserved';
    web.document.body!.setAttribute('data-result', 'pass');
  } catch (error) {
    web.document.body!.textContent = 'FAIL: $error';
    web.document.body!.setAttribute('data-result', 'fail');
  } finally {
    storage.clear();
  }
}
