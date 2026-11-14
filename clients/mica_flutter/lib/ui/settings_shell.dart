import 'typography.dart';
import 'package:flutter/material.dart';

import 'theme_tokens.dart';

class MicaSettingsDestination {
  const MicaSettingsDestination({
    required this.group,
    required this.title,
    required this.description,
    required this.icon,
    required this.content,
  });

  final String group;
  final String title;
  final String description;
  final IconData icon;
  final Widget content;
}

/// One row of the settings navigation. Public so a shell footer can use the
/// same type, height and icon column as the categories above it.
class MicaSettingsNavItem extends StatelessWidget {
  const MicaSettingsNavItem({
    required this.icon,
    required this.title,
    required this.onTap,
    this.selected = false,
    this.subtitle,
    this.trailing,
    super.key,
  });

  final IconData icon;
  final String title;
  final VoidCallback onTap;
  final bool selected;
  final String? subtitle;
  final Widget? trailing;

  @override
  Widget build(BuildContext context) {
    final tokens = MicaTheme.of(context);
    return Material(
      color: selected ? tokens.surface.hover : Colors.transparent,
      borderRadius: BorderRadius.circular(6),
      child: ListTile(
        contentPadding: const EdgeInsets.symmetric(horizontal: 10),
        minLeadingWidth: 18,
        horizontalTitleGap: 10,
        leading: Icon(icon, size: 18),
        title: Text(
          title,
          style: MicaTypography.body.copyWith(
            fontWeight: selected ? FontWeight.w500 : FontWeight.w400,
            color: selected ? tokens.text.primary : tokens.text.muted,
          ),
        ),
        subtitle: subtitle == null
            ? null
            : Text(
                subtitle!,
                style: MicaTypography.caption.copyWith(
                  color: tokens.text.muted,
                ),
              ),
        trailing: trailing,
        onTap: onTap,
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(6)),
      ),
    );
  }
}

/// Navigation stays reachable while only the selected settings page scrolls.
/// Narrow windows use a category list and a detail page instead of squeezing
/// two columns into the same space. This also works in a resized desktop window.
class MicaSettingsShell extends StatefulWidget {
  const MicaSettingsShell({
    required this.title,
    required this.destinations,
    required this.selectedIndex,
    required this.onSelected,
    required this.closeLabel,
    required this.backLabel,
    required this.onClose,
    this.busy = false,
    this.footer,
    super.key,
  });

  final String title;
  final List<MicaSettingsDestination> destinations;
  final int selectedIndex;
  final ValueChanged<int> onSelected;
  final String closeLabel;
  final String backLabel;
  final VoidCallback onClose;
  final bool busy;
  final Widget? footer;

  @override
  State<MicaSettingsShell> createState() => _MicaSettingsShellState();
}

class _MicaSettingsShellState extends State<MicaSettingsShell> {
  bool _showDetail = false;

  void _select(int index, bool compact) {
    FocusManager.instance.primaryFocus?.unfocus();
    widget.onSelected(index);
    if (compact) setState(() => _showDetail = true);
  }

