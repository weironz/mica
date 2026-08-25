import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/client.dart';

/// Both rules here were found by a development build quietly signing in to the
/// PRODUCTION server, with a write-heavy test about to run against it.
void main() {
  group('a blank saved origin is absent, not a server', () {
    test('nothing configured anywhere keeps the existing default', () {
      expect(resolveApiBase(pinned: '', saved: ''), isNull);
    });

    test('the empty string does not become a Uri with no host', () {
      // Uri.tryParse('') SUCCEEDS. That is the whole trap: the old code checked
      // for null, got a valid Uri whose host is '', and assigned it — after
      // which `_isLocalBackend()` (which reads the host) went false on every
      // fresh profile and dev auto-login switched itself off.
      expect(Uri.tryParse(''), isNotNull, reason: 'documents the trap');
      expect(Uri.tryParse('')!.host, isEmpty);
      expect(resolveApiBase(pinned: '', saved: ''), isNull);
    });

    test('a bare word is not a server either', () {
      // 'localhost' parses as a relative reference with no authority. Accepting
      // it would produce requests to a path, not a host.
      expect(resolveApiBase(pinned: '', saved: 'localhost'), isNull);
    });
  });

  group('a build-time pin outranks the saved origin', () {
    test('the pin wins even when a real server is saved', () {
      // The shipped app never sets MICA_API_BASE_URL, so this only fires for a
      // deliberate dev/CI build — which must not inherit the installed copy's
      // server out of shared prefs.
      final base = resolveApiBase(
        pinned: 'http://127.0.0.1:8080',
        saved: 'https://cloud.example.com',
      );
      expect(base?.host, '127.0.0.1');
      expect(base?.port, 8080);
    });

    test('with no pin, the saved origin is used', () {
      final base = resolveApiBase(
        pinned: '',
        saved: 'https://cloud.example.com',
      );
      expect(base?.host, 'cloud.example.com');
    });

    test('a blank pin does not shadow a good saved origin', () {
      // `--dart-define=MICA_API_BASE_URL=` resolves to the empty string, not to
      // "unset" — the same blank-vs-absent rule, applied to the other input.
      final base = resolveApiBase(
        pinned: '',
        saved: 'https://cloud.example.com',
      );
      expect(base?.host, 'cloud.example.com');
    });
  });
}
