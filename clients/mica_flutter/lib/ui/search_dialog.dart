import 'typography.dart';
import '../l10n/locale_controller.dart';
import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import '../api/models.dart';
import '../editor/model.dart';
import '../main_tree_helpers.dart';
import 'home_screen.dart';
import 'search_data.dart';
import 'status_kit.dart';
import 'theme_tokens.dart';

/// Workspace search: type to find pages by title or body text; click to open.
/// A keyboard-cap chip: mono glyph on a pale slab.
///
/// `kMonoFont`, not `'monospace'` — that family name does not resolve on web (see
/// `model.dart`), which is exactly where a key hint would silently fall back to a
/// proportional font and stop reading as a key.
class _KeyCap extends StatelessWidget {
  const _KeyCap(this.label);

  final String label;

  @override
  Widget build(BuildContext context) {
    return Container(
      decoration: BoxDecoration(
        color: MicaTheme.of(context).surface.sunken,
        borderRadius: BorderRadius.circular(6),
      ),
      padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
      child: Text(
        label,
        style: MicaTypography.micro.copyWith(
          fontFamily: kMonoFont,
          color: MicaTheme.of(context).text.faint,
        ),
      ),
    );
  }
}

/// Fixed row height in the search result list.
///
/// Fixed so the keyboard selection can be scrolled into view by arithmetic
/// (`index * extent`) rather than hanging a GlobalKey off every row.
///
/// Three lines now — name / path / matched text — so it is taller than the two
/// it held before. Fixed also means every row is the SAME height whether or not
/// it has a path or a snippet, which is what keeps the list from looking ragged
/// (Notion's rows vary; ours cannot, and evenness is the better trade at a fixed
/// extent).
const double _searchRowHeight = 82;

/// Move the search result selection by [delta] rows.
///
/// A real Intent rather than a `CallbackShortcuts` binding, because of where the
/// key events go. `CallbackShortcuts` is a `Focus.onKeyEvent`, and those run from
/// the INNERMOST focused node outwards — the focused node here is inside the
/// TextField, whose `DefaultTextEditingShortcuts` map ArrowUp/ArrowDown to
/// move-caret-to-start/end and consume them. `Shortcuts` placed BETWEEN the
/// TextField and the app is nearer than the default map, so it wins the lookup;
/// the intent then resolves against the [Actions] wrapper below.
class _SearchMoveIntent extends Intent {
  const _SearchMoveIntent(this.delta);

  final int delta;
}

class SearchDialog extends StatefulWidget {
  const SearchDialog({
    super.key,
    required this.onSearch,
    required this.onOpen,
    required this.onReveal,
    this.onSearchAll,
    this.views = const [],
    this.workspaceName,
    this.workspaceId,
    this.initialQuery,
    this.recents = const [],
    this.workspaces = const [],
    this.onOpenWorkspace,
  });

  /// Every workspace the user belongs to, matched by NAME as you type.
  ///
  /// Filtered here rather than on the server because the list is already in
  /// memory — switching among 50 workspaces should not cost a round trip, and
  /// there is no endpoint that would answer it anyway.
  final List<({String id, String name})> workspaces;

  /// Switch to a workspace. Null drops workspace matching entirely (the local
  /// world, where the picker has one implicit workspace and nothing to find).
  final void Function(String workspaceId)? onOpenWorkspace;

  /// Recently edited pages in THIS workspace, shown before anything is typed.
  /// Built by the host (`buildRecents`), so this dialog does not grow a second
  /// notion of "recent" alongside the home screen's.
  final List<HomeDocEntry> recents;

  /// The workspace tree, for turning a hit's `parent_view_id` into the trail
  /// shown beside its name. Resolved locally on purpose: the tree is already in
  /// memory here, so asking the server per row (AppFlowy runs a bloc + cache per
  /// result because its frontend does NOT hold the tree) would be inventing a
  /// problem we do not have.
  final List<DocumentView> views;

