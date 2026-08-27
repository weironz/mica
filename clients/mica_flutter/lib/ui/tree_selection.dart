/// Multi-select arithmetic for the sidebar page tree.
///
/// Split out of the shell for the same reason `page_tree_state.dart` was: this
/// is the only part of multi-select that can be WRONG rather than merely ugly,
/// and it is one pure function over a list.
///
/// Why a tree needs its own rule at all — none of the references do this.
/// AppFlowy has no sidebar multi-select (open feature request); AFFiNE does it
/// in the flat "All docs" list with checkboxes; Windows Explorer, the analogy
/// this was asked for by, multi-selects only in its flat right pane and NOT in
/// the folder tree on the left. All three avoid the same thing: in a tree,
/// "select from here to there" stops having an obvious meaning once the two ends
/// sit under different parents. Does the range include the folders in between?
/// Their collapsed children?
///
/// The rule chosen here (user, 2026-08-27) sidesteps that instead of answering
/// it: a Shift range is only meaningful WITHIN one parent. Across parents the
/// gesture degrades to a plain toggle, which is always unambiguous.
library;

/// One row of the visible tree, reduced to what the selection rule needs.
///
/// Deliberately not `DocumentView`: the rule is about position and parentage, so
/// taking the whole model would drag names, icons and positions into a test that
/// has nothing to say about them.
typedef SelectableRow = ({String id, String? parentId});

/// The ids a Shift-click should select, walking from [anchorId] to [targetId].
///
/// [visibleRows] must be the tree AS DISPLAYED, in order — a collapsed folder's
/// children are simply absent, which is what makes "visible" the right basis:
/// selecting rows you cannot see is how a bulk delete surprises someone.
///
/// Returns `null` when the gesture has no unambiguous meaning, and the caller
/// must then treat the click as a plain toggle:
///   * either end is not on screen (scrolled tree, stale anchor), or
///   * the two ends live under different parents.
///
/// Rows BETWEEN the ends that belong to a deeper level are skipped rather than
/// refused. That case is an expanded folder sitting inside the range, and it is
/// ordinary: "select these five siblings" should not fail because the third one
/// happens to be open. Its children are not added either — a selected folder
/// already carries its whole subtree everywhere this selection gets used (trash,
/// transfer), so adding them would double-count, not widen.
List<String>? shiftRangeSelection({
  required List<SelectableRow> visibleRows,
  required String anchorId,
  required String targetId,
}) {
  final anchor = visibleRows.indexWhere((r) => r.id == anchorId);
  final target = visibleRows.indexWhere((r) => r.id == targetId);
  if (anchor < 0 || target < 0) return null;

  final parentId = visibleRows[anchor].parentId;
  if (visibleRows[target].parentId != parentId) return null;

  final from = anchor <= target ? anchor : target;
  final to = anchor <= target ? target : anchor;
  return [
    for (var i = from; i <= to; i++)
      if (visibleRows[i].parentId == parentId) visibleRows[i].id,
  ];
}

/// The selection after a Ctrl-click on [id]: in if it was out, out if it was in.
Set<String> selectionAfterToggle(Set<String> current, String id) {
  final next = {...current};
  if (!next.add(id)) next.remove(id);
  return next;
}

/// One row of a tree, reduced to what [flattenedFolderOptions] needs.
typedef FolderCandidate = ({
  String id,
  String? parentId,
  String name,
  String position,
  bool isFolder,
});

/// The FOLDERS of a workspace, depth-first in display order, each with its
/// nesting depth — the options for "where should this land".
///
/// Pages are excluded because the tree's central invariant is that only a folder
/// may hold children: offering a page as a destination produces a dropdown whose
/// entries the server is guaranteed to refuse.
///
/// Children are ordered by `position`, the same key the sidebar sorts by, so the
/// dropdown lists folders in the order the user is looking at them.
///
/// A folder whose parent is missing from [views] — a partially-loaded tree — is
/// dropped rather than promoted to the root. Promoting it would offer a folder
/// at a depth it does not have, and the indentation is the only thing telling
/// two same-named folders apart.
List<({String id, String name, int depth})> flattenedFolderOptions(
  Iterable<FolderCandidate> views,
) {
  final byParent = <String?, List<FolderCandidate>>{};
  for (final v in views) {
    if (!v.isFolder) continue;
    byParent.putIfAbsent(v.parentId, () => []).add(v);
  }
  for (final children in byParent.values) {
    children.sort((a, b) => a.position.compareTo(b.position));
  }
  final out = <({String id, String name, int depth})>[];
  final seen = <String>{};
  void walk(String? parentId, int depth) {
    for (final f in byParent[parentId] ?? const <FolderCandidate>[]) {
      // The walk descends from the root, so a parent cycle is unreachable and
      // needs no guard. This one is for a DUPLICATED row in the response, which
      // would otherwise list the same folder twice — and its whole subtree
      // under each copy.
      if (!seen.add(f.id)) continue;
      out.add((id: f.id, name: f.name, depth: depth));
      walk(f.id, depth + 1);
    }
  }

  walk(null, 0);
  return out;
}

