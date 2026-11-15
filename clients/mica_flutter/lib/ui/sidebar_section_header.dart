import 'package:flutter/material.dart';

import 'theme_tokens.dart';
import 'typography.dart';

/// Shared label and disclosure geometry for the sidebar's page groups.
class SidebarSectionHeader extends StatelessWidget {
  const SidebarSectionHeader({
    super.key,
    required this.label,
    required this.collapsed,
    required this.onToggle,
  });

  final String label;
  final bool collapsed;
  final VoidCallback onToggle;

  @override
  Widget build(BuildContext context) {
    final color = MicaTheme.of(context).text.faint;
    return Semantics(
      button: true,
      expanded: !collapsed,
      child: InkWell(
        borderRadius: BorderRadius.circular(6),
        onTap: onToggle,
        child: SizedBox(
          height: 28,
          child: Padding(
            padding: const EdgeInsets.only(left: 4),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  label,
                  style: MicaTypography.caption.copyWith(
                    fontWeight: FontWeight.w500,
                    color: color,
                  ),
                ),
                const SizedBox(width: 2),
                Icon(
                  collapsed ? Icons.chevron_right : Icons.expand_more,
                  size: 16,
                  color: color,
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
