import 'package:flutter/foundation.dart';

import '../api/models.dart';

typedef PageTreeRow = ({DocumentView view, int depth, bool hasChildren});

/// The sidebar's sorted structure and flattened visible rows. Page edits,
/// sync-status updates and hovering do not change either of these.
class PageTreeCache {
  List<DocumentView> _views = const [];
  Map<String, DocumentView> _byId = const {};
  Map<String?, List<DocumentView>> _children = const {};
  Set<String>? _expanded;
  List<PageTreeRow> _rows = const [];
  Map<String, int> _rowIndices = const {};

  /// Keep an element snapshot rather than the caller's list: a host may mutate
  /// that list in place. DocumentView itself is immutable.
  bool updateViews(List<DocumentView> views) {
    if (listEquals(_views, views)) return false;
    _views = List.of(views);
    _byId = {for (final view in views) view.id: view};
    _children = {};
    for (final view in views) {
      (_children[view.parentViewId] ??= []).add(view);
    }
    for (final children in _children.values) {
      children.sort((left, right) => left.position.compareTo(right.position));
    }
    _expanded = null;
    _rowIndices = const {};
    return true;
  }

  DocumentView? viewById(String id) => _byId[id];
  int? visibleIndexOf(String id) => _rowIndices[id];

  List<PageTreeRow> visibleRows(Set<String> expanded) {
    if (_expanded != null && setEquals(_expanded, expanded)) return _rows;
    _expanded = Set.of(expanded);
    final ordered = <PageTreeRow>[];
    final visited = <String>{};
    void appendChildren(String? parentId, int depth) {
      for (final child in _children[parentId] ?? const <DocumentView>[]) {
        if (!visited.add(child.id)) continue;
        final hasChildren = (_children[child.id] ?? const []).isNotEmpty;
        ordered.add((view: child, depth: depth, hasChildren: hasChildren));
        if (expanded.contains(child.id)) appendChildren(child.id, depth + 1);
      }
    }

    appendChildren(null, 0);
    // Genuine orphans remain visible. Descendants of a collapsed ancestor stay
    // hidden, matching the tree's previous traversal semantics.
    for (final view in _views) {
      if (visited.contains(view.id)) continue;
      final parentId = view.parentViewId;
      if (parentId != null && _byId.containsKey(parentId)) continue;
      if (visited.add(view.id)) {
        ordered.add((
          view: view,
          depth: 0,
          hasChildren: (_children[view.id] ?? const []).isNotEmpty,
        ));
      }
    }
    _rowIndices = {
      for (final (index, row) in ordered.indexed) row.view.id: index,
    };
    return _rows = List.unmodifiable(ordered);
  }
}