  Widget _navigation(bool compact) {
    final tokens = MicaTheme.of(context);
    final footer = widget.footer;
    final items = [
      for (var i = 0; i < widget.destinations.length; i++) ...[
        if (i == 0 ||
            widget.destinations[i].group != widget.destinations[i - 1].group)
          Padding(
            padding: EdgeInsets.fromLTRB(10, i == 0 ? 0 : 18, 10, 6),
            child: Text(
              widget.destinations[i].group,
              style: MicaTypography.caption.copyWith(
                fontWeight: FontWeight.w500,
                color: tokens.text.muted,
              ),
            ),
          ),
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 2),
          child: MicaSettingsNavItem(
            key: ValueKey('settings-category-$i'),
            icon: widget.destinations[i].icon,
            title: widget.destinations[i].title,
            selected: !compact && widget.selectedIndex == i,
            subtitle: compact ? widget.destinations[i].description : null,
            trailing: compact
                ? const Icon(Icons.chevron_right, size: 18)
                : null,
            onTap: () => _select(i, compact),
          ),
        ),
      ],
    ];
    if (footer == null) {
      return ListView(
        key: const ValueKey('settings-navigation'),
        padding: const EdgeInsets.fromLTRB(12, 12, 12, 20),
        children: items,
      );
    }
    final pinned = [
      Divider(
        height: 1,
        indent: 12,
        endIndent: 12,
        color: tokens.border.subtle,
      ),
      Padding(padding: const EdgeInsets.fromLTRB(12, 6, 12, 8), child: footer),
    ];
    // Held at the bottom behind a rule rather than trailing the list: it is
    // about the app, not one more page of settings. A window too short for
    // both lets it scroll after the categories instead of covering them.
    return LayoutBuilder(
      builder: (context, box) {
        if (box.maxHeight < _pinFooterMinHeight) {
          return ListView(
            key: const ValueKey('settings-navigation'),
            padding: const EdgeInsets.fromLTRB(0, 12, 0, 0),
            children: [
              Padding(
                padding: const EdgeInsets.fromLTRB(12, 0, 12, 12),
                child: Column(children: items),
              ),
              ...pinned,
            ],
          );
        }
        return Column(
          children: [
            Expanded(
              child: ListView(
                key: const ValueKey('settings-navigation'),
                padding: const EdgeInsets.fromLTRB(12, 12, 12, 12),
                children: items,
              ),
            ),
            ...pinned,
          ],
        );
      },
    );
  }

  static const _pinFooterMinHeight = 320.0;

  Widget _detail(bool compact) {
    final destination = widget.destinations[widget.selectedIndex];
    final tokens = MicaTheme.of(context);
    return SingleChildScrollView(
      key: ValueKey('settings-detail-${widget.selectedIndex}'),
      padding: EdgeInsets.all(compact ? 18 : 28),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            destination.title,
            style: MicaTypography.heading2.copyWith(
              fontWeight: FontWeight.w600,
              color: tokens.text.primary,
            ),
          ),
          const SizedBox(height: 8),
          Text(
            destination.description,
            style: MicaTypography.small.copyWith(color: tokens.text.muted),
          ),
          const SizedBox(height: 28),
          destination.content,
        ],
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final compact = MediaQuery.sizeOf(context).width < 700;
    final tokens = MicaTheme.of(context);
    return PopScope(
      canPop: !compact || !_showDetail,
      onPopInvokedWithResult: (didPop, result) {
        if (!didPop && compact && _showDetail) {
          setState(() => _showDetail = false);
        }
      },
      child: Dialog(
        insetPadding: EdgeInsets.all(compact ? 12 : 32),
        backgroundColor: tokens.surface.overlay,
        clipBehavior: Clip.none,
        child: ClipRRect(
          borderRadius: BorderRadius.circular(12),
          child: SizedBox(
            width: 960,
            height: 720,
            child: Column(
              children: [
                Padding(
                  padding: const EdgeInsets.fromLTRB(12, 10, 8, 10),
                  child: Row(
                    children: [
                      if (compact && _showDetail)
                        IconButton(
                          key: const ValueKey('settings-back'),
                          tooltip: widget.backLabel,
                          onPressed: () => setState(() => _showDetail = false),
                          icon: const Icon(Icons.arrow_back, size: 20),
                        )
                      else
                        const Padding(
                          padding: EdgeInsets.symmetric(horizontal: 10),
                          child: Icon(Icons.settings_outlined, size: 20),
                        ),
                      const SizedBox(width: 8),
                      Expanded(
                        child: Text(
                          widget.title,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: MicaTypography.large.copyWith(
                            fontWeight: FontWeight.w600,
                            color: tokens.text.primary,
                          ),
                        ),
                      ),
                      if (widget.busy)
                        const Padding(
                          padding: EdgeInsets.symmetric(horizontal: 8),
                          child: SizedBox(
                            width: 16,
                            height: 16,
                            child: CircularProgressIndicator(strokeWidth: 2),
                          ),
                        ),
                      IconButton(
                        key: const ValueKey('settings-close'),
                        tooltip: widget.closeLabel,
                        onPressed: widget.busy ? null : widget.onClose,
                        icon: const Icon(Icons.close, size: 20),
                      ),
                    ],
                  ),
                ),
                Divider(height: 1, color: tokens.border.subtle),
                Expanded(
                  child: compact
                      ? (_showDetail ? _detail(true) : _navigation(true))
                      : Row(
                          crossAxisAlignment: CrossAxisAlignment.stretch,
                          children: [
                            SizedBox(
                              width: 200,
                              child: Material(
                                color: tokens.surface.raised,
                                child: _navigation(false),
                              ),
                            ),
                            VerticalDivider(
                              width: 1,
                              color: tokens.border.subtle,
                            ),
                            Expanded(child: _detail(false)),
                          ],
                        ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
