import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/session/session_controller.dart';

/// Characterization tests for [SessionController], pinned against the
/// behavior the four hub fields had as `_WorkspaceShellState` fields
/// (docs/reviews/code-optimization/OPTIMIZATION_PLAN_ROUND3.md §7, third step).
/// The two selection-fallback rules differ ON PURPOSE — see
/// [SessionController.replaceWorkspaces] and .remove.
void main() {
  AuthSession session(String token) => AuthSession(
        accessToken: token,
        user: User(id: 'u', email: 'u@t.dev', displayName: 'U'),
      );

  Workspace ws(String id, {String name = 'W'}) =>
      Workspace(id: id, name: name, ownerId: 'o', role: 'owner');

  DocumentView view(String id) => DocumentView(
        id: id,
        parentViewId: null,
        objectId: 'obj-$id',
        objectType: 'document',
        name: id,
        position: '0',
      );

  group('sign-in / restore / session lifecycle', () {
    test('signIn selects the first workspace', () {
      final c = SessionController();
      c.signIn(session('t'), [ws('a'), ws('b')]);
      expect(c.session?.accessToken, 't');
      expect(c.selectedWorkspace?.id, 'a');
    });

    test('restore lands exactly what the caller computed', () {
      final c = SessionController();
      c.restore(session('t'), [ws('a'), ws('b')], ws('b'), {'b': [view('v')]});
      expect(c.selectedWorkspace?.id, 'b');
      expect(c.viewsByWorkspace.keys, ['b']);
    });

    test('adoptSession moves only the session, nothing else', () {
      final c = SessionController();
      c.signIn(session('t1'), [ws('a')]);
      c.select(ws('a'));
      final before = c.workspaces;
      c.adoptSession(session('t2'));
      expect(c.session?.accessToken, 't2');
      expect(c.workspaces, same(before));
      expect(c.selectedWorkspace?.id, 'a');
    });

    test('expireSession drops ONLY the session — the list survives', () {
      final c = SessionController();
      c.signIn(session('t'), [ws('a'), ws('b')]);
      c.expireSession();
      expect(c.session, isNull);
      expect(c.workspaces, hasLength(2),
          reason: 'the local world still needs the list to fall back to');
      expect(c.selectedWorkspace?.id, 'a');
    });

    test('signOut clears everything cloud-side', () {
      final c = SessionController();
      c.restore(session('t'), [ws('a')], ws('a'), {'a': [view('v')]});
      c.signOut();
      expect(c.session, isNull);
      expect(c.workspaces, isEmpty);
      expect(c.selectedWorkspace, isNull);
      expect(c.viewsByWorkspace, isEmpty);
    });
  });

  group('workspace list replacement — the survival rule', () {
    test('a selected workspace survives a refetch when its id persists', () {
      final c = SessionController();
      c.signIn(session('t'), [ws('a'), ws('b')]);
      c.replaceWorkspaces([ws('a', name: 'A2'), ws('b')]);
      expect(c.selectedWorkspace?.id, 'a');
      expect(c.workspaces.first.name, 'A2');
    });

    test('a selection that vanished falls to NULL, not to the first', () {
      final c = SessionController();
      c.signIn(session('t'), [ws('a'), ws('b')]);
      c.replaceWorkspaces([ws('b')]);
      expect(c.selectedWorkspace, isNull,
          reason: 'arriving where the user did not put themselves is a lie');
    });

    test('an empty selection takes the first workspace', () {
      final c = SessionController();
      c.replaceWorkspaces([ws('a'), ws('b')]);
      expect(c.selectedWorkspace?.id, 'a');
    });
  });

  group('rename / create / delete', () {
    test('renaming keeps the workspace selected and replaced in place', () {
      final c = SessionController();
      c.signIn(session('t'), [ws('a'), ws('b')]);
      c.select(ws('b'));
      c.applyRename(ws('b', name: 'renamed'));
      expect(c.workspaces[1].name, 'renamed');
      expect(c.selectedWorkspace?.name, 'renamed');
    });

    test('addCreated appends, selects, and seeds the first tree', () {
      final c = SessionController();
      c.signIn(session('t'), [ws('a')]);
      c.addCreated(ws('b'), [view('v1')]);
      expect(c.workspaces, hasLength(2));
      expect(c.selectedWorkspace?.id, 'b');
      expect(c.viewsByWorkspace['b'], hasLength(1));
    });

    test('remove drops the tree and reports whether it was selected', () {
      final c = SessionController();
      c.signIn(session('t'), [ws('a'), ws('b')]);
      c.replaceViews('a', [view('v1')]);
      final wasSelected = c.remove(ws('a'));
      expect(wasSelected, isTrue);
      expect(c.workspaces.map((w) => w.id), ['b']);
      expect(c.viewsByWorkspace, isEmpty);
      expect(c.selectedWorkspace?.id, 'b',
          reason: 'deletion falls to the first remaining, unlike a refetch');
    });

    test('remove of an unselected workspace keeps the selection untouched', () {
      final c = SessionController();
      c.signIn(session('t'), [ws('a'), ws('b')]);
      c.select(ws('b'));
      expect(c.remove(ws('a')), isFalse);
      expect(c.selectedWorkspace?.id, 'b');
    });

    test('removing the last workspace leaves a null selection', () {
      final c = SessionController();
      c.signIn(session('t'), [ws('a')]);
      c.remove(ws('a'));
      expect(c.selectedWorkspace, isNull);
      expect(c.workspaces, isEmpty);
    });
  });

  group('view trees', () {
    test('replaceViews overwrites one workspace, leaves the others', () {
      final c = SessionController();
      c.restore(session('t'), [ws('a'), ws('b')], ws('a'), {
        'a': [view('v1')],
        'b': [view('v2')],
      });
      c.replaceViews('a', [view('v1'), view('v3')]);
      expect(c.viewsByWorkspace['a'], hasLength(2));
      expect(c.viewsByWorkspace['b'], hasLength(1));
    });

    test('seedViews fills only the gaps — held entries win', () {
      final c = SessionController();
      c.restore(session('t'), [ws('a'), ws('b')], ws('a'), {
        'a': [view('server-fresh')],
      });
      c.seedViews({
        'a': [view('mirror-stale')],
        'b': [view('mirror-only')],
      });
      expect(c.viewsByWorkspace['a']!.single.id, 'server-fresh',
          reason: 'the server overwrites the mirror as it arrives');
      expect(c.viewsByWorkspace['b']!.single.id, 'mirror-only');
    });
  });

  group('plain list replacement', () {
    test('setWorkspaces leaves the selection untouched even when it vanishes', () {
      final c = SessionController();
      c.signIn(session('t'), [ws('a'), ws('b')]);
      c.select(ws('b'));
      c.setWorkspaces([ws('a')]);
      expect(c.selectedWorkspace?.id, 'b',
          reason: 'plain refetch had no selection opinion; reconcile is replaceWorkspaces');
    });
  });

  group('notifications', () {
    test('every transition notifies exactly once', () {
      final c = SessionController();
      var n = 0;
      c.addListener(() => n++);
      c.select(ws('a'));
      expect(n, 1);
      c.replaceWorkspaces(const []);
      expect(n, 2);
      c.replaceViews('a', const []);
      expect(n, 3);
    });

    test('selecting the same workspace still notifies (rebuild is cheap, silence is a bug finder)', () {
      final c = SessionController();
      c.select(ws('a'));
      var n = 0;
      c.addListener(() => n++);
      c.select(ws('a'));
      expect(n, 1);
    });
  });
}
