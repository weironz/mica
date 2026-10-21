/// The two halves of workspace invitations in the UI
/// (docs/features/workspace-invitations.md):
///
///  - [MemberInviteSection] — in the workspace settings dialog, for admins:
///    invite by email, see what is still pending, revoke it.
///  - [ReceivedInvitationsCard] — in the sidebar under the workspace switcher,
///    for the invitee: which workspace, who asked, accept / decline.
///
/// Kept out of `main_workspace_view.dart` (a ratcheted monolith, see
/// scripts/file-size-budget.txt); that file only places these widgets.
library;

import 'package:flutter/material.dart';

import '../api/models.dart';
import '../l10n/locale_controller.dart';
import '../views/invitations_controller.dart';
import 'theme_tokens.dart';

/// Invite form + pending list. Reports results IN the dialog: the shell's
/// error banner sits behind the modal barrier, where nobody can read it.
class MemberInviteSection extends StatefulWidget {
  const MemberInviteSection({
    required this.controller,
    required this.workspaceId,
    super.key,
  });

  final InvitationsController controller;
  final String workspaceId;

  @override
  State<MemberInviteSection> createState() => _MemberInviteSectionState();
}

class _MemberInviteSectionState extends State<MemberInviteSection> {
  final _email = TextEditingController();
  WorkspaceRole _role = WorkspaceRole.editor;
  List<WorkspaceInvitation> _pending = const [];
  bool _busy = false;

  /// The outcome of the last invite: (text, isError). Null before any.
  (String, bool)? _status;

  @override
  void initState() {
    super.initState();
    _loadPending();
  }

  @override
  void dispose() {
    _email.dispose();
    super.dispose();
  }

  Future<void> _loadPending() async {
    try {
      final pending = await widget.controller.pending(widget.workspaceId);
      if (mounted) setState(() => _pending = pending);
    } catch (_) {
      // The list is a convenience; failing to load it must not block inviting.
    }
  }

  Future<void> _invite() async {
    final l10n = context.l10n;
    setState(() => _busy = true);
    try {
      await widget.controller.invite(
        widget.workspaceId,
        _email.text,
        _role.apiValue,
      );
      _email.clear();
      _status = (l10n.workspaceInviteSent, false);
      await _loadPending();
    } on ApiException catch (e) {
      _status = (
        e.statusCode == 409 ? l10n.workspaceInviteAlreadyMember : e.message,
        true,
      );
    } catch (e) {
      _status = (e.toString(), true);
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _revoke(WorkspaceInvitation invitation) async {
    try {
      await widget.controller.revoke(invitation);
    } catch (_) {
      // Already gone (accepted, declined, revoked elsewhere): the reload
      // below shows the truth either way.
    }
    await _loadPending();
  }

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final tokens = MicaTheme.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        TextField(
          controller: _email,
          keyboardType: TextInputType.emailAddress,
          decoration: InputDecoration(
            labelText: l10n.workspaceMemberEmail,
            prefixIcon: const Icon(Icons.alternate_email),
            border: const OutlineInputBorder(),
          ),
        ),
        const SizedBox(height: 10),
        DropdownButtonFormField<WorkspaceRole>(
          initialValue: _role,
          decoration: InputDecoration(
            labelText: l10n.widgetRoleLabel,
            border: const OutlineInputBorder(),
          ),
          items: [
            for (final role in WorkspaceRole.values)
              DropdownMenuItem(value: role, child: Text(role.apiValue)),
          ],
          onChanged: (role) {
            if (role != null) setState(() => _role = role);
          },
        ),
        const SizedBox(height: 10),
        FilledButton.icon(
          onPressed: _busy ? null : _invite,
          icon: const Icon(Icons.person_add),
          label: Text(l10n.workspaceMemberInvite),
        ),
        if (_status case (final text, final isError)) ...[
          const SizedBox(height: 8),
          Text(
            text,
            style: TextStyle(
              color: isError ? tokens.status.danger : tokens.status.success,
            ),
          ),
        ],
        if (_pending.isNotEmpty) ...[
          const SizedBox(height: 14),
          Text(
            l10n.workspaceInvitesPending,
            style: Theme.of(context).textTheme.titleSmall,
          ),
          const SizedBox(height: 6),
          for (final invitation in _pending)
            _PendingInvitationRow(
              invitation: invitation,
              onRevoke: () => _revoke(invitation),
            ),
        ],
      ],
    );
  }
}

class _PendingInvitationRow extends StatelessWidget {
  const _PendingInvitationRow({
    required this.invitation,
    required this.onRevoke,
  });

