part of 'main.dart';

/// The page's path, workspace first, one name per segment.
///
/// This is the single source for BOTH things that show a path: "copy path"
/// joins it with `/`, and [PageBreadcrumb] renders the same list left to right.
/// They were computed separately once, and drifted — the clipboard led with the
/// workspace while the breadcrumb started one level below it, so you could not
/// tell what the button next to it was about to copy. Nothing failed when they
/// disagreed; only a human reading both at once could notice. Hence one
/// function, and `test/page_path_test.dart` pinning the two against each other.
///
/// Returns null when there is no open page (or no workspace) — the caller says
/// "open a page first" rather than showing half a path.
///
/// Names are NOT escaped: a `/` inside a page name is ambiguous here on
/// purpose. The path is for a human to read and for an agent to resolve by
/// searching the last segment and checking the chain with `parent_view_id`, not
/// by splitting on a delimiter.
List<String>? pagePathSegments({
  required String? workspaceName,
  required DocumentView? view,
  required List<DocumentView> views,
}) {
  if (workspaceName == null || view == null) return null;
  return [
    ...ancestorPathSegments(
      workspaceName: workspaceName,
      parentViewId: view.parentViewId,
      views: views,
      startedAt: view.id,
    ),
    view.name,
  ];
}

/// Where something LIVES: workspace first, then each folder down to (and
/// including) [parentViewId]. The thing itself is not in here.
///
/// Split out of [pagePathSegments] for the search results list, which shows the
/// trail beside a hit's name. Walking the tree a second time there would put two
/// implementations of "the path" back in the codebase — which is the exact shape
/// of the bug this pair exists to prevent.
///
/// Returns empty (never a placeholder) when there is nothing to say: a page at
/// the workspace root with no workspace name has no trail, and the caller draws
/// nothing rather than a lone separator.
List<String> ancestorPathSegments({
  required String? workspaceName,
  required String? parentViewId,
  required List<DocumentView> views,
  String? startedAt,
}) {
  final byId = {for (final v in views) v.id: v};
  final folders = <String>[];
  // Same `seen` guard as _revealFolder: the tree forbids cycles, but walking it
  // should not be the thing that hangs if one ever appears.
  final seen = <String>{?startedAt};
  var cursor = parentViewId;
  while (cursor != null && seen.add(cursor)) {
    final parent = byId[cursor];
    if (parent == null) break; // not loaded — stop rather than invent a gap
    folders.add(parent.name);
    cursor = parent.parentViewId;
  }
  return [
    if (workspaceName != null && workspaceName.trim().isNotEmpty) workspaceName,
    ...folders.reversed,
  ];
}

/// What the world picker should offer for the server it is pointing at.
enum WorldCardMode {
  /// Ask for credentials — we hold none, or this flow needs a fresh sign-in.
  signIn,

  /// We are already in this world: the app is behind this screen, go back to it.
  back,

  /// We hold credentials but are somewhere else (本地模式, another server):
  /// enter this one, no password.
  enter,
}

/// Pure, because this decision was wrong twice in a row (2026-08-12).
///
/// First it keyed on the live `_session` — which 本地模式 clears while the
/// credentials stay on disk, so entering local mode and opening the picker
/// asked for a password to a server it could have walked into. Then it also
/// excluded local mode outright, hiding the one case that made the bug visible.
///
/// [storedName] is the identity persisted for [cloudOrigin] (null = none), NOT
/// whatever session happens to be in memory. That distinction is the fix.
/// The workspace, file and (cosmetic) name behind one of OUR image links, or
/// null for any other url. The shape is the one `_resolveEditorImageUrls`
/// builds for copy — keep the two in step:
///
///     <origin>/api/workspaces/<workspace_id>/files/<file_id>/blob[/<name>]
///
/// Reading the ids back out is what lets a copied image be recognised as
/// already ours (see `_importEditorImageUrl`) instead of being re-hosted from
/// scratch. [expectedOrigin] must match: a document can carry any url it likes,
/// and a look-alike from another host must not be able to name a `file_id` in
/// this workspace.
({String workspaceId, String fileId, String name})? micaBlobLink(
  String url,
  String expectedOrigin,
) {
  final uri = Uri.tryParse(url.trim());
  if (uri == null || !uri.hasScheme) return null;
  if (apiOrigin(uri) != expectedOrigin) return null;
  // api / workspaces / <ws> / files / <id> / blob [/ <cosmetic name>]
  final p = uri.pathSegments;
  if (p.length < 6 || p.length > 7) return null;
  if (p[0] != 'api' ||
      p[1] != 'workspaces' ||
      p[3] != 'files' ||
      p[5] != 'blob') {
    return null;
  }
  if (p[2].isEmpty || p[4].isEmpty) return null;
  return (workspaceId: p[2], fileId: p[4], name: p.length == 7 ? p[6] : '');
}

WorldCardMode worldCardMode({
  required bool migrating,
  required String? storedName,
  required String cloudOrigin,
  required String activeOrigin,
}) {
  // A migration genuinely needs a sign-in, whatever we have stored.
  if (migrating) return WorldCardMode.signIn;
  if (storedName == null || storedName.isEmpty) return WorldCardMode.signIn;
  return activeOrigin == cloudOrigin ? WorldCardMode.back : WorldCardMode.enter;
}

/// Width of the tree row's expand-toggle column. Always present, even on a page
/// that has no toggle, so every icon in the tree shares one column — which is
/// what makes the icon's left edge the thing the eye reads a level from.
const kTreeExpandColumnWidth = 18.0;

