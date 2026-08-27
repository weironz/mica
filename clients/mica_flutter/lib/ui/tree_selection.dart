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

/// Whether a right-click on [id] should act on the whole selection.
///
/// Right-clicking a row that is NOT in the selection means "I want this one" —
/// every file manager treats it that way, and acting on a selection the user has
/// scrolled away from and forgotten is how a context menu deletes the wrong
/// thing.
bool rightClickActsOnSelection(Set<String> selection, String id) =>
    selection.length > 1 && selection.contains(id);
