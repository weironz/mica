import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/main.dart';

void main() {
  ServerConfig resolve({
    String? mode,
    String url = '',
    String token = '',
    bool web = false,
  }) =>
      ServerConfig.resolve(
        savedMode: mode,
        savedUrl: url,
        authToken: token,
        isWeb: web,
      );

  group('ServerConfig.resolve — fresh-install default', () {
    test('a truly fresh desktop install is local-first', () {
      final c = resolve(); // no mode, no token, no url, desktop
      expect(c.mode, ServerMode.localOffline);
    });

    test('a returning signed-in user (auth token) stays ONLINE, not stranded', () {
      final c = resolve(token: 'jwt.abc.def');
      expect(c.mode, ServerMode.online,
          reason: 'upgrading must not drop an existing account into local mode');
    });

    test('a user who had set a server URL stays online', () {
      final c = resolve(url: 'https://my.server');
      expect(c.mode, ServerMode.online);
      expect(c.url, 'https://my.server');
    });

    test('web is always online (no on-device store on web)', () {
      expect(resolve(web: true).mode, ServerMode.online);
    });
  });

  group('ServerConfig.resolve — explicit + legacy modes', () {
    test('local stays local', () {
      expect(resolve(mode: 'local').mode, ServerMode.localOffline);
    });

    test('legacy "cloud" migrates to online @ Mica Cloud', () {
      final c = resolve(mode: 'cloud');
      expect(c.mode, ServerMode.online);
      expect(c.url, kMicaCloudUrl);
    });

    test('legacy "self" migrates to online, keeping its URL', () {
      final c = resolve(mode: 'self', url: 'https://home.lan:8080');
      expect(c.mode, ServerMode.online);
      expect(c.url, 'https://home.lan:8080');
    });

    test('"online" keeps its URL', () {
      final c = resolve(mode: 'online', url: 'https://mica.example.com');
      expect(c.mode, ServerMode.online);
      expect(c.url, 'https://mica.example.com');
    });
  });
}
