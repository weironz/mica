import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../l10n/locale_controller.dart';
import 'theme_tokens.dart';
import 'typography.dart';
import 'update_controls.dart';
import 'user_avatar.dart';

/// Account details belong in the popover; the resident row only names its owner.
class AccountMenu extends StatefulWidget {
  const AccountMenu({
    super.key,
    required this.name,
    required this.initial,
    required this.onSettings,
    required this.onSwitchWorld,
    required this.updates,
    this.email,
    this.avatarUrl,
    this.onSignOut,
    this.onSignIn,
  });

  final String name, initial;
  final String? email, avatarUrl;
  final VoidCallback onSettings, onSwitchWorld;
  final VoidCallback? onSignOut, onSignIn;
  final DesktopUpdateController updates;

  @override
  State<AccountMenu> createState() => _AccountMenuState();
}

class _AccountMenuState extends State<AccountMenu> {
  static const _cardWidth = 272.0;

  final _menu = MenuController();

  Widget _avatar(double radius) => UserAvatar(
    url: widget.avatarUrl,
    fallback: widget.initial,
    radius: radius,
  );

  Widget _card(MicaTokens tokens, List<Widget> children) => Container(
    width: _cardWidth,
    margin: const EdgeInsets.symmetric(vertical: 3),
    padding: const EdgeInsets.all(4),
    decoration: BoxDecoration(
      color: tokens.surface.overlay,
      borderRadius: BorderRadius.circular(10),
    ),
    child: Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: children,
    ),
  );

  Widget _action(
    MicaTokens tokens,
    IconData icon,
    String label,
    VoidCallback? action, {
    String? trailing,
    MenuSerializableShortcut? shortcut,
  }) => MenuItemButton(
    onPressed: action == null
        ? null
        : () {
            _menu.close();
            action();
          },
    leadingIcon: Icon(icon, size: 18),
    shortcut: shortcut,
    trailingIcon: trailing == null
        ? null
        : Text(
            trailing,
            style: MicaTypography.caption.copyWith(color: tokens.text.faint),
          ),
    style: MenuItemButton.styleFrom(
      // Desktop themes default to compact density, which would take 8px back.
      visualDensity: VisualDensity.standard,
      minimumSize: const Size(0, 38),
      padding: const EdgeInsets.symmetric(horizontal: 10),
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(7)),
      textStyle: MicaTypography.body,
      foregroundColor: tokens.text.primary,
      iconColor: tokens.text.muted,
    ),
    child: Text(label),
  );

  Widget _identity(MicaTokens tokens) => Padding(
    padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 8),
    child: Row(
      children: [
        _avatar(20),
        const SizedBox(width: 12),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              Text(
                widget.name,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: MicaTypography.body.copyWith(
                  fontWeight: FontWeight.w600,
                ),
              ),
              if (widget.email != null) ...[
                const SizedBox(height: 2),
                Text(
                  widget.email!,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: MicaTypography.caption.copyWith(
                    color: tokens.text.muted,
                  ),
                ),
              ],
            ],
          ),
        ),
      ],
    ),
  );

  Widget? _updateAction(BuildContext context, MicaTokens tokens) {
    final updates = widget.updates;
    if (!updates.supported) return null;
    final l10n = context.l10n;
    final available = updates.stage == DesktopUpdateStage.available;
    final (label, trailing) = switch (updates.stage) {
      DesktopUpdateStage.available => (
        l10n.updateNow,
        'v${updates.info!.version}',
      ),
      DesktopUpdateStage.checking => (l10n.updateChecking, null),
      DesktopUpdateStage.downloading => (
        l10n.updateDownloading((updates.progress * 100).round()),
        null,
      ),
      _ => (l10n.updateCheck, 'v${updates.currentVersion}'),
    };
    return _action(
      tokens,
      available ? Icons.arrow_circle_up : Icons.arrow_circle_up_outlined,
      label,
      updates.isBusy
          ? null
          : () => unawaited(activateDesktopUpdate(context, updates)),
      trailing: trailing,
    );
  }

  @override
  Widget build(BuildContext context) => UpdateWatcher(
    controller: widget.updates,
    child: ListenableBuilder(
      listenable: widget.updates,
      builder: (context, _) => _build(context),
    ),
  );

  Widget _build(BuildContext context) {
    final l10n = context.l10n;
    final tokens = MicaTheme.of(context);
    final update = _updateAction(context, tokens);
    final hasUpdate = widget.updates.stage == DesktopUpdateStage.available;
    final mac = defaultTargetPlatform == TargetPlatform.macOS;
    return MenuAnchor(
      controller: _menu,
      alignmentOffset: const Offset(0, 6),
      style: MenuStyle(
        backgroundColor: WidgetStatePropertyAll(tokens.surface.sunken),
        padding: const WidgetStatePropertyAll(EdgeInsets.all(5)),
        shape: WidgetStatePropertyAll(
          RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(14),
            side: BorderSide(color: tokens.border.subtle),
          ),
        ),
      ),
      menuChildren: [
        _card(tokens, [_identity(tokens)]),
        _card(tokens, [
          _action(
            tokens,
            Icons.settings_outlined,
            l10n.settingsTitle,
            widget.onSettings,
            shortcut: SingleActivator(
              LogicalKeyboardKey.comma,
              control: !mac,
              meta: mac,
            ),
          ),
          if (!kIsWeb)
            _action(
              tokens,
              Icons.swap_horiz,
              l10n.accountSwitchWorld,
              widget.onSwitchWorld,
            ),
          ?update,
        ]),
        if (widget.onSignOut case final signOut?)
          _card(tokens, [
            _action(tokens, Icons.logout, l10n.commonSignOut, signOut),
          ])
        else if (widget.onSignIn case final signIn?)
          _card(tokens, [
            _action(tokens, Icons.login, l10n.workspaceRowSignInCloud, signIn),
          ]),
      ],
      builder: (context, controller, child) => InkWell(
        borderRadius: BorderRadius.circular(6),
        onTap: () => controller.isOpen ? controller.close() : controller.open(),
        child: Padding(
          padding: const EdgeInsets.symmetric(vertical: 5, horizontal: 4),
          child: Row(
            children: [
              _avatar(14),
              const SizedBox(width: 8),
              Flexible(
                child: Text(
                  widget.name,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: MicaTypography.body.copyWith(
                    fontWeight: FontWeight.w500,
                  ),
                ),
              ),
              // The resident row no longer carries an update button, so a
              // waiting release is announced by a dot that leads to the menu.
              if (hasUpdate)
                Padding(
                  key: const ValueKey('account-update-dot'),
                  padding: const EdgeInsets.only(left: 6),
                  child: Container(
                    width: 7,
                    height: 7,
                    decoration: BoxDecoration(
                      color: tokens.text.primary,
                      shape: BoxShape.circle,
                    ),
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }
}
