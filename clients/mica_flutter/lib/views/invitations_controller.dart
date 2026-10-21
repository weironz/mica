import 'package:flutter/foundation.dart';

import '../api/client.dart';
import '../api/models.dart';

/// Workspace invitations, both sides (docs/features/workspace-invitations.md).
///
/// "Add a member" became an invitation the other person accepts, so the
/// client has two new jobs: an admin sees and revokes what is pending, and an
/// invitee sees what they were invited to and answers it. Both go through
/// here so the shell only has to construct one object and say when to
/// [refreshReceived] (startup, sign-in, workspace-list refresh — no push).
///
/// Errors are NOT swallowed: the settings dialog and the sidebar card show
/// them in place. The shell's `_run` banner sits behind a modal dialog, where
/// nobody would see it.
class InvitationsController extends ChangeNotifier {
  InvitationsController({
    required this.api,
    required this.token,
    required this.onJoined,
  });

  final ApiClient api;

  /// The current access token, or null when signed out.
  final String? Function() token;

  /// Called after an accept, with the workspace just joined — the shell
  /// refreshes its list and switches there.
  final Future<void> Function(Workspace workspace) onJoined;

  /// Invitations addressed to the signed-in user. Empty when signed out.
  List<ReceivedInvitation> received = const [];

  String _requireToken() {
    final t = token();
    if (t == null) throw const ApiException('not signed in');
    return t;
  }

  /// Re-read the invitee inbox. Quiet on failure: an inbox that could not be
  /// loaded just shows nothing, it must not put an error in front of the tree.
  Future<void> refreshReceived() async {
    final t = token();
    if (t == null) return clear();
    try {
      received = await api.listReceivedInvitations(t);
      notifyListeners();
    } catch (_) {}
  }

  Future<Workspace> accept(ReceivedInvitation invitation) async {
    final workspace = await api.acceptInvitation(
      _requireToken(),
      invitation.id,
    );
    _drop(invitation.id);
    await onJoined(workspace);
    return workspace;
  }

  Future<void> decline(ReceivedInvitation invitation) async {
    await api.declineInvitation(_requireToken(), invitation.id);
    _drop(invitation.id);
  }

  // ---- admin side: no state kept, the dialog owns its list ----

  Future<WorkspaceInvitation> invite(
    String workspaceId,
    String email,
    String role,
  ) => api.inviteWorkspaceMember(_requireToken(), workspaceId, email, role);

  Future<List<WorkspaceInvitation>> pending(String workspaceId) =>
      api.listWorkspaceInvitations(_requireToken(), workspaceId);

  Future<void> revoke(WorkspaceInvitation invitation) =>
      api.revokeWorkspaceInvitation(
        _requireToken(),
        invitation.workspaceId,
        invitation.id,
      );

  /// Signing out: the next account must not see this one's inbox.
  void clear() {
    received = const [];
    notifyListeners();
  }

  void _drop(String id) {
    received = [
      for (final i in received)
        if (i.id != id) i,
    ];
    notifyListeners();
  }
}
