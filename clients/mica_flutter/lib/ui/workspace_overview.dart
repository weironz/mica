import 'package:flutter/material.dart';

import '../editor/render.dart' show EditorTheme;

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
            _header(),
            const SizedBox(height: 18),
            if (items.isEmpty)
              _emptyState()
            else if (mode == WorkspaceOverviewMode.cards)
              _cardGrid(width)
            else
              _directoryList(width),
          ],
        );
      },
    );
  }

  Widget _header() {
    return Row(
      children: [
        Text(
          strings.sectionLabel,
          style: const TextStyle(
            fontSize: 13,
            fontWeight: FontWeight.w600,
            color: EditorTheme.muted,
          ),
        ),
        const Spacer(),
        // Stays visible when the workspace is empty: it reflects a stored
        // preference, so hiding it would make the setting look lost.
        _modeToggle(),
      ],
    );
  }

  Widget _modeToggle() {
    return Container(
      padding: const EdgeInsets.all(2),
      decoration: BoxDecoration(
        border: Border.all(color: _containerBorder),
        borderRadius: BorderRadius.circular(_controlRadius),
      ),
      child: Row(
        children: [
          _modeSegment(
            mode: WorkspaceOverviewMode.cards,
            icon: Icons.grid_view_outlined,
            label: strings.cardsModeLabel,
            keyName: 'cards',
          ),
          const SizedBox(width: 2),
          _modeSegment(
            mode: WorkspaceOverviewMode.list,
            icon: Icons.format_list_bulleted,
            label: strings.listModeLabel,
            keyName: 'list',
          ),
        ],
      ),
    );
  }

  Widget _modeSegment({
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
                ? _accentWash
                : (hovered ? EditorTheme.codeBg : null),
            borderRadius: BorderRadius.circular(_nestedRadius),
          ),
          child: Icon(
            icon,
            size: 16,
            color: selected ? EditorTheme.caret : EditorTheme.faint,
          ),
        ),
      ),
    );
  }

  Widget _cardGrid(double width) {
    // Columns, not a fixed aspect ratio: a card is icon + name + meta, so its
    // height is content-driven. Thresholds are the widths at which a card would
    // fall under ~215px and start ellipsizing ordinary page names.
    final columns = width >= 720
        ? 3
        : width >= 460
        ? 2
        : 1;
    final cardWidth =
        ((width - _gridGap * (columns - 1)) / columns).floorToDouble();
    return Wrap(
      spacing: _gridGap,
      runSpacing: _gridGap,
      children: [
        for (final item in items)
          SizedBox(width: cardWidth, child: _card(item)),
      ],
    );
  }

  Widget _card(WorkspaceItem item) {
    return _Pressable(
      key: ValueKey('workspaceOverview.card.${item.id}'),
      onTap: () => onOpen(item.id),
      builder: (hovered) => Container(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
        decoration: BoxDecoration(
          border: Border.all(color: hovered ? _accentBorder : _innerBorder),
          borderRadius: BorderRadius.circular(_cardRadius),
        ),
        child: Row(
          children: [
            _itemGlyph(item, size: 20),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    item.name,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(
                      fontSize: 14,
                      fontWeight: FontWeight.w600,
                      color: EditorTheme.text,
                    ),
                  ),
                  if (item.meta.isNotEmpty) ...[
                    const SizedBox(height: 2),
                    Text(
                      item.meta,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: const TextStyle(
                        fontSize: 12,
                        color: EditorTheme.faint,
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
                style: const TextStyle(fontSize: 12, color: EditorTheme.faint),
              ),
            ],
            if (onMore != null) ...[
              const SizedBox(width: 4),
              _moreButton(item),
            ],
          ],
        ),
      ),
    );
  }

  Widget _directoryList(double width) {
    // The name is capped instead of flexed so the leader dashes and the trailing
    // meta keep their space: a long name ellipsizes rather than pushing the
    // right-hand column off the row.
    final nameMaxWidth = (width * 0.55).clamp(80.0, 520.0);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [for (final item in items) _row(item, nameMaxWidth)],
    );
  }

  Widget _row(WorkspaceItem item, double nameMaxWidth) {
    final trailing = item.isFolder ? '${item.childCount}' : item.meta;
    return _Pressable(
      key: ValueKey('workspaceOverview.row.${item.id}'),
      onTap: () => onOpen(item.id),
      builder: (hovered) => Container(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 7),
        decoration: BoxDecoration(
          color: hovered ? EditorTheme.codeBg : null,
          borderRadius: BorderRadius.circular(_nestedRadius),
        ),
        child: Row(
          children: [
            _itemGlyph(item, size: 16),
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
                  fontWeight: item.isFolder
                      ? FontWeight.w600
                      : FontWeight.w400,
                  color: EditorTheme.text,
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
                style: const TextStyle(fontSize: 12, color: EditorTheme.faint),
              ),
            if (onMore != null) ...[
              const SizedBox(width: 4),
              _moreButton(item),
            ],
          ],
        ),
      ),
    );
  }

  Widget _moreButton(WorkspaceItem item) {
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
            color: hovered ? _accentWash : null,
            borderRadius: BorderRadius.circular(_nestedRadius),
          ),
          child: Icon(
            Icons.more_horiz,
            size: 16,
            color: hovered ? EditorTheme.caret : EditorTheme.faint,
          ),
        ),
      ),
    );
  }

  /// The user's emoji, or a neutral glyph. Never a substitute emoji: an invented
  /// one reads as a choice the user made.
  Widget _itemGlyph(WorkspaceItem item, {required double size}) {
    final icon = item.icon;
    if (icon != null && icon.isNotEmpty) {
      return Text(icon, style: TextStyle(fontSize: size));
    }
    return Icon(
      item.isFolder ? Icons.folder_outlined : Icons.description_outlined,
      size: size,
      color: EditorTheme.faint,
    );
  }

  Widget _emptyState() {
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
                color: EditorTheme.codeBg,
                borderRadius: BorderRadius.circular(_cardRadius),
              ),
              child: const Icon(
                Icons.folder_outlined,
                size: 21,
                color: EditorTheme.faint,
              ),
            ),
            const SizedBox(height: 11),
            Text(
              strings.emptyTitle,
              textAlign: TextAlign.center,
              style: const TextStyle(
                fontSize: 14,
                fontWeight: FontWeight.w600,
                color: EditorTheme.text,
              ),
            ),
            const SizedBox(height: 9),
            Text(
              strings.emptyBody,
              textAlign: TextAlign.center,
              style: const TextStyle(
                fontSize: 13,
                height: 1.75,
                color: EditorTheme.faint,
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
                    color: _accentWash,
                    border: Border.all(
                      color: hovered ? _accentBorder : _accentHairline,
                    ),
                    borderRadius: BorderRadius.circular(_controlRadius),
                  ),
                  child: Text(
                    actionLabel,
                    style: const TextStyle(
                      fontSize: 13,
                      fontWeight: FontWeight.w600,
                      color: EditorTheme.caret,
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

/// Container hairline (#E5E9F0) and inner hairline (#EEF1F5) from the design
/// system. EditorTheme has no border tokens — it is a text/ink palette.
const Color _containerBorder = Color(0xFFE5E9F0);
const Color _innerBorder = Color(0xFFEEF1F5);

/// Accent washes, not new greys: the two tinted surfaces in the mockups are the
/// selected toggle segment and the empty-state action. Both derive from
/// [EditorTheme.caret] (#2563EB) rather than introducing another hue.
const Color _accentWash = Color(0xFFEFF6FF);
const Color _accentHairline = Color(0xFFDBEAFE);
final Color _accentBorder = EditorTheme.caret.withValues(alpha: 0.35);

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
    return const SizedBox(
      height: 1,
      child: CustomPaint(painter: _DashedLeaderPainter()),
    );
  }
}

class _DashedLeaderPainter extends CustomPainter {
  const _DashedLeaderPainter();

  @override
  void paint(Canvas canvas, Size size) {
    const dash = 3.0;
    const gap = 3.0;
    final paint = Paint()
      ..color = _containerBorder
      ..strokeWidth = 1;
    for (double x = 0; x < size.width; x += dash + gap) {
      final end = (x + dash).clamp(0.0, size.width);
      canvas.drawLine(Offset(x, 0.5), Offset(end, 0.5), paint);
    }
  }

  @override
  bool shouldRepaint(_DashedLeaderPainter oldDelegate) => false;
}
