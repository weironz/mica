import 'package:flutter/material.dart';

import '../api/models.dart';
import 'page_tree_cache.dart';
import 'theme_tokens.dart';
import 'typography.dart';

/// Where in a workspace something should go: the workspace's tree, drawn the
/// way the sidebar draws it, with a filter on top (gh#183).
///
/// It was a dropdown of folder names indented with full-width spaces: no
/// pages, no icons, no way to collapse, and finding one folder among dozens
/// meant scrolling. Now it is the sidebar's structure — the same
/// [PageTreeCache], so the order is the sidebar's order by construction, the
/// same folder / page / emoji glyphs — with pages shown for orientation but
/// not pickable (only a folder can hold children; the server refuses a page as
/// a parent). Typing filters by name and keeps each hit's ancestors, expanded,
/// so a match is always seen where it lives.
class DestinationTreePicker extends StatefulWidget {
  const DestinationTreePicker({
    super.key,
    required this.views,
    required this.selectedId,
    required this.onSelected,
    required this.rootLabel,
    required this.filterHint,
    this.enabled = true,
    this.height = 240,
  });

  /// The destination workspace's whole tree (folders and pages).
  final List<DocumentView> views;

  /// The picked folder; null = the workspace root.
  final String? selectedId;
  final ValueChanged<String?> onSelected;
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

  @override
  Widget build(BuildContext context) {
    final tokens = MicaTheme.of(context);
    final rows = _rows();
    final filtering = _filter.text.trim().isNotEmpty;
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
                  selected: widget.selectedId == null,
                  onTap: () => widget.onSelected(null),
                ),
              for (final row in rows) _treeRow(context, row, filtering),
            ],
          ),
        ),
      ],
    );
  }

  Widget _treeRow(BuildContext context, PageTreeRow row, bool filtering) {
    final tokens = MicaTheme.of(context);
    final v = row.view;
    final folder = v.isFolder;
    final open = filtering || _expanded.contains(v.id);
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
      selected: widget.selectedId == v.id,
      // A page cannot hold children: shown for orientation, never picked.
      onTap: folder ? () => widget.onSelected(v.id) : null,
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
  }) {
    final tokens = MicaTheme.of(context);
    final enabled = widget.enabled && onTap != null;
    return Material(
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
              SizedBox(width: 4.0 + depth * 16),
              SizedBox(
                width: 20,
                child: chevron == null
                    ? null
                    : InkWell(
                        onTap: widget.enabled ? chevron : null,
                        child: Icon(
                          expanded ? Icons.expand_more : Icons.chevron_right,
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
                    fontWeight: selected ? FontWeight.w500 : FontWeight.w400,
                  ),
                ),
              ),
              if (selected)
                Icon(Icons.check, size: 16, color: tokens.text.primary),
              const SizedBox(width: 6),
            ],
          ),
        ),
      ),
    );
  }
}
