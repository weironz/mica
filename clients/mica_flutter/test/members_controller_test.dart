import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/views/members_controller.dart';

/// Characterization tests for [MembersController], pinned against the
/// `_membersByWorkspace` behaviors of `_WorkspaceShellState` (god-class fifth
/// step; docs/code-optimization/OPTIMIZATION_PLAN_ROUND3.md §7).
void main() {
  WorkspaceMember member(String id, {String role = 'editor'}) =>
      WorkspaceMember(
        userId: id,
        email: '$id@t.dev',
        displayName: id.toUpperCase(),
        role: role,
      );

  final calls = <String>[];
  var listAnswer = <WorkspaceMember>[];

  setUp(() {
    calls.clear();
    listAnswer = [];
  });

  MembersController build() => MembersController(
        listMembers: (token, workspaceId) async {
          calls.add('list:$workspaceId');
          return listAnswer;
        },
        addMember: (token, workspaceId, email, role) async {
          calls.add('add:$workspaceId/$email/$role');
        },
        updateMember: (token, workspaceId, userId, role) async {
          calls.add('update:$workspaceId/$userId/$role');
        },
        removeMember: (token, workspaceId, userId) async {
          calls.add('remove:$workspaceId/$userId');
          return listAnswer;
        },
      );

  test('add RELOADS the list afterwards (the POST answer is one member)',
      () async {
    final c = build();
    await c.add('t', 'w1', 'a@t.dev', 'editor');
    expect(calls, ['add:w1/a@t.dev/editor', 'list:w1'],
        reason: 'addWorkspaceMember answered a member, not the list');
  });

  test('update RELOADS the list afterwards', () async {
    final c = build();
    await c.update('t', 'w1', 'u1', 'viewer');
    expect(calls, ['update:w1/u1/viewer', 'list:w1']);
  });

  test('remove files the server\'s DELETE answer directly — no reload',
      () async {
    listAnswer = [member('u2')];
    final c = build();
    await c.remove('t', 'w1', 'u1');
    expect(calls, ['remove:w1/u1'],
        reason: 'the DELETE answer already IS the new list');
    expect(c.membersByWorkspace['w1']!.single.userId, 'u2');
  });

  test('replace files a list as-is (the offline switch writes empty)', () {
    final c = build();
    c.replace('w1', [member('u1')]);
    c.replace('w2', const []);
    expect(c.membersByWorkspace['w1']!.single.userId, 'u1');
    expect(c.membersByWorkspace['w2'], isEmpty,
        reason: '_openWorkspaceFromMirror: members are unknowable offline');
  });

  test('clear empties everything (the sign-out path)', () {
    final c = build();
    c.replace('w1', [member('u1')]);
    c.replace('w2', [member('u2')]);
    c.clear();
    expect(c.membersByWorkspace, isEmpty);
  });

  test('notifications fire on replace and clear', () {
    final c = build();
    var notifications = 0;
    c.addListener(() => notifications++);
    c.replace('w1', [member('u1')]);
    c.clear();
    expect(notifications, 2);
  });
}
