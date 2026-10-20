import 'package:flutter/foundation.dart';

import '../api/models.dart';

/// The workspace-members map of `_WorkspaceShellState`, extracted verbatim
/// (god-class fifth step; docs/reviews/code-optimization/OPTIMIZATION_PLAN_ROUND3.md
/// §7). Owns the only `membersByWorkspace` copy — the State keeps a synced
/// view-model mirror for its build helpers.
///
/// Rules pinned by tests, all inherited from the original code:
///  - add and update RELOAD the member list afterwards (the server's POST
///    answer is a single member, not the list);
///  - remove does NOT reload — the server's DELETE answer already IS the new
///    list, and refetching after a deliberate act is a wasted round trip;
///  - the offline world cannot know members, so a mirror-driven workspace
///    switch files the empty list via [replace].
class MembersController extends ChangeNotifier {
  MembersController({
    required this.listMembers,
    required this.addMember,
    required this.updateMember,
    required this.removeMember,
  });

  final Future<List<WorkspaceMember>> Function(
    String token,
    String workspaceId,
  ) listMembers;
  final Future<void> Function(
    String token,
    String workspaceId,
    String email,
    String role,
  ) addMember;
  final Future<void> Function(
    String token,
    String workspaceId,
    String userId,
    String role,
  ) updateMember;
  final Future<List<WorkspaceMember>> Function(
    String token,
    String workspaceId,
    String userId,
  ) removeMember;

  Map<String, List<WorkspaceMember>> membersByWorkspace = const {};

  /// Fetch the selected workspace's member list into the map.
  Future<void> load(String token, String workspaceId) async {
    final members = await listMembers(token, workspaceId);
    replace(workspaceId, members);
  }

  /// Invite: the server's answer is the new member, not the list — reload.
  Future<void> add(
    String token,
    String workspaceId,
    String email,
    String role,
  ) async {
    await addMember(token, workspaceId, email, role);
    await load(token, workspaceId);
  }

  /// Change a role: same reload contract as [add].
  Future<void> update(
    String token,
    String workspaceId,
    String userId,
    String role,
  ) async {
    await updateMember(token, workspaceId, userId, role);
    await load(token, workspaceId);
  }

  /// Remove: the server's DELETE answer already is the new list — no reload.
  Future<void> remove(String token, String workspaceId, String userId) async {
    final members = await removeMember(token, workspaceId, userId);
    replace(workspaceId, members);
  }

  /// File a list as-is (the offline switch writes the empty list with this).
  void replace(String workspaceId, List<WorkspaceMember> members) {
    membersByWorkspace = {...membersByWorkspace, workspaceId: members};
    notifyListeners();
  }

  /// Signing out: everything cloud-side goes.
  void clear() {
    membersByWorkspace = const {};
    notifyListeners();
  }
}
