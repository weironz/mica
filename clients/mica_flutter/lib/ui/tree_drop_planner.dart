import '../api/models.dart';

/// The sidebar drag-drop DECISION logic, extracted verbatim from
/// `_WorkspaceViewState`'s `_dropSlot`/`_isSelfOrDescendant`/`_ancestorAtLevel`
/// family (god-class wrap-up; docs/code-optimization/OPTIMIZATION_PLAN_ROUND3.md
/// §7). Pure tree reasoning over a view list — no widgets, no pointer math
/// (`reparentLevelFor` in main_tree_helpers.dart already owns the pointer-to-
/// level part, and `canNestUnder` the page-vs-folder rule).
///
/// A drop has three modes; only `into` nests, `before`/`after` make the dragged
/// row a SIBLING of the target — which is why the effective parent of a
/// before/after drop is the target's own parent.
class TreeDropPlanner {
  TreeDropPlanner(this.views);

  final List<DocumentView> views;

  /// True when [candidateId] is [rootId] itself or anywhere inside its
  /// subtree: dropping there would create a cycle, so it must be rejected.
  /// The `seen` guard is belt-and-braces (the tree forbids cycles server-side
  /// and by trigger) — but this walk should not be the thing that hangs if
  /// one ever appears.
  bool isSelfOrDescendant(String candidateId, String rootId) {
    if (candidateId == rootId) {
      return true;
    }
    final parents = {for (final v in views) v.id: v.parentViewId};
    String? cursor = candidateId;
    final seen = <String>{};
    while (cursor != null && seen.add(cursor)) {
      final parent = parents[cursor];
      if (parent == rootId) {
        return true;
      }
      cursor = parent;
    }
    return false;
  }

  /// The view a drop would reparent the dragged item under (null = workspace
  /// root): `into` nests under the target; before/after makes it the target's
  /// sibling, i.e. under the target's own parent.
  String? dropParentId(DocumentView target, {required bool into}) =>
      into ? target.id : target.parentViewId;

  bool parentAllowsChildren(String? parentId) => canNestUnder(views, parentId);

  /// Whether a batch of dragged rows may land on [target] in the given mode:
  /// refused when the target sits inside ANY dragged row's subtree (dropping
  /// a batch into one of its own folders is the same impossibility as for one
  /// row, and only needs to be true of one of them), or when the effective
  /// parent cannot hold children.
  bool canDrop(List<DocumentView> dragged, DocumentView target,
      {required bool into}) {
    if (dragged.any((d) => isSelfOrDescendant(target.id, d.id))) return false;
    return parentAllowsChildren(dropParentId(target, into: into));
  }

  /// Resolve a level back to the row the `after` drop is really "after": level
  /// L means "become the next sibling of my ancestor at depth L", which is
  /// precisely what the existing after drop already does — only the target
  /// changes. That is why picking a level needed no new drop machinery.
  /// A missing parent chain link stops at the nearest resolvable ancestor.
  DocumentView ancestorAtLevel(DocumentView row, int depth, int level) {
    final byId = {for (final v in views) v.id: v};
    var cursor = row;
    for (var d = depth; d > level; d--) {
      final parent = byId[cursor.parentViewId];
      if (parent == null) return cursor;
      cursor = parent;
    }
    return cursor;
  }

  /// The target an accepted drop actually acts on. A null or out-of-range
  /// [level] is the row itself; a sideways-picked level resolves to the
  /// ancestor at that depth (see [ancestorAtLevel]).
  DocumentView resolveAfterTarget(
      DocumentView target, int depth, int? level) {
    if (level == null || level >= depth) return target;
    return ancestorAtLevel(target, depth, level);
  }
}
