import 'package:flutter/material.dart';

import 'theme_tokens.dart';

/// One entry of a workspace, already resolved and already formatted.
///
/// [meta] is the caller's line of secondary text ("12 个页面 · 昨天更新"): every
/// string that needs interpolation is built where the locale lives, never here.
/// [childCount] stays a number so the folder badge does not need a template.
/// [icon] is the user's own emoji — absent means "the user never picked one",
/// which is a fallback glyph, never a guessed emoji.
typedef WorkspaceItem = ({
  String id,
  String? icon,
  String name,
  bool isFolder,
  String meta,
  int childCount,
});

/// How the body of the overview is laid out. The caller owns this value (it is a
/// persisted user preference), so the widget only reports changes — see
/// [WorkspaceOverview.onModeChanged].
enum WorkspaceOverviewMode { cards, list }

/// Every user-visible string of [WorkspaceOverview].
///
/// A plain data class rather than a localization lookup so this widget stays
/// independent of the app's l10n plumbing: whoever builds it already knows the
/// locale, and anything that needs interpolation ("397 个页面") arrives through
/// [WorkspaceItem.meta] already formatted.
@immutable
class WorkspaceOverviewStrings {
  const WorkspaceOverviewStrings({
    required this.sectionLabel,
    required this.cardsModeLabel,
    required this.listModeLabel,
    required this.moreActionsLabel,
    required this.emptyTitle,
    required this.emptyBody,
    this.emptyActionLabel,
  });

  /// Heading above the body ("全部工作区" / "目录").
  final String sectionLabel;

  /// Accessible names for the two icon-only toggle segments.
  final String cardsModeLabel;
  final String listModeLabel;

  /// Accessible name of the per-item overflow affordance.
  final String moreActionsLabel;

  /// Empty state: what happened, then one next step. Never "出错了".
  final String emptyTitle;
  final String emptyBody;

  /// The single accent action of the empty state. Null (or a null
  /// [WorkspaceOverview.onEmptyAction]) means the empty state is copy only.
  final String? emptyActionLabel;
}

/// A workspace's contents, as cards or as a directory list.
///
/// Design: `docs/design/知识库软件开发协作/02 工作区总览.dc.html` (body + view
/// toggle) and `19 空状态与故障态.dc.html` (empty state).
///
/// Deliberately dumb: it holds no data, resolves no ids, formats no strings and
/// keeps no mode of its own. It draws [items] and calls back. That is what lets
/// the same widget serve the workspace root and any folder inside it.
///
/// Widget keys for tests/hosts: `workspaceOverview.mode.cards`,
/// `workspaceOverview.mode.list`, `workspaceOverview.card.<id>`,
/// `workspaceOverview.row.<id>`, `workspaceOverview.more.<id>`,
/// `workspaceOverview.empty`, `workspaceOverview.emptyAction`.
class WorkspaceOverview extends StatelessWidget {
  const WorkspaceOverview({
    required this.items,
    required this.mode,
    required this.onModeChanged,
    required this.onOpen,
    required this.strings,
    this.onMore,
    this.onEmptyAction,
    super.key,
  });

  final List<WorkspaceItem> items;

  /// Current layout. Controlled by the caller — see [WorkspaceOverviewMode].
  final WorkspaceOverviewMode mode;
  final void Function(WorkspaceOverviewMode mode) onModeChanged;

  /// Open a folder or a page. The host decides what "open" means (navigate into
  /// the folder, or load the document).
  final void Function(String id) onOpen;

  /// Per-item overflow (rename / move / trash…). Null → no affordance is drawn,
  /// because an empty menu button is worse than none.
  final void Function(String id)? onMore;

  /// The one next step offered by the empty state.
  final VoidCallback? onEmptyAction;

