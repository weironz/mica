part of '../main.dart';

// Account tab: avatar, profile, password, delete — cloud only.
extension _SettingsAccountTab on _SettingsDialogState {
  /// Pick and upload a new picture. A dismissed picker is not an error and
  /// says nothing; only a failed upload gets a message.
  Future<void> _changeAvatar() => _runAvatarAction(widget.onChangeAvatar!);

  Future<void> _removeAvatar() => _runAvatarAction(widget.onRemoveAvatar!);

  Future<void> _runAvatarAction(Future<String?> Function() action) async {
    setUi(() {
      _accountBusy = true;
      _accountMsg = null;
      _accountMessageIsError = false;
    });
    try {
      final url = await action();
      if (mounted) setUi(() => _avatarUrl = url);
    } catch (error) {
      if (mounted) {
        setUi(() {
          _accountMsg = error.toString();
          _accountMessageIsError = true;
        });
      }
    } finally {
      if (mounted) setUi(() => _accountBusy = false);
    }
  }

  Future<void> _saveProfile() async {
    final l10n = context.l10n;
    setUi(() {
      _accountBusy = true;
      _accountMsg = null;
      _accountMessageIsError = false;
    });
    try {
      await widget.onUpdateProfile!(_name.text.trim());
      // Stay open: saving is not leaving. ("like server config" — the thing it
      // copied — no longer closes either; nothing in Settings does.)
      if (mounted) setUi(() => _accountMsg = l10n.accountSaved);
    } catch (error) {
      if (mounted) {
        setUi(() {
          _accountMsg = error.toString();
          _accountMessageIsError = true;
        });
      }
    } finally {
      if (mounted) setUi(() => _accountBusy = false);
    }
  }

  Future<void> _changeAccountPassword() async {
    final l10n = context.l10n;
    if (_newPass.text.length < 8) {
      setUi(() {
        _accountMsg = l10n.accountPasswordTooShort;
        _accountMessageIsError = true;
      });
      return;
    }
    setUi(() {
      _accountBusy = true;
      _accountMsg = null;
      _accountMessageIsError = false;
    });
    try {
      await widget.onChangePassword!(_curPass.text, _newPass.text);
      if (!mounted) return;
      // Stay open, and clear the fields — leaving a password sitting in a live
      // text box is the reason closing felt like the tidy option.
      _curPass.clear();
      _newPass.clear();
      // Accurate, not reassuring: change_password revokes EVERY family of this
      // user (auth.rs `revoke_user_sessions`) — this device included. Saying
      // "other devices" would be a lie, and the surprise would land later, when
      // this session quietly fails to renew.
      setUi(() => _accountMsg = l10n.accountPasswordChanged);
    } catch (error) {
      if (mounted) {
        setUi(() {
          _accountMsg = error.toString();
          _accountMessageIsError = true;
        });
      }
    } finally {
      if (mounted) setUi(() => _accountBusy = false);
    }
  }

