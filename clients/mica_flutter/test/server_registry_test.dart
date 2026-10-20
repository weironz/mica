import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/server_registry.dart';

/// Characterization tests for [ServerRegistry], pinned against the
/// server-management behavior it had as `_WorkspaceShellState` methods
/// (god-class wrap-up; docs/reviews/code-optimization/OPTIMIZATION_PLAN_ROUND3.md §7).
void main() {
  late Map<String, String> prefs;
  late List<String> forgotten;
  late List<Uri> rebased;
  late int restores;
  late List<bool> activeRemovals;

  ServerRegistry make({String defaultCloudUrl = ''}) {
    prefs = {};
    forgotten = [];
    rebased = [];
    restores = 0;
    activeRemovals = [];
    return ServerRegistry(
      readPref: (k) => prefs[k],
      savePref: (k, v) => prefs[k] = v,
      forgetOrigin: (origin) async => forgotten.add(origin),
      rebaseApi: (base) => rebased.add(base),
      restoreSession: () async => restores++,
      defaultCloudUrl: defaultCloudUrl,
    );
  }

  group('adding', () {
    test('a valid URL joins the list and persists — and ONLY that', () {
      final r = make();
      expect(r.addServer('https://a.example.com'), ServerAddResult.added);
      expect(r.servers, ['https://a.example.com']);
      expect(prefs['servers'], '["https://a.example.com"]');
      expect(r.cloudOrigin, '', reason: 'adding is not switching');
    });

    test('an unparseable or schemeless URL is refused, nothing changes', () {
      final r = make();
      expect(r.addServer('not a url'), ServerAddResult.invalidUrl);
      expect(r.addServer('localhost/x'), ServerAddResult.invalidUrl);
      expect(r.servers, isEmpty);
      expect(prefs.containsKey('servers'), isFalse);
    });

    test('a duplicate URL is refused (normalization included)', () {
      final r = make();
      r.addServer('https://a.example.com');
      expect(r.addServer('https://a.example.com'), ServerAddResult.alreadyAdded);
      expect(r.addServer('  https://a.example.com  '), ServerAddResult.alreadyAdded);
      expect(r.servers, hasLength(1));
    });
  });

  group('removing', () {
    test(
      'an aborted in-flight drain preserves the server, credentials and mirror',
      () async {
        final r = make();
        const origin = 'https://a.example.com';
        r.addServer(origin);
        r.bind(origin);
        prefs['authToken:$origin'] = 'access';
        prefs['authUser:$origin'] = 'user';
        prefs['refreshToken:$origin'] = 'refresh';
        final before = Map<String, String>.of(prefs);
        final drain = Completer<void>();
        var mounted = true;
        final removal = r.removeServer(
          origin,
          onBeforeRemove: () async {
            await drain.future;
            return mounted;
          },
        );

        mounted = false;
        drain.complete();
        await removal;

        expect(r.servers, [origin]);
        expect(r.cloudOrigin, origin);
        expect(prefs, before);
        expect(forgotten, isEmpty);
      },
    );

    test(
      'an inactive server can be removed without draining the active one',
      () async {
        final r = make();
        r.addServer('https://active.example.com');
        r.addServer('https://inactive.example.com');
        r.bind('https://active.example.com');
        await r.removeServer(
          'https://inactive.example.com',
          onBeforeRemove: () async => true,
        );
        expect(r.servers, ['https://active.example.com']);
        expect(r.cloudOrigin, 'https://active.example.com');
        expect(forgotten, ['https://inactive.example.com']);
      },
    );

    test('forgets credentials AND mirror, keyed surgically by origin', () async {
      final r = make();
      r.addServer('https://a.example.com');
      r.addServer('https://b.example.com');
      r.bind('https://a.example.com');
      await r.removeServer(
        'https://a.example.com',
        onBeforeRemove: () async {
          activeRemovals.add(true);
          return true;
        },
      );
      expect(r.servers, ['https://b.example.com']);
      expect(forgotten, ['https://a.example.com']);
      expect(prefs['authToken:https://a.example.com'], '');
      expect(prefs['authToken:https://b.example.com'], isNull,
          reason: 'no other server\'s data can be caught by the removal');
      expect(activeRemovals, [true],
          reason: 'the drain-and-switch hook runs first, while a session may still exist');
    });

    test('an unknown origin is a no-op', () async {
      final r = make();
      r.addServer('https://a.example.com');
      await r.removeServer('https://zz.example.com', onBeforeRemove: () async => true);
      expect(r.servers, hasLength(1));
      expect(forgotten, isEmpty);
    });

    test('removing the LAST server falls back to the build default', () async {
      final r = make(defaultCloudUrl: 'https://default.example.com');
      r.addServer('https://a.example.com');
      r.bind('https://a.example.com');
      await r.removeServer('https://a.example.com', onBeforeRemove: () async => true);
      expect(r.cloudOrigin, 'https://default.example.com');
      expect(r.servers, ['https://default.example.com'],
          reason: 'the default is not in the list, so it becomes the list');
      expect(rebased.last.toString(), 'https://default.example.com');
    });

    test('an empty build default leaves no server, none is invented', () async {
      final r = make();
      r.addServer('https://a.example.com');
      r.bind('https://a.example.com');
      await r.removeServer('https://a.example.com', onBeforeRemove: () async => true);
      expect(r.cloudOrigin, '');
      expect(r.servers, isEmpty);
      expect(rebased, hasLength(1),
          reason: 'only the initial bind rebased; the empty fallback never does');
    });
  });

  group('connecting / binding', () {
    test('a new origin persists, rebases, and restores stored credentials', () async {
      final r = make();
      r.bind('https://old.example.com');
      final changed = await r.connect('https://new.example.com');
      expect(changed, isTrue);
      expect(r.cloudOrigin, 'https://new.example.com');
      expect(prefs['cloudOrigin'], 'https://new.example.com');
      expect(rebased.last.toString(), 'https://new.example.com');
      expect(restores, 1, reason: 'the new origin may already have stored credentials');
    });

    test('reconnecting to the bound origin still restores the session', () async {
      final r = make();
      r.bind('https://a.example.com');
      final changed = await r.connect('https://a.example.com');
      expect(changed, isFalse);
      expect(restores, 1,
          reason: 'cloud → local → cloud must not cost a password: local mode '
              'drops the session but the origin binding stays');
      expect(rebased, hasLength(1), reason: 'a same-origin connect never rebinds again');
    });

    test('an unparseable connect target changes nothing', () async {
      final r = make();
      r.bind('https://a.example.com');
      expect(await r.connect('::bad::'), isFalse);
      expect(r.cloudOrigin, 'https://a.example.com');
      expect(restores, 0);
    });

    test('load reads the persisted list through knownServers', () {
      final r = make();
      prefs['servers'] = '["https://a.example.com"]';
      r.load(seed: 'https://seed.example.com');
      expect(r.servers.firstOrNull, 'https://seed.example.com',
          reason: 'knownServers puts the seed FIRST so it is the default pick');
      expect(r.servers, contains('https://a.example.com'));
    });
  });
}
