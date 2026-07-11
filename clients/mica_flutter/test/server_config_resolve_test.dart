import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/main.dart';

// P3c-2: the legacy serverMode/serverUrl prefs migrate ONCE into the dissolved
// model — which cloud server is configured (cloudOrigin) and which world starts
// active (activeOrigin). Same intent as the old ServerConfig.resolve tests:
// fresh desktop installs are local-first, but existing online users must not be
// stranded behind that default.
void main() {
  ({String cloudOrigin, String activeOrigin}) resolve({
    String? mode,
    String url = '',
    String token = '',
    bool web = false,
  }) => resolveLegacyCloudSetup(
    savedMode: mode,
    savedUrl: url,
    authToken: token,
    isWeb: web,
  );

  group('resolveLegacyCloudSetup — fresh-install default', () {
    test('a truly fresh desktop install starts in the local world', () {
      final r = resolve(); // no mode, no token, no url, desktop
      expect(r.activeOrigin, 'local');
      expect(r.cloudOrigin, kMicaCloudUrl,
          reason: 'a cloud server is still configured (sign-in is one click)');
    });

    test('a returning signed-in user (auth token) starts CLOUD-active', () {
      final r = resolve(token: 'jwt.abc.def');
      expect(r.activeOrigin, isNot('local'),
          reason: 'upgrading must not drop an existing account into local');
      expect(r.activeOrigin, r.cloudOrigin);
    });

    test('a user who had set a server URL keeps it, cloud-active', () {
      final r = resolve(url: 'https://my.server');
      expect(r.cloudOrigin, 'https://my.server');
      expect(r.activeOrigin, 'https://my.server');
    });

    test('web is always cloud-active (no on-device store on web)', () {
      final r = resolve(web: true);
      expect(r.activeOrigin, isNot('local'));
    });
  });

  group('resolveLegacyCloudSetup — explicit + legacy modes', () {
    test('local starts local; the default cloud server stays configured', () {
      final r = resolve(mode: 'local');
      expect(r.activeOrigin, 'local');
      expect(r.cloudOrigin, kMicaCloudUrl);
    });

    test('legacy "cloud" migrates to Mica Cloud, cloud-active', () {
      final r = resolve(mode: 'cloud');
      expect(r.cloudOrigin, kMicaCloudUrl);
      expect(r.activeOrigin, kMicaCloudUrl);
    });

    test('legacy "self" keeps its URL, cloud-active', () {
      final r = resolve(mode: 'self', url: 'https://home.lan:8080');
      expect(r.cloudOrigin, 'https://home.lan:8080');
      expect(r.activeOrigin, 'https://home.lan:8080');
    });

    test('"online" keeps its URL', () {
      final r = resolve(mode: 'online', url: 'https://mica.example.com');
      expect(r.cloudOrigin, 'https://mica.example.com');
      expect(r.activeOrigin, 'https://mica.example.com');
    });
  });

  group('resolveLegacyCloudSetup — origin normalization (P3c-2 review)', () {
    // Everything downstream keys off Dart-normalized URLs; a raw legacy string
    // would file the auth token under a never-read-again key = silent sign-out.
    test('mixed-case host normalizes to the canonical lowercase form', () {
      final r = resolve(mode: 'self', url: 'https://Notes.MyCompany.com');
      expect(r.cloudOrigin, 'https://notes.mycompany.com');
      expect(
        r.cloudOrigin,
        Uri.parse(r.cloudOrigin).toString(),
        reason: 'the emitted origin is a normalization fixed point',
      );
    });

    test('explicit default port is stripped', () {
      final r = resolve(mode: 'self', url: 'https://my.server:443');
      expect(r.cloudOrigin, 'https://my.server');
    });

    test("'local' with a configured self-host keeps THAT origin (token files "
        'under the right server, never sent to Mica Cloud)', () {
      final r = resolve(
        mode: 'local',
        url: 'https://home.lan:8080',
        token: 'jwt.stale.selfhost',
      );
      expect(r.activeOrigin, 'local');
      expect(r.cloudOrigin, 'https://home.lan:8080');
    });

    test('token with no saved URL falls back to the build-default server '
        '(legacy parity — dev setups keep working)', () {
      final r = resolve(token: 'jwt.abc.def');
      expect(r.cloudOrigin, ApiClient.defaultBaseUri().toString());
    });
  });
}
