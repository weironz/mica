import 'package:flutter/material.dart';

import '../api/models.dart';
import '../l10n/locale_controller.dart';
import 'page_tree_cache.dart';
import 'theme_tokens.dart';
import 'typography.dart';

/// Chosen destination inside a workspace: either inside a folder / root, or
/// immediately before / after a specific sibling view (gh#186).
class DestinationPick {
  const DestinationPick({
    this.parentViewId,
    this.beforeViewId,
    this.afterViewId,
  });

  static const root = DestinationPick();

  /// Target parent folder; null = workspace root.
  final String? parentViewId;

  /// Optional sibling anchor under [parentViewId]: insert immediately BEFORE.
  final String? beforeViewId;

  /// Optional sibling anchor under [parentViewId]: insert immediately AFTER.
  final String? afterViewId;

  @override
  bool operator ==(Object other) =>
      other is DestinationPick &&
      other.parentViewId == parentViewId &&
      other.beforeViewId == beforeViewId &&
      other.afterViewId == afterViewId;

  @override
  int get hashCode => Object.hash(parentViewId, beforeViewId, afterViewId);
}

/// Where in a workspace something should go: the workspace's tree, drawn the
/// way the sidebar draws it, with a filter on top (gh#183, gh#186).
///
/// Clicking a folder name selects it and automatically expands its children
/// (clicking the chevron still toggles collapse/expand). Clicking a page
/// targets inserting right after that page, and the compact placement bar below
/// the tree lets the user switch between inside / before / after (or top / end
/// of the workspace root).
class DestinationTreePicker extends StatefulWidget {
  const DestinationTreePicker({
    super.key,
    required this.views,
    required this.selection,
    required this.onSelected,
    required this.rootLabel,
    required this.filterHint,
    this.enabled = true,
    this.height = 240,
  });

  /// The destination workspace's whole tree (folders and pages).
  final List<DocumentView> views;

  /// Current destination pick (folder/root or sibling anchor).
  final DestinationPick selection;
  final ValueChanged<DestinationPick> onSelected;
  final String rootLabel;
  final String filterHint;
  final bool enabled;
  final double height;

  @override
  State<DestinationTreePicker> createState() => _DestinationTreePickerState();
}

class _DestinationTreePickerState extends State<DestinationTreePicker> {
  final PageTreeCache _cache = PageTreeCache();
  final TextEditingController _filter = TextEditingController();
  final Set<String> _expanded = {};

  @override
  void dispose() {
    _filter.dispose();
    super.dispose();
  }

  /// Rows to draw. Unfiltered: the sidebar's visible rows for the folders the
  /// user expanded. Filtered: every name match plus its ancestors, with those
  /// ancestors expanded — whatever the user had collapsed.
  List<PageTreeRow> _rows() {
    _cache.updateViews(widget.views);
    final q = _filter.text.trim().toLowerCase();
    if (q.isEmpty) return _cache.visibleRows(_expanded);
    final keep = <String>{};
    final open = <String>{};
    for (final v in widget.views) {
      if (!v.name.toLowerCase().contains(q)) continue;
      keep.add(v.id);
      var parent = v.parentViewId;
      final seen = <String>{};
      while (parent != null && seen.add(parent)) {
        keep.add(parent);
        open.add(parent);
        parent = _cache.viewById(parent)?.parentViewId;
      }
    }
    return [
      for (final row in _cache.visibleRows(open))
        if (keep.contains(row.view.id)) row,
    ];
  }

  DocumentView? _firstRootView() {
    final top = [
      for (final v in widget.views)
        if (v.parentViewId == null) v,
    ]..sort((a, b) => a.position.compareTo(b.position));
    return top.firstOrNull;
  }

