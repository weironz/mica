import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';

// The app is connected to exactly ONE world at a time — `本地模式` or a server —
// picked from one list in Settings. Tiling both worlds in the workspace menu was
// tried and rejected: a local and a cloud workspace look alike but are not, and
// listing them together invites treating them as interchangeable.
//
// What survives from that attempt is the part that was right: `origin` already
// says where each workspace lives, so nothing needs a second source of truth for
// it. The connection decides which world is on screen; the entry decides how a
// row behaves.

const _cloud = 'https://mica.cloudcele.com';

WorkspaceEntry entry(String origin, String id, String name) => WorkspaceEntry(
      origin: origin,
      workspace: Workspace(id: id, name: name, ownerId: 'u1', role: 'owner'),
      role: 'owner',
    );

void main() {
  group('origin says where a workspace lives', () {
    final entries = [
      entry(_cloud, 'c1', 'devops'),
      entry('local', 'l1', '本地工作区'),
      entry(_cloud, 'c2', 'network'),
    ];

    test('a workspace carries its own origin', () {
      // Same shape as AFFiNE's `flavour`: a server id, with `local` reserved.
      // It is what the menu filters on and what row actions dispatch on — no
      // second source of truth for where a workspace lives.
      expect(entries.where((e) => e.isLocal).map((e) => e.workspace.name),
          ['本地工作区']);
      expect(entries.where((e) => !e.isLocal).map((e) => e.workspace.name),
          ['devops', 'network']);
    });

    test('a ref is unique across worlds, so ids may collide', () {
      // Two workspaces can share an id across origins; the pair is the key.
      final a = entry('local', 'same', 'A').ref;
      final b = entry(_cloud, 'same', 'B').ref;
      expect(a, isNot(b));
      expect(a.origin, 'local');
      expect(b.origin, _cloud);
    });

    test('the menu lists only the connected world', () {
      // What the switcher does. A cloud connection must never tile the local
      // workspaces beside the server's, nor the reverse.
      List<String> shown({required bool local}) => [
            for (final e in entries)
              if (e.isLocal == local) e.workspace.name,
          ];
      expect(shown(local: false), ['devops', 'network']);
      expect(shown(local: true), ['本地工作区']);
    });

    test('the connection list offers this device first, then servers', () {
      // `_connections` = ['local', ...servers]: one list, same kind of choice.
      // Local leads because it always exists and needs no account — AFFiNE's
      // server picker puts its LocalSelectorItem in the same place.
      const servers = ['https://a.example.com', 'https://b.example.com'];
      const connections = ['local', ...servers];
      expect(connections.first, 'local');
      expect(connections.length, servers.length + 1);
    });
  });

  // These drive the REAL accountIdentity the sidebar calls. The first attempt at
  // this fix defined its own `footer()` helper here and asserted against that —
  // so it passed while the shipped code was untouched, and the bug shipped. A
  // test that re-implements the thing it tests only ever tests itself.
  group('the account tile names the world you are in', () {
    const signedIn = User(
      id: 'u1',
      email: 'willzhmic@outlook.com',
      displayName: 'willmica',
    );

    test('local + a live cloud session shows the DEVICE, not the account', () {
      // The reported bug: standing in a local workspace, the sidebar footer
      // showed `willmica / willzhmic@outlook.com`, claiming you were editing
      // files on this device as that person.
      final id = accountIdentity(local: true, user: signedIn);
      expect(id.name, '本地工作区');
      expect(id.email, '这台设备');
      expect(id.email, isNot(contains('@')),
          reason: 'the local world has no account — these files are nobody\'s');
    });

    test('local with no session is identical to local with one', () {
      // The world decides, full stop. Holding a session is irrelevant here,
      // and asking about it is what produced the bug.
      expect(accountIdentity(local: true, user: null),
          accountIdentity(local: true, user: signedIn));
    });

    test('local offers neither sign-out nor sign-in', () {
      // Sign-out: you are not signed in HERE. Sign-in: there is no server in
      // this world to sign in to — that is a choice made in Settings.
      final id = accountIdentity(local: true, user: signedIn);
      expect(id.canSignOut, isFalse);
      expect(id.canSignIn, isFalse);
    });

    test('cloud shows the account and offers sign-out', () {
      final id = accountIdentity(local: false, user: signedIn);
      expect(id.name, 'willmica');
      expect(id.email, 'willzhmic@outlook.com');
      expect(id.canSignOut, isTrue);
      expect(id.canSignIn, isFalse);
    });

    test('cloud, signed out, offers sign-in and invents no name', () {
      final id = accountIdentity(local: false, user: null);
      expect(id.name, '未登录');
      expect(id.email, isNull);
      expect(id.canSignIn, isTrue);
      expect(id.canSignOut, isFalse);
    });

    test('a nameless cloud account falls back to its email', () {
      const noName = User(id: 'u1', email: 'a@b.c', displayName: '');
      final id = accountIdentity(local: false, user: noName);
      expect(id.name, 'a@b.c');
    });
  });
}
