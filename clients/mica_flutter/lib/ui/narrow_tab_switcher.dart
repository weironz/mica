// The tab strip's stand-in on the narrow shell (below kNarrowShellWidth).
//
// A row of 180px tabs does not fit a phone-width window, and the narrow shell
// already spends 48px on its top bar; another 34px strip under it would cost
// a tenth of a short screen for something that scrolls sideways. Mobile
// browsers solve the same problem with a tab-count button that opens a list,
// so this is that: a small square with the number of open tabs, in the top
// bar, opening a menu that switches, closes and opens tabs.
//
// Neither reference product has a narrow form to copy: AFFiNE's web build
// drops tabs entirely (the browser's own tabs stand in) and AppFlowy's mobile
// app has no tabs. Dropping them here would strand the tabs a wide window
// opened, so they stay reachable instead.
import 'package:flutter/material.dart';

import '../doc_tab.dart';
import 'doc_tab_strip.dart';
import 'theme_tokens.dart';

class NarrowTabSwitcher extends StatelessWidget {
  const NarrowTabSwitcher({
    super.key,
    required this.tabs,
    required this.activeIndex,
    required this.onSelect,
    required this.onClose,
    required this.untitledLabel,
    required this.tooltip,
    this.onNewTab,
    this.newTabLabel,
  });

  final List<DocTab> tabs;
  final int activeIndex;
  final void Function(int index) onSelect;
  final void Function(int index) onClose;
  final String untitledLabel;
  final String tooltip;

  /// Same as the strip's `+`: receives a global position to anchor its menu.
  final void Function(Offset globalPosition)? onNewTab;
  final String? newTabLabel;

  @override
  Widget build(BuildContext context) {
    final theme = MicaTheme.of(context);
    return IconButton(
      tooltip: tooltip,
      onPressed: () => _open(context),
      icon: Container(
        width: 22,
        height: 22,
        alignment: Alignment.center,
        decoration: BoxDecoration(
          border: Border.all(color: theme.text.muted, width: 1.5),
          borderRadius: BorderRadius.circular(5),
        ),
        child: Text(
          tabs.length > 99 ? '99+' : '${tabs.length}',
          style: TextStyle(
            fontSize: 11,
            fontWeight: FontWeight.w600,
            color: theme.text.primary,
          ),
        ),
      ),
    );
  }

  Future<void> _open(BuildContext context) async {
    final box = context.findRenderObject() as RenderBox?;
    final overlay =
        Overlay.of(context).context.findRenderObject() as RenderBox?;
    if (box == null || overlay == null) return;
    final anchor = box.localToGlobal(
      box.size.bottomRight(Offset.zero),
      ancestor: overlay,
    );
    final canClose = tabs.length > 1;
    final picked = await showMenu<String>(
      context: context,
      position: RelativeRect.fromLTRB(
        anchor.dx,
        anchor.dy,
        overlay.size.width - anchor.dx,
        0,
      ),
      items: [
        for (var i = 0; i < tabs.length; i++)
          PopupMenuItem(
            value: 'select:$i',
            child: _TabRow(
              label: docTabLabel(tabs[i], untitledLabel),
              active: i == activeIndex,
              // The last tab cannot be closed (same rule as the strip).
              onClose: canClose
                  ? () => Navigator.of(context).pop('close:$i')
                  : null,
            ),
          ),
        if (onNewTab != null) ...[
          const PopupMenuDivider(),
          PopupMenuItem(
            value: 'new',
            child: Row(
              children: [
                const Icon(Icons.add, size: 18),
                const SizedBox(width: 10),
                Text(newTabLabel ?? ''),
              ],
            ),
          ),
        ],
      ],
    );
    if (picked == null || !context.mounted) return;
    final colon = picked.indexOf(':');
    final index = colon < 0 ? -1 : int.parse(picked.substring(colon + 1));
    switch (picked.substring(0, colon < 0 ? picked.length : colon)) {
      case 'select':
        if (index != activeIndex) onSelect(index);
      case 'close':
        onClose(index);
      case 'new':
        onNewTab?.call(box.localToGlobal(box.size.bottomRight(Offset.zero)));
    }
  }
}

class _TabRow extends StatelessWidget {
  const _TabRow({required this.label, required this.active, this.onClose});

  final String label;
  final bool active;
  final VoidCallback? onClose;

  @override
  Widget build(BuildContext context) {
    final theme = MicaTheme.of(context);
    return Row(
      children: [
        SizedBox(
          width: 18,
          child: active ? const Icon(Icons.check, size: 16) : null,
        ),
        const SizedBox(width: 10),
        Expanded(
          child: Text(
            label,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(
              fontWeight: active ? FontWeight.w600 : FontWeight.normal,
              color: theme.text.primary,
            ),
          ),
        ),
        if (onClose != null)
          IconButton(
            icon: const Icon(Icons.close, size: 16),
            visualDensity: VisualDensity.compact,
            color: theme.text.muted,
            onPressed: onClose,
          ),
      ],
    );
  }
}