  Future<void> _deleteAccount() async {
    // Password re-entry lives in the confirm dialog, not the Account form: it is
    // asked for once, right next to the irreversible warning, and never sits in
    // a live field afterwards. A local controller keeps it out of _SettingsDialog
    // state entirely — it dies with the dialog.
    final passCtl = TextEditingController();
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (dialogContext) {
        final l10n = dialogContext.l10n;
        return AlertDialog(
          title: Text(l10n.accountDeleteTitle),
          content: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                l10n.accountDeleteWarning,
                style: TextStyle(
                  color: MicaTheme.of(context).text.muted,
                  fontSize: 13,
                ),
              ),
              const SizedBox(height: 16),
              TextField(
                controller: passCtl,
                obscureText: true,
                autofocus: true,
                decoration: InputDecoration(
                  labelText: l10n.accountDeletePasswordLabel,
                  border: const OutlineInputBorder(),
                ),
              ),
            ],
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.of(dialogContext).pop(false),
              child: Text(l10n.commonCancel),
            ),
            FilledButton(
              style: FilledButton.styleFrom(
                backgroundColor: MicaTheme.of(context).status.danger,
              ),
              onPressed: () => Navigator.of(dialogContext).pop(true),
              child: Text(l10n.accountDeleteConfirm),
            ),
          ],
        );
      },
    );
    final password = confirmed == true ? passCtl.text : null;
    // The pop transition still renders the field after the future resolves —
    // disposing its controller now is the lessons-file "used after dispose"
    // crash. Dispose once the transition has finished.
    unawaited(
      Future<void>.delayed(const Duration(milliseconds: 300), passCtl.dispose),
    );
    if (password == null) return;
    setUi(() {
      _accountBusy = true;
      _accountMsg = null;
      _accountMessageIsError = false;
    });
    try {
      await widget.onDeleteAccount!(password);
      // Success tears down the session (the account is gone) — the parent's
      // callback signs out, which pops Settings with it. Nothing to show here.
    } catch (error) {
      if (mounted) {
        setUi(() {
          _accountMsg = error.toString();
          _accountMessageIsError = true;
          _accountBusy = false;
        });
      }
    }
  }

  List<Widget> _accountSection(BuildContext context) => [
    const SizedBox(height: 10),
    if (widget.onChangeAvatar != null) ...[
      Row(
        children: [
          UserAvatar(
            url: _avatarUrl,
            radius: 28,
            fallback: widget.userName.isNotEmpty
                ? widget.userName.substring(0, 1).toUpperCase()
                : '?',
          ),
          const SizedBox(width: 14),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  widget.userEmail,
                  style: Theme.of(context).textTheme.bodySmall?.copyWith(
                    color: MicaTheme.of(context).text.muted,
                  ),
                ),
                const SizedBox(height: 8),
                Wrap(
                  spacing: 8,
                  children: [
                    OutlinedButton.icon(
                      onPressed: _accountBusy ? null : _changeAvatar,
                      icon: const Icon(Icons.photo_camera_outlined, size: 16),
                      label: Text(context.l10n.accountChangeAvatar),
                    ),
                    // Only offered when there is something to remove — a
                    // greyed-out button here would be one more control that
                    // cannot say why it is dead.
                    if (_avatarUrl != null && widget.onRemoveAvatar != null)
                      TextButton(
                        onPressed: _accountBusy ? null : _removeAvatar,
                        child: Text(context.l10n.accountRemoveAvatar),
                      ),
                  ],
                ),
              ],
            ),
          ),
        ],
      ),
      const SizedBox(height: 16),
    ] else ...[
      const SizedBox(height: 4),
      Text(
        widget.userEmail,
        style: Theme.of(context).textTheme.bodySmall?.copyWith(
          color: MicaTheme.of(context).text.muted,
        ),
      ),
      const SizedBox(height: 12),
    ],
    TextField(
      controller: _name,
      enabled: !_accountBusy,
      decoration: InputDecoration(
        labelText: context.l10n.accountDisplayName,
        border: const OutlineInputBorder(),
      ),
    ),
    const SizedBox(height: 8),
    Align(
      alignment: Alignment.centerLeft,
      child: OutlinedButton.icon(
        onPressed: _accountBusy ? null : _saveProfile,
        icon: const Icon(Icons.save, size: 16),
        label: Text(context.l10n.accountSaveName),
      ),
    ),
    const SizedBox(height: 16),
    TextField(
      controller: _curPass,
      enabled: !_accountBusy,
      obscureText: true,
      decoration: InputDecoration(
        labelText: context.l10n.accountCurrentPassword,
        border: const OutlineInputBorder(),
      ),
    ),
    const SizedBox(height: 8),
    TextField(
      controller: _newPass,
      enabled: !_accountBusy,
      obscureText: true,
      decoration: InputDecoration(
        labelText: context.l10n.accountNewPassword,
        border: const OutlineInputBorder(),
      ),
    ),
    const SizedBox(height: 8),
    Align(
      alignment: Alignment.centerLeft,
      child: OutlinedButton.icon(
        onPressed: _accountBusy ? null : _changeAccountPassword,
        icon: const Icon(Icons.lock_outline, size: 16),
        label: Text(context.l10n.accountChangePassword),
      ),
    ),
    if (_accountMsg != null) ...[
      const SizedBox(height: 10),
      if (_accountMessageIsError)
        ErrorBanner(_accountMsg!)
      else
        Row(
          children: [
            Icon(
              Icons.check_circle_outline,
              size: 18,
              color: MicaTheme.of(context).status.success,
            ),
            const SizedBox(width: 8),
            Expanded(
              child: Text(
                _accountMsg!,
                style: TextStyle(
                  color: MicaTheme.of(context).status.success,
                  fontSize: 13,
                ),
              ),
            ),
          ],
        ),
    ],
    if (widget.onDeleteAccount != null) ...[
      const Divider(height: 32),
      Align(
        alignment: Alignment.centerLeft,
        child: OutlinedButton.icon(
          onPressed: _accountBusy ? null : _deleteAccount,
          style: OutlinedButton.styleFrom(
            foregroundColor: MicaTheme.of(context).status.danger,
            side: BorderSide(
              color: MicaTheme.of(context).status.danger.withValues(alpha: 0.5),
            ),
          ),
          icon: const Icon(Icons.delete_forever_outlined, size: 16),
          label: Text(context.l10n.accountDelete),
        ),
      ),
    ],
  ];
}