  final WorkspaceInvitation invitation;
  final VoidCallback onRevoke;

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final tokens = MicaTheme.of(context);
    return Padding(
      padding: const EdgeInsets.only(bottom: 4),
      child: Row(
        children: [
          const Icon(Icons.mail_outline, size: 18),
          const SizedBox(width: 8),
          Expanded(
            child: Text(
              '${invitation.email} · ${invitation.role}',
              overflow: TextOverflow.ellipsis,
            ),
          ),
          const SizedBox(width: 8),
          DecoratedBox(
            decoration: BoxDecoration(
              color: tokens.status.warningWash,
              borderRadius: BorderRadius.circular(4),
            ),
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
              child: Text(
                l10n.workspaceInvitePendingBadge,
                style: TextStyle(fontSize: 12, color: tokens.status.warning),
              ),
            ),
          ),
          IconButton(
            tooltip: l10n.workspaceInviteRevoke,
            visualDensity: VisualDensity.compact,
            onPressed: onRevoke,
            icon: const Icon(Icons.close, size: 18),
          ),
        ],
      ),
    );
  }
}

/// The invitee's side: one row per pending invitation, under the workspace
/// switcher. Renders nothing at all when there are none.
class ReceivedInvitationsCard extends StatefulWidget {
  const ReceivedInvitationsCard({required this.controller, super.key});

  final InvitationsController controller;

  @override
  State<ReceivedInvitationsCard> createState() =>
      _ReceivedInvitationsCardState();
}

class _ReceivedInvitationsCardState extends State<ReceivedInvitationsCard> {
  /// The invitation whose accept/decline is in flight (buttons disabled).
  String? _busyId;
  String? _error;

  Future<void> _answer(
    ReceivedInvitation invitation,
    Future<void> Function() action,
  ) async {
    final l10n = context.l10n;
    setState(() {
      _busyId = invitation.id;
      _error = null;
    });
    try {
      await action();
    } on ApiException catch (e) {
      // 404: accepted/declined elsewhere, or revoked. Re-read the inbox
      // instead of leaving a row that can never succeed.
      _error = e.statusCode == 404
          ? l10n.invitationGone
          : l10n.invitationActionFailed;
      if (e.statusCode == 404) await widget.controller.refreshReceived();
    } catch (_) {
      _error = l10n.invitationActionFailed;
    } finally {
      if (mounted) setState(() => _busyId = null);
    }
  }

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: widget.controller,
      builder: (context, _) {
        final received = widget.controller.received;
        if (received.isEmpty) return const SizedBox.shrink();
        final l10n = context.l10n;
        final tokens = MicaTheme.of(context);
        return Padding(
          padding: const EdgeInsets.only(top: 10),
          child: DecoratedBox(
            decoration: BoxDecoration(
              color: tokens.accent.wash,
              borderRadius: BorderRadius.circular(8),
              border: Border.all(color: tokens.border.subtle),
            ),
            child: Padding(
              padding: const EdgeInsets.fromLTRB(12, 10, 12, 6),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    l10n.invitationsReceivedTitle,
                    style: Theme.of(context).textTheme.titleSmall,
                  ),
                  for (final invitation in received)
                    _row(context, invitation),
                  if (_error case final error?)
                    Padding(
                      padding: const EdgeInsets.only(top: 4, bottom: 4),
                      child: Text(
                        error,
                        style: TextStyle(color: tokens.status.danger),
                      ),
                    ),
                ],
              ),
            ),
          ),
        );
      },
    );
  }

  Widget _row(BuildContext context, ReceivedInvitation invitation) {
    final l10n = context.l10n;
    final busy = _busyId != null;
    return Padding(
      padding: const EdgeInsets.only(top: 8),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            invitation.workspaceName,
            style: const TextStyle(fontWeight: FontWeight.w600),
            overflow: TextOverflow.ellipsis,
          ),
          Text(
            invitation.invitedByName.isEmpty
                ? l10n.invitationInvitedAs(invitation.role)
                : l10n.invitationInvitedBy(
                    invitation.invitedByName,
                    invitation.role,
                  ),
            style: TextStyle(
              fontSize: 12,
              color: MicaTheme.of(context).text.muted,
            ),
          ),
          Row(
            mainAxisAlignment: MainAxisAlignment.end,
            children: [
              TextButton(
                onPressed: busy
                    ? null
                    : () => _answer(
                        invitation,
                        () => widget.controller.decline(invitation),
                      ),
                child: Text(l10n.invitationDecline),
              ),
              const SizedBox(width: 4),
              FilledButton(
                onPressed: busy
                    ? null
                    : () => _answer(
                        invitation,
                        () => widget.controller.accept(invitation),
                      ),
                child: Text(l10n.invitationAccept),
              ),
            ],
          ),
        ],
      ),
    );
  }
}
