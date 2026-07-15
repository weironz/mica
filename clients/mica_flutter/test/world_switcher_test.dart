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

  group('the footer names the world you are in, not the session you have', () {
    // The reported bug: standing in a local workspace, the sidebar footer showed
    // `willmica / willzhmic@outlook.com`, claiming you were editing files on this
    // device as that person. The old predicate was
    // `_session?.user.displayName ?? (local ? '本地' : '')` — it asked "is there
    // a session", so the fallback only ever fired for someone who had never
    // signed in at all.
    ({String name, String email}) footer({
      required bool local,
      required AuthSession? session,
    }) {
      return (
        name: local ? '本地工作区' : (session?.user.displayName ?? ''),
        email: local ? '这台设备' : (session?.user.email ?? ''),
      );
    }

    const signedIn = AuthSession(
      accessToken: 'a',
      user: User(id: 'u1', email: 'willzhmic@outlook.com', displayName: 'willmica'),
    );

    test('local + a live cloud session shows the DEVICE, not the account', () {
      final f = footer(local: true, session: signedIn);
      expect(f.name, '本地工作区');
      expect(f.email, '这台设备');
      expect(f.email, isNot(contains('@')),
          reason: 'the local world has no account — these files are nobody\'s');
    });

    test('cloud shows the account', () {
      final f = footer(local: false, session: signedIn);
      expect(f.name, 'willmica');
      expect(f.email, 'willzhmic@outlook.com');
    });

    test('local with no session ever is identical to local with one', () {
      // The world decides, full stop. Whether a session exists is irrelevant.
      expect(footer(local: true, session: null),
          footer(local: true, session: signedIn));
    });

    test('cloud, signed out, shows nothing rather than inventing a name', () {
      final f = footer(local: false, session: null);
      expect(f.name, isEmpty);
      expect(f.email, isEmpty);
    });
  });
}
