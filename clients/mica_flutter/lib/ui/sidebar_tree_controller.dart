import 'package:flutter/foundation.dart';

import '../api/models.dart';

/// The sidebar tree's visibility/focus state, extracted verbatim from
/// `_WorkspaceViewState` (the second god-class step after the import pilot —
/// see docs/code-optimization/OPTIMIZATION_PLAN_ROUND3.md §7). Owns WHICH rows
/// are expanded, WHICH row is "located" (focused), and the pending-reveal
/// bookkeeping for a row that has not arrived yet. Multi-selection already
/// lives in `ui/tree_selection.dart` and pagination in `PageTreeCache`; this
/// is the last piece of sidebar-tree state that was still State fields.
///
/// The widget keeps its `_expandedViewIds` / `_focusedNavId` / `_rootFocused`
/// fields as view-model copies synced from a listener, so build helpers read
/// them unchanged; every mutation goes through here.
class SidebarTreeController extends ChangeNotifier {
  final Set<String> _expandedIds = {};

  /// The row a keyboard gesture acts on — the last row the user located. A
  /// folder becomes this via its toggle; a page via any tap on its row.
  String? focusedNavId;

  /// Focus sits on the tree root (the drop-to-root target / "new page"
  /// default), not on any row.
  bool rootFocused = false;

  /// A reveal asked for a row this workspace's tree does not have YET — a
  /// cross-workspace search hit arrives before that workspace's views do.
  /// Re-checked as views arrive; see [bumpRevealAttempt] for the timeout.
  String? pendingRevealId;

  /// Frames spent waiting for the pending row, so a reveal that can never land
  /// (id vanished, tree never loads) stops scheduling callbacks.
  int revealAttempts = 0;

  /// The expanded ids, read by the tree's row flattening. Read-only by
  /// convention: mutate through this controller only.
  Set<String> get expandedIds => _expandedIds;

  bool isExpanded(String id) => _expandedIds.contains(id);

  /// Tapping a folder both toggles its subtree AND "locates" it — folders
  /// never become `selectedView`, so this is how a folder becomes the target
  /// for the top New-page/New-folder buttons (and gets the row highlight).
  void toggle(String viewId) {
    focusedNavId = viewId;
    rootFocused = false; // locating a folder takes us off the root
    if (!_expandedIds.add(viewId)) {
      _expandedIds.remove(viewId);
    }
    notifyListeners();
  }

  void focusRoot() {
    if (rootFocused && focusedNavId == null) return;
    rootFocused = true;
    focusedNavId = null;
    notifyListeners();
  }

  /// A page row was tapped: the row is now located. Guarded so an identical
  /// re-tap is a no-op (and triggers no rebuild) — one rule, one place; the
  /// folder path always writes, the page path only appeared to.
  void locateRow(String viewId) {
    if (rootFocused || focusedNavId != viewId) {
      focusedNavId = viewId;
      rootFocused = false;
      notifyListeners();
    }
  }

  /// A document opened by any non-sidebar path (tab strip, link, a
  /// workspace's auto-opened page): locating it keeps the tree honest about
  /// where the user is. Nullable because the open doc can go away — the
  /// highlight simply lands nowhere until something is located again.
  void focusView(String? viewId) {
    focusedNavId = viewId;
    rootFocused = false;
    notifyListeners();
  }

  /// Expand every ANCESTOR of [id] so a (possibly nested) node is revealed in
  /// the sidebar — used on navigate/create so the active/new page is never
  /// hidden under a collapsed parent (AppFlowy/Notion "reveal current page").
  /// The node itself is not expanded (that would show ITS children).
  void revealAncestors(List<DocumentView> views, String id) {
    _expandedIds.addAll(ancestorIds(views, id));
    notifyListeners();
  }

  /// Expand [id] itself (so a freshly-created/dropped child under it is
  /// visible) plus its ancestor chain.
  void expandForChildOf(List<DocumentView> views, String id) {
    _expandedIds.add(id);
    revealAncestors(views, id);
  }

  /// Locate [viewId] and expand it plus its ancestors. A row that is not in
  /// [views] does nothing rather than focus an id that renders nowhere.
  void revealFolder(List<DocumentView> views, String viewId) {
    final byId = {for (final view in views) view.id: view};
    if (!byId.containsKey(viewId)) return;
    focusedNavId = viewId;
    rootFocused = false;
    _expandedIds.add(viewId);
    // Every ancestor too, or the row stays hidden inside a collapsed parent
    // and "locating" it shows the user nothing. The `seen` guard is
    // belt-and-braces — the page tree forbids cycles (folder-only parents,
    // enforced server-side and by a DB trigger) — but this walk should not be
    // the thing that hangs if one ever appears.
    final seen = <String>{viewId};
    var cursor = byId[viewId]?.parentViewId;
    while (cursor != null && seen.add(cursor)) {
      _expandedIds.add(cursor);
      cursor = byId[cursor]?.parentViewId;
    }
    notifyListeners();
  }

  /// Locate [viewId] when its tree has it, and remember it either way: the
  /// row may belong to a workspace whose views have not arrived yet, and the
  /// pending id is what gets re-checked when they do.
  void revealInTree(List<DocumentView> views, String viewId) {
    final byId = {for (final view in views) view.id: view};
    if (!byId.containsKey(viewId)) {
      pendingRevealId = viewId;
      revealAttempts = 0;
      notifyListeners();
      return;
    }
    revealFolder(views, viewId);
    pendingRevealId = viewId;
    revealAttempts = 0;
    notifyListeners();
  }

  /// One more frame spent waiting for [pendingRevealId]. Returns false when
  /// the reveal has given up (~2s of frames: something is not coming — a
  /// deleted page, a workspace that failed to load) and cleared itself, so
  /// the caller stops scheduling callbacks rather than running one every
  /// frame forever.
  bool bumpRevealAttempt() {
    if (pendingRevealId == null) return false;
    if (++revealAttempts > 120) {
      pendingRevealId = null;
      revealAttempts = 0;
      notifyListeners();
      return false;
    }
    return true;
  }

  /// The pending reveal landed (or was abandoned): stop asking for it. Split
  /// from [bumpRevealAttempt] because the success path clears the id directly
  /// rather than counting one more frame.
  void clearPendingReveal() {
    pendingRevealId = null;
    revealAttempts = 0;
    notifyListeners();
  }

  /// The tree changed shape wholesale (workspace switch): drop every
  /// expansion, then reveal whatever is now open.
  void clearExpansion() {
    _expandedIds.clear();
    notifyListeners();
  }
}
