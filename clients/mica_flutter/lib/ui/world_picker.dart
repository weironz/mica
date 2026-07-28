// Which world you are about to enter: 本地模式, or one of the configured servers.
//
// This belongs on the sign-in screen because on desktop that screen IS the front
// door. Until now the only way to add, remove or switch servers was the account
// menu — which lives *inside* the app shell, i.e. behind the door. Signing in to
// a server you had not added yet meant guessing that the way in was a menu you
// could only reach after getting in.
//
// Web passes nothing here: it is served BY one server and has no local world
// (`local_offline_web.dart` hard-codes `available => false`), so a picker there
// would be a control with exactly one dead option.
//
// **All copy arrives already localized** (the `status_kit.dart` contract).

import 'package:flutter/material.dart';

import '../server_list.dart' show serverLabel;

/// The origin string that means "the on-device world", matching the store's own
/// `origin` column.
const String kLocalOrigin = 'local';

/// Finished strings for [WorldPicker].
class WorldPickerStrings {
  const WorldPickerStrings({
    required this.heading,
    required this.localName,
    required this.localSubtitle,
    required this.addServer,
    required this.removeServer,
  });

  final String heading;

  /// 「本地模式」 and the line under it.
  final String localName;
  final String localSubtitle;

  final String addServer;

  /// Tooltip on each server's remove button.
  final String removeServer;
}

/// A list of worlds — the local one first, then every configured server — with
/// add and remove.
class WorldPicker extends StatelessWidget {
  const WorldPicker({
    required this.origins,
    required this.active,
    required this.strings,
    required this.onSelect,
    this.onAdd,
    this.onRemove,
    super.key,
  });

  /// Configured server origins, in the user's order. [kLocalOrigin] is NOT
  /// expected here — the local row is always drawn first and is not a server.
  final List<String> origins;

  /// The world currently in effect ([kLocalOrigin] or one of [origins]).
  final String active;

  final WorldPickerStrings strings;

  /// Enter this world. For [kLocalOrigin] the caller usually also leaves the
  /// sign-in screen: there is nothing left to sign in to.
  final void Function(String origin) onSelect;

  /// Null hides the row rather than showing a dead one.
  final VoidCallback? onAdd;

  /// Null hides every remove button. 本地模式 never has one — it is not a server,
  /// and its workspaces exist nowhere else.
  final void Function(String origin)? onRemove;

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(
          strings.heading,
          style: const TextStyle(
            fontSize: 11,
            letterSpacing: 0.6,
            color: Color(0xFF94A3B8),
          ),
        ),
        const SizedBox(height: 8),
        DecoratedBox(
          decoration: BoxDecoration(
            color: const Color(0xFFF8FAFC),
            borderRadius: BorderRadius.circular(10),
            border: Border.all(color: const Color(0xFFE2E8F0)),
          ),
          child: Column(
            children: [
              _row(
                context,
                origin: kLocalOrigin,
                label: strings.localName,
                subtitle: strings.localSubtitle,
                icon: Icons.computer_outlined,
              ),
              for (final origin in origins)
                _row(
                  context,
                  origin: origin,
                  label: serverLabel(origin),
                  icon: Icons.cloud_outlined,
                ),
              if (onAdd != null)
                InkWell(
                  onTap: onAdd,
                  child: Padding(
                    padding: const EdgeInsets.symmetric(
                      horizontal: 12,
                      vertical: 11,
                    ),
                    child: Row(
                      children: [
                        const Icon(
                          Icons.add,
                          size: 16,
                          color: Color(0xFF2563EB),
                        ),
                        const SizedBox(width: 10),
                        Text(
                          strings.addServer,
                          style: const TextStyle(
                            fontSize: 13,
                            color: Color(0xFF2563EB),
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
            ],
          ),
        ),
      ],
    );
  }

  Widget _row(
    BuildContext context, {
    required String origin,
    required String label,
    required IconData icon,
    String? subtitle,
  }) {
    final selected = origin == active;
    final removable = origin != kLocalOrigin && onRemove != null;
    return InkWell(
      onTap: () => onSelect(origin),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
        child: Row(
          children: [
            // The check LEADS, and the type icon stays: 🖥 vs ☁ is the whole
            // statement that these are not the same kind of thing.
            SizedBox(
              width: 16,
              child: selected
                  ? const Icon(Icons.check, size: 15, color: Color(0xFF2563EB))
                  : null,
            ),
            const SizedBox(width: 4),
            Icon(icon, size: 16, color: const Color(0xFF475569)),
            const SizedBox(width: 10),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                mainAxisSize: MainAxisSize.min,
                children: [
                  Text(
                    label,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      fontSize: 13,
                      fontWeight: selected ? FontWeight.w600 : FontWeight.w400,
                      color: const Color(0xFF0F172A),
                    ),
                  ),
                  if (subtitle != null)
                    Text(
                      subtitle,
                      style: const TextStyle(
                        fontSize: 11,
                        color: Color(0xFF94A3B8),
                      ),
                    ),
                ],
              ),
            ),
            if (removable)
              IconButton(
                onPressed: () => onRemove!(origin),
                icon: const Icon(Icons.delete_outline, size: 16),
                tooltip: strings.removeServer,
                visualDensity: VisualDensity.compact,
                padding: EdgeInsets.zero,
                constraints: const BoxConstraints(minWidth: 28, minHeight: 28),
              ),
          ],
        ),
      ),
    );
  }
}
