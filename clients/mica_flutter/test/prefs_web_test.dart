@TestOn('browser')
library;

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/prefs.dart';
import 'package:web/web.dart' as web;

void main() {
  test(
    'first preference read sweeps all legacy secrets without losing prefs or session',
    () {
      final storage = web.window.localStorage;
      storage.clear();
      addTearDown(() => storage.clear());
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
      storage.setItem('mica.cloudOrigin', 'https://a.example');
      storage.setItem('mica.themeMode', 'dark');
      storage.setItem('mica.authUser:https://a.example', 'will');
      storage.setItem('other.authToken', 'theirs');
      storage.setItem('mica.authTokenBackup', 'ordinary preference');
      storage.setItem('mica.refreshToken', 'not in the secret-key rule');

      // Startup reads ordinary prefs before choosing a server or refreshing its
      // cookie. Even the old tokens for inactive servers must disappear here.
      expect(loadPref('cloudOrigin'), 'https://a.example');
      for (final key in secrets) {
        expect(storage.getItem(key), isNull, reason: '$key must be purged');
      }
      expect(loadPref('authToken'), isNull);
      expect(loadPref('authToken:https://b.example'), isNull);
      expect(storage.getItem('mica.cloudOrigin'), 'https://a.example');
      expect(storage.getItem('mica.themeMode'), 'dark');
      expect(storage.getItem('mica.authUser:https://a.example'), 'will');
      expect(storage.getItem('other.authToken'), 'theirs');
      expect(storage.getItem('mica.authTokenBackup'), 'ordinary preference');
      expect(
        storage.getItem('mica.refreshToken'),
        'not in the secret-key rule',
      );

      // Repeated startup reads are harmless. The live session remains memory
      // only; cleanup must never clear the current page's credentials.
      savePref('authToken:https://a.example', 'live-session');
      for (var i = 0; i < 3; i++) {
        expect(loadPref('cloudOrigin'), 'https://a.example');
        expect(loadPref('authToken:https://a.example'), 'live-session');
        expect(storage.getItem('mica.authToken:https://a.example'), isNull);
      }
      savePref('themeMode', 'light');
      expect(storage.getItem('mica.themeMode'), 'light');
    },
  );
}
