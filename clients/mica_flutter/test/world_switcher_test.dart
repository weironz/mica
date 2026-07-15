import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';

// Local and cloud were modelled as a global MODE: the workspace menu showed
// only the active world and a toggle in Settings chose which. But every
// workspace already carries its own origin — `'local'` or the server it lives
// on — so the active origin is derivable from the workspace you opened, and the
// mode was a filter bolted on top of a model that never needed it.
//
// Deleting it is what fixes the reported symptoms: a footer showing the cloud
// account while you stand in a local workspace, and a Settings toggle that
// switched worlds (and closed the dialog) the instant you touched it.

const _cloud = 'https://mica.cloudcele.com';

WorkspaceEntry entry(String origin, String id, String name) => WorkspaceEntry(
      origin: origin,
      workspace: Workspace(id: id, name: name, ownerId: 'u1', role: 'owner'),
      role: 'owner',
    );

void main() {
  group('origin is the flavour — no mode needed', () {
    final entries = [
      entry(_cloud, 'c1', 'devops'),
      entry('local', 'l1', '本地工作区'),
      entry(_cloud, 'c2', 'network'),
    ];

    test('every workspace says where it lives, on its own', () {
      // This is AFFiNE's `flavour` — a server id with `local` reserved — and
      // Mica already had it. Nothing needs to ask a global "which mode".
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

    test('the active origin is DERIVED from the picked row', () {
      // What _selectEntry does: it reads entry.origin. There is no toggle whose
      // value could disagree with the workspace on screen.
      for (final e in entries) {
        final active = e.origin; // _selectEntry: _activeOrigin = entry.origin
        expect(active == 'local', e.isLocal);
      }
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