  /// Leads the trail, same as in the breadcrumb and in "copy path".
  final String? workspaceName;

  /// The CURRENT workspace's id — what a hit's `workspace_id` is compared
  /// against to decide whose trail it wears. Without this every cross-workspace
  /// hit was dressed in the current workspace's name (2026-08-28): the trail
  /// below is built from [workspaceName] + [views], both of which describe the
  /// workspace being LOOKED FROM, not the one the hit lives in.
  final String? workspaceId;

  /// Null when the active world has no workspace search at all (本地模式), which
  /// is not the same thing as a search that finds nothing. It used to be wired
  /// to a stub returning `const []`, so every local query answered 「没有找到与
  /// 「x」匹配的内容」 — the app stating as fact that the page isn't there.
  final Future<List<SearchResult>> Function(String query)? onSearch;

  /// The same search across every workspace. Null hides the toggle entirely —
  /// the local world has one workspace, so there is nothing to widen to.
  final Future<List<SearchResult>> Function(String query)? onSearchAll;

  /// Open a hit. The second argument is the hit's workspace, null when the
  /// server did not say — callers read that as "the workspace we searched".
  ///
  /// [inNewTab] is the accelerator-held variant (Ctrl/Cmd + click or + Enter):
  /// put it in a fresh tab instead of replacing what is open. It matters most
  /// for a hit from ANOTHER workspace, which otherwise takes the whole shell
  /// there — a tab carries its own workspace, so the new-tab route can hold a
  /// foreign page without moving the user off what they were doing.
  final void Function(String viewId, String? workspaceId, {bool inNewTab})
  onOpen;

  /// A FOLDER hit. Folders cannot be opened — there is no document behind one —
  /// so they get their own exit: locate the folder in the sidebar tree instead.
  /// Routing them through [onOpen] silently did nothing at all, because the
  /// open path already refuses folders.
  ///
  /// Carries the hit's workspace for the same reason [onOpen] does: a folder in
  /// ANOTHER workspace cannot be located in the tree on screen, and revealing
  /// without switching there first is a click that does nothing. Null means
  /// "the workspace we searched" (an older server does not send the field).
  final void Function(String viewId, String? workspaceId) onReveal;

  /// Pre-filled query, run immediately on open — e.g. clicking a page-property
  /// tag opens search already looking for that tag.
  final String? initialQuery;

  @override
  State<SearchDialog> createState() => _SearchDialogState();
}

class _SearchDialogState extends State<SearchDialog> {
  final _query = TextEditingController();
  Timer? _debounce;
  bool _loading = false;
  bool _failed = false;
  List<SearchResult> _results = const [];
  String _lastQuery = '';

  /// Keyboard-highlighted row, -1 when none. Separate from "hovered": the whole
  /// point is to be able to pick a result without touching the mouse.
  int _selected = -1;

  final _listScroll = ScrollController();

  @override
  void initState() {
    super.initState();
    final q = widget.initialQuery?.trim();
    if (q != null && q.isNotEmpty) {
      _query.text = q;
      // Run after first frame so the initial setState in _run is safe.
      WidgetsBinding.instance.addPostFrameCallback((_) => _run(q));
    }
  }

  @override
  void dispose() {
    _debounce?.cancel();
    _query.dispose();
    _listScroll.dispose();
    super.dispose();
  }

  void _onChanged(String value) {
    _debounce?.cancel();
    _debounce = Timer(const Duration(milliseconds: 280), () => _run(value));
  }

