import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:mica_flutter/api/client.dart';
import 'package:mica_flutter/main.dart' as app;
import 'package:mica_flutter/prefs.dart';

import 'support/online_only_local_offline.dart';

/// Issue #147: a startup refresh error must only clear credentials when the
/// sign-in that STARTED the request is still the one in force.
///
/// The shell's restore chain captures `origin` and checks `stale()` on its
/// success branches, but both refresh `on ApiException` branches used to call
/// `_auth.clearPersistedSession()` unconditionally. That writes empty values
/// under whatever origin is CURRENT when the refusal lands — so a refusal for
/// a request sent before a re-login (or a server switch) erased the freshly
/// persisted credentials of the account now in use.
///
/// Seam: the real [MicaApp] with a scripted `MockClient`; the refresh request
/// for the persisted (expired) session is held on a Completer while the user
/// signs in through the gate that is on screen, then released as a 401.
void main() {
  setUp(useOnlineOnlyLocalOffline);
  tearDown(resetLocalOfflineFactory);
  tearDown(resetHttpClientForTesting);

  /// Bounded settle: the app keeps background work (profile/AI/settings
  /// fetches) alive, so `pumpAndSettle` is not dependable here.
  Future<void> settle(WidgetTester tester, [int frames = 12]) async {
    for (var i = 0; i < frames; i++) {
      await tester.pump(const Duration(milliseconds: 20));
    }
  }

  String originFor(String tag) =>
      'https://restore-$tag-${DateTime.now().microsecondsSinceEpoch}.test';

  void seed(String origin, {required String accessToken}) {
    savePref('cloudOrigin', origin);
    savePref('activeOrigin', origin);
    savePref('servers', jsonEncode([origin]));
    savePref(
      'authUser:$origin',
      jsonEncode({
        'id': 'u1',
        'email': 'old@mica.test',
        'display_name': 'Old User',
      }),
    );
    savePref('authToken:$origin', accessToken);
    savePref('refreshToken:$origin', 'rt-old');
    removePref('lastWorkspaceId');
  }

  http.Response userJson(String id, String email) => http.Response(
    jsonEncode({'id': id, 'email': email, 'display_name': 'User $id'}),
    200,
    headers: {'content-type': 'application/json; charset=utf-8'},
  );

  http.Response workspacesJson() => http.Response(
    jsonEncode({
      'workspaces': [
        {'id': 'w1', 'name': 'W', 'owner_id': 'u1', 'role': 'owner'},
      ],
    }),
    200,
    headers: {'content-type': 'application/json; charset=utf-8'},
  );

  testWidgets(
    'a refusal for the refresh that started before a re-login must keep the '
    'new credentials (issue #147)',
    (tester) async {
      final origin = originFor('relogin');
      seed(origin, accessToken: _expiredToken());

      final refreshGate = Completer<http.Response>();
      final newToken = _nonExpiredToken();
      var sawRefresh = false;
      useHttpClientForTesting(
        MockClient((request) async {
          final path = request.url.path;
          if (path == '/api/auth/refresh') {
            sawRefresh = true;
            return refreshGate.future;
          }
          if (path == '/api/auth/login') {
            return http.Response(
              jsonEncode({
                'access_token': newToken,
                'refresh_token': 'rt-new',
                'user': {
                  'id': 'u2',
                  'email': 'new@mica.test',
                  'display_name': 'New User',
                },
              }),
              200,
              headers: {'content-type': 'application/json; charset=utf-8'},
            );
          }
          if (path == '/api/workspaces') return workspacesJson();
          if (path == '/api/auth/me') return userJson('u2', 'new@mica.test');
          if (path == '/api/workspaces/w1/views') {
            return http.Response(jsonEncode({'views': []}), 200);
          }
          if (path == '/api/workspaces/w1/members') {
            return http.Response(jsonEncode({'members': []}), 200);
          }
          return http.Response('{}', 200);
        }),
      );

      await tester.pumpWidget(const app.MicaApp());
      // Let the post-frame restore chain reach the held refresh request.
      for (var i = 0; i < 40 && !sawRefresh; i++) {
        await tester.pump(const Duration(milliseconds: 20));
      }
      expect(
        sawRefresh,
        isTrue,
        reason: 'the expired persisted token must be renewed at startup',
      );

      // While that request is in flight, sign in again through the gate the
      // shell is showing. This is the "A refresh pending -> A re-login"
      // schedule from the report.
      final fields = find.byType(TextField);
      expect(fields, findsNWidgets(2), reason: 'login mode: email + password');
      await tester.enterText(fields.at(0), 'new@mica.test');
      await tester.enterText(fields.at(1), 'secret');
      await tester.pump();
      await tester.tap(find.byIcon(Icons.login));
      await settle(tester);

      expect(
        loadPref('authToken:$origin'),
        newToken,
        reason: 'the re-login persisted its own credentials',
      );

      // The stale request now answers 401. It belongs to the sign-in that was
      // already replaced, so it must not touch the current credentials.
      refreshGate.complete(
        http.Response(
          jsonEncode({'code': 'unauthorized', 'message': 'token expired'}),
          401,
        ),
      );
      await settle(tester);

      expect(
        loadPref('authToken:$origin'),
        newToken,
        reason: 'a refusal for the OLD sign-in must not erase the new one',
      );
      expect(
        loadPref('refreshToken:$origin'),
        'rt-new',
        reason: 'the rotated refresh token of the new sign-in must survive',
      );
      expect(
        loadPref('authUser:$origin'),
        contains('u2'),
        reason: 'the new account is still the one persisted',
      );
    },
  );

  testWidgets(
    'a refusal for the CURRENT sign-in still ends it (normal expiry cleanup)',
    (tester) async {
      final origin = originFor('expiry');
      seed(origin, accessToken: _expiredToken());

      useHttpClientForTesting(
        MockClient((request) async {
          if (request.url.path == '/api/auth/refresh') {
            return http.Response(
              jsonEncode({'code': 'unauthorized', 'message': 'gone'}),
              401,
            );
          }
          return http.Response('{}', 200);
        }),
      );

      await tester.pumpWidget(const app.MicaApp());
      await settle(tester, 30);

      expect(
        loadPref('authToken:$origin'),
        '',
        reason:
            'a refused refresh for the live sign-in is a real end of '
            'session and must still clear the token',
      );
      expect(loadPref('refreshToken:$origin'), '');
      expect(loadPref('authUser:$origin'), '');
    },
  );

  testWidgets(
    'a pending workspaces 401 landing after a re-login must keep the new '
    'credentials (issue #147)',
    (tester) async {
      final origin = originFor('workspaces');
      // Non-expired: restore goes straight to the workspace list, no refresh.
      seed(origin, accessToken: _nonExpiredToken());

      final newToken = _nonExpiredToken();
      final workspacesGate = Completer<http.Response>();
      var workspaceCalls = 0;
      var sawRestoreWorkspaces = false;
      useHttpClientForTesting(
        MockClient((request) async {
          final path = request.url.path;
          if (path == '/api/workspaces') {
            workspaceCalls++;
            if (workspaceCalls == 1) {
              sawRestoreWorkspaces = true;
              return workspacesGate.future;
            }
            return workspacesJson();
          }
          if (path == '/api/auth/login') {
            return http.Response(
              jsonEncode({
                'access_token': newToken,
                'refresh_token': 'rt-new',
                'user': {
                  'id': 'u2',
                  'email': 'new@mica.test',
                  'display_name': 'New User',
                },
              }),
              200,
              headers: {'content-type': 'application/json; charset=utf-8'},
            );
          }
          if (path == '/api/auth/me') return userJson('u2', 'new@mica.test');
          if (path == '/api/workspaces/w1/views') {
            return http.Response(jsonEncode({'views': []}), 200);
          }
          if (path == '/api/workspaces/w1/members') {
            return http.Response(jsonEncode({'members': []}), 200);
          }
          return http.Response('{}', 200);
        }),
      );

      await tester.pumpWidget(const app.MicaApp());
      for (var i = 0; i < 40 && !sawRestoreWorkspaces; i++) {
        await tester.pump(const Duration(milliseconds: 20));
      }
      expect(
        sawRestoreWorkspaces,
        isTrue,
        reason: 'the restore must be waiting on the workspace list',
      );

      // Re-login through the gate while the restore's request is pending.
      final fields = find.byType(TextField);
      expect(fields, findsNWidgets(2));
      await tester.enterText(fields.at(0), 'new@mica.test');
      await tester.enterText(fields.at(1), 'secret');
      await tester.pump();
      await tester.tap(find.byIcon(Icons.login));
      await settle(tester);
      expect(
        loadPref('authToken:$origin'),
        newToken,
        reason: 'precondition: the re-login persisted its own credentials',
      );

      // The restore's old request now answers 401. It belongs to the session
      // that was replaced, so the new credentials must survive.
      workspacesGate.complete(
        http.Response(
          jsonEncode({'code': 'unauthorized', 'message': 'revoked'}),
          401,
        ),
      );
      await settle(tester, 30);

      expect(
        loadPref('authToken:$origin'),
        newToken,
        reason: 'a 401 for the OLD restore must not erase the new sign-in',
      );
      expect(loadPref('refreshToken:$origin'), 'rt-new');
      expect(loadPref('authUser:$origin'), contains('u2'));
    },
  );
 }

String _tokenExpiring(DateTime expiry) {
  String seg(Map<String, dynamic> m) =>
      base64Url.encode(utf8.encode(jsonEncode(m))).replaceAll('=', '');
  return '${seg({'alg': 'HS256'})}.'
      '${seg({'sub': 'u1', 'exp': expiry.millisecondsSinceEpoch ~/ 1000})}.'
      'sig';
}

String _expiredToken() =>
    _tokenExpiring(DateTime.now().toUtc().subtract(const Duration(hours: 1)));

String _nonExpiredToken() =>
    _tokenExpiring(DateTime.now().toUtc().add(const Duration(days: 1)));