  final WorkspaceOverviewStrings strings;

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        // Unbounded width (inside a horizontally scrolling parent) has no column
        // count to derive; fall back to the design's content width instead of
        // crashing on infinity.
        final width = constraints.maxWidth.isFinite
            ? constraints.maxWidth
            : _fallbackWidth;
        return Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            _header(context),
            const SizedBox(height: 18),
            if (items.isEmpty)
              _emptyState(context)
            else if (mode == WorkspaceOverviewMode.cards)
              _cardGrid(context, width)
            else
              _directoryList(context, width),
          ],
        );
      },
    );
  }

  Widget _header(BuildContext context) {
    return Row(
      children: [
        Text(
          strings.sectionLabel,
          style: TextStyle(
            fontSize: 13,
            fontWeight: FontWeight.w600,
            color: MicaTheme.of(context).text.muted,
          ),
        ),
        const Spacer(),
        // Stays visible when the workspace is empty: it reflects a stored
        // preference, so hiding it would make the setting look lost.
        _modeToggle(context),
      ],
    );
  }

  Widget _modeToggle(BuildContext context) {
    return Container(
      padding: const EdgeInsets.all(2),
      decoration: BoxDecoration(
        border: Border.all(color: MicaTheme.of(context).border.normal),
        borderRadius: BorderRadius.circular(_controlRadius),
      ),
      child: Row(
        children: [
          _modeSegment(
            context,
            mode: WorkspaceOverviewMode.cards,
            icon: Icons.grid_view_outlined,
            label: strings.cardsModeLabel,
            keyName: 'cards',
          ),
          const SizedBox(width: 2),
          _modeSegment(
            context,
            mode: WorkspaceOverviewMode.list,
            icon: Icons.format_list_bulleted,
            label: strings.listModeLabel,
            keyName: 'list',
          ),
        ],
      ),
    );
  }

  Widget _modeSegment(
    BuildContext context, {
    required WorkspaceOverviewMode mode,
    required IconData icon,
    required String label,
    required String keyName,
  }) {
    final selected = this.mode == mode;
    return Semantics(
      label: label,
      button: true,
      selected: selected,
      child: _Pressable(
        key: ValueKey('workspaceOverview.mode.$keyName'),
        // Fires even when already selected: the host persists the preference,
        // and swallowing the tap here would hide a failed write.
        onTap: () => onModeChanged(mode),
        builder: (hovered) => Container(
          width: 32,
          height: 28,
          decoration: BoxDecoration(
            color: selected
                ? MicaTheme.of(context).accent.wash
                : (hovered ? MicaTheme.of(context).surface.sunken : null),
            borderRadius: BorderRadius.circular(_nestedRadius),
          ),
          child: Icon(
            icon,
            size: 16,
            color: selected
                ? MicaTheme.of(context).accent.primary
                : MicaTheme.of(context).text.faint,
          ),
        ),
      ),
    );
  }

  Widget _cardGrid(BuildContext context, double width) {
    // Columns, not a fixed aspect ratio: a card is icon + name + meta, so its
    // height is content-driven. Thresholds are the widths at which a card would
    // fall under ~215px and start ellipsizing ordinary page names.
    final columns = width >= 720
        ? 3
        : width >= 460
        ? 2
        : 1;
    final cardWidth = ((width - _gridGap * (columns - 1)) / columns)
        .floorToDouble();
    return Wrap(
      spacing: _gridGap,
      runSpacing: _gridGap,
      children: [
        for (final item in items)
          SizedBox(width: cardWidth, child: _card(context, item)),
      ],
    );
  }

  Widget _card(BuildContext context, WorkspaceItem item) {
    return _Pressable(
      key: ValueKey('workspaceOverview.card.${item.id}'),
      onTap: () => onOpen(item.id),
      builder: (hovered) => Container(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
        decoration: BoxDecoration(
          border: Border.all(
            color: hovered
                ? MicaTheme.of(context).accent.primary.withValues(alpha: 0.35)
                : MicaTheme.of(context).border.subtle,
          ),
          borderRadius: BorderRadius.circular(_cardRadius),
        ),
        child: Row(
          children: [
            _itemGlyph(context, item, size: 20),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    item.name,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      fontSize: 14,
                      fontWeight: FontWeight.w600,
                      color: MicaTheme.of(context).text.primary,
                    ),
                  ),
                  if (item.meta.isNotEmpty) ...[
                    const SizedBox(height: 2),
                    Text(
                      item.meta,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontSize: 12,
                        color: MicaTheme.of(context).text.faint,
                      ),
                    ),
                  ],
                ],
              ),
            ),
            // Only folders carry a count — a page has nothing to count.
            if (item.isFolder) ...[
              const SizedBox(width: 8),
              Text(
                '${item.childCount}',
                style: TextStyle(
                  fontSize: 12,
                  color: MicaTheme.of(context).text.faint,
                ),
              ),
            ],
            if (onMore != null) ...[
              const SizedBox(width: 4),
              _moreButton(context, item),
            ],
          ],
        ),
      ),
    );
  }

  Widget _directoryList(BuildContext context, double width) {
    // The name is capped instead of flexed so the leader dashes and the trailing
    // meta keep their space: a long name ellipsizes rather than pushing the
    // right-hand column off the row.
    final nameMaxWidth = (width * 0.55).clamp(80.0, 520.0);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [for (final item in items) _row(context, item, nameMaxWidth)],
    );
  }

  Widget _row(BuildContext context, WorkspaceItem item, double nameMaxWidth) {
    final trailing = item.isFolder ? '${item.childCount}' : item.meta;
    return _Pressable(
      key: ValueKey('workspaceOverview.row.${item.id}'),
      onTap: () => onOpen(item.id),
      builder: (hovered) => Container(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 7),
        decoration: BoxDecoration(
          color: hovered ? MicaTheme.of(context).surface.sunken : null,
          borderRadius: BorderRadius.circular(_nestedRadius),
        ),
        child: Row(
          children: [
            _itemGlyph(context, item, size: 16),
            const SizedBox(width: 8),
            ConstrainedBox(
              constraints: BoxConstraints(maxWidth: nameMaxWidth),
              child: Text(
                item.name,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  fontSize: 13,
                  // Folders read as the heading of their run of pages.
                  fontWeight: item.isFolder ? FontWeight.w600 : FontWeight.w400,
                  color: MicaTheme.of(context).text.primary,
                ),
              ),
            ),
            const Expanded(
              child: Padding(
                padding: EdgeInsets.symmetric(horizontal: 10),
                child: _DashedLeader(),
              ),
            ),
            if (trailing.isNotEmpty)
              Text(
                trailing,
                style: TextStyle(
                  fontSize: 12,
                  color: MicaTheme.of(context).text.faint,
                ),
              ),
            if (onMore != null) ...[
              const SizedBox(width: 4),
              _moreButton(context, item),
            ],
          ],
        ),
      ),
    );
  }

  Widget _moreButton(BuildContext context, WorkspaceItem item) {
    return Semantics(
      label: strings.moreActionsLabel,
      button: true,
      child: _Pressable(
        key: ValueKey('workspaceOverview.more.${item.id}'),
        onTap: () => onMore!(item.id),
        builder: (hovered) => Container(
          width: 24,
          height: 24,
          decoration: BoxDecoration(
            color: hovered ? MicaTheme.of(context).accent.wash : null,
            borderRadius: BorderRadius.circular(_nestedRadius),
          ),
          child: Icon(
            Icons.more_horiz,
            size: 16,
            color: hovered
                ? MicaTheme.of(context).accent.primary
                : MicaTheme.of(context).text.faint,
          ),
        ),
      ),
    );
  }

  /// The user's emoji, or a neutral glyph. Never a substitute emoji: an invented
  /// one reads as a choice the user made.
  Widget _itemGlyph(
    BuildContext context,
    WorkspaceItem item, {
    required double size,
  }) {
    final icon = item.icon;
    if (icon != null && icon.isNotEmpty) {
      return Text(icon, style: TextStyle(fontSize: size));
    }
    return Icon(
      item.isFolder ? Icons.folder_outlined : Icons.description_outlined,
      size: size,
      color: MicaTheme.of(context).text.faint,
    );
  }

  Widget _emptyState(BuildContext context) {
    final actionLabel = strings.emptyActionLabel;
    return SizedBox(
      key: const ValueKey('workspaceOverview.empty'),
      width: double.infinity,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 26, vertical: 40),
        child: Column(
          children: [
            Container(
              width: 46,
              height: 46,
              decoration: BoxDecoration(
                color: MicaTheme.of(context).surface.sunken,
                borderRadius: BorderRadius.circular(_cardRadius),
              ),
              child: Icon(
                Icons.folder_outlined,
                size: 21,
                color: MicaTheme.of(context).text.faint,
              ),
            ),
            const SizedBox(height: 11),
            Text(
              strings.emptyTitle,
              textAlign: TextAlign.center,
              style: TextStyle(
                fontSize: 14,
                fontWeight: FontWeight.w600,
                color: MicaTheme.of(context).text.primary,
              ),
            ),
            const SizedBox(height: 9),
            Text(
              strings.emptyBody,
              textAlign: TextAlign.center,
              style: TextStyle(
                fontSize: 13,
                height: 1.75,
                color: MicaTheme.of(context).text.faint,
              ),
            ),
            // One action at most — an empty state that offers three next steps
            // is a menu, not an answer.
            if (actionLabel != null && onEmptyAction != null) ...[
              const SizedBox(height: 13),
              _Pressable(
                key: const ValueKey('workspaceOverview.emptyAction'),
                onTap: onEmptyAction!,
                builder: (hovered) => Container(
                  padding: const EdgeInsets.symmetric(
                    horizontal: 14,
                    vertical: 7,
                  ),
                  decoration: BoxDecoration(
                    color: MicaTheme.of(context).accent.wash,
                    border: Border.all(
                      color: hovered
                          ? MicaTheme.of(
                              context,
                            ).accent.primary.withValues(alpha: 0.35)
                          : MicaTheme.of(
                              context,
                            ).accent.primary.withValues(alpha: 0.22),
                    ),
                    borderRadius: BorderRadius.circular(_controlRadius),
                  ),
                  child: Text(
                    actionLabel,
                    style: TextStyle(
                      fontSize: 13,
                      fontWeight: FontWeight.w600,
                      color: MicaTheme.of(context).accent.primary,
                    ),
                  ),
                ),
              ),
            ],
          ],
        ),
      ),
    );
  }
}