/// Left inset of a drop indicator for a row at [depth].
///
/// `2 + depth * 16` is where `DocumentListItem` starts its CONTENT, and the
/// first thing in that content is the expand column — so a dot drawn there sits
/// a whole column left of the icons, which is the gap reported as 「这个圆点与
/// 要对齐的层级图标左侧有点远」. Adding the column lines the dot up with the
/// icon edge, and the icon edge is what you compare levels by.
///
/// The indicator used to be a full-width line at every depth, which made the
/// two states either side of the tree's last row — "inside that last folder"
/// and "at the workspace root" — draw IDENTICALLY. You could only tell them
/// apart by where the pointer was, which is what "稍微靠上一点，就会出现进最后
/// 一个子目录的情况" is: not a hit-target problem, a feedback problem.
///
/// Top-level and pure so the arithmetic is pinned; getting it wrong is a line
/// two pixels off, which nobody would report and everybody would misread.
double dropIndicatorInset(int depth) =>
    2 + depth * kTreeIndentUnit + kTreeExpandColumnWidth;

/// Per-level indent of the sidebar tree, and the width of one horizontal step
/// while re-parenting a drop.
const kTreeIndentUnit = 16.0;

/// What the tree's drop indicator is currently saying.
enum DropIndicatorState {
  /// No drag is over this slot.
  idle,

  /// The drop would be taken.
  allowed,

  /// The drop is REFUSED, and the mark is drawn anyway, in the warning colour.
  ///
  /// Atlassian's tree calls this `instruction-blocked` and renders the
  /// instruction you were ASKING for rather than nothing; AFFiNE keeps that.
  /// Drawing nothing is the worse answer twice over: an indicator that vanishes
  /// reads as "the app stopped noticing me", and it withholds the one thing
  /// worth saying — that this particular place is the problem. A folder cannot
  /// be dropped inside its own subtree, and until now the only feedback for
  /// trying was silence.
  blocked,
}

/// The level a drop below [rowDepth] lands at, given how far right the pointer
/// is. This is the "drag left to pop out a level" of AFFiNE's tree, which is
/// Atlassian pragmatic-drag-and-drop's `reparent` instruction.
///
/// The range is not free. Levels run from [rowDepth] (stay a sibling of the row
/// you are under) down to:
///
///  * the depth of the NEXT visible row, when there is one. You cannot pop out
///    past it: the gap you are pointing at is between this row and that one, and
///    a level shallower than the next row would put the drop somewhere that gap
///    does not touch. When the next row is DEEPER — the row is an expanded
///    folder and the gap is between it and its own first child — the range
///    collapses to a single level and there is nothing to pick.
///  * 0 (the workspace root) when this is the last row in the tree.
///
/// Every level in that range is a legal parent by construction: the ancestors of
/// a row are folders, and folders are the only thing allowed to hold children.
/// So the indicator can never stop somewhere the drop would be refused, which is
/// the failure mode a free-floating depth picker has.
int reparentLevelFor({
  required double pointerX,
  required int rowDepth,
  required int? nextRowDepth,
  double indentUnit = kTreeIndentUnit,
  // The SAME origin the dot is drawn from. Left at the content start (2) while
  // the dot moved right by the expand column, the level would flip a whole
  // column before the dot reached the next position — the mark would lag the
  // gesture, which is the one thing an aiming affordance must not do.
  double baseInset = 2 + kTreeExpandColumnWidth,
}) {
  final minLevel = nextRowDepth == null
      ? 0
      : math.min(nextRowDepth, rowDepth);
  if (minLevel >= rowDepth) return rowDepth;
  final raw = ((pointerX - baseInset) / indentUnit).round();
  return raw.clamp(minLevel, rowDepth);
}

/// The workspace root's children after [dragged] is dropped on the root zone:
/// every current root-level view except [dragged], in position order, with
/// [dragged] appended.
///
/// Top-level and pure for the same reason as [treeRevealOffset]: the sidebar
/// tree lives inside a widget with 97 required parameters, so the ordering can
/// only be tested if it is not inside it. And the ordering is the part worth
/// testing — dropping the row is visible, dropping it in the wrong PLACE is a
/// silent reorder of somebody's sidebar.
///
/// [dragged] is filtered out before it is appended, so re-dropping a row that
/// is already at the root moves it to the end instead of listing it twice.
List<DocumentView> rootDropOrder(
  List<DocumentView> views,
  List<DocumentView> dragged,
) {
  final moving = {for (final d in dragged) d.id};
  final roots =
      views.where((v) => v.parentViewId == null && !moving.contains(v.id)).toList()
        ..sort((a, b) => a.position.compareTo(b.position));
  return [...roots, ...dragged];
}

/// Scroll offset that centres row [index] of the sidebar tree in the viewport.
///
/// Top-level and pure because getting it wrong is invisible: the first attempt
/// at "reveal in tree" used `Scrollable.ensureVisible` on the row's GlobalKey,
/// which cannot work — the tree is a lazy `ListView`, so a row below the fold
/// has no element and no context, and the call silently did nothing for exactly
/// the rows that needed it. Arithmetic over a widget lookup, and tested.
///
/// Clamped into the scrollable's real range, so revealing the first or last row
/// asks for a position that exists rather than one the list will refuse.
double treeRevealOffset({
  required int index,
  required double rowExtent,
  required double viewportHeight,
  required double maxScrollExtent,
  double minScrollExtent = 0,
}) {
  final centred = (index * rowExtent) - (viewportHeight / 2) + (rowExtent / 2);
  if (maxScrollExtent <= minScrollExtent) return minScrollExtent;
  return centred.clamp(minScrollExtent, maxScrollExtent);
}

