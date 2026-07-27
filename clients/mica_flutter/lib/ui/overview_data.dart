// Workspace-overview data derivation — pure functions, no widgets, no services.
//
// Same split as home_data.dart, and a standalone library for the same reason:
// main.dart should hold wiring, not logic.
//
// The overview answers "what is inside this container?" — a folder, or the
// workspace root. It is the destination for tapping a FOLDER (a folder is not a
// document, so opening it as one does nothing), which is exactly the gap the home
// screen's directory list would otherwise walk into.

import '../api/models.dart';
import 'workspace_overview.dart' show WorkspaceItem;

/// The direct children of [parentViewId] (null = the workspace root), folders
/// first then pages, alphabetical within each group.
///
/// Folders lead because they are containers: mixing them into one alphabetical
/// run makes a nested structure read as a flat pile. [formatPageMeta] and
/// [formatFolderMeta] arrive already localized — this file owns no copy and no
/// plural rules.
List<WorkspaceItem> buildOverviewItems({
  required List<DocumentView> views,
  required String? parentViewId,
  required String Function(DocumentView view) formatPageMeta,
  required String Function(int childCount) formatFolderMeta,
}) {
  final children = views.where((v) => v.parentViewId == parentViewId).toList();
  children.sort((a, b) {
    if (a.isFolder != b.isFolder) return a.isFolder ? -1 : 1;
    return a.name.compareTo(b.name);
  });
  return [
    for (final view in children)
      (
        id: view.id,
        icon: view.icon,
        name: view.name,
        isFolder: view.isFolder,
        meta: view.isFolder
            ? formatFolderMeta(_childCount(views, view.id))
            : formatPageMeta(view),
        childCount: view.isFolder ? _childCount(views, view.id) : 0,
      ),
  ];
}

int _childCount(List<DocumentView> views, String folderId) =>
    views.where((v) => v.parentViewId == folderId).length;

/// The breadcrumb from the workspace root down to [viewId], root-first.
///
/// Walks parent links with a visited set: a corrupted tree with a parent cycle
/// (the database has a trigger against it, but a stale client cache is not the
/// database) must not hang the UI — it stops instead of looping forever.
List<DocumentView> overviewTrail(List<DocumentView> views, String? viewId) {
  if (viewId == null) return const [];
  final byId = {for (final v in views) v.id: v};
  final trail = <DocumentView>[];
  final seen = <String>{};
  var cursor = viewId;
  while (seen.add(cursor)) {
    final view = byId[cursor];
    if (view == null) break;
    trail.insert(0, view);
    final parent = view.parentViewId;
    if (parent == null) break;
    cursor = parent;
  }
  return trail;
}