/// What a click on a tree row means, once its modifier keys are read.
enum TreeClickIntent {
  /// No modifier: open the page (or expand the folder), and drop any selection.
  open,

  /// Ctrl/Cmd: add this row to the selection, or take it back out.
  toggleSelection,

  /// Shift: select from the anchor to here — degrading to [toggleSelection] when
  /// the range has no unambiguous meaning (see [shiftRangeSelection]).
  extendSelection,
}

/// Read a click's modifiers into an intent.
///
/// Shift wins over Ctrl, which is what Explorer and Finder both do: Ctrl+Shift
/// is "extend, keeping what I had", and the extend half is the part that decides
/// which rows are involved.
///
/// Its own function because the alternative is this rule spelled out inline in a
/// gesture callback, where "which branch runs" is only observable by clicking.
TreeClickIntent treeClickIntent({required bool ctrl, required bool shift}) {
  if (shift) return TreeClickIntent.extendSelection;
  if (ctrl) return TreeClickIntent.toggleSelection;
  return TreeClickIntent.open;
}

/// Whether a gesture on [id] should act on the whole selection.
///
/// Acting on a row that is NOT in the selection means "I want this one" — every
/// file manager treats a right-click that way, and acting on a selection the
/// user has scrolled away from and forgotten is how a context menu deletes the
/// wrong thing. Dragging follows the identical rule, so it is the identical
/// function: grabbing a selected row carries the selection, grabbing any other
/// row carries that row and quietly abandons the selection.
bool actsOnSelection(Set<String> selection, String id) =>
    selection.length > 1 && selection.contains(id);

/// The selected ids with anything nested inside another selected row dropped.
///
/// The same rule the server applies to a batch transfer ([independent_roots] in
/// `documents.rs`), needed here too and for a DIFFERENT failure. Select a folder
/// and a page inside it, then drag both into somewhere else: a reorder sets the
/// parent of every id it is given, so the page would be re-parented alongside
/// its own folder — dragged OUT of the folder the user just moved it with.
/// Nobody asked for that, and it looks like the tree silently rearranged itself.
///
/// [parentOf] maps an id to its parent (null at the workspace root); ids it does
/// not know are treated as roots.
///
/// Order is preserved, because for the drag path it is the order the rows land
/// in at the destination.
List<String> independentSelection(
  Iterable<String> selected,
  String? Function(String id) parentOf,
) {
  final wanted = selected.toSet();
  final kept = <String>[];
  final seen = <String>{};
  for (final id in selected) {
    if (!seen.add(id)) continue;
    var cursor = parentOf(id);
    // Seeded with the row itself: a row is never "inside another selected row"
    // by virtue of being itself. That also closes a parent cycle — the tree
    // cannot legally have one, but a stale client snapshot mid-move can, and a
    // spin in a drag handler freezes the app rather than misplacing a row.
    // Without the seed the walk came back round to `id`, found it in the
    // selection, and silently dropped the very row being dragged.
    final walked = <String>{id};
    var nested = false;
    while (cursor != null && walked.add(cursor)) {
      if (wanted.contains(cursor)) {
        nested = true;
        break;
      }
      cursor = parentOf(cursor);
    }
    if (!nested) kept.add(id);
  }
  return kept;
}

/// The order [siblings] should end up in when [dragged] is dropped next to
/// [targetId] — the whole block moving together, in its own order.
///
/// [siblings] is the destination parent's children in display order, [dragged]
/// the rows being moved (already reduced by [independentSelection]). Dragged
/// rows are removed from wherever they were before the insertion point is read,
/// which is the part that has to be done in that order: taking the index first
/// and removing after puts the block one slot off whenever a dragged row sat
/// above the target.
///
/// Returns null when the target is not among the siblings — a tree that changed
/// under the drag. The caller does nothing, rather than guessing an index.
List<String>? siblingOrderAfterDrop({
  required List<String> siblings,
  required List<String> dragged,
  required String targetId,
  required bool before,
}) {
  final moving = dragged.toSet();
  // Dropping onto a row that is itself being dragged has no meaning — there is
  // no "before the thing that is moving with me".
  if (moving.contains(targetId)) return null;
  final rest = siblings.where((id) => !moving.contains(id)).toList();
  final at = rest.indexOf(targetId);
  if (at < 0) return null;
  return [...rest.take(before ? at : at + 1), ...dragged, ...rest.skip(before ? at : at + 1)];
}
