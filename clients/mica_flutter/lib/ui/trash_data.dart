// Derivations for the recycle bin: what each deleted row actually is, what
// restoring it will bring back, and where it will land.
//
// Pure functions, no widgets — the dialog that shows this is `part of main.dart`
// and therefore untestable, so the thinking lives here where a test can reach it.

import '../api/models.dart';

/// One row of the recycle bin: a deleted subtree ROOT, plus what comes back with
/// it.
///
/// Restore and purge are both subtree-wide on the server, so the unit the user
/// acts on is the root — and [pages]/[folders] are the part the old UI never said
/// out loud. Deleting a folder produced one row wearing a *page* icon, and
/// pressing restore beside it silently brought back everything underneath.
typedef TrashEntry = ({
  DocumentView view,

  /// Pages inside the subtree, NOT counting the root itself.
  int pages,

  /// Folders inside the subtree, NOT counting the root itself.
  int folders,

  /// Where restoring puts it back: the live ancestor chain, `''` at the
  /// workspace root.
  String path,
});

/// Group [deleted] into the rows the bin should show.
///
/// Only subtree ROOTS are returned — a deleted page whose parent folder is also
/// deleted comes back with that parent, so listing it separately would offer a
/// restore that means nothing on its own. Input order is preserved (the server
/// sorts by deletion time, and re-sorting here would quietly override that).
List<TrashEntry> buildTrashEntries({
  required List<DocumentView> deleted,
  required List<DocumentView> live,
}) {
  final deletedIds = {for (final v in deleted) v.id};
  final childrenOf = <String, List<DocumentView>>{};
  for (final v in deleted) {
    final p = v.parentViewId;
    if (p != null && deletedIds.contains(p)) {
      (childrenOf[p] ??= []).add(v);
    }
  }
  final liveById = {for (final v in live) v.id: v};

  final entries = <TrashEntry>[];
  for (final root in deleted) {
    final parent = root.parentViewId;
    final isRoot = parent == null || !deletedIds.contains(parent);
    if (!isRoot) continue;

    var pages = 0;
    var folders = 0;
    // Iterative, and every node marked seen: a parent cycle in the data would
    // otherwise hang the dialog rather than show a wrong number.
    final seen = <String>{root.id};
    final queue = <DocumentView>[...?childrenOf[root.id]];
    while (queue.isNotEmpty) {
      final node = queue.removeLast();
      if (!seen.add(node.id)) continue;
      if (node.isFolder) {
        folders++;
      } else {
        pages++;
      }
      queue.addAll(childrenOf[node.id] ?? const []);
    }

    entries.add((
      view: root,
      pages: pages,
      folders: folders,
      path: _pathOf(root, liveById),
    ));
  }
  return entries;
}

/// The live ancestor chain above [view], outermost first, `''` at the top level.
///
/// Resolved against LIVE views on purpose: a root's parent is by definition not
/// deleted, so this is exactly where a restore will land. An id that resolves to
/// nothing ends the walk rather than guessing — the server re-parents an orphaned
/// root to the top level, and claiming a path we cannot see would be worse than
/// saying nothing.
String _pathOf(DocumentView view, Map<String, DocumentView> liveById) {
  final names = <String>[];
  final seen = <String>{view.id};
  var id = view.parentViewId;
  while (id != null && seen.add(id)) {
    final parent = liveById[id];
    if (parent == null) break;
    names.add(parent.name);
    id = parent.parentViewId;
  }
  return names.reversed.join(' / ');
}
