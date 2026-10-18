import 'dart:async';
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/auth/auth_controller.dart';
import 'package:mica_flutter/session/session_controller.dart';

/// Characterization tests for [AuthController], pinned against the behavior
/// the session-persistence block had as `_WorkspaceShellState` methods
/// (docs/code-optimization/OPTIMIZATION_PLAN_ROUND3.md §7, sixth step). Nearly
/// every guarded branch below carries a "why" comment in the shipped code —
/// a historical lesson — and each one is pinned here.
void main() {
  /// A JWT-shaped token whose `exp` is [expiry]; the refresher reads it to
  /// decide whether a renewal is due.
  String tokenExpiring(DateTime expiry) {
    String seg(Map<String, dynamic> m) =>
        base64Url.encode(utf8.encode(jsonEncode(m))).replaceAll('=', '');
    return '${seg({'alg': 'HS256'})}.'
        '${seg({'sub': 'u1', 'exp': expiry.millisecondsSinceEpoch ~/ 1000})}.'
        'sig';
  }

  DateTime inMinutes(int n) => DateTime.now().toUtc().add(Duration(minutes: n));

  AuthSession session({
    String user = 'u1',
    String name = 'U',
    String? avatar,
    String refresh = 'r1',
    String? access,
    DateTime? expiry,
  }) => AuthSession(
    accessToken: access ?? tokenExpiring(expiry ?? inMinutes(20 * 60)),
    refreshToken: refresh,
    user: User(
      id: user,
      email: '$user@t.dev',
      displayName: name,
      avatarVersion: avatar,
    ),
  );

  Workspace ws(String id) =>
      Workspace(id: id, name: 'W-$id', ownerId: 'o', role: 'owner');

  late Map<String, String> prefs;
  late String originValue;
  late List<String> serverList;
  late bool mountedFlag;
  late SessionController hub;

  setUp(() {
    prefs = {};
    originValue = 'https://cloud.example';
    serverList = const ['https://cloud.example', 'https://other.example'];
    mountedFlag = true;
    hub = SessionController();
  });

  AuthController build({
    Future<bool> Function(AuthFormValue form)? register,
    Future<AuthSession> Function(AuthFormValue form)? login,
    Future<List<Workspace>> Function(String token)? listWorkspaces,
    Future<User> Function(String token)? fetchMe,
    Future<AuthSession> Function(String refreshToken)? refreshSession,
    Future<void> Function(String token)? removeAvatar,
    Future<http.Response> Function(Uri url)? httpGet,
    void Function(String url)? evictImage,
    Duration? probeTimeout,
  }) => AuthController(
    session: hub,
    origin: () => originValue,
    servers: () => serverList,
    readPref: (key) => prefs[key],
    savePref: (key, value) => prefs[key] = value,
    isMounted: () => mountedFlag,
    register: register ?? ((form) async => false),
    login: login ?? ((form) async => throw const ApiException('no login')),
    listWorkspaces: listWorkspaces ?? ((token) async => [ws('a'), ws('b')]),
    fetchMe: fetchMe ?? ((token) async => throw StateError('no fetchMe')),
    refreshSession:
        refreshSession ?? ((token) async => throw StateError('no refresh')),
    removeAvatar: removeAvatar ?? ((token) async {}),
    httpGet: httpGet ?? ((uri) async => http.Response('{}', 200)),
    avatarUrlFor:
        (version, {userId}) =>
            version == null ? null : 'avatar:$userId/$version',
    evictImage: evictImage ?? ((url) {}),
    probeTimeout: probeTimeout ?? const Duration(seconds: 4),
  );

  const form = AuthFormValue(
    email: 'u@t.dev',
    displayName: 'U',
    password: 'pw',
  );

  group('credential persistence — per-origin keys', () {
    test('persistSession files token + user + refresh under the CURRENT origin, and re-filing a rotation is what keeps the sign-in alive', () {
      final auth = build();
      auth.persistSession(session(name: 'A', access: 'a1', refresh: 'r1'));
      auth.persistSession(session(name: 'A', access: 'a2', refresh: 'r2'));

      expect(prefs['authToken:$originValue'], 'a2');
      // The refresh token ROTATES on every use: persisting the replacement is
      // the whole point — the next launch would otherwise spend a burnt token.
      expect(prefs['refreshToken:$originValue'], 'r2');
      expect(
        jsonDecode(prefs['authUser:$originValue']!)['display_name'],
        'A',
      );
    });

    test('clearPersistedSession blanks the per-origin keys AND the legacy single-key copies, without touching other origins', () {
      final auth = build();
      auth.persistSession(session());
      prefs['authToken'] = 'legacy';
      prefs['authUser'] = 'legacy';
      prefs['authToken:https://other.example'] = 'theirs';

      auth.clearPersistedSession();

      expect(prefs['authToken:$originValue'], '');
      expect(prefs['authUser:$originValue'], '');
      expect(prefs['refreshToken:$originValue'], '');
      // The legacy copies must go too, or a future migration re-run
      // resurrects an explicit sign-out.
      expect(prefs['authToken'], '');
      expect(prefs['authUser'], '');
      // Surgical: another server's credentials survive.
      expect(prefs['authToken:https://other.example'], 'theirs');
    });

    test('signedInOrigins keys off the stored USER, not the token', () {
      final auth = build();
      // Token absent (expired — most mornings), user stored: still signed in.
      prefs['authUser:$originValue'] = '{"id":"u1"}';
      // Token present, user blank: NOT a sign-in.
      prefs['authToken:https://other.example'] = 'tok';

      expect(auth.signedInOrigins(), {originValue});
    });

    test('storedDisplayName reads prefs, not the session; corrupt JSON answers null', () {
      final auth = build();
      prefs['authUser:$originValue'] = jsonEncode(
        session(name: 'Stored').user.toJson(),
      );

      expect(auth.storedDisplayName(originValue), 'Stored');
      expect(auth.storedDisplayName('https://other.example'), isNull);

      prefs['authUser:$originValue'] = 'not json{';
      expect(auth.storedDisplayName(originValue), isNull);

      prefs['authUser:$originValue'] = '';
      expect(auth.storedDisplayName(originValue), isNull);
    });
  });

  group('profile refresh — the copyWith discipline', () {
    test('an unchanged profile adopts nothing', () async {
      final s = session(name: 'Same', avatar: 'v1');
      hub.signIn(s, [ws('a')]);
      final auth = build(fetchMe: (token) async => s.user);

      await auth.refreshProfile();

      expect(hub.session!.user, same(s.user));
    });

    test('a changed profile is adopted with copyWith — the refresh token SURVIVES', () async {
      final s = session(name: 'Old', avatar: 'v1', refresh: 'keep-me');
      hub.signIn(s, [ws('a')]);
      final auth = build(
        fetchMe:
            (token) async => User(
              id: 'u1',
              email: 'u1@t.dev',
              displayName: 'New',
              avatarVersion: 'v1',
            ),
      );

      await auth.refreshProfile();

      expect(hub.session!.user.displayName, 'New');
      // The overnight-logout lesson: rebuilding the session instead of
      // copyWith dropped the refresh token, and the profile edit cost the
      // ability to renew.
      expect(hub.session!.refreshToken, 'keep-me');
      expect(
        jsonDecode(prefs['authUser:$originValue']!)['display_name'],
        'New',
      );
    });

    test('a poll that came back for a DIFFERENT account is discarded', () async {
      final s = session(user: 'u1', name: 'A');
      hub.signIn(s, [ws('a')]);
      final fetched = Completer<User>();
      final auth = build(fetchMe: (token) => fetched.future);

      final done = auth.refreshProfile();
      // While the poll was in flight, the user switched accounts.
      hub.adoptSession(session(user: 'u2', name: 'B', refresh: 'other'));
      fetched.complete(
        User(id: 'u1', email: 'u1@t.dev', displayName: 'A-changed'),
      );
      await done;

      expect(hub.session!.user.id, 'u2');
      expect(hub.session!.user.displayName, 'B');
      expect(prefs['authUser:$originValue'], isNull);
    });

    test('a failed poll is swallowed', () async {
      hub.signIn(session(), [ws('a')]);
      final auth = build(
        fetchMe: (token) async => throw const ApiException('offline'),
      );

      await auth.refreshProfile();

      expect(hub.session, isNotNull);
    });

    test('no session → no fetch at all', () async {
      var fetches = 0;
      final auth = build(
        fetchMe: (token) {
          fetches++;
          throw StateError('must not be called');
        },
      );

      await auth.refreshProfile();

      expect(fetches, 0);
    });
  });

  group('session renewal — the identity guard', () {
    test('a matching renewal is adopted and the ROTATED refresh token persisted immediately', () async {
      hub.signIn(session(refresh: 'r1', expiry: inMinutes(1)), [ws('a')]);
      final auth = build(
        refreshSession:
            (t) async => session(refresh: 'r2', expiry: inMinutes(600)),
      );

      await auth.ensureFreshSession();

      expect(hub.session!.refreshToken, 'r2');
      // The disk copy is now burnt; losing this write costs the sign-in at
      // next launch.
      expect(prefs['refreshToken:$originValue'], 'r2');
    });

    test('a renewal that no longer matches is discarded — no resurrection', () async {
      hub.signIn(session(user: 'u1', refresh: 'r1', expiry: inMinutes(1)), [
        ws('a'),
      ]);
      final renewed = Completer<AuthSession>();
      final auth = build(refreshSession: (t) => renewed.future);

      final done = auth.ensureFreshSession();
      // While the refresh was in flight, the user switched accounts.
      hub.adoptSession(session(user: 'u2', refresh: 'zz'));
      renewed.complete(session(user: 'u1', refresh: 'r2'));
      await done;

      expect(hub.session!.user.id, 'u2');
      expect(prefs['refreshToken:$originValue'], isNot('r2'));
    });

    test('an unauthorized refresh for the SAME sign-in ends it — only the session drops', () async {
      hub.signIn(
        session(user: 'u1', refresh: 'r1', expiry: inMinutes(1)),
        [ws('a'), ws('b')],
      );
      final auth = build(
        refreshSession:
            (t) async => throw const ApiException('expired', statusCode: 401),
      );
      auth.persistSession(hub.session!);
      var expired = 0;
      auth.onSessionExpired = () => expired++;

      await auth.ensureFreshSession();

      expect(expired, 1);
      expect(hub.session, isNull);
      // The workspace list survives so the local world can fall back
      // coherently.
      expect(hub.workspaces, hasLength(2));
      expect(prefs['authToken:$originValue'], '');
    });

    test('an unauthorized refresh for an ALREADY-LEFT sign-in does not end the current one', () async {
      hub.signIn(session(user: 'u1', refresh: 'r1', expiry: inMinutes(1)), [
        ws('a'),
      ]);
      final refused = Completer<AuthSession>();
      final auth = build(refreshSession: (t) => refused.future);
      var expired = 0;
      auth.onSessionExpired = () => expired++;

      final done = auth.ensureFreshSession();
      hub.adoptSession(session(user: 'u2', refresh: 'q'));
      refused.completeError(const ApiException('expired', statusCode: 401));
      await done;

      expect(expired, 0);
      expect(hub.session, isNotNull);
    });

    test('a network failure keeps the session', () async {
      hub.signIn(session(refresh: 'r1', expiry: inMinutes(1)), [ws('a')]);
      final auth = build(
        refreshSession: (t) async => throw Exception('offline'),
      );
      var expired = 0;
      auth.onSessionExpired = () => expired++;

      await auth.ensureFreshSession();

      expect(hub.session, isNotNull);
      expect(expired, 0);
    });
  });

  group('authenticate — the sign-in form flow', () {
    for (final switcher in [false, true]) {
      for (final disposeDuringLogin in [false, true]) {
        test(
          '${switcher ? 'signIn' : 'authenticate'} ignores a late ${disposeDuringLogin ? 'login' : 'workspace list'} after unmount',
          () async {
            final login = Completer<AuthSession>();
            final workspaces = Completer<List<Workspace>>();
            final listStarted = Completer<void>();
            final auth = build(
              login: (_) => login.future,
              listWorkspaces: (_) {
                listStarted.complete();
                return workspaces.future;
              },
            );
            final done = switcher
                ? auth.signIn(form)
                : auth.authenticate(AuthMode.login, form);
            if (disposeDuringLogin) {
              mountedFlag = false;
              login.complete(session());
            } else {
              login.complete(session());
              await listStarted.future;
              mountedFlag = false;
              workspaces.complete([ws('late')]);
            }
            await done;

            expect(hub.session, isNull);
            expect(hub.workspaces, isEmpty);
            expect(prefs, isEmpty);
            expect(listStarted.isCompleted, !disposeDuringLogin);
          },
        );
      }
    }

    test('register stops at the round trip and reports the server\'s verified answer', () async {
      final auth = build(register: (form) async => true);

      final outcome = await auth.authenticate(AuthMode.register, form);

      expect(outcome, isA<AuthRegistered>().having((o) => o.verified, 'verified', isTrue));
      expect(hub.session, isNull);
      expect(prefs, isEmpty);
    });

    test('a successful login signs in, selects the first workspace, and persists', () async {
      final s = session(name: 'In', refresh: 'rr');
      final auth = build(
        login: (form) async => s,
        listWorkspaces: (token) async => [ws('a'), ws('b')],
      );

      final outcome = await auth.authenticate(AuthMode.login, form);

      expect(outcome, isA<AuthSignedIn>());
      expect(hub.session, same(s));
      expect(hub.selectedWorkspace?.id, 'a');
      expect(prefs['refreshToken:$originValue'], 'rr');
    });

    test('401 → badCredentials', () async {
      final auth = build(
        login:
            (form) async => throw const ApiException('no', statusCode: 401),
      );

      final outcome = await auth.authenticate(AuthMode.login, form);

      expect(
        outcome,
        isA<AuthRefused>().having((o) => o.refusal, 'refusal', AuthRefusal.badCredentials),
      );
    });

    test('409 → emailTaken', () async {
      final auth = build(
        login: (form) async => throw const ApiException('no', statusCode: 409),
      );

      final outcome = await auth.authenticate(AuthMode.login, form);

      expect(
        outcome,
        isA<AuthRefused>().having((o) => o.refusal, 'refusal', AuthRefusal.emailTaken),
      );
    });

    test('email_not_verified code → emailNotVerified', () async {
      final auth = build(
        login:
            (form) async => throw const ApiException(
              'confirm',
              code: 'email_not_verified',
            ),
      );

      final outcome = await auth.authenticate(AuthMode.login, form);

      expect(
        outcome,
        isA<AuthRefused>().having((o) => o.refusal, 'refusal', AuthRefusal.emailNotVerified),
      );
    });

    test('any other failure propagates raw', () async {
      final auth = build(
        login:
            (form) async => throw const ApiException('boom', statusCode: 500),
      );

      await expectLater(
        auth.authenticate(AuthMode.login, form),
        throwsA(
          isA<ApiException>().having((e) => e.statusCode, 'status', 500),
        ),
      );
    });

    test('a workspace-list failure after login signs nothing in and persists nothing', () async {
      final auth = build(
        login: (form) async => session(),
        listWorkspaces:
            (token) async => throw const ApiException('list failed'),
      );

      await expectLater(
        auth.authenticate(AuthMode.login, form),
        throwsA(isA<ApiException>()),
      );
      expect(hub.session, isNull);
      expect(prefs, isEmpty);
    });

    test('signIn (the switcher path) leaves failures untranslated', () async {
      final auth = build(
        login:
            (form) async => throw const ApiException('no', statusCode: 401),
      );

      // Not an AuthRefused — the raw error surfaces through the busy
      // wrapper as the server worded it.
      await expectLater(
        auth.signIn(form),
        throwsA(
          isA<ApiException>().having((e) => e.statusCode, 'status', 401),
        ),
      );
    });
  });

  group('probeServer — readiness, not liveness', () {
    test('an unparseable origin answers false without a request', () async {
      var requests = 0;
      final auth = build(
        httpGet: (uri) {
          requests++;
          throw StateError('must not be called');
        },
      );

      expect(await auth.probeServer('http://'), isFalse);
      expect(await auth.probeServer('::::'), isFalse);
      expect(requests, 0);
    });

    test('a 200 with registration_open records the answer for THAT origin', () async {
      final seen = <Uri>[];
      final auth = build(
        httpGet: (uri) {
          seen.add(uri);
          return Future.value(
            http.Response(jsonEncode({'registration_open': true}), 200),
          );
        },
      );

      expect(await auth.probeServer(originValue), isTrue);
      expect(auth.registrationOpen, isTrue);
      expect(auth.regOrigin, originValue);
      // `/api/ready`, not `/api/health`: the question is "can you serve me".
      expect(seen.single.toString(), '$originValue/api/ready');

      // Probing another origin moves the pair together — switching servers
      // must not show the previous server's answer.
      final second = build(
        httpGet:
            (uri) async => http.Response(
              jsonEncode({'registration_open': false}),
              200,
            ),
      );
      expect(await second.probeServer('https://other.example'), isTrue);
      expect(second.registrationOpen, isFalse);
      expect(second.regOrigin, 'https://other.example');
    });

    test('a 200 with an unreadable body answers reachable but registration UNKNOWN', () async {
      final auth = build(
        httpGet: (uri) async => http.Response('not json{', 200),
      );

      expect(await auth.probeServer(originValue), isTrue);
      expect(auth.registrationOpen, isNull);

      final nonBool = build(
        httpGet:
            (uri) async => http.Response(
              jsonEncode({'registration_open': 'yes'}),
              200,
            ),
      );
      expect(await nonBool.probeServer(originValue), isTrue);
      expect(nonBool.registrationOpen, isNull);
    });

    test('a non-200 answers false and leaves the previous answer untouched', () async {
      final auth = build(
        httpGet: (uri) async => http.Response('down', 503),
      );
      auth.regOrigin = 'https://other.example';
      auth.registrationOpen = true;

      expect(await auth.probeServer(originValue), isFalse);
      expect(auth.regOrigin, 'https://other.example');
      expect(auth.registrationOpen, isTrue);
    });

    test('a server that never answers is down after the timeout', () async {
      final auth = build(
        httpGet: (uri) => Completer<http.Response>().future,
        probeTimeout: const Duration(milliseconds: 30),
      );

      expect(await auth.probeServer(originValue), isFalse);
    });

    test('an unmounted probe answers reachable but records NOTHING', () async {
      mountedFlag = false;
      final auth = build(
        httpGet:
            (uri) async => http.Response(
              jsonEncode({'registration_open': true}),
              200,
            ),
      );

      expect(await auth.probeServer(originValue), isTrue);
      expect(auth.registrationOpen, isNull);
      expect(auth.regOrigin, isNull);
    });
  });

  group('avatar version adoption', () {
    test(
      'an avatar removal preserves a token rotation and profile change made in flight',
      () async {
        final original = session(name: 'Before', avatar: 'v1', refresh: 'old');
        hub.signIn(original, [ws('a')]);
        final removed = Completer<void>();
        final auth = build(removeAvatar: (_) => removed.future);
        final done = auth.removeAvatar(original);
        hub.adoptSession(
          session(
            name: 'After',
            avatar: 'v1',
            refresh: 'rotated',
            access: 'new-access',
          ),
        );
        removed.complete();
        await done;

        expect(hub.session!.refreshToken, 'rotated');
        expect(hub.session!.accessToken, 'new-access');
        expect(hub.session!.user.displayName, 'After');
        expect(hub.session!.user.avatarVersion, isNull);
        expect(prefs['refreshToken:$originValue'], 'rotated');
      },
    );

    for (final transition in ['sign out', 'switch user', 'switch origin']) {
      test(
        'an avatar response after $transition does not adopt or persist the old account',
        () async {
          final original = session(user: 'u1', avatar: 'v1');
          hub.signIn(original, [ws('a')]);
          final removed = Completer<void>();
          final auth = build(removeAvatar: (_) => removed.future);
          final done = auth.removeAvatar(original);
          switch (transition) {
            case 'sign out':
              hub.signOut();
            case 'switch user':
              hub.adoptSession(session(user: 'u2', avatar: 'other'));
            case 'switch origin':
              originValue = 'https://other.example';
              hub.adoptSession(session(user: 'u1', avatar: 'other'));
          }
          final current = hub.session;
          removed.complete();
          await done;

          expect(hub.session, same(current));
          expect(prefs, isEmpty);
        },
      );
    }

    test(
      'a picked avatar cannot start a request after leaving its account',
      () {
        final original = session();
        hub.signIn(original, [ws('a')]);
        final auth = build();
        final requestOrigin = originValue;
        expect(auth.canApplyAvatar(original, requestOrigin), isTrue);
        hub.signOut();
        expect(auth.canApplyAvatar(original, requestOrigin), isFalse);
        hub.signIn(original, [ws('a')]);
        originValue = 'https://other.example';
        expect(auth.canApplyAvatar(original, requestOrigin), isFalse);
      },
    );

    test('applyAvatarVersion keeps the refresh token, adopts, persists, and evicts the new URL', () {
      final s = session(user: 'u1', avatar: 'v1', refresh: 'survives');
      hub.signIn(s, [ws('a')]);
      final evicted = <String>[];
      final auth = build(evictImage: evicted.add);

      final url = auth.applyAvatarVersion(s, 'v2');

      expect(url, 'avatar:u1/v2');
      // The copyWith lesson: rebuilding the session would drop the refresh
      // token, and changing your picture would cost the ability to renew.
      expect(hub.session!.refreshToken, 'survives');
      expect(hub.session!.user.avatarVersion, 'v2');
      expect(
        jsonDecode(prefs['authUser:$originValue']!)['avatar_version'],
        'v2',
      );
      expect(evicted, ['avatar:u1/v2']);
    });

    test('applyAvatarVersion while unmounted still answers with the right URL', () {
      mountedFlag = false;
      final s = session(user: 'u1', avatar: 'v1');
      final evicted = <String>[];
      final auth = build(evictImage: evicted.add);

      final url = auth.applyAvatarVersion(s, 'v2');

      // No adoption, no persistence — but the answer must be right even
      // when the widget is already gone.
      expect(url, 'avatar:u1/v2');
      expect(hub.session, isNull);
      expect(prefs, isEmpty);
      expect(evicted, ['avatar:u1/v2']);
    });

    test('removing the avatar answers null and evicts nothing', () {
      final s = session(user: 'u1', avatar: 'v1');
      hub.signIn(s, [ws('a')]);
      final evicted = <String>[];
      final auth = build(evictImage: evicted.add);

      expect(auth.applyAvatarVersion(s, null), isNull);
      expect(hub.session!.user.avatarVersion, isNull);
      expect(evicted, isEmpty);
    });

    test('removeAvatar calls the api with the session token and applies the removal', () async {
      final s = session(user: 'u1', avatar: 'v1');
      hub.signIn(s, [ws('a')]);
      String? tokenSeen;
      final auth = build(removeAvatar: (token) async => tokenSeen = token);

      final url = await auth.removeAvatar(s);

      expect(tokenSeen, s.accessToken);
      expect(url, isNull);
      expect(hub.session!.user.avatarVersion, isNull);
    });
  });
}