// The design's two hairlines (#E5E9F0 container, #EEF1F5 inner) and its two
// accent washes used to live here as top-level constants. They are token roles
// now — `border.normal` / `border.subtle` and `accent.wash` — because a constant
// cannot change when the palette does, and these are precisely the surfaces that
// have to invert in dark mode. The greys land within ~3/255 of the originals:
// collapsing near-duplicate hairlines is the point of having roles at all.
//
// The accent HAIRLINE had no exact role, so it is expressed the way it was always
// meant: the accent at low opacity. Over white that is the old #DBEAFE; over a
// dark base it tints instead of glowing.

const double _controlRadius = 8;
const double _cardRadius = 14;

/// A control nested inside a radius-8 track (toggle segments, list rows). Kept
/// smaller so the corners stay concentric with the 2px inset.
const double _nestedRadius = 6;

const double _gridGap = 12;
const double _fallbackWidth = 960;

/// Tap + hover without a Material ancestor.
///
/// The overview is dropped into plain panes as well as Scaffolds, and an InkWell
/// without a Material throws. Hover state is passed to the builder so each
/// surface decides what it means (border tint on a card, wash on a row).
class _Pressable extends StatefulWidget {
  const _Pressable({required this.onTap, required this.builder, super.key});

  final VoidCallback onTap;
  final Widget Function(bool hovered) builder;

