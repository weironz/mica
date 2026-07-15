import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/api/session_refresher.dart';

// A session used to die 24h after login with no way to renew it: the server
// already returned `expires_at`, the client threw it away, and there was no
// refresh endpoint at all. What the client now has to get right is narrow but
// sharp — a refresh token is SINGLE-USE, and spending one twice is what the
// server reads as theft.

/// A JWT-shaped token whose `exp` is [expiry]. Only the payload is real; the
/// signature is never checked client-side (and must not be — the server is what
/// enforces it).
String tokenExpiring(DateTime expiry) {
  String seg(Map<String, dynamic> m) =>
      base64Url.encode(utf8.encode(jsonEncode(m))).replaceAll('=', '');
  return '${seg({'alg': 'HS256'})}.'
      '${seg({'sub': 'u1', 'exp': expiry.millisecondsSinceEpoch ~/ 1000})}.'
      'sig';
}

void main() {
  group('AuthSession', () {
    Map<String, dynamic> payload({String? refresh}) => {
          'access_token': tokenExpiring(
            DateTime.now().toUtc().add(const Duration(hours: 24)),
          ),
          if (refresh != null) 'refresh_token': refresh,
          'user': {
            'id': 'u1',
            'email': 'a@b.c',
            'display_name': 'A',
          },
        };

    test('the refresh token is read off the login response', () {
      // It was silently dropped, which is why sessions could not be renewed.
      final s = AuthSession.fromJson(payload(refresh: 'mica_rt_abc'));
      expect(s.refreshToken, 'mica_rt_abc');
    });

    test('a server without refresh tokens still yields a usable session', () {
      // Old server / self-hosted on an older build: no refresh_token in the
      // response. That must degrade to the previous behaviour, not crash.
      final s = AuthSession.fromJson(payload());
      expect(s.refreshToken, isEmpty);
      expect(s.accessToken, isNotEmpty);
    });

    test('expiresAt is read from the access token itself', () {
      final expiry = DateTime.now().toUtc().add(const Duration(hours: 3));
      final s = AuthSession(
        accessToken: tokenExpiring(expiry),
        user: const User(id: 'u1', email: 'a@b.c', displayName: 'A'),
      );
      expect(s.expiresAt, isNotNull);
      // Whole seconds: `exp` has no sub-second resolution.
      expect(
        s.expiresAt!.difference(expiry).inSeconds.abs(),
        lessThanOrEqualTo(1),
      );
    });

    test('a token with no readable exp reports null, it does not throw', () {
      // Never assume the token is a JWT we can parse — an opaque one must not
      // take the app down, it just means "renew reactively instead".
      const s = AuthSession(
        accessToken: 'not-a-jwt',
        user: User(id: 'u1', email: 'a@b.c', displayName: 'A'),
      );
      expect(s.expiresAt, isNull);
      expect(jwtExpiry(''), isNull);
      expect(jwtExpiry('a.b'), isNull);
      expect(jwtExpiry('a.!!!.c'), isNull);
    });

    test('copyWith keeps the refresh token', () {
      // The bug this pins: renaming yourself rebuilt AuthSession from scratch,
      // dropping refreshToken to '' — you'd silently lose the ability to renew
      // and get signed out that night.
      const s = AuthSession(
        accessToken: 'a',
        refreshToken: 'mica_rt_keepme',
        user: User(id: 'u1', email: 'a@b.c', displayName: 'A'),
      );
      final renamed =
          s.copyWith(user: const User(id: 'u1', email: 'a@b.c', displayName: 'B'));
      expect(renamed.refreshToken, 'mica_rt_keepme');
      expect(renamed.user.displayName, 'B');
      expect(renamed.accessToken, 'a');
    });

    test('copyWith swaps in a rotated pair', () {
      const s = AuthSession(
        accessToken: 'old',
        refreshToken: 'rt_old',
        user: User(id: 'u1', email: 'a@b.c', displayName: 'A'),
      );
      final next = s.copyWith(accessToken: 'new', refreshToken: 'rt_new');
      expect(next.accessToken, 'new');
      expect(next.refreshToken, 'rt_new');
      expect(next.user.id, 'u1');
    });
  });

  group('ApiException', () {
    test('carries the status so 401 is identifiable', () {
      // Dropping the status is why an expired session surfaced as a bare
      // `unauthorized` banner: nothing downstream could tell it apart from any
      // other failure, so nothing could say "sign in again".
      const e = ApiException('unauthorized', statusCode: 401);
      expect(e.isUnauthorized, isTrue);
    });

    test('other failures are not mistaken for an expired session', () {
      expect(const ApiException('bad request', statusCode: 400).isUnauthorized,
          isFalse);
      expect(const ApiException('nope', statusCode: 403).isUnauthorized, isFalse);
      expect(const ApiException('boom', statusCode: 500).isUnauthorized, isFalse);
    });

    test('a 400 that merely says "unauthorized" is not a 401', () {
      // The old check sniffed the message text for the word. A status is a
      // fact; a message is prose.
      const e = ApiException('unauthorized image host', statusCode: 400);
      expect(e.isUnauthorized, isFalse);
    });

    test('a hand-thrown exception has no status and is not a 401', () {
      const e = ApiException('Select a page first.');
      expect(e.statusCode, isNull);
      expect(e.isUnauthorized, isFalse);
    });
  });

  AuthSession sessionExpiringIn(Duration d, {String refresh = 'rt'}) =>
      AuthSession(
        accessToken: tokenExpiring(DateTime.now().toUtc().add(d)),
        refreshToken: refresh,
        user: const User(id: 'u1', email: 'a@b.c', displayName: 'A'),
      );

  // These drive the REAL SessionRefresher the app wires up — a re-implementation
  // of the rules in the test file would happily stay green while the shipped
  // code drifted.
  group('SessionRefresher: renew early', () {
    SessionRefresher never() =>
        SessionRefresher(refresh: (_) async => fail('must not refresh'));

    test('a token good for hours is left alone', () async {
      final r = never();
      final s = sessionExpiringIn(const Duration(hours: 12));
      expect(r.needsRenewal(s), isFalse);
      expect(await r.ensureFresh(s), isNull);
    });

    test('a token inside the lead window renews', () {
      expect(never().needsRenewal(sessionExpiringIn(const Duration(minutes: 2))),
          isTrue);
    });

    test('an already-expired token renews', () {
      expect(never().needsRenewal(sessionExpiringIn(const Duration(hours: -1))),
          isTrue);
    });

    test('an unparseable token never triggers renewal', () {
      // Otherwise every call would refresh — and each refresh rotates, so that
      // is a token-burning treadmill.
      const s = AuthSession(
        accessToken: 'opaque',
        refreshToken: 'rt',
        user: User(id: 'u1', email: 'a@b.c', displayName: 'A'),
      );
      expect(never().needsRenewal(s), isFalse);
    });

    test('no refresh token means nothing to renew with', () {
      final s = sessionExpiringIn(const Duration(hours: -1), refresh: '');
      expect(never().needsRenewal(s), isFalse);
    });

    test('the lead is honoured as given', () {
      final r = SessionRefresher(
        refresh: (_) async => fail('must not refresh'),
        lead: const Duration(hours: 2),
      );
      expect(r.needsRenewal(sessionExpiringIn(const Duration(hours: 1))), isTrue);
      expect(r.needsRenewal(sessionExpiringIn(const Duration(hours: 3))), isFalse);
    });
  });

  group('SessionRefresher: never two at once', () {
    // THE hazard of rotation: a refresh token is single-use, and the server
    // cannot tell our own second spend from a stolen one — it burns the whole
    // sign-in. Every API call funnels through _run, so two overlapping calls
    // near expiry would sign the user out by our own hand.
    test('concurrent callers share one refresh, not one each', () async {
      var refreshes = 0;
      final r = SessionRefresher(refresh: (_) async {
        refreshes++;
        await Future<void>.delayed(const Duration(milliseconds: 20));
        return sessionExpiringIn(const Duration(hours: 24));
      });
      final expiring = sessionExpiringIn(const Duration(minutes: 1));

      final results = await Future.wait([
        r.ensureFresh(expiring),
        r.ensureFresh(expiring),
        r.ensureFresh(expiring),
      ]);

      expect(refreshes, 1,
          reason: 'a second spend of a rotating token reads as theft — the '
              'server would kill the session we were trying to keep alive');
      // Everyone gets the renewed session, not just the caller who won.
      expect(results.every((s) => s != null), isTrue);
    });

    test('a later caller refreshes again once the first has finished', () async {
      var refreshes = 0;
      final r = SessionRefresher(refresh: (_) async {
        refreshes++;
        return sessionExpiringIn(const Duration(hours: 24));
      });
      final expiring = sessionExpiringIn(const Duration(minutes: 1));

      await r.ensureFresh(expiring);
      await r.ensureFresh(expiring);
      expect(refreshes, 2, reason: 'the latch must clear, not wedge shut');
    });

    test('a failed refresh clears the latch', () async {
      // whenComplete, not then: if a failure left the latch set, every later
      // renewal would silently no-op and the session would die anyway.
      var attempts = 0;
      final r = SessionRefresher(refresh: (_) async {
        attempts++;
        throw const ApiException('boom', statusCode: 500);
      });
      final expiring = sessionExpiringIn(const Duration(minutes: 1));

      await expectLater(r.ensureFresh(expiring), throwsA(isA<ApiException>()));
      await expectLater(r.ensureFresh(expiring), throwsA(isA<ApiException>()));
      expect(attempts, 2);
    });

    test('the token spent is the one the session currently holds', () async {
      final spent = <String>[];
      final r = SessionRefresher(refresh: (t) async {
        spent.add(t);
        return sessionExpiringIn(const Duration(hours: 24));
      });
      await r.ensureFresh(
        sessionExpiringIn(const Duration(minutes: 1), refresh: 'mica_rt_current'),
      );
      expect(spent, ['mica_rt_current']);
    });
  });
}
