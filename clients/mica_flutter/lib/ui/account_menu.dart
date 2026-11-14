import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

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
  final _menu = MenuController();

  Widget _avatar(double radius) => UserAvatar(
    url: widget.avatarUrl,
    fallback: widget.initial,
    radius: radius,
  );

  Widget _action(IconData icon, String label, VoidCallback action) =>
      MenuItemButton(
        onPressed: () {
          _menu.close();
          action();
        },
        leadingIcon: Icon(icon, size: 18),
        child: Text(label),
      );

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final tokens = MicaTheme.of(context);
    return MenuAnchor(
      controller: _menu,
      style: const MenuStyle(
        minimumSize: WidgetStatePropertyAll(Size(248, 0)),
        maximumSize: WidgetStatePropertyAll(Size(300, double.infinity)),
        padding: WidgetStatePropertyAll(EdgeInsets.all(6)),
      ),
      menuChildren: [
        Padding(
          padding: const EdgeInsets.all(10),
          child: Row(
            children: [
              _avatar(20),
              const SizedBox(width: 10),
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
                        fontWeight: FontWeight.w500,
                      ),
                    ),
                    if (widget.email != null)
                      Text(
                        widget.email!,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: MicaTypography.caption.copyWith(
                          color: tokens.text.muted,
                        ),
                      ),
                  ],
                ),
              ),
            ],
          ),
        ),
        const Divider(height: 8),
        _action(Icons.settings_outlined, l10n.settingsTitle, widget.onSettings),
        if (!kIsWeb)
          _action(
            Icons.swap_horiz,
            l10n.accountSwitchWorld,
            widget.onSwitchWorld,
          ),
        if (widget.updates.supported)
          _action(
            Icons.system_update_alt,
            l10n.updateCheck,
            () => unawaited(activateDesktopUpdate(context, widget.updates)),
          ),
        if (widget.onSignOut != null || widget.onSignIn != null) ...[
          const Divider(height: 8),
          if (widget.onSignOut case final signOut?)
            _action(Icons.logout, l10n.commonSignOut, signOut)
          else if (widget.onSignIn case final signIn?)
            _action(Icons.login, l10n.workspaceRowSignInCloud, signIn),
        ],
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
              Expanded(
                child: Text(
                  widget.name,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: MicaTypography.body.copyWith(
                    fontWeight: FontWeight.w500,
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