  @override
  State<_Pressable> createState() => _PressableState();
}

class _PressableState extends State<_Pressable> {
  bool _hovered = false;

  @override
  Widget build(BuildContext context) {
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: GestureDetector(
        // Opaque so a tap on the padding of a card counts as opening it.
        behavior: HitTestBehavior.opaque,
        onTap: widget.onTap,
        child: widget.builder(_hovered),
      ),
    );
  }
}

/// The dotted leader between a name and its right-hand meta, as in the design's
/// 目录. Painted rather than composed of widgets so the dash count follows the
/// available width for free.
class _DashedLeader extends StatelessWidget {
  const _DashedLeader();

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      height: 1,
      child: CustomPaint(
        painter: _DashedLeaderPainter(MicaTheme.of(context).border.normal),
      ),
    );
  }
}

class _DashedLeaderPainter extends CustomPainter {
  const _DashedLeaderPainter(this.color);

  /// Passed in: a painter has no BuildContext to read tokens from.
  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    const dash = 3.0;
    const gap = 3.0;
    final paint = Paint()
      ..color = color
      ..strokeWidth = 1;
    for (double x = 0; x < size.width; x += dash + gap) {
      final end = (x + dash).clamp(0.0, size.width);
      canvas.drawLine(Offset(x, 0.5), Offset(end, 0.5), paint);
    }
  }

  @override
  bool shouldRepaint(_DashedLeaderPainter oldDelegate) =>
      oldDelegate.color != color;
}