  Future<void> _run(String value) async {
    final search = widget.onSearch;
    if (search == null) return; // no search in this world; see [_buildResults]
    final query = value.trim();
    if (query.isEmpty) {
      setState(() {
        _results = const [];
        _loading = false;
        _selected = -1;
      });
      return;
    }
    setState(() => _loading = true);
    try {
      // Every workspace, ranked with this one preferred — see [searchScopeFor]
      // for why the scope is no longer conditional.
      final results = await searchScopeFor(
        searchHere: () => search(query),
        searchEverywhere: widget.onSearchAll == null
            ? null
            : () => widget.onSearchAll!(query),
        onGlobalError: (e) => debugPrint('global search unavailable: $e'),
      );
      if (!mounted || _query.text.trim() != query) return;
      setState(() {
        _results = results;
        _lastQuery = query;
        _loading = false;
        _failed = false;
        // Preselect the top row: ↵ should act on the obvious answer without an
        // extra ↓ first. Counts workspace hits too — searching a workspace by
        // name normally finds no PAGE of that name, and keying off `results`
        // alone left ↵ inert on exactly the query that matched a workspace.
        _selected = (results.isEmpty && _workspaceHits.isEmpty) ? -1 : 0;
      });
    } catch (e) {
      // Surface the failure — a swallowed error reads as "no results" and
      // hides real breakage (this dialog masked a 404 for a while).
      //
      if (mounted) {
        setState(() {
          _loading = false;
          _failed = true;
          _results = const [];
          _lastQuery = query;
          _selected = -1;
        });
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: Text(context.l10n.searchTitle),
      content: SizedBox(
        // 600 (design 06); the result area's height is unchanged.
        width: 600,
        height: 420,
        child: Column(
          children: [
            Shortcuts(
              shortcuts: const {
                SingleActivator(LogicalKeyboardKey.arrowDown):
                    _SearchMoveIntent(1),
                SingleActivator(LogicalKeyboardKey.arrowUp): _SearchMoveIntent(
                  -1,
                ),
              },
              child: Actions(
                actions: {
                  _SearchMoveIntent: CallbackAction<_SearchMoveIntent>(
                    onInvoke: (intent) {
                      _move(intent.delta);
                      return null;
                    },
                  ),
                },
                child: TextField(
                  controller: _query,
                  // Nothing to type into when there is no search behind it: an
                  // enabled field that eats your keystrokes and answers "no results"
                  // is worse than one that is visibly unavailable.
                  enabled: widget.onSearch != null,
                  autofocus: widget.onSearch != null,
                  onChanged: _onChanged,
                  // ↵ opens the highlighted row. Not a Shortcuts binding: Enter in a
                  // single-line field already means "submit", and hijacking it higher
                  // up would also swallow IME confirmation.
                  onSubmitted: (_) => _openSelected(),
                  decoration: InputDecoration(
                    hintText: context.l10n.searchHint,
                    prefixIcon: const Icon(Icons.search),
                    // Spinner while a query is in flight, otherwise the `esc`
                    // hint: one slot, because both answer the same question —
                    // what is this field doing right now.
                    suffixIcon: _loading
                        ? const Padding(
                            padding: EdgeInsets.all(12),
                            child: SizedBox(
                              width: 16,
                              height: 16,
                              child: CircularProgressIndicator(strokeWidth: 2),
                            ),
                          )
                        : const Padding(
                            padding: EdgeInsets.only(right: 10),
                            child: _KeyCap('esc'),
                          ),
                    border: const OutlineInputBorder(),
                  ),
                ),
              ),
            ),
            const SizedBox(height: 12),
            Expanded(child: _buildResults(context)),
            // Only with results on screen: advertising ↑↓/↵ over an empty list
            // would promise keys that do nothing.
            if (_results.isNotEmpty) ...[
              const SizedBox(height: 17),
              Row(
                children: [
                  const _KeyCap('↑'),
                  const SizedBox(width: 4),
                  const _KeyCap('↓'),
                  const SizedBox(width: 6),
                  Text(
                    context.l10n.searchHintSelect,
                    style: MicaTypography.caption.copyWith(
                      color: MicaTheme.of(context).text.faint,
                    ),
                  ),
                  const SizedBox(width: 12),
                  const _KeyCap('↵'),
                  const SizedBox(width: 6),
                  Text(
                    context.l10n.searchHintOpen,
                    style: MicaTypography.caption.copyWith(
                      color: MicaTheme.of(context).text.faint,
                    ),
                  ),
                  const Spacer(),
                  // In-page find, NOT 「全局搜索 ⌘⇧F」: that is the shortcut that
                  // opened this very dialog (self-referential), and Ctrl is the
                  // right glyph for a Windows-first product.
                  Text(
                    context.l10n.searchHintInPage,
                    style: MicaTypography.caption.copyWith(
                      color: MicaTheme.of(context).text.faint,
                    ),
                  ),
                  const SizedBox(width: 6),
                  const _KeyCap('Ctrl+F'),
                ],
              ),
            ],
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(context.l10n.commonClose),
        ),
      ],
    );
  }

  /// Workspaces whose NAME contains the query, newest-typed-first order kept.
  ///
  /// Synchronous: unlike the page search this needs no request, so a workspace
  /// appears on the first keystroke while the page results are still debounced.
  /// That is also why they render above — the fast half should not sit below a
  /// spinner.
  ///
  /// Capped, and the cap is visible in the UI as "just the closest few": an
  /// uncapped list of similarly-named workspaces would push the page results
  /// off the panel entirely.
  List<({String id, String name})> get _workspaceHits {
    if (widget.onOpenWorkspace == null) return const [];
    return matchingWorkspaces(
      workspaces: widget.workspaces,
      query: _query.text,
    );
  }

  /// The keyboard sequence runs over workspace hits FIRST, then page results —
  /// the same order they are drawn in, because a selection that jumps around
  /// relative to what is on screen is worse than no keyboard nav at all.
  int get _rowCount => _workspaceHits.length + _results.length;

  /// Move the keyboard selection and keep it on screen.
  void _move(int delta) {
    final next = moveSelection(
      current: _selected,
      count: _rowCount,
      delta: delta,
    );
    if (next == _selected) return;
    setState(() => _selected = next);
    // Workspace rows sit above the scroll view and are always visible, so only
    // a selection that landed among the page results needs scrolling into view.
    final inResults = next - _workspaceHits.length;
    if (inResults < 0 || !_listScroll.hasClients) return;
    // Rows are a fixed height here, so the offset is computable — no need for a
    // per-row GlobalKey just to scroll.
    const rowExtent = _searchRowHeight;
    final target = inResults * rowExtent;
    final top = _listScroll.offset;
    final bottom = top + _listScroll.position.viewportDimension - rowExtent;
    if (target < top) {
      _listScroll.jumpTo(target);
    } else if (target > bottom) {
      _listScroll.jumpTo(
        (target - _listScroll.position.viewportDimension + rowExtent).clamp(
          0.0,
          _listScroll.position.maxScrollExtent,
        ),
      );
    }
  }

  /// Act on a hit: pages open, folders get located in the tree.
  ///
  /// The ONE place that decides, because there are two ways to trigger a hit —
  /// Enter on the keyboard selection and a tap on the row — and a rule that
  /// lives in both is a rule that will eventually only be true in one.
  void _activate(SearchResult result) {
    if (result.isFolder) {
      // A folder has no document, so "in a new tab" has nothing to mean here —
      // the accelerator is simply ignored rather than doing something else.
      widget.onReveal(result.viewId, result.workspaceId);
    } else {
      widget.onOpen(result.viewId, result.workspaceId, inNewTab: _accelHeld);
    }
  }

  /// Whether Ctrl/Cmd is down RIGHT NOW — read at activation rather than
  /// tracked, because a modifier pressed after the list was built still counts
  /// and one released before the click no longer does.
  bool get _accelHeld =>
      HardwareKeyboard.instance.isControlPressed ||
      HardwareKeyboard.instance.isMetaPressed;

  /// Act on the keyboard-selected row, if there is one.
  ///
  /// Dispatches on the same index space [_move] walks: the first
  /// `_workspaceHits.length` rows are workspaces, the rest are page results.
  void _openSelected() {
    if (_selected < 0 || _selected >= _rowCount) return;
    final hits = _workspaceHits;
    if (_selected < hits.length) {
      widget.onOpenWorkspace?.call(hits[_selected].id);
      return;
    }
    _activate(_results[_selected - hits.length]);
  }

  /// The pre-typing list. Rows route through the same [SearchDialog.onOpen] as
  /// a search hit, so "open in a new tab" and every other host behaviour applies
  /// to a recent exactly as it does to a result.
  Widget _recentsList(BuildContext context) {
    final theme = MicaTheme.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.only(left: 4, bottom: 6),
          child: Text(
            context.l10n.searchRecentLabel,
            style: MicaTypography.caption.copyWith(color: theme.text.faint),
          ),
        ),
        Expanded(
          child: ListView.builder(
            itemCount: widget.recents.length,
            itemBuilder: (context, i) {
              final entry = widget.recents[i];
              return ListTile(
                dense: true,
                leading: entry.icon != null && entry.icon!.isNotEmpty
                    ? Text(entry.icon!, style: MicaTypography.large)
                    : Icon(
                        Icons.description_outlined,
                        size: 18,
                        color: theme.text.muted,
                      ),
                title: Text(
                  entry.name,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
                trailing: Text(
                  entry.meta,
                  style: MicaTypography.caption.copyWith(
                    color: theme.text.faint,
                  ),
                ),
                // Recents are this workspace's, so no workspace to carry.
                onTap: () => widget.onOpen(entry.id, null),
              );
            },
          ),
        ),
      ],
    );
  }