  @override
  Widget build(BuildContext context) {
    final tokens = MicaTheme.of(context);
    final rows = _rows();
    final filtering = _filter.text.trim().isNotEmpty;
    final sel = widget.selection;
    final isRootEnd =
        sel.parentViewId == null &&
        sel.beforeViewId == null &&
        sel.afterViewId == null;
    final placementBar = _placementBar(context);

    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        TextField(
          key: const ValueKey('destination-tree-filter'),
          controller: _filter,
          enabled: widget.enabled,
          style: MicaTypography.small,
          decoration: InputDecoration(
            isDense: true,
            hintText: widget.filterHint,
            prefixIcon: Icon(Icons.search, size: 16, color: tokens.text.faint),
            prefixIconConstraints: const BoxConstraints(minWidth: 32),
            border: const OutlineInputBorder(),
            contentPadding: const EdgeInsets.symmetric(
              horizontal: 8,
              vertical: 8,
            ),
          ),
          onChanged: (_) => setState(() {}),
        ),
        const SizedBox(height: 6),
        Container(
          height: widget.height,
          decoration: BoxDecoration(
            border: Border.all(color: tokens.border.normal),
            borderRadius: BorderRadius.circular(6),
          ),
          child: ListView(
            padding: const EdgeInsets.all(4),
            children: [
              if (!filtering)
                _row(
                  context,
                  key: const ValueKey('destination-root'),
                  depth: 0,
                  icon: Icon(
                    Icons.workspaces_outline,
                    size: 18,
                    color: tokens.text.muted,
                  ),
                  label: widget.rootLabel,
                  selected: isRootEnd,
                  onTap: () => widget.onSelected(DestinationPick.root),
                ),
              for (final row in rows) _treeRow(context, row, filtering),
            ],
          ),
        ),
        if (placementBar != null) ...[
          const SizedBox(height: 8),
          placementBar,
        ],
      ],
    );
  }

  Widget? _placementBar(BuildContext context) {
    final l10n = context.l10n;
    final sel = widget.selection;
    final anchorId = sel.beforeViewId ?? sel.afterViewId;
    if (anchorId != null) {
      final v = _cache.viewById(anchorId);
      if (v == null) return null;
      return Wrap(
        spacing: 6,
        runSpacing: 6,
        children: [
          if (v.isFolder)
            _modeChip(
              context,
              key: const ValueKey('placement-inside'),
              label: l10n.transferInsertInside,
              selected: false,
              onTap: () => widget.onSelected(DestinationPick(parentViewId: v.id)),
            ),
          _modeChip(
            context,
            key: const ValueKey('placement-before'),
            label: l10n.transferInsertBefore,
            selected: sel.beforeViewId == v.id,
            onTap: () => widget.onSelected(
              DestinationPick(
                parentViewId: v.parentViewId,
                beforeViewId: v.id,
              ),
            ),
          ),
          _modeChip(
            context,
            key: const ValueKey('placement-after'),
            label: l10n.transferInsertAfter,
            selected: sel.afterViewId == v.id,
            onTap: () => widget.onSelected(
              DestinationPick(
                parentViewId: v.parentViewId,
                afterViewId: v.id,
              ),
            ),
          ),
        ],
      );
    }
    if (sel.parentViewId != null) {
      final folder = _cache.viewById(sel.parentViewId!);
      if (folder == null) return null;
      return Wrap(
        spacing: 6,
        runSpacing: 6,
        children: [
          _modeChip(
            context,
            key: const ValueKey('placement-inside'),
            label: l10n.transferInsertInside,
            selected: true,
            onTap: () =>
                widget.onSelected(DestinationPick(parentViewId: folder.id)),
          ),
          _modeChip(
            context,
            key: const ValueKey('placement-before'),
            label: l10n.transferInsertBefore,
            selected: false,
            onTap: () => widget.onSelected(
              DestinationPick(
                parentViewId: folder.parentViewId,
                beforeViewId: folder.id,
              ),
            ),
          ),
          _modeChip(
            context,
            key: const ValueKey('placement-after'),
            label: l10n.transferInsertAfter,
            selected: false,
            onTap: () => widget.onSelected(
              DestinationPick(
                parentViewId: folder.parentViewId,
                afterViewId: folder.id,
              ),
            ),
          ),
        ],
      );
    }
    final firstRoot = _firstRootView();
    if (firstRoot == null) return null;
    return Wrap(
      spacing: 6,
      runSpacing: 6,
      children: [
        _modeChip(
          context,
          key: const ValueKey('placement-root-end'),
          label: l10n.transferRootEnd,
          selected: true,
          onTap: () => widget.onSelected(DestinationPick.root),
        ),
        _modeChip(
          context,
          key: const ValueKey('placement-root-top'),
          label: l10n.transferRootTop,
          selected: false,
          onTap: () => widget.onSelected(
            DestinationPick(beforeViewId: firstRoot.id),
          ),
        ),
      ],
    );
  }

  Widget _modeChip(
    BuildContext context, {
    required Key key,
    required String label,
    required bool selected,
    required VoidCallback onTap,
  }) {
    return ChoiceChip(
      key: key,
      label: Text(label, style: MicaTypography.small),
      selected: selected,
      onSelected: widget.enabled ? (_) => onTap() : null,
      visualDensity: VisualDensity.compact,
      materialTapTargetSize: MaterialTapTargetSize.shrinkWrap,
    );
  }

  Widget _treeRow(BuildContext context, PageTreeRow row, bool filtering) {
    final tokens = MicaTheme.of(context);
    final v = row.view;
    final folder = v.isFolder;
    final open = filtering || _expanded.contains(v.id);
    final sel = widget.selection;
    final isInside =
        folder &&
        sel.parentViewId == v.id &&
        sel.beforeViewId == null &&
        sel.afterViewId == null;
    final isBefore = sel.beforeViewId == v.id;
    final isAfter = sel.afterViewId == v.id;
    final Widget glyph = v.icon != null
        ? SizedBox(
            width: 18,
            height: 18,
            child: Center(
              child: Text(
                v.icon!,
                style: const TextStyle(fontSize: 16, height: 18 / 16),
              ),
            ),
          )
        : Icon(
            folder
                ? (open ? Icons.folder_open_outlined : Icons.folder_outlined)
                : Icons.description_outlined,
            size: 18,
            color: tokens.text.muted,
          );
    return _row(
      context,
      key: ValueKey('destination-${v.id}'),
      depth: row.depth + 1,
      chevron: folder && row.hasChildren && !filtering
          ? () => setState(() {
              if (!_expanded.remove(v.id)) _expanded.add(v.id);
            })
          : null,
      expanded: open,
      icon: glyph,
      label: v.name,
      selected: isInside || isBefore || isAfter,
      showCheck: isInside,
      insertBefore: isBefore,
      insertAfter: isAfter,
      onTap: folder
          ? () {
              // gh#186: clicking a folder name selects it and automatically
              // expands its subtree so nested folders/pages are revealed.
              if (row.hasChildren && !filtering && !_expanded.contains(v.id)) {
                setState(() => _expanded.add(v.id));
              }
              widget.onSelected(DestinationPick(parentViewId: v.id));
            }
          : () => widget.onSelected(
              DestinationPick(
                parentViewId: v.parentViewId,
                afterViewId: v.id,
              ),
            ),
    );
  }

  Widget _row(
    BuildContext context, {
    required Key key,
    required int depth,
    required Widget icon,
    required String label,
    required bool selected,
    required VoidCallback? onTap,
    VoidCallback? chevron,
    bool expanded = false,
    bool showCheck = true,
    bool insertBefore = false,
    bool insertAfter = false,
  }) {
    final tokens = MicaTheme.of(context);
    final enabled = widget.enabled && onTap != null;
    final indent = 4.0 + depth * 16;
    final accent = tokens.accent.primary;
    Widget indicator() => Padding(
      padding: EdgeInsets.only(left: indent, right: 6),
      child: Container(
        height: 2,
        decoration: BoxDecoration(
          color: accent,
          borderRadius: BorderRadius.circular(1),
        ),
      ),
    );
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (insertBefore) indicator(),
        Material(
          key: key,
          color: selected ? tokens.surface.hover : Colors.transparent,
          borderRadius: BorderRadius.circular(6),
          child: InkWell(
            borderRadius: BorderRadius.circular(6),
            onTap: enabled ? onTap : null,
            child: SizedBox(
              height: 30,
              child: Row(
                children: [
                  SizedBox(width: indent),
                  SizedBox(
                    width: 20,
                    child: chevron == null
                        ? null
                        : InkWell(
                            onTap: widget.enabled ? chevron : null,
                            child: Icon(
                              expanded
                                  ? Icons.expand_more
                                  : Icons.chevron_right,
                              size: 16,
                              color: tokens.text.faint,
                            ),
                          ),
                  ),
                  icon,
                  const SizedBox(width: 6),
                  Expanded(
                    child: Text(
                      label,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: MicaTypography.body.copyWith(
                        color: onTap == null
                            ? tokens.text.faint
                            : tokens.text.primary,
                        fontWeight: selected
                            ? FontWeight.w500
                            : FontWeight.w400,
                      ),
                    ),
                  ),
                  if (selected && showCheck)
                    Icon(Icons.check, size: 16, color: tokens.text.primary),
                  if (insertBefore || insertAfter)
                    Icon(
                      insertBefore
                          ? Icons.vertical_align_top
                          : Icons.vertical_align_bottom,
                      size: 15,
                      color: accent,
                    ),
                  const SizedBox(width: 6),
                ],
              ),
            ),
          ),
        ),
        if (insertAfter) indicator(),
      ],
    );
  }
}