  /// The trail beside a hit's name — whose workspace it actually lives in.
  ///
  /// A hit from the CURRENT workspace resolves its folder chain out of the tree
  /// in memory. A hit from another workspace shows that workspace's NAME and no
  /// folders: this client only holds trees it has visited, and resolving the
  /// chain against the current tree is how every foreign hit came to wear the
  /// current workspace's name. `workspaceId == null` (older server) reads as
  /// "the workspace searched", same as everywhere else in this file.
  List<String> _trailFor(SearchResult result) {
    final hitWs = result.workspaceId;
    if (hitWs != null &&
        widget.workspaceId != null &&
        hitWs != widget.workspaceId) {
      return workspaceTrailFor(hitWs, widget.workspaces);
    }
    return ancestorPathSegments(
      workspaceName: widget.workspaceName,
      parentViewId: result.parentViewId,
      views: widget.views,
      startedAt: result.viewId,
    );
  }

  Widget _buildResults(BuildContext context) {
    // Say the true thing: this world has no workspace search yet, and point at
    // the tool that does work here (in-page find). The old stub claimed instead
    // that the workspace contains no such page.
    if (widget.onSearch == null) {
      return EmptyState(
        icon: Icons.search_off,
        title: context.l10n.searchLocalUnsupportedTitle,
        detail: context.l10n.searchLocalUnsupportedDetail,
      );
    }
    if (_query.text.trim().isEmpty) {
      // Recently edited, before a single keystroke — AppFlowy's command palette
      // does the same (`command_palette/widgets/recent_views_list.dart`), and
      // for the same reason: most of the time the page you want is one you had
      // open recently, and typing its name is a worse way to say so.
      //
      // These are the CURRENT workspace's pages, matching what the search below
      // can actually find. Offering pages from other workspaces would put rows
      // here that no query in this box could ever return.
      if (widget.recents.isNotEmpty) return _recentsList(context);
      return EmptyState(
        icon: Icons.search,
        title: context.l10n.searchEmptyTitle,
        detail: context.l10n.searchEmptyDetail,
      );
    }

    // Workspace matches ride ABOVE whatever the page half has to say — including
    // its "no matches" state. Typing a workspace name usually finds no PAGE by
    // that name, so putting this below the early return meant the panel
    // answered 「无匹配结果」 while holding the exact thing the user asked for.
    final workspaceHits = _workspaceHits;
    if (workspaceHits.isEmpty) return _buildPageResults(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _workspaceSection(context, workspaceHits),
        Expanded(child: _buildPageResults(context)),
      ],
    );
  }

  Widget _workspaceSection(
    BuildContext context,
    List<({String id, String name})> hits,
  ) {
    final theme = MicaTheme.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisSize: MainAxisSize.min,
      children: [
        Padding(
          padding: const EdgeInsets.only(left: 4, bottom: 4),
          child: Text(
            context.l10n.searchWorkspaceLabel,
            style: MicaTypography.caption.copyWith(color: theme.text.faint),
          ),
        ),
        for (var i = 0; i < hits.length; i++)
          Material(
            color: _selected == i ? theme.surface.hover : Colors.transparent,
            borderRadius: BorderRadius.circular(6),
            child: InkWell(
              borderRadius: BorderRadius.circular(6),
              onTap: () => widget.onOpenWorkspace?.call(hits[i].id),
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 8),
                child: Row(
                  children: [
                    Icon(
                      Icons.workspaces_outline,
                      size: 16,
                      color: theme.text.muted,
                    ),
                    const SizedBox(width: 8),
                    Expanded(
                      child: Text(
                        hits[i].name,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                      ),
                    ),
                    Text(
                      context.l10n.searchWorkspaceSwitch,
                      style: MicaTypography.caption.copyWith(
                        color: theme.text.faint,
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        const SizedBox(height: 8),
      ],
    );
  }

  Widget _buildPageResults(BuildContext context) {
    if (!_loading && _results.isEmpty) {
      // A failure is not an empty result set. Both used to render under the
      // title 「无匹配结果」, which told the user their workspace has no such
      // page when in fact the request never reached the server — and it hid
      // the one useful next step (retry). Separate icon, title and action.
      if (_failed) {
        return EmptyState(
          icon: Icons.cloud_off,
          title: context.l10n.searchFailedTitle,
          detail: context.l10n.searchFailed,
          actionLabel: context.l10n.commonRetry,
          onAction: () => _run(_query.text),
        );
      }
      return EmptyState(
        icon: Icons.search_off,
        title: context.l10n.searchNoMatches,
        detail: context.l10n.searchNothingFound(_lastQuery),
      );
    }
    return Column(
      children: [
        // How many hits, so a long list doesn't have to be counted by eye.
        Align(
          alignment: Alignment.centerLeft,
          child: Padding(
            padding: const EdgeInsets.only(left: 4, bottom: 6),
            child: Text(
              context.l10n.searchResultCount(_results.length),
              style: MicaTypography.micro.copyWith(
                fontWeight: FontWeight.w500,
                color: MicaTheme.of(context).text.faint,
              ),
            ),
          ),
        ),
        Expanded(
          child: ListView.builder(
            controller: _listScroll,
            // Fixed extent so [_move] can scroll the selection into view by
            // arithmetic instead of hanging a GlobalKey off every row.
            itemExtent: _searchRowHeight,
            itemCount: _results.length,
            itemBuilder: (context, i) => SearchResultTile(
              result: _results[i],
              query: _lastQuery,
              path: _trailFor(_results[i]),
              // Offset by the workspace rows drawn above: `_selected` indexes
              // both sections, so comparing it to the raw list index would
              // highlight the wrong page whenever a workspace also matched.
              selected: i + _workspaceHits.length == _selected,
              onTap: () => _activate(_results[i]),
            ),
          ),
        ),
      ],
    );
  }
}

/// One row of the search results list.
///
/// Public (and therefore testable) only because the dialog around it is not:
/// the invariant below has no other seam to be pinned through.
///
/// **The selected row's background must be [ListTile.tileColor], never a
/// `Container(color:)` around the tile.** A ListTile paints its background AND
/// its ink splash onto the nearest [Material] ancestor; a coloured box wedged
/// between the two covers the splash, so the one row you are most likely to
/// click is the one row that answers a click with nothing. Flutter says so out
/// loud in debug ("ListTile background color or ink splashes may be invisible")
/// — but only in debug, and only into a log nobody reads, which is how this
/// survived to 0.13.9. `tileColor` paints through [Ink], i.e. *under* the
/// splash, which is what it exists for.
class SearchResultTile extends StatelessWidget {
  const SearchResultTile({
    required this.result,
    required this.query,
    required this.selected,
    required this.onTap,
    this.path = const [],
    super.key,
  });

  final SearchResult result;

  /// The query these hits came back for — what gets tinted inside the snippet.
  final String query;

  /// Where the hit lives: workspace first, then folders, WITHOUT the hit itself
  /// (its name is on the line above this). Empty draws nothing.
  ///
  /// Two names in a workspace can read identically — "问题记录" under three
  /// different projects is the normal case, not the exotic one — and a hit list
  /// that shows only names makes you open pages to find out which is which.
  ///
  /// Shown WHOLE. An earlier cut of this collapsed the middle out of deep paths
  /// (`tools / … / reasonix`, which is what AppFlowy does on its one-line
  /// layout); on a line of its own there is room for the real thing, and a path
  /// with a hole in it is not the path.
  final List<String> path;

  final bool selected;
  final VoidCallback onTap;

  /// The snippet with query matches tinted.
  ///
  /// Case-insensitive to match the server's `ILIKE` — see [highlightRuns]; a
  /// case-sensitive version would return real hits with nothing marked in them.
  Widget _snippet(BuildContext context, String text) {
    final runs = highlightRuns(text, query);
    return Text.rich(
      TextSpan(
        children: [
          for (final r in runs)
            TextSpan(
              text: r.text,
              style: r.hit
                  ? TextStyle(
                      backgroundColor: MicaTheme.of(
                        context,
                      ).editor.commentHighlight,
                    )
                  : null,
            ),
        ],
      ),
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
    );
  }

  @override
  Widget build(BuildContext context) {
    return ListTile(
      contentPadding: const EdgeInsets.symmetric(horizontal: 4),
      tileColor: selected ? MicaTheme.of(context).surface.hover : null,
      // The icon is the only thing telling you this row will take you somewhere
      // different — a folder locates, a page opens.
      leading: Icon(
        result.isFolder ? Icons.folder_outlined : Icons.description_outlined,
        size: 18,
      ),
      // Three lines, Notion's shape: WHAT it is, WHERE it lives, WHY it matched.
      // Each answer gets its own line — squeezing the path onto the title row
      // (the earlier cut of this) reads as one run-on line, and the eye has to
      // find the boundary before it can use either half.
      isThreeLine: true,
      title: Text(result.name, overflow: TextOverflow.ellipsis),
      subtitle: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        mainAxisSize: MainAxisSize.min,
        children: [
          if (path.isNotEmpty)
            Text(
              path.join(' / '),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: MicaTypography.micro.copyWith(
                color: MicaTheme.of(context).text.faint,
              ),
            ),
          // One line, not two. With the path above it the row is already three
          // deep, and a snippet free to wrap is what made the old list look
          // busy — the window around the hit is the point, not the paragraph.
          _matchLine(context),
        ],
      ),
      onTap: onTap,
    );
  }

  /// WHY this row is here: the matched text, or — for a hit with no body to
  /// quote — what it matched on instead. A folder has no body at all, so
  /// leaving this blank would make the row look like a mistake.
  Widget _matchLine(BuildContext context) {
    if (result.snippet.isNotEmpty) return _snippet(context, result.snippet);
    final style = MicaTypography.caption.copyWith(
      color: MicaTheme.of(context).text.faint,
    );
    if (result.isFolder) {
      return Text(context.l10n.searchFolderMatch, style: style, maxLines: 1);
    }
    if (result.titleMatch) {
      return Text(context.l10n.searchTitleMatch, style: style, maxLines: 1);
    }
    return const SizedBox.shrink();
  }
}

/// System prompt for AI that should produce a whole document with a title line.
