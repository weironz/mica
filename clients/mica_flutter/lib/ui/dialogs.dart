// Modal dialogs for the Mica client (search / settings / AI /
// recycle bin). `part of main.dart` — same library, so they keep using its
// imports and private helpers. Extracted 2026-07 for navigability.
part of '../main.dart';

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
        borderRadius: BorderRadius.circular(5),
      ),
      padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
      child: Text(
        label,
        style: TextStyle(
          fontSize: 11,
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

/// The graph view, in a dialog. Loads ONCE when opened: the layout is a
/// deterministic one-shot pass, so re-running it per rebuild would be pure waste
/// and would also make the picture jump.
class _GraphDialog extends StatefulWidget {
  const _GraphDialog({
    required this.load,
    required this.onOpen,
    this.currentViewId,
  });

  final Future<PageGraph> Function() load;
  final void Function(String viewId) onOpen;
  final String? currentViewId;

  @override
  State<_GraphDialog> createState() => _GraphDialogState();
}

class _GraphDialogState extends State<_GraphDialog> {
  late final Future<PageGraph> _graph = widget.load();

  @override
  Widget build(BuildContext context) {
    final tokens = MicaTheme.of(context);
    return Dialog(
      insetPadding: const EdgeInsets.all(24),
      child: SizedBox(
        width: 900,
        height: 640,
        child: Column(
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(20, 16, 12, 12),
              child: Row(
                children: [
                  Text(
                    context.l10n.graphTitle,
                    style: TextStyle(
                      fontSize: 16,
                      fontWeight: FontWeight.w600,
                      color: tokens.text.primary,
                    ),
                  ),
                  const Spacer(),
                  IconButton(
                    onPressed: () => Navigator.of(context).pop(),
                    icon: const Icon(Icons.close, size: 20),
                  ),
                ],
              ),
            ),
            Divider(height: 1, color: tokens.border.subtle),
            Expanded(
              child: FutureBuilder<PageGraph>(
                future: _graph,
                builder: (context, snapshot) {
                  if (snapshot.connectionState != ConnectionState.done) {
                    return const Center(child: CircularProgressIndicator());
                  }
                  // A failed load must NOT render as an empty graph — that would
                  // state "nothing links to anything" as a fact.
                  if (snapshot.hasError) {
                    return Center(
                      child: Padding(
                        padding: const EdgeInsets.all(24),
                        child: ErrorBanner('${snapshot.error}'),
                      ),
                    );
                  }
                  return PageGraphView(
                    graph: snapshot.data ?? PageGraph.empty,
                    currentViewId: widget.currentViewId,
                    onOpen: widget.onOpen,
                  );
                },
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _SearchDialog extends StatefulWidget {
  const _SearchDialog({
    required this.onSearch,
    required this.onOpen,
    required this.onReveal,
    this.onSearchAll,
    this.views = const [],
    this.workspaceName,
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
  final void Function(String viewId, String? workspaceId) onOpen;

  /// A FOLDER hit. Folders cannot be opened — there is no document behind one —
  /// so they get their own exit: locate the folder in the sidebar tree instead.
  /// Routing them through [onOpen] silently did nothing at all, because the
  /// open path already refuses folders.
  final void Function(String viewId) onReveal;

  /// Pre-filled query, run immediately on open — e.g. clicking a page-property
  /// tag opens search already looking for that tag.
  final String? initialQuery;

  @override
  State<_SearchDialog> createState() => _SearchDialogState();
}

class _SearchDialogState extends State<_SearchDialog> {
  final _query = TextEditingController();
  Timer? _debounce;
  bool _loading = false;
  bool _failed = false;
  List<SearchResult> _results = const [];
  String _lastQuery = '';

  /// Keyboard-highlighted row, -1 when none. Separate from "hovered": the whole
  /// point is to be able to pick a result without touching the mouse.
  int _selected = -1;

  /// Whether the query runs across every workspace.
  ///
  /// Off by default and NOT remembered between openings. Cross-workspace search
  /// reads every visible document's body, so it costs roughly N times the
  /// per-workspace one — a preference that quietly persisted would make search
  /// permanently slow for someone who tried it once.
  bool _allWorkspaces = false;

  /// The last failure was "this server has no cross-workspace search", not a
  /// network problem. Kept apart from [_failed] so the panel can say which.
  bool _scopeUnsupported = false;
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
    // The toggle picks the endpoint. `onSearch` still gates the whole thing:
    // a world with no search at all has neither.
    final search = _allWorkspaces
        ? (widget.onSearchAll ?? widget.onSearch)
        : widget.onSearch;
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
      final results = await search(query);
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
      // A 404 while searching ALL workspaces is not a broken network: it is a
      // server too old to have `GET /search` (added 2026-08-12). Mica is
      // self-hosted, so old servers are a normal thing to be talking to, and
      // telling that user to check their network sends them to debug the one
      // thing that is working. Observed for real against a v0.13.17 node.
      final unsupported =
          _allWorkspaces && e is ApiException && e.statusCode == 404;
      if (mounted) {
        setState(() {
          _loading = false;
          _failed = true;
          _scopeUnsupported = unsupported;
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
            // The scope toggle. A checkbox rather than a mode the panel
            // remembers: it costs roughly N times a normal search, so it should
            // be a thing you reach for, not a state you can end up in.
            if (widget.onSearchAll != null) ...[
              const SizedBox(height: 6),
              Align(
                alignment: Alignment.centerLeft,
                child: InkWell(
                  borderRadius: BorderRadius.circular(6),
                  onTap: () {
                    setState(() => _allWorkspaces = !_allWorkspaces);
                    // Re-run immediately: flipping the scope with a query
                    // already typed and leaving the old results on screen would
                    // show the previous scope's answer under the new label.
                    _run(_query.text);
                  },
                  child: Padding(
                    padding: const EdgeInsets.symmetric(
                      horizontal: 6,
                      vertical: 4,
                    ),
                    child: Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        Icon(
                          _allWorkspaces
                              ? Icons.check_box
                              : Icons.check_box_outline_blank,
                          size: 16,
                          color: _allWorkspaces
                              ? MicaTheme.of(context).accent.primary
                              : MicaTheme.of(context).text.muted,
                        ),
                        const SizedBox(width: 6),
                        Text(
                          context.l10n.searchAllWorkspaces,
                          style: TextStyle(
                            fontSize: 12,
                            color: MicaTheme.of(context).text.muted,
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
              ),
            ],
            const SizedBox(height: 12),
            Expanded(child: _buildResults(context)),
            // Only with results on screen: advertising ↑↓/↵ over an empty list
            // would promise keys that do nothing.
            if (_results.isNotEmpty) ...[
              const Divider(height: 17),
              Row(
                children: [
                  const _KeyCap('↑'),
                  const SizedBox(width: 4),
                  const _KeyCap('↓'),
                  const SizedBox(width: 6),
                  Text(
                    context.l10n.searchHintSelect,
                    style: TextStyle(
                      fontSize: 12,
                      color: MicaTheme.of(context).text.faint,
                    ),
                  ),
                  const SizedBox(width: 12),
                  const _KeyCap('↵'),
                  const SizedBox(width: 6),
                  Text(
                    context.l10n.searchHintOpen,
                    style: TextStyle(
                      fontSize: 12,
                      color: MicaTheme.of(context).text.faint,
                    ),
                  ),
                  const Spacer(),
                  // In-page find, NOT 「全局搜索 ⌘⇧F」: that is the shortcut that
                  // opened this very dialog (self-referential), and Ctrl is the
                  // right glyph for a Windows-first product.
                  Text(
                    context.l10n.searchHintInPage,
                    style: TextStyle(
                      fontSize: 12,
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
      widget.onReveal(result.viewId);
    } else {
      widget.onOpen(result.viewId, result.workspaceId);
    }
  }

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

  /// The pre-typing list. Rows route through the same [_SearchDialog.onOpen] as
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
            style: TextStyle(fontSize: 12, color: theme.text.faint),
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
                    ? Text(entry.icon!, style: const TextStyle(fontSize: 16))
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
                  style: TextStyle(fontSize: 12, color: theme.text.faint),
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
            style: TextStyle(fontSize: 12, color: theme.text.faint),
          ),
        ),
        for (var i = 0; i < hits.length; i++)
          Material(
            color: _selected == i ? theme.accent.wash : Colors.transparent,
            borderRadius: BorderRadius.circular(6),
            child: InkWell(
              borderRadius: BorderRadius.circular(6),
              onTap: () => widget.onOpenWorkspace?.call(hits[i].id),
              child: Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: 8,
                  vertical: 8,
                ),
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
                      style: TextStyle(fontSize: 12, color: theme.text.faint),
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
      if (_scopeUnsupported) {
        // Not a failure the user can retry their way out of — the action is to
        // untick the box, so that is the action offered.
        return EmptyState(
          icon: Icons.update,
          title: context.l10n.searchAllUnsupportedTitle,
          detail: context.l10n.searchAllUnsupportedDetail,
          actionLabel: context.l10n.searchAllUnsupportedAction,
          onAction: () {
            setState(() {
              _allWorkspaces = false;
              _scopeUnsupported = false;
            });
            _run(_query.text);
          },
        );
      }
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
              style: TextStyle(
                fontSize: 11,
                fontWeight: FontWeight.w600,
                letterSpacing: 0.6,
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
              path: ancestorPathSegments(
                workspaceName: widget.workspaceName,
                parentViewId: _results[i].parentViewId,
                views: widget.views,
                startedAt: _results[i].viewId,
              ),
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
      tileColor: selected ? MicaTheme.of(context).accent.wash : null,
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
              style: TextStyle(
                fontSize: 11,
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
    final style = TextStyle(
      fontSize: 12,
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
const String kAiDocSystem =
    'You are a writing assistant inside a Markdown document editor. Respond with '
    'clean GitHub-Flavored Markdown only — no preamble or commentary. Begin the '
    'document with a single level-1 heading (a "# Title" line).';

enum _AiTarget { newPage, currentPage, newWorkspace }

/// Preset AI providers. Each maps to a backend provider dialect (openai/anthropic)
/// plus default base URL + model; "Local / Custom" lets the user point at any
/// OpenAI-compatible server (Ollama, LM Studio, vLLM, …).
/// Endpoint shortcuts for the providers people actually reach for. A preset
/// only fills the Base URL — nothing here is a supported-provider list, because
/// anything speaking the OpenAI or Anthropic wire format works once its URL is
/// typed in, and pretending otherwise would mean shipping a list that goes
/// stale every time a vendor appears.
///
/// Deliberately NOT here: model names. Every one of these vendors renames and
/// retires models on its own schedule, so a name baked into this build is wrong
/// soon after it ships — and wrong in the worst way, since it looks like a
/// working default. The model list is fetched from the provider instead
/// ([_fetchModels]); [model] below is only a seed for a blank form.
enum _AiPreset { deepseek, zhipu, kimi, openai, anthropic, custom }

extension _AiPresetInfo on _AiPreset {
  String get label => switch (this) {
    _AiPreset.deepseek => 'DeepSeek',
    _AiPreset.zhipu => '智谱 GLM',
    _AiPreset.kimi => '月之暗面 Kimi',
    _AiPreset.openai => 'OpenAI',
    _AiPreset.anthropic => 'Anthropic (Claude)',
    _AiPreset.custom => 'Local / Custom',
  };

  /// The wire FORMAT. Several vendors speak `openai`; this is what the request
  /// code branches on, not what the dropdown selects.
  String get provider => this == _AiPreset.anthropic ? 'anthropic' : 'openai';

  /// The VENDOR id, and the key this provider's config is stored under. Must
  /// match the ids migration 0022 attributes existing rows to.
  String get id => switch (this) {
    _AiPreset.deepseek => 'deepseek',
    _AiPreset.zhipu => 'zhipu',
    _AiPreset.kimi => 'kimi',
    _AiPreset.openai => 'openai',
    _AiPreset.anthropic => 'anthropic',
    _AiPreset.custom => 'custom',
  };

  /// Corroborated against cc-switch's provider presets, which are maintained
  /// against these endpoints daily; the ones I could not corroborate are
  /// deliberately absent rather than guessed — an endpoint that 404s is worse
  /// than an empty dropdown, because it looks like the key is wrong.
  String get baseUrl => switch (this) {
    _AiPreset.deepseek => 'https://api.deepseek.com',
    _AiPreset.zhipu => 'https://open.bigmodel.cn/api/coding/paas/v4',
    _AiPreset.kimi => 'https://api.moonshot.cn/v1',
    _AiPreset.openai => 'https://api.openai.com/v1',
    _AiPreset.anthropic => 'https://api.anthropic.com',
    _AiPreset.custom => 'http://localhost:11434/v1',
  };

  /// Always blank. There is no seed because a seeded name is a claim about the
  /// vendor's catalogue that this build cannot keep.
  ///
  /// It was `deepseek-chat` here, and on 2026-08-19 a live `/v1/models` against
  /// api.deepseek.com returned exactly `deepseek-v4-flash` and
  /// `deepseek-v4-pro` — `deepseek-chat` was gone. The instance in production
  /// was still configured with it. That is the whole failure mode in one
  /// example: a retired name does not look broken, it looks like a working
  /// default, right up until a request fails for reasons that point nowhere
  /// near this line. The fetch button beside the field is the answer that
  /// cannot go stale.
  String get model => '';
}

/// The provider presets, exposed for the regression that forbids shipping model
/// NAMES (see test/ai_preset_no_baked_models_test.dart). The enum itself stays
/// private — this is a read-only window onto it, not a second way to build one.
@visibleForTesting
List<({String label, String seedModel, String baseUrlValue})>
get aiPresetsForTest => [
  for (final preset in _AiPreset.values)
    (
      label: preset.label,
      seedModel: preset.model,
      baseUrlValue: preset.baseUrl,
    ),
];

/// Settings dialog. Currently hosts the AI provider configuration; appearance and
/// account sections will slot in alongside it.
class _SettingsDialog extends StatefulWidget {
  const _SettingsDialog({
    required this.onLoadAiSettings,
    required this.onListAiModels,
    required this.onSaveAiSettings,
    this.onLoadTokens,
    this.onCreateToken,
    this.onRevokeToken,
    required this.userName,
    required this.userEmail,
    required this.onUpdateProfile,
    required this.currentAvatarUrl,
    required this.onChangeAvatar,
    required this.onRemoveAvatar,
    required this.onChangePassword,
    required this.onDeleteAccount,
    required this.appearance,
    required this.pageWidth,
    required this.reHostImages,
    required this.onReHostImagesChanged,
    required this.showFormatBar,
    required this.onShowFormatBarChanged,
    required this.showPageTitle,
    required this.onShowPageTitleChanged,
    required this.aiEnabled,
    required this.onAiEnabledChanged,
    required this.onAppearanceChanged,
    required this.onImportWorkspace,
    this.onExportAllWorkspaces,
    this.onLoadExportStats,
    this.onLoadCacheStats,
    this.onClearMirrorCache,
  });

  final String userName;
  final String userEmail;

  /// Null in 本地模式 — there is no account to edit, so the Account tab is not
  /// offered at all. Null, not a do-nothing function: the same distinction
  /// [onLoadTokens] already draws. A no-op here rendered a whole page of live
  /// controls — a Display name you could type into, a Save button, a password
  /// change — that silently did nothing.
  final Future<void> Function(String displayName)? onUpdateProfile;

  /// The signed-in user's picture URL when Settings opens (null = none). Only
  /// the starting value: this dialog is a route, so it never sees the rebuild an
  /// upload causes upstream — from then on it follows what the actions report.
  final String? Function() currentAvatarUrl;

  /// Null in 本地模式 — no account, so no picture to change. Each returns the
  /// URL that is true afterwards.
  final Future<String?> Function()? onChangeAvatar;
  final Future<String?> Function()? onRemoveAvatar;
  final Future<void> Function(String current, String next)? onChangePassword;

  /// Null in 本地模式 — no account to delete. Deletes the cloud account and
  /// every workspace it owns (server cascade); the caller signs out on success.
  final Future<void> Function(String password)? onDeleteAccount;

  /// Null in 本地模式 — AI settings live on the server, so there is nothing to
  /// configure and the tab is absent. Null, not a no-op: same rule as
  /// [onUpdateProfile] and [onLoadTokens]. These two were the stragglers, and
  /// a no-op here meant a whole provider form — base URL, model, API key —
  /// that took your typing and dropped it.
  final Future<Map<String, dynamic>> Function()? onLoadAiSettings;

  /// Ask the provider what models it has. Null in 本地模式 and on a server too
  /// old to have the route — the model field stays free text either way, which
  /// is the same fallback a failed fetch lands on.
  final Future<Map<String, dynamic>> Function({
    String? provider,
    String? baseUrl,
    String? apiKey,
  })?
  onListAiModels;
  /// Writes one provider's config and makes it active; answers with that
  /// provider's stored state. Returns the payload so a switch can populate the
  /// form from the server rather than from a local guess.
  final Future<Map<String, dynamic>> Function({
    required String provider,
    required String providerId,
    required String baseUrl,
    required String model,
    String? apiKey,
  })?
  onSaveAiSettings;
  final Future<List<Map<String, dynamic>>> Function()? onLoadTokens;
  final Future<Map<String, dynamic>> Function(
    String name,
    List<String> scopes,
    int? expiresInDays,
  )?
  onCreateToken;
  final Future<void> Function(String id)? onRevokeToken;
  final EditorAppearance appearance;
  final double pageWidth;
  final bool reHostImages;
  final void Function(bool value) onReHostImagesChanged;
  final bool showFormatBar;
  final void Function(bool value) onShowFormatBarChanged;
  final bool showPageTitle;
  final void Function(bool value) onShowPageTitleChanged;
  final bool aiEnabled;
  final void Function(bool value) onAiEnabledChanged;
  final void Function(EditorAppearance appearance, double pageWidth)
  onAppearanceChanged;
  /// Null hides the import zone. Local mode has no archive importer, and an
  /// upload target that silently does nothing is worse than no target at all.
  final Future<void> Function()? onImportWorkspace;

  /// Null in 本地模式 — "export all workspaces" is a cloud endpoint
  /// (`GET /api/workspaces/export.zip`, every workspace this account belongs
  /// to). Null hides the button rather than offering a dead control.
  final Future<void> Function()? onExportAllWorkspaces;

  /// Counts for the whole-account export. Null where there is nothing to
  /// describe (本地模式 has no cross-workspace export), and a failure just leaves
  /// the meta line absent — a number nobody can act on is not worth an error.
  final Future<({int workspaces, int pages, int imageBytes})> Function()?
  onLoadExportStats;

  /// What the on-device store holds. Null on web (there is no on-device store).
  final Future<LocalCacheStats> Function()? onLoadCacheStats;

  /// Reclaim the mirrored half. Returns the numbers that hold afterwards.
  final Future<LocalCacheStats> Function()? onClearMirrorCache;

  @override
  State<_SettingsDialog> createState() => _SettingsDialogState();
}

class _SettingsDialogState extends State<_SettingsDialog> {
  final _baseUrl = TextEditingController();
  final _model = TextEditingController();
  final _apiKey = TextEditingController();
  _AiPreset _preset = _AiPreset.deepseek;

  /// The AI fields commit when they lose focus — settings here apply as you
  /// touch them, and a text field's equivalent of a toggle flipping is you
  /// finishing with it. Not on every keystroke: each save is a network call,
  /// and half an API key is not a value anyone wants stored.
  late final List<FocusNode> _aiFocus = [
    for (var i = 0; i < 3; i++) FocusNode()..addListener(_saveAiOnBlur),
  ];

  /// What the server already has, so blurring an untouched field is silent.
  String _aiSaved = '';

  String get _aiNow =>
      '${_preset.provider}|${_baseUrl.text.trim()}|${_model.text.trim()}|'
      '${_apiKey.text.trim()}';

  /// Index into the built tab list. Not persisted, so reordering the list is
  /// safe — 0 is whatever comes first (外观).
  int _tab = 0;

  /// Gates the WHOLE dialog (build: `_loading ? spinner : the tabs`), but the
  /// only thing it ever waits for is [_load]'s AI-settings fetch. So it starts
  /// true only when there is a fetch: in 本地模式 [onLoadAiSettings] is null,
  /// _load returns straight away, and every line that clears this sits after
  /// that return — Settings was a spinner that never resolved.
  late bool _loading = widget.onLoadAiSettings != null;
  bool _saving = false;
  bool _hasKey = false;
  /// Last 4 characters of the saved key, when there is one. The field itself
  /// can never show the key — the server does not return it — so without this
  /// the dialog had only a row of dots as a hint, which reads exactly like a
  /// filled-in field and left "is a key set?" unanswerable.
  String _keyHint = '';
  /// Whether this account may change the instance-wide AI settings. Instance
  /// settings carry the operator's provider key, so only an admin may.
  bool _canEdit = true;
  /// Models the provider itself reported, empty until fetched. Not seeded from
  /// any built-in list: a stale name that looks official is worse than no list.
  List<String> _models = const [];
  bool _fetchingModels = false;
  String? _modelsError;
  // API Tokens tab state.
  List<Map<String, dynamic>>? _tokens;
  bool _tokensLoaded = false;
  bool _tokenBusy = false;
  ({int workspaces, int pages, int imageBytes})? _exportStats;
  bool _exportStatsAsked = false;

  /// What the avatar circle shows right now. Seeded from the parent, then kept
  /// current by what each action reports back.
  String? _avatarUrl;
  LocalCacheStats? _cacheStats;
  bool _cacheBusy = false;

  /// Result of the last clear, shown in the Data section — NOT in `_accountMsg`,
  /// which only renders under Account. Reporting "freed 3.2 MB" into a tab the
  /// user is not looking at is the same as not reporting it: the numbers just
  /// changed and nothing said why.
  String? _cacheMsg;
  bool _cacheStatsAsked = false;
  bool _tokenWrite = false;
  final _tokenName = TextEditingController();

  /// Chosen token lifetime in days; null = never expires.
  ///
  /// Was a free-text numeric field parsed with `int.tryParse`, so any
  /// non-number silently became null — you typed a lifetime, agreed to it,
  /// and were handed a token that never expires. A closed set of choices
  /// removes the failure instead of validating against it.
  int? _tokenExpiryDays;
  String? _tokensError;
  String? _error;

  late final _name = TextEditingController(text: widget.userName);
  final _curPass = TextEditingController();
  final _newPass = TextEditingController();
  bool _accountBusy = false;
  String? _accountMsg;

  late double _fontScale = widget.appearance.fontScale;
  late String? _fontFamily = widget.appearance.fontFamily;
  late double _pageWidth = widget.pageWidth;
  late bool _reHostImages = widget.reHostImages;

  /// Read straight from prefs rather than passed in: nothing else in the app
  /// reacts to it, so threading it through the widget tree would be ceremony.
  late bool _diagnostics = diagnosticsOn;
  late bool _showFormatBar = widget.showFormatBar;
  // Read straight from prefs rather than threaded through widget params: the
  // window layer owns this one, and Settings is its only editor.
  late String _closeBehavior = loadCloseBehavior();
  late bool _showPageTitle = widget.showPageTitle;
  late bool _aiEnabled = widget.aiEnabled;

  @override
  void initState() {
    super.initState();
    _load();
    // Prefetch the export numbers rather than waiting for the Data tab to be
    // opened: it is one aggregate query, and asking on first paint of that tab
    // would render the export row without its meta line and then reflow under
    // the user.
    _avatarUrl = widget.currentAvatarUrl();
    _ensureExportStats();
    _ensureCacheStats();
  }

  void _applyAppearance() {
    widget.onAppearanceChanged(
      EditorAppearance(fontScale: _fontScale, fontFamily: _fontFamily),
      _pageWidth,
    );
  }

  /// Page width as 11 discrete stops (AppFlowy-style), plus a reset to the
  /// readable default. Fixed range (not window-relative) so the stops are stable
  /// "levels"; the editor caps the render at the window, so a wide stop on a
  /// small window just fills it. Reset → [kPageWidthDefault], NOT full width.
  Widget _pageWidthRow(BuildContext context) {
    final w = _pageWidth.clamp(kPageWidthMin, kPageWidthMax);
    final atDefault = w.round() == kPageWidthDefault.round();
    return Row(
      children: [
        SizedBox(width: 90, child: Text(context.l10n.settingsPageWidth)),
        Expanded(
          child: Slider(
            value: w,
            min: kPageWidthMin,
            max: kPageWidthMax,
            divisions: kPageWidthDivisions,
            onChanged: (value) {
              setState(() => _pageWidth = value);
              _applyAppearance();
            },
          ),
        ),
        SizedBox(
          width: 52,
          child: Text(
            '${w.round()} px',
            textAlign: TextAlign.right,
            style: TextStyle(
              color: MicaTheme.of(context).text.muted,
              fontSize: 13,
            ),
          ),
        ),
        IconButton(
          tooltip: context.l10n.settingsResetPageWidth,
          visualDensity: VisualDensity.compact,
          onPressed: atDefault
              ? null
              : () {
                  setState(() => _pageWidth = kPageWidthDefault);
                  _applyAppearance();
                },
          icon: const Icon(Icons.restart_alt, size: 18),
        ),
      ],
    );
  }

  Widget _sliderRow({
    required String label,
    required double value,
    required double min,
    required double max,
    required String display,
    required ValueChanged<double> onChanged,
  }) {
    return Row(
      children: [
        SizedBox(width: 90, child: Text(label)),
        Expanded(
          child: Slider(value: value, min: min, max: max, onChanged: onChanged),
        ),
        SizedBox(
          width: 56,
          child: Text(
            display,
            textAlign: TextAlign.right,
            style: TextStyle(
              color: MicaTheme.of(context).text.muted,
              fontSize: 13,
            ),
          ),
        ),
      ],
    );
  }

  Widget _fontChip(String label, String? family) {
    return ChoiceChip(
      label: Text(label),
      selected: _fontFamily == family,
      onSelected: (_) {
        setState(() => _fontFamily = family);
        _applyAppearance();
      },
    );
  }

  /// Theme chip. Writes the pref and flips [themeModeController], which rebuilds
  /// MaterialApp — this open dialog included, so the switch is visible on the
  /// very surface you flipped it from.
  Widget _themeChip(String label, MicaThemeMode mode) {
    return ChoiceChip(
      label: Text(label),
      selected: themeModeController.value == mode,
      onSelected: (_) => setState(() {
        savePref('themeMode', themeModePref(mode));
        themeModeController.value = mode;
      }),
    );
  }

  /// UI-language chip. Writes through [setLanguage] (persists + flips
  /// localeController), which rebuilds MaterialApp — the whole app, including
  /// this open dialog, re-renders in the chosen language immediately.
  Widget _langChip(String label, String choice) {
    return ChoiceChip(
      label: Text(label),
      selected: currentLanguageChoice == choice,
      onSelected: (_) => setState(() => setLanguage(choice)),
    );
  }

  @override
  void dispose() {
    _baseUrl.dispose();
    _model.dispose();
    _apiKey.dispose();
    for (final f in _aiFocus) {
      f.dispose();
    }
    _name.dispose();
    _curPass.dispose();
    _newPass.dispose();
    _tokenName.dispose();
    super.dispose();
  }

  /// Pick and upload a new picture. A dismissed picker is not an error and
  /// says nothing; only a failed upload gets a message.
  Future<void> _changeAvatar() => _runAvatarAction(widget.onChangeAvatar!);

  Future<void> _removeAvatar() => _runAvatarAction(widget.onRemoveAvatar!);

  Future<void> _runAvatarAction(Future<String?> Function() action) async {
    setState(() {
      _accountBusy = true;
      _accountMsg = null;
    });
    try {
      final url = await action();
      if (mounted) setState(() => _avatarUrl = url);
    } catch (error) {
      if (mounted) setState(() => _accountMsg = error.toString());
    } finally {
      if (mounted) setState(() => _accountBusy = false);
    }
  }

  Future<void> _saveProfile() async {
    final l10n = context.l10n;
    setState(() {
      _accountBusy = true;
      _accountMsg = null;
    });
    try {
      await widget.onUpdateProfile!(_name.text.trim());
      // Stay open: saving is not leaving. ("like server config" — the thing it
      // copied — no longer closes either; nothing in Settings does.)
      if (mounted) setState(() => _accountMsg = l10n.accountSaved);
    } catch (error) {
      if (mounted) setState(() => _accountMsg = error.toString());
    } finally {
      if (mounted) setState(() => _accountBusy = false);
    }
  }

  Future<void> _changeAccountPassword() async {
    final l10n = context.l10n;
    if (_newPass.text.length < 8) {
      setState(() => _accountMsg = l10n.accountPasswordTooShort);
      return;
    }
    setState(() {
      _accountBusy = true;
      _accountMsg = null;
    });
    try {
      await widget.onChangePassword!(_curPass.text, _newPass.text);
      if (!mounted) return;
      // Stay open, and clear the fields — leaving a password sitting in a live
      // text box is the reason closing felt like the tidy option.
      _curPass.clear();
      _newPass.clear();
      // Accurate, not reassuring: change_password revokes EVERY family of this
      // user (auth.rs `revoke_user_sessions`) — this device included. Saying
      // "other devices" would be a lie, and the surprise would land later, when
      // this session quietly fails to renew.
      setState(() => _accountMsg = l10n.accountPasswordChanged);
    } catch (error) {
      if (mounted) setState(() => _accountMsg = error.toString());
    } finally {
      if (mounted) setState(() => _accountBusy = false);
    }
  }

  Future<void> _deleteAccount() async {
    // Password re-entry lives in the confirm dialog, not the Account form: it is
    // asked for once, right next to the irreversible warning, and never sits in
    // a live field afterwards. A local controller keeps it out of _SettingsDialog
    // state entirely — it dies with the dialog.
    final passCtl = TextEditingController();
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (dialogContext) {
        final l10n = dialogContext.l10n;
        return AlertDialog(
          title: Text(l10n.accountDeleteTitle),
          content: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                l10n.accountDeleteWarning,
                style: TextStyle(
                  color: MicaTheme.of(context).text.muted,
                  fontSize: 13,
                ),
              ),
              const SizedBox(height: 16),
              TextField(
                controller: passCtl,
                obscureText: true,
                autofocus: true,
                decoration: InputDecoration(
                  labelText: l10n.accountDeletePasswordLabel,
                  border: const OutlineInputBorder(),
                ),
              ),
            ],
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.of(dialogContext).pop(false),
              child: Text(l10n.commonCancel),
            ),
            FilledButton(
              style: FilledButton.styleFrom(
                backgroundColor: MicaTheme.of(context).status.danger,
              ),
              onPressed: () => Navigator.of(dialogContext).pop(true),
              child: Text(l10n.accountDeleteConfirm),
            ),
          ],
        );
      },
    );
    if (confirmed != true) {
      passCtl.dispose();
      return;
    }
    final password = passCtl.text;
    passCtl.dispose();
    setState(() {
      _accountBusy = true;
      _accountMsg = null;
    });
    try {
      await widget.onDeleteAccount!(password);
      // Success tears down the session (the account is gone) — the parent's
      // callback signs out, which pops Settings with it. Nothing to show here.
    } catch (error) {
      if (mounted) {
        setState(() {
          _accountMsg = error.toString();
          _accountBusy = false;
        });
      }
    }
  }

  Future<void> _load() async {
    // 本地模式: no AI provider to load, and no AI tab to load it into. initState
    // calls this unconditionally, so the absence has to be handled here.
    final load = widget.onLoadAiSettings;
    if (load == null) return;
    try {
      final settings = await load();
      if (!mounted) return;
      setState(() => _loading = false);
      // Same population path a provider switch takes, so a reload and a switch
      // cannot end up showing different things.
      _adoptSettings(settings);
    } catch (error) {
      if (!mounted) return;
      setState(() {
        _error = error.toString();
        _loading = false;
      });
    }
  }

  /// Ask the provider for its models, using the values currently in the form so
  /// a provider can be evaluated before it is saved. A failure is reported and
  /// nothing else changes — the model field stays a text box, which is the same
  /// affordance as before this button existed.
  Future<void> _fetchModels() async {
    final fetch = widget.onListAiModels;
    if (fetch == null || _fetchingModels) return;
    setState(() {
      _fetchingModels = true;
      _modelsError = null;
    });
    try {
      final result = await fetch(
        provider: _preset.provider,
        baseUrl: _baseUrl.text.trim(),
        // Only send a key the user just typed; an empty one tells the server to
        // use the stored key, which is the common case on a configured server.
        apiKey: _apiKey.text.trim().isEmpty ? null : _apiKey.text.trim(),
      );
      if (!mounted) return;
      final models = (result['models'] as List?)?.cast<String>() ?? const [];
      setState(() {
        _models = models;
        _fetchingModels = false;
        _modelsError = models.isEmpty ? context.l10n.aiModelsEmpty : null;
      });
    } catch (error) {
      if (!mounted) return;
      setState(() {
        _fetchingModels = false;
        _modelsError = _modelsFailure(context, error);
      });
    }
  }

  /// Turn a fetch failure into something the reader can act on.
  ///
  /// The raw exception was `not found` — four grey words that read like the
  /// button did nothing, when in fact the answer was specific and actionable:
  /// this route does not exist on the server being talked to, i.e. the api is
  /// older than this build. Status codes, not message text: the message is
  /// English server prose and matching on it lets a reworded sentence break
  /// this silently.
  String _modelsFailure(BuildContext context, Object error) {
    final status = error is ApiException ? error.statusCode : null;
    return switch (status) {
      404 => context.l10n.aiModelsRouteMissing,
      401 || 403 => context.l10n.aiModelsRejected,
      _ => error is ApiException ? error.message : error.toString(),
    };
  }

  /// Which preset a stored `provider_id` corresponds to. Falls back to matching
  /// the base URL for rows written before providers had ids.
  _AiPreset _presetFor(String providerId, String provider, String base) {
    for (final preset in _AiPreset.values) {
      if (preset.id == providerId) return preset;
    }
    if (provider == 'anthropic') return _AiPreset.anthropic;
    if (base.contains('deepseek')) return _AiPreset.deepseek;
    if (base.contains('bigmodel.cn') || base.contains('z.ai')) return _AiPreset.zhipu;
    if (base.contains('moonshot.cn')) return _AiPreset.kimi;
    if (base.contains('openai.com')) return _AiPreset.openai;
    return base.isEmpty ? _AiPreset.deepseek : _AiPreset.custom;
  }

  /// Switch provider by ASKING THE SERVER for that vendor's own config.
  ///
  /// It used to rewrite the fields locally from the preset table, which is what
  /// made switching incoherent: the URL and model changed while the key — a
  /// single instance-wide value — stayed behind, so the screen showed a green
  /// "key configured" for a provider that had never had one, and a warning
  /// underneath explaining the contradiction. Each vendor now has its own row
  /// (migration 0022), so the switch is a write that returns that row and every
  /// field follows it. Nothing is left over, and the warning is gone rather
  /// than reworded.
  Future<void> _applyPreset(_AiPreset preset) async {
    final save = widget.onSaveAiSettings;
    if (save == null) return;
    setState(() {
      _preset = preset;
      _saving = true;
      // These belong to the provider being left.
      _models = const [];
      _modelsError = null;
    });
    try {
      final result = await save(
        provider: preset.provider,
        providerId: preset.id,
        // Only a seed: the server keeps this vendor's stored URL when it has one.
        baseUrl: preset.baseUrl,
        model: '',
      );
      if (!mounted) return;
      _adoptSettings(result);
    } catch (error) {
      if (!mounted) return;
      setState(() {
        _error = error.toString();
        _saving = false;
      });
    }
  }

  /// Populate every field from one settings payload — the single place that
  /// decides what the screen shows, so a switch and a reload cannot disagree.
  void _adoptSettings(Map<String, dynamic> settings) {
    final providerId = settings['provider_id'] as String? ?? '';
    final provider = settings['provider'] as String? ?? 'openai';
    final base = settings['base_url'] as String? ?? '';
    final model = settings['model'] as String? ?? '';
    setState(() {
      _preset = _presetFor(providerId, provider, base);
      _baseUrl.text = base.isEmpty ? _preset.baseUrl : base;
      _model.text = model;
      _apiKey.clear();
      _hasKey = settings['has_key'] == true;
      _keyHint = settings['key_hint'] as String? ?? '';
      _canEdit = settings['can_edit'] as bool? ?? true;
      _saving = false;
      _error = null;
    });
    _aiSaved = _aiNow;
    // A configured provider with no model is configured-but-unusable, and the
    // user cannot pick a name they have no way to know.
    if (_hasKey && _model.text.trim().isEmpty) unawaited(_fetchModels());
  }

  /// The built-in About popup, marking the current app version. Opened from the
  /// Settings nav's "About" item (stacks over the Settings dialog).
  void _showAboutDialog(BuildContext context) {
    showAboutDialog(
      context: context,
      applicationName: 'Mica',
      applicationVersion: 'v$kAppVersion',
      applicationIcon: const MicaLogo(size: 40),
      applicationLegalese: context.l10n.aboutLegalese,
      children: [
        const SizedBox(height: 16),
        // AGPL-3.0-or-later §13: anyone interacting with Mica over a network must
        // be prominently offered the Corresponding Source. Link the public repo;
        // the version shown above tells them which tag to check out.
        Align(
          alignment: Alignment.centerLeft,
          child: TextButton.icon(
            onPressed: () => openUrl('https://github.com/weironz/mica'),
            icon: const Icon(Icons.code, size: 16),
            label: Text(context.l10n.aboutSourceCode),
          ),
        ),
        // Self-update lives here on desktop; hidden where it can't apply (web, and
        // platforms with no packaged installer).
        if (updateSupported) ...const [SizedBox(height: 8), UpdateChecker()],
      ],
    );
  }

  Future<void> _showTokenSecret(Map<String, dynamic> created) {
    final token = created['token'] as String? ?? '';
    return showDialog<void>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text(context.l10n.tokenCreated),
        content: SizedBox(
          width: 420,
          // Green, and only here: this is the one moment the secret exists in the
          // UI, and the panel has to look different from every other grey box so
          // 「立即复制」 is not read as boilerplate.
          child: Container(
            decoration: BoxDecoration(
              color: MicaTheme.of(context).status.successWash,
              border: Border.all(
                color: MicaTheme.of(
                  context,
                ).status.success.withValues(alpha: 0.45),
                width: 1.5,
              ),
              borderRadius: BorderRadius.circular(14),
            ),
            padding: const EdgeInsets.all(14),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Row(
                  children: [
                    Icon(
                      Icons.check_circle_outline,
                      size: 16,
                      color: MicaTheme.of(context).status.success,
                    ),
                    const SizedBox(width: 7),
                    Expanded(
                      child: Text(
                        context.l10n.tokenCopyNow,
                        style: TextStyle(
                          fontSize: 12.5,
                          height: 1.5,
                          color: MicaTheme.of(context).status.success,
                        ),
                      ),
                    ),
                  ],
                ),
                const SizedBox(height: 11),
                Container(
                  width: double.infinity,
                  padding: const EdgeInsets.symmetric(
                    horizontal: 11,
                    vertical: 10,
                  ),
                  decoration: BoxDecoration(
                    color: MicaTheme.of(context).surface.base,
                    border: Border.all(
                      color: MicaTheme.of(
                        context,
                      ).status.success.withValues(alpha: 0.3),
                    ),
                    borderRadius: BorderRadius.circular(8),
                  ),
                  // Monospace via kMonoFont, not 'monospace': that family name
                  // does not resolve on web (model.dart). Selectable so the
                  // secret can still be taken by hand if the clipboard is
                  // locked down.
                  child: SelectableText(
                    token,
                    maxLines: 1,
                    style: const TextStyle(fontSize: 13, fontFamily: kMonoFont),
                  ),
                ),
              ],
            ),
          ),
        ),
        actions: [
          // The secret is unrecoverable, so "did that work?" is a question
          // worth answering — but the button answers it in place. A snackbar
          // here had to crawl out from under the dialog to be seen at all.
          InlineCopyButton(
            label: context.l10n.commonCopy,
            tooltip: context.l10n.commonCopy,
            size: 18,
            onCopy: () async {
              await Clipboard.setData(ClipboardData(text: token));
              return true;
            },
          ),
          FilledButton(
            onPressed: () => Navigator.of(ctx).pop(),
            child: Text(context.l10n.commonDone),
          ),
        ],
      ),
    );
  }

  /// One settings nav row. Active = accent wash + accent ink + 600, rather than
  /// Material's `selected` (a tinted title only), which at 180px read as barely
  /// distinguishable from its neighbours.
  Widget _navRow(BuildContext context, int i, String title, IconData icon) {
    final active = _tab == i;
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 1),
      child: Material(
        color: active ? MicaTheme.of(context).accent.wash : Colors.transparent,
        borderRadius: BorderRadius.circular(8),
        child: InkWell(
          borderRadius: BorderRadius.circular(8),
          onTap: () => setState(() => _tab = i),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 9),
            child: Row(
              children: [
                Icon(
                  icon,
                  size: 18,
                  color: active
                      ? MicaTheme.of(context).accent.primary
                      : MicaTheme.of(context).text.muted,
                ),
                const SizedBox(width: 10),
                Expanded(
                  child: Text(
                    title,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      fontSize: 13,
                      fontWeight: active ? FontWeight.w600 : FontWeight.w400,
                      color: active
                          ? MicaTheme.of(context).accent.primary
                          : MicaTheme.of(context).text.primary,
                    ),
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  /// Whether this token's `expires_at` is already in the past.
  ///
  /// An unparseable or absent value reads as NOT expired: guessing "expired"
  /// would dim a live token and invite someone to revoke a key their backups
  /// still depend on.
  bool _tokenExpired(dynamic expiresAt) {
    if (expiresAt == null) return false;
    final at = DateTime.tryParse(expiresAt.toString());
    if (at == null) return false;
    return at.isBefore(DateTime.now());
  }

  /// Read-only vs read-write, as a badge.
  ///
  /// The meta line used to print the server's own scope list — `read` /
  /// `read, write` — straight at the user: English, comma-joined, and in a
  /// Chinese UI. What a person needs from it is one bit, and write access is the
  /// half worth colouring.
  Widget _scopeBadge(BuildContext context, dynamic scopes) {
    final list =
        (scopes as List<dynamic>?)?.map((e) => '$e').toList() ?? const [];
    final canWrite = list.contains('write');
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 7, vertical: 1),
      decoration: BoxDecoration(
        color: canWrite
            ? MicaTheme.of(context).status.warningWash
            : MicaTheme.of(context).surface.hover,
        borderRadius: BorderRadius.circular(5),
      ),
      child: Text(
        canWrite ? context.l10n.tokenScopeWrite : context.l10n.tokenScopeRead,
        style: TextStyle(
          fontSize: 11,
          fontWeight: FontWeight.w500,
          color: canWrite
              ? MicaTheme.of(context).status.warning
              : MicaTheme.of(context).text.muted,
        ),
      ),
    );
  }

  /// Not static: a null timestamp means "no expiry", and that word has to be
  /// localized — it shipped as the English literal `never`, so the Chinese UI
  /// read 「过期 never」 while `tokenNever` sat unused behind the input hint.
  String _shortTime(dynamic value) {
    if (value == null) return context.l10n.tokenNever;
    final s = value.toString();
    return s.length >= 10 ? s.substring(0, 10) : s;
  }

  /// Revocation takes effect on the server the instant it returns: every cron
  /// job, script and CLI holding this token starts getting 401s. It used to
  /// fire straight off the icon button, so one mis-click silently broke someone
  /// else's backup with no way to put the secret back.
  Future<bool> _confirmRevoke(String label) {
    final l10n = context.l10n;
    return showDestructiveConfirm(
      context,
      title: l10n.tokenRevokeConfirmTitle(label),
      body: l10n.tokenRevokeConfirmBody,
      confirmLabel: l10n.tokenRevoke,
      cancelLabel: l10n.commonCancel,
    );
  }

  List<Widget> _tokensSection(BuildContext context) {
    final onLoad = widget.onLoadTokens;
    final onCreate = widget.onCreateToken;
    final onRevoke = widget.onRevokeToken;
    if (onLoad == null || onCreate == null || onRevoke == null) {
      return const [];
    }

    // Lazy-load the list the first time the tab is shown.
    if (!_tokensLoaded) {
      _tokensLoaded = true;
      onLoad()
          .then((list) {
            if (mounted) setState(() => _tokens = list);
          })
          .catchError((Object e) {
            if (mounted) setState(() => _tokensError = e.toString());
          });
    }

    Future<void> refresh() async {
      try {
        final list = await onLoad();
        if (mounted) {
          setState(() {
            _tokens = list;
            _tokensError = null;
          });
        }
      } catch (e) {
        if (mounted) setState(() => _tokensError = e.toString());
      }
    }

    return [
      MicaEyebrow(context.l10n.tokenTitle, icon: Icons.key_outlined),
      const SizedBox(height: 4),
      Text(
        context.l10n.tokenDescription,
        style: Theme.of(context).textTheme.bodySmall?.copyWith(
          color: MicaTheme.of(context).text.muted,
        ),
      ),
      const SizedBox(height: 16),
      TextField(
        controller: _tokenName,
        decoration: InputDecoration(
          labelText: context.l10n.tokenName,
          hintText: context.l10n.tokenNameHint,
          border: const OutlineInputBorder(),
          isDense: true,
        ),
      ),
      const SizedBox(height: 10),
      Row(
        children: [
          Switch(
            value: _tokenWrite,
            onChanged: _tokenBusy
                ? null
                : (v) => setState(() => _tokenWrite = v),
          ),
          Text(context.l10n.tokenWriteAccess),
          const Spacer(),
          SizedBox(
            width: 160,
            child: DropdownButtonFormField<int?>(
              initialValue: _tokenExpiryDays,
              isDense: true,
              decoration: InputDecoration(
                labelText: context.l10n.tokenExpires,
                border: const OutlineInputBorder(),
                isDense: true,
              ),
              // `never` stays first and stays the default: it is what shipped,
              // and quietly starting to expire tokens people already treat as
              // permanent would break their cron jobs on a timer.
              items: [
                DropdownMenuItem(
                  value: null,
                  child: Text(context.l10n.tokenNever),
                ),
                for (final d in const [30, 90, 365])
                  DropdownMenuItem(
                    value: d,
                    child: Text(context.l10n.tokenExpiryDays(d)),
                  ),
              ],
              onChanged: _tokenBusy
                  ? null
                  : (v) => setState(() => _tokenExpiryDays = v),
            ),
          ),
        ],
      ),
      const SizedBox(height: 10),
      Align(
        alignment: Alignment.centerLeft,
        child: FilledButton.icon(
          icon: const Icon(Icons.add, size: 18),
          label: Text(context.l10n.tokenCreate),
          onPressed: _tokenBusy
              ? null
              : () async {
                  final name = _tokenName.text.trim();
                  if (name.isEmpty) {
                    setState(
                      () => _tokensError = context.l10n.tokenNameRequired,
                    );
                    return;
                  }
                  setState(() {
                    _tokenBusy = true;
                    _tokensError = null;
                  });
                  try {
                    final scopes = _tokenWrite
                        ? <String>['read', 'write']
                        : <String>['read'];
                    final days = _tokenExpiryDays;
                    final created = await onCreate(name, scopes, days);
                    _tokenName.clear();

                    if (mounted) {
                      setState(() {
                        _tokenWrite = false;
                        _tokenBusy = false;
                      });
                      await _showTokenSecret(created);
                    }
                    await refresh();
                  } catch (e) {
                    if (mounted) {
                      setState(() {
                        _tokensError = e.toString();
                        _tokenBusy = false;
                      });
                    }
                  }
                },
        ),
      ),
      if (_tokensError != null) ...[
        const SizedBox(height: 12),
        ErrorBanner(_tokensError!),
      ],
      const SizedBox(height: 18),
      const Divider(height: 1),
      const SizedBox(height: 8),
      if (_tokens == null)
        const Padding(
          padding: EdgeInsets.all(16),
          child: Center(child: CircularProgressIndicator()),
        )
      else if (_tokens!.isEmpty)
        // A title with no body and no next step was the shipped state. No action
        // button: the next step is the form directly above this, so a button
        // would just point at itself.
        EmptyState(
          icon: Icons.key_outlined,
          title: context.l10n.tokenNone,
          detail: context.l10n.tokenEmptyBody,
        )
      else
        for (final t in _tokens!)
          Opacity(
            // An expired token still lists (you may want to revoke it), but it
            // can't do anything — reading as live is the misleading part.
            opacity: _tokenExpired(t['expires_at']) ? 0.62 : 1,
            child: ListTile(
              contentPadding: EdgeInsets.zero,
              dense: true,
              title: Row(
                children: [
                  Flexible(
                    child: Text(
                      t['name'] as String? ?? context.l10n.tokenUnnamed,
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
                  const SizedBox(width: 8),
                  _scopeBadge(context, t['scopes']),
                ],
              ),
              subtitle: Text(
                context.l10n.tokenMeta(
                  _shortTime(t['last_used_at']),
                  _shortTime(t['expires_at']),
                ),
              ),
              trailing: IconButton(
                icon: const Icon(Icons.delete_outline, size: 20),
                tooltip: context.l10n.tokenRevoke,
                onPressed: _tokenBusy
                    ? null
                    : () async {
                        final label =
                            t['name'] as String? ?? context.l10n.tokenUnnamed;
                        if (!await _confirmRevoke(label)) return;
                        setState(() => _tokenBusy = true);
                        try {
                          await onRevoke(t['id'] as String);
                          await refresh();
                        } catch (e) {
                          if (mounted)
                            setState(() => _tokensError = e.toString());
                        } finally {
                          if (mounted) setState(() => _tokenBusy = false);
                        }
                      },
              ),
            ),
          ),
    ];
  }

  /// Commit the AI fields when one loses focus — and only if something actually
  /// changed, so tabbing through untouched fields is silent.
  void _saveAiOnBlur() {
    if (_loading || _aiFocus.any((f) => f.hasFocus)) return;
    if (_aiNow == _aiSaved) return;
    unawaited(_saveAi());
  }

  /// Persist the AI provider settings. No longer behind a Save button: every
  /// other setting here applies as you touch it (toggles, sliders), and this was
  /// the only holdout — the button existed for these three text fields and then
  /// sat under every page, saving AI settings no matter what you were looking at.
  Future<void> _saveAi() async {
    // Unreachable in 本地模式 (the blur listeners belong to fields that only the
    // AI tab builds), but this is what makes that a fact rather than a hope.
    final save = widget.onSaveAiSettings;
    if (save == null) return;
    final attempt = _aiNow;
    setState(() {
      _saving = true;
      _error = null;
    });
    try {
      await save(
        provider: _preset.provider,
        providerId: _preset.id,
        baseUrl: _baseUrl.text.trim(),
        model: _model.text.trim(),
        apiKey: _apiKey.text.trim().isEmpty ? null : _apiKey.text.trim(),
      );
      if (!mounted) return;
      // Stay open. Saving is not leaving — the dialog closes when the user says
      // so, which is the whole point of settings that apply as you go.
      setState(() {
        _saving = false;
        _aiSaved = attempt;
      });
    } catch (error) {
      if (!mounted) return;
      setState(() {
        _saving = false;
        _error = error.toString();
      });
    }
  }

  List<Widget> _appearanceSection(BuildContext context) => [
    MicaEyebrow(context.l10n.settingsAppearance, icon: Icons.tune),
    const SizedBox(height: 12),
    Row(
      children: [
        SizedBox(width: 90, child: Text(context.l10n.languageLabel)),
        Expanded(
          child: Wrap(
            spacing: 8,
            children: [
              _langChip(context.l10n.languageSystem, kLangSystem),
              _langChip(context.l10n.languageChinese, kLangChinese),
              _langChip(context.l10n.languageEnglish, kLangEnglish),
            ],
          ),
        ),
      ],
    ),
    const SizedBox(height: 8),
    Row(
      children: [
        SizedBox(width: 90, child: Text(context.l10n.themeLabel)),
        Expanded(
          child: Wrap(
            spacing: 8,
            children: [
              _themeChip(context.l10n.themeSystem, MicaThemeMode.system),
              _themeChip(context.l10n.themeLight, MicaThemeMode.light),
              _themeChip(context.l10n.themeDark, MicaThemeMode.dark),
            ],
          ),
        ),
      ],
    ),
    const SizedBox(height: 8),
    _pageWidthRow(context),
    _sliderRow(
      label: context.l10n.settingsFontSize,
      value: _fontScale,
      min: 0.85,
      max: 1.4,
      display: '${(_fontScale * 100).round()}%',
      onChanged: (value) {
        setState(() => _fontScale = value);
        _applyAppearance();
      },
    ),
    const SizedBox(height: 4),
    Row(
      children: [
        SizedBox(width: 90, child: Text(context.l10n.settingsFont)),
        Expanded(
          child: Wrap(
            spacing: 8,
            children: [
              _fontChip(context.l10n.settingsFontSystem, null),
              _fontChip('Serif', 'serif'),
              _fontChip('Mono', kMonoFont),
            ],
          ),
        ),
      ],
    ),
    const SizedBox(height: 8),
    SwitchListTile(
      contentPadding: EdgeInsets.zero,
      dense: true,
      value: _reHostImages,
      title: Text(context.l10n.settingsReHostImages),
      subtitle: Text(context.l10n.settingsReHostImagesSub),
      onChanged: (value) {
        setState(() => _reHostImages = value);
        widget.onReHostImagesChanged(value);
      },
    ),
    SwitchListTile(
      contentPadding: EdgeInsets.zero,
      dense: true,
      value: _showFormatBar,
      title: Text(context.l10n.settingsFormatBar),
      subtitle: Text(context.l10n.settingsFormatBarSub),
      onChanged: (value) {
        setState(() => _showFormatBar = value);
        widget.onShowFormatBarChanged(value);
      },
    ),
    SwitchListTile(
      contentPadding: EdgeInsets.zero,
      dense: true,
      value: _showPageTitle,
      title: Text(context.l10n.settingsPageTitle),
      subtitle: Text(context.l10n.settingsPageTitleSub),
      onChanged: (value) {
        setState(() => _showPageTitle = value);
        widget.onShowPageTitleChanged(value);
      },
    ),
    // Desktop only — a browser tab's close button belongs to the browser, and
    // no app code can intercept it.
    if (!kIsWeb) ...[
      const SizedBox(height: 8),
      Text(
        context.l10n.closeWindowHeader,
        style: TextStyle(
          fontWeight: FontWeight.w600,
          color: MicaTheme.of(context).text.muted,
        ),
      ),
      // RadioGroup, not per-tile groupValue/onChanged — those were deprecated
      // after Flutter 3.32 in favour of this ancestor.
      RadioGroup<String>(
        groupValue: _closeBehavior,
        onChanged: (value) {
          if (value == null) return;
          setState(() => _closeBehavior = value);
          saveCloseBehavior(value);
        },
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            for (final option in _closeBehaviorOptions)
              RadioListTile<String>(
                contentPadding: EdgeInsets.zero,
                dense: true,
                value: option.$1,
                title: Text(option.$2),
                subtitle: Text(option.$3),
              ),
          ],
        ),
      ),
    ],
  ];

  /// The X-button choices. "Ask every time" is not offered as a standing
  /// setting — it is only the pre-answer default; once you have answered, an
  /// explicit choice is what you want, and the question is reachable again by
  /// picking a different option here.
  ///
  /// Tray is Windows-only for now (see `trayIsSupported`): where it is not
  /// available, offering it would promise a restore path we cannot deliver.
  List<(String, String, String)> get _closeBehaviorOptions => [
    (kCloseQuit, context.l10n.closeQuitTitle, context.l10n.closeQuitSub),
    if (trayIsSupported)
      (kCloseTray, context.l10n.closeTrayTitle, context.l10n.closeTraySub),
  ];

  List<Widget> _aiSection(BuildContext context) => [
    MicaEyebrow(context.l10n.settingsAiProvider, icon: Icons.auto_awesome),
    SwitchListTile(
      contentPadding: EdgeInsets.zero,
      dense: true,
      value: _aiEnabled,
      title: Text(context.l10n.aiEnable),
      subtitle: Text(context.l10n.aiEnableSub),
      onChanged: (value) {
        setState(() => _aiEnabled = value);
        widget.onAiEnabledChanged(value);
      },
    ),
    const SizedBox(height: 12),
    DropdownButtonFormField<_AiPreset>(
      initialValue: _preset,
      decoration: InputDecoration(
        labelText: context.l10n.aiProviderLabel,
        border: const OutlineInputBorder(),
      ),
      items: _AiPreset.values
          .map(
            (preset) =>
                DropdownMenuItem(value: preset, child: Text(preset.label)),
          )
          .toList(),
      onChanged: (_saving || !_canEdit)
          ? null
          : (preset) {
              if (preset != null) _applyPreset(preset);
            },
    ),
    const SizedBox(height: 12),
    TextField(
      controller: _baseUrl,
      focusNode: _aiFocus[0],
      enabled: !_saving && _canEdit,
      decoration: InputDecoration(
        labelText: context.l10n.aiBaseUrl,
        hintText: 'https://api.deepseek.com',
        border: const OutlineInputBorder(),
      ),
    ),
    const SizedBox(height: 12),
    // Model stays a TEXT FIELD with a fetch button beside it, rather than a
    // dropdown alone. A dropdown would have to be populated from somewhere, and
    // the only honest sources are the provider (which needs a network round
    // trip and a key) or a baked-in list (which is wrong within weeks). So the
    // field always accepts a typed name, and the button turns it into a
    // pick-from-list once the provider has answered.
    Row(
      children: [
        Expanded(
          child: TextField(
            controller: _model,
            focusNode: _aiFocus[1],
            enabled: !_saving && _canEdit,
            decoration: InputDecoration(
              labelText: context.l10n.aiModel,
              // No example name here either — a hint is still a claim, and this
              // one named a model DeepSeek has since retired.
              hintText: context.l10n.aiModelHint,
              border: const OutlineInputBorder(),
            ),
          ),
        ),
        if (widget.onListAiModels != null) ...[
          const SizedBox(width: 8),
          Tooltip(
            message: context.l10n.aiFetchModels,
            child: IconButton.outlined(
              onPressed: (_saving || !_canEdit || _fetchingModels)
                  ? null
                  : _fetchModels,
              icon: _fetchingModels
                  ? const SizedBox(
                      width: 16,
                      height: 16,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    )
                  : const Icon(Icons.cloud_download_outlined, size: 18),
            ),
          ),
        ],
      ],
    ),
    if (_models.isNotEmpty) ...[
      const SizedBox(height: 8),
      // Chips rather than a second dropdown: the list is short enough to scan,
      // and it keeps the typed field as the single source of the saved value —
      // there is no second control that could disagree with it.
      Wrap(
        spacing: 6,
        runSpacing: 6,
        children: [
          for (final id in _models)
            ChoiceChip(
              label: Text(id, style: const TextStyle(fontSize: 12)),
              selected: _model.text.trim() == id,
              onSelected: (_saving || !_canEdit)
                  ? null
                  : (_) => setState(() => _model.text = id),
            ),
        ],
      ),
    ],
    if (_models.isEmpty && _model.text.trim().isEmpty && _modelsError == null) ...[
      const SizedBox(height: 6),
      Row(
        children: [
          Icon(
            Icons.error_outline,
            size: 15,
            color: MicaTheme.of(context).text.muted,
          ),
          const SizedBox(width: 6),
          Expanded(
            child: Text(
              context.l10n.aiModelMissing,
              style: Theme.of(context).textTheme.bodySmall?.copyWith(
                color: MicaTheme.of(context).text.muted,
              ),
            ),
          ),
        ],
      ),
    ],
    if (_modelsError != null) ...[
      const SizedBox(height: 6),
      // Danger colour + icon: rendered in the muted grey the hints use, a
      // failure was indistinguishable from a caption and the button looked
      // inert. A failed action has to look different from an explanation.
      Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Icon(
            Icons.error_outline,
            size: 15,
            color: MicaTheme.of(context).status.danger,
          ),
          const SizedBox(width: 6),
          Expanded(
            child: Text(
              _modelsError!,
              style: Theme.of(context).textTheme.bodySmall?.copyWith(
                color: MicaTheme.of(context).status.danger,
              ),
            ),
          ),
        ],
      ),
    ],
    const SizedBox(height: 12),
    // The saved key is never returned, so the field is always empty on open.
    // It used to say so with a row of dots as the hint — which looks exactly
    // like a filled-in password field, so "did I ever set this?" had no answer.
    // A badge answers it, and the last 4 characters answer the follow-up
    // ("is it the key I think it is?") without revealing anything usable.
    Row(
      children: [
        Icon(
          _hasKey ? Icons.check_circle : Icons.error_outline,
          size: 16,
          color: _hasKey
              ? MicaTheme.of(context).status.success
              : MicaTheme.of(context).text.muted,
        ),
        const SizedBox(width: 6),
        Text(
          _hasKey
              ? (_keyHint.isEmpty
                    ? context.l10n.aiKeyConfigured
                    : context.l10n.aiKeyConfiguredHint(_keyHint))
              : context.l10n.aiKeyMissing,
          style: Theme.of(context).textTheme.bodySmall?.copyWith(
            color: _hasKey
                ? MicaTheme.of(context).status.success
                : MicaTheme.of(context).text.muted,
          ),
        ),
      ],
    ),
    const SizedBox(height: 8),
    TextField(
      controller: _apiKey,
      focusNode: _aiFocus[2],
      enabled: !_saving && _canEdit,
      obscureText: true,
      decoration: InputDecoration(
        labelText: context.l10n.aiApiKey,
        // Float the label unconditionally so the hint is actually VISIBLE. With
        // Material's default the label sits inside an empty field and the hint
        // is hidden underneath it — so the dots that say "a key is stored,
        // leave this blank to keep it" were never once seen, and the field read
        // as "nothing configured" even while the badge above said otherwise.
        floatingLabelBehavior: FloatingLabelBehavior.always,
        hintText: _hasKey
            ? context.l10n.aiApiKeyHintHasKey
            : context.l10n.aiApiKeyHintRequired,
        border: const OutlineInputBorder(),
      ),
    ),
    const SizedBox(height: 6),
    Text(
      context.l10n.aiKeyHelp,
      style: Theme.of(
        context,
      ).textTheme.bodySmall?.copyWith(color: MicaTheme.of(context).text.muted),
    ),
    if (!_canEdit) ...[
      const SizedBox(height: 8),
      Text(
        context.l10n.aiAdminOnly,
        style: Theme.of(context).textTheme.bodySmall?.copyWith(
          color: MicaTheme.of(context).text.muted,
        ),
      ),
    ],
    if (_error != null) ...[const SizedBox(height: 12), ErrorBanner(_error!)],
  ];

  List<Widget> _accountSection(BuildContext context) => [
    MicaEyebrow(context.l10n.settingsAccount, icon: Icons.person_outline),
    const SizedBox(height: 10),
    if (widget.onChangeAvatar != null) ...[
      Row(
        children: [
          UserAvatar(
            url: _avatarUrl,
            radius: 28,
            fallback: widget.userName.isNotEmpty
                ? widget.userName.substring(0, 1).toUpperCase()
                : '?',
          ),
          const SizedBox(width: 14),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  widget.userEmail,
                  style: Theme.of(context).textTheme.bodySmall?.copyWith(
                    color: MicaTheme.of(context).text.muted,
                  ),
                ),
                const SizedBox(height: 8),
                Wrap(
                  spacing: 8,
                  children: [
                    OutlinedButton.icon(
                      onPressed: _accountBusy ? null : _changeAvatar,
                      icon: const Icon(Icons.photo_camera_outlined, size: 16),
                      label: Text(context.l10n.accountChangeAvatar),
                    ),
                    // Only offered when there is something to remove — a
                    // greyed-out button here would be one more control that
                    // cannot say why it is dead.
                    if (_avatarUrl != null && widget.onRemoveAvatar != null)
                      TextButton(
                        onPressed: _accountBusy ? null : _removeAvatar,
                        child: Text(context.l10n.accountRemoveAvatar),
                      ),
                  ],
                ),
              ],
            ),
          ),
        ],
      ),
      const SizedBox(height: 16),
    ] else ...[
      const SizedBox(height: 4),
      Text(
        widget.userEmail,
        style: Theme.of(context).textTheme.bodySmall?.copyWith(
          color: MicaTheme.of(context).text.muted,
        ),
      ),
      const SizedBox(height: 12),
    ],
    TextField(
      controller: _name,
      enabled: !_accountBusy,
      decoration: InputDecoration(
        labelText: context.l10n.accountDisplayName,
        border: const OutlineInputBorder(),
      ),
    ),
    const SizedBox(height: 8),
    Align(
      alignment: Alignment.centerLeft,
      child: OutlinedButton.icon(
        onPressed: _accountBusy ? null : _saveProfile,
        icon: const Icon(Icons.save, size: 16),
        label: Text(context.l10n.accountSaveName),
      ),
    ),
    const SizedBox(height: 16),
    TextField(
      controller: _curPass,
      enabled: !_accountBusy,
      obscureText: true,
      decoration: InputDecoration(
        labelText: context.l10n.accountCurrentPassword,
        border: const OutlineInputBorder(),
      ),
    ),
    const SizedBox(height: 8),
    TextField(
      controller: _newPass,
      enabled: !_accountBusy,
      obscureText: true,
      decoration: InputDecoration(
        labelText: context.l10n.accountNewPassword,
        border: const OutlineInputBorder(),
      ),
    ),
    const SizedBox(height: 8),
    Align(
      alignment: Alignment.centerLeft,
      child: OutlinedButton.icon(
        onPressed: _accountBusy ? null : _changeAccountPassword,
        icon: const Icon(Icons.lock_outline, size: 16),
        label: Text(context.l10n.accountChangePassword),
      ),
    ),
    if (_accountMsg != null) ...[
      const SizedBox(height: 10),
      Text(
        _accountMsg!,
        style: TextStyle(color: MicaTheme.of(context).text.muted, fontSize: 13),
      ),
    ],
    if (widget.onDeleteAccount != null) ...[
      const Divider(height: 32),
      Align(
        alignment: Alignment.centerLeft,
        child: OutlinedButton.icon(
          onPressed: _accountBusy ? null : _deleteAccount,
          style: OutlinedButton.styleFrom(
            foregroundColor: MicaTheme.of(context).status.danger,
            side: BorderSide(
              color: MicaTheme.of(context).status.danger.withValues(alpha: 0.5),
            ),
          ),
          icon: const Icon(Icons.delete_forever_outlined, size: 16),
          label: Text(context.l10n.accountDelete),
        ),
      ),
    ],
  ];

  /// Drop the mirrored half of the on-device store, after saying what that costs.
  Future<void> _clearMirrorCache() async {
    final l10n = context.l10n;
    final before = _cacheStats;
    final ok = await showDestructiveConfirm(
      context,
      title: l10n.cacheClearConfirmTitle,
      body: l10n.cacheClearConfirmBody,
      confirmLabel: l10n.cacheClearConfirm,
      cancelLabel: l10n.commonCancel,
      // Recoverable: reconnecting re-caches all of it. See destructive_confirm.
      destructive: false,
    );
    if (!ok || !mounted) return;
    setState(() => _cacheBusy = true);
    try {
      final after = await widget.onClearMirrorCache!();
      if (!mounted) return;
      final freed = (before?.mirroredBytes ?? 0) - after.mirroredBytes;
      setState(() {
        _cacheStats = after;
        // The freed figure is measured, not assumed: a blob that could not be
        // unlinked is still counted in `after`, so this never overstates.
        _cacheMsg = l10n.cacheCleared(formatBytes(freed));
      });
    } catch (error) {
      if (mounted) setState(() => _cacheMsg = error.toString());
    } finally {
      if (mounted) setState(() => _cacheBusy = false);
    }
  }

  /// Ask for the on-device numbers once.
  void _ensureCacheStats() {
    final load = widget.onLoadCacheStats;
    if (load == null || _cacheStatsAsked) return;
    _cacheStatsAsked = true;
    load()
        .then((st) {
          if (mounted) setState(() => _cacheStats = st);
        })
        .catchError((_) {});
  }

  /// Ask for the export numbers once, the first time this tab is built. Failure
  /// leaves the meta line absent rather than surfacing an error — a count nobody
  /// can act on is not worth interrupting the screen for.
  void _ensureExportStats() {
    final load = widget.onLoadExportStats;
    if (load == null || _exportStatsAsked) return;
    _exportStatsAsked = true;
    load()
        .then((st) {
          if (mounted) setState(() => _exportStats = st);
        })
        .catchError((_) {});
  }

  List<Widget> _dataSection(BuildContext context) => [
    MicaEyebrow(context.l10n.settingsData, icon: Icons.import_export),
    const SizedBox(height: 12),
    Text(
      context.l10n.dataImportDescription,
      style: TextStyle(
        color: MicaTheme.of(context).text.muted,
        fontSize: 12.5,
        height: 1.6,
      ),
    ),
    const SizedBox(height: 14),
    // A CLICK zone, not a drop zone: the app accepts no drops (see MicaPickZone),
    // so the copy says 「选择…」 and there is no 「拖到这里」 anywhere.
    if (widget.onImportWorkspace case final import?)
      MicaPickZone(
        icon: Icons.upload_file_outlined,
        title: context.l10n.dataImportButton,
        subtitle: context.l10n.dataImportZoneHint,
        onTap: import,
      ),
    if (widget.onExportAllWorkspaces != null) ...[
      const SizedBox(height: 18),
      MicaEyebrow(context.l10n.commonExport),
      const SizedBox(height: 10),
      // One bordered row rather than a loose button, so export reads as its own
      // thing and not as a second import action.
      Container(
        decoration: BoxDecoration(
          border: Border.all(color: MicaTheme.of(context).border.subtle),
          borderRadius: BorderRadius.circular(12),
        ),
        padding: const EdgeInsets.fromLTRB(14, 10, 10, 10),
        child: Row(
          children: [
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                mainAxisSize: MainAxisSize.min,
                children: [
                  Text(
                    context.l10n.dataExportAllButton,
                    style: const TextStyle(fontSize: 14),
                  ),
                  // Shown only once the numbers are in. Images are called images
                  // rather than "download size": the archive is a zip, Markdown
                  // compresses hard and images do not, so quoting a total here
                  // would be wrong by an amount that depends on the user's own
                  // content.
                  if (_exportStats case final st?)
                    Padding(
                      padding: const EdgeInsets.only(top: 3),
                      child: Text(
                        context.l10n.dataExportAllMeta(
                          st.workspaces,
                          st.pages,
                          formatBytes(st.imageBytes),
                        ),
                        style: TextStyle(
                          fontSize: 12,
                          color: MicaTheme.of(context).text.faint,
                        ),
                      ),
                    ),
                ],
              ),
            ),
            const SizedBox(width: 10),
            OutlinedButton(
              onPressed: () => widget.onExportAllWorkspaces!(),
              child: Text(context.l10n.commonExport),
            ),
          ],
        ),
      ),
    ],
    if (_cacheStats case final c?) ...[
      const SizedBox(height: 18),
      MicaEyebrow(context.l10n.cacheTitle),
      const SizedBox(height: 10),
      // Two lines, never one total. The store holds re-downloadable mirrors AND
      // pages that exist nowhere else; a single "已缓存 X" would invite someone
      // to reclaim the only copy of something. Each line is only drawn when it
      // has content, so a cloud-only user never sees a 「本地独有 0」 row.
      if (c.mirroredPages > 0 || c.mirroredBytes > 0)
        Row(
          children: [
            Expanded(
              child: Text(
                context.l10n.cacheMirrored(
                  c.mirroredPages,
                  formatBytes(c.mirroredBytes),
                ),
                style: TextStyle(
                  fontSize: 12.5,
                  color: MicaTheme.of(context).text.muted,
                ),
              ),
            ),
            // Offered on the mirror line only. The local-only line below has no
            // button and never will: there is nowhere to re-download it from, so
            // a "reclaim" control there would be a delete button wearing the
            // wrong word.
            if (widget.onClearMirrorCache != null)
              TextButton(
                onPressed: _cacheBusy ? null : _clearMirrorCache,
                child: Text(context.l10n.cacheClearButton),
              ),
          ],
        ),
      if (c.localOnlyPages > 0 || c.localOnlyBytes > 0)
        Padding(
          padding: const EdgeInsets.only(top: 4),
          child: Text(
            context.l10n.cacheLocalOnly(
              c.localOnlyPages,
              formatBytes(c.localOnlyBytes),
            ),
            style: TextStyle(
              fontSize: 12.5,
              color: MicaTheme.of(context).text.muted,
            ),
          ),
        ),
      if (_cacheMsg case final msg?) ...[
        const SizedBox(height: 8),
        Text(
          msg,
          style: TextStyle(
            fontSize: 12.5,
            color: MicaTheme.of(context).text.muted,
          ),
        ),
      ],
    ],
    const SizedBox(height: 14),
    Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Icon(
          Icons.info_outline,
          size: 14,
          color: MicaTheme.of(context).text.faint,
        ),
        const SizedBox(width: 7),
        Expanded(
          child: Text(
            context.l10n.dataExportTip,
            style: TextStyle(
              fontSize: 12.5,
              height: 1.6,
              color: MicaTheme.of(context).text.faint,
            ),
          ),
        ),
      ],
    ),
  ];

  Widget _kbd(String text) => Container(
    padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
    decoration: BoxDecoration(
      color: MicaTheme.of(context).surface.sunken,
      borderRadius: BorderRadius.circular(5),
      border: Border.all(color: MicaTheme.of(context).border.strong),
    ),
    child: Text(text, style: const TextStyle(fontSize: 12)),
  );

  /// Opt-in capture, for handing a reproducible bug over instead of describing
  /// it. Off by default and stated plainly, because it records real content.
  List<Widget> _diagnosticsSection(BuildContext context) {
    final l = context.l10n;
    // Errors the app drops by design (an unreachable WebSocket is a state, not a
    // crash — see swallowed.dart). Shown ONLY when something has fired: a row
    // reading "none" every time trains the eye to skip it, and the absent row
    // already says as much. Not gated on the capture switch — there is nothing to
    // record, and the value is being able to look AFTER the fact, which is
    // exactly when arming a switch in advance was not an option.
    final dropped = swallowedSummary();
    return [
      SwitchListTile(
        contentPadding: EdgeInsets.zero,
        dense: true,
        value: _diagnostics,
        title: Text(l.settingsDiagnosticsToggle),
        subtitle: Text(l.settingsDiagnosticsToggleSub),
        onChanged: (value) {
          setDiagnostics(value);
          setState(() => _diagnostics = value);
        },
      ),
      const SizedBox(height: 12),
      Row(
        children: [
          OutlinedButton.icon(
            onPressed: openDiagnosticsFolder,
            icon: const Icon(Icons.folder_open_outlined, size: 18),
            label: Text(l.settingsDiagnosticsFolder),
          ),
          const SizedBox(width: 12),
          Expanded(
            child: SelectableText(
              diagnosticsDir,
              style: Theme.of(context).textTheme.bodySmall,
            ),
          ),
        ],
      ),
      if (dropped != null) ...[
        const SizedBox(height: 12),
        SelectableText(
          '${l.settingsDiagnosticsDropped}: $dropped',
          style: Theme.of(context).textTheme.bodySmall,
        ),
      ],
      const SizedBox(height: 12),
      Text(
        l.settingsDiagnosticsPrivacy,
        style: Theme.of(context).textTheme.bodySmall,
      ),
    ];
  }

  List<Widget> _shortcutsSection(BuildContext context) {
    Widget head(String t) => Padding(
      padding: const EdgeInsets.only(top: 8, bottom: 4),
      child: Text(
        t,
        style: TextStyle(
          fontWeight: FontWeight.w600,
          color: MicaTheme.of(context).text.muted,
        ),
      ),
    );
    Widget row(String keys, String desc) => Padding(
      padding: const EdgeInsets.symmetric(vertical: 5),
      child: Row(
        children: [
          _kbd(keys),
          const SizedBox(width: 16),
          Expanded(child: Text(desc)),
        ],
      ),
    );
    return [
      head(context.l10n.shortcutsGroupApp),
      row('Ctrl + N', context.l10n.shortcutsNewPage),
      row('Ctrl + F', context.l10n.shortcutsFindInPage),
      row('F3 / Shift + F3', context.l10n.shortcutsFindNextPrev),
      row('Ctrl + Shift + F', context.l10n.shortcutsSearchWorkspace),
      row('Ctrl + ,', context.l10n.shortcutsOpenSettings),
      row('F2', context.l10n.shortcutsRename),
      const SizedBox(height: 8),
      head(context.l10n.shortcutsGroupFormat),
      row('Ctrl + B', context.l10n.shortcutsBold),
      row('Ctrl + I', context.l10n.shortcutsItalic),
      row('Ctrl + E', context.l10n.shortcutsInlineCode),
      row('Ctrl + K', context.l10n.shortcutsLink),
      row('Ctrl + Alt + 1…6', context.l10n.shortcutsHeadings),
      row('Ctrl + Alt + 0', context.l10n.shortcutsParagraph),
      row('Tab / Shift + Tab', context.l10n.shortcutsIndent),
      const SizedBox(height: 8),
      head(context.l10n.shortcutsGroupEdit),
      row('Ctrl + Z', context.l10n.shortcutsUndo),
      row('Ctrl + Shift + Z', context.l10n.shortcutsRedo),
      row('Ctrl + A', context.l10n.shortcutsSelectAll),
      row('Ctrl + C / X / V', context.l10n.shortcutsCopyCutPaste),
      row('Ctrl + Shift + V', context.l10n.shortcutsPastePlain),
      row('/', context.l10n.shortcutsSlashMenu),
      const SizedBox(height: 12),
      Text(
        context.l10n.shortcutsNote,
        style: Theme.of(context).textTheme.bodySmall?.copyWith(
          color: MicaTheme.of(context).text.faint,
        ),
      ),
    ];
  }

  @override
  Widget build(BuildContext context) {
    // These are the settings of the world you are in — WHICH world that is gets
    // picked on the account tile, not here. A null callback means the active
    // world does not have that thing, so the tab is absent rather than present
    // and inert. In 本地模式 that leaves three tabs, every one of them a
    // this-device preference, and not one ternary among them.
    //
    // No `onSave` per tab any more: there is no Save button to feed. Each
    // section commits its own changes as they are made.
    // Grouped (design 16). Seven flat rows read as one undifferentiated list;
    // the groups are what make it scannable. Order is by group so the labels can
    // be emitted inline, and a group whose every row is absent prints no label —
    // in 本地模式 the whole 账户 group disappears, which is the same
    // null-means-absent rule the tabs themselves already follow.
    final tabs =
        <({String group, String title, IconData icon, List<Widget> section})>[
          (
            group: context.l10n.settingsGroupGeneral,
            title: context.l10n.settingsAppearance,
            icon: Icons.tune,
            section: _appearanceSection(context),
          ),
          (
            group: context.l10n.settingsGroupGeneral,
            title: context.l10n.settingsShortcuts,
            icon: Icons.keyboard_outlined,
            section: _shortcutsSection(context),
          ),
          // 本地模式 has no account — same reason API Tokens below is absent there.
          if (widget.onUpdateProfile != null)
            (
              group: context.l10n.settingsGroupAccount,
              title: context.l10n.settingsAccount,
              icon: Icons.person_outline,
              section: _accountSection(context),
            ),
          if (widget.onLoadTokens != null)
            (
              group: context.l10n.settingsGroupAccount,
              title: context.l10n.tokenTitle,
              icon: Icons.key_outlined,
              section: _tokensSection(context),
            ),
          (
            group: context.l10n.settingsGroupWorkspace,
            title: context.l10n.settingsData,
            icon: Icons.import_export,
            section: _dataSection(context),
          ),
          // AI settings live on the server — 本地模式 has none to configure.
          if (widget.onLoadAiSettings != null)
            (
              group: context.l10n.settingsGroupOther,
              title: context.l10n.settingsAiProvider,
              icon: Icons.auto_awesome,
              section: _aiSection(context),
            ),
          // Web has no filesystem to capture into, so the tab is simply absent
          // there rather than offering a switch that cannot do anything.
          if (diagnosticsSupported)
            (
              group: context.l10n.settingsGroupOther,
              title: context.l10n.settingsDiagnostics,
              icon: Icons.bug_report_outlined,
              section: _diagnosticsSection(context),
            ),
        ];
    final titles = [for (final t in tabs) t.title];
    final icons = [for (final t in tabs) t.icon];
    final sections = [for (final t in tabs) t.section];
    return AlertDialog(
      title: Row(
        children: [
          const Icon(Icons.settings_outlined, size: 22),
          const SizedBox(width: 8),
          Text(context.l10n.settingsTitle),
        ],
      ),
      contentPadding: const EdgeInsets.fromLTRB(0, 12, 0, 0),
      content: SizedBox(
        width: 720,
        height: 460,
        child: _loading
            ? const Center(child: CircularProgressIndicator())
            : Row(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  SizedBox(
                    width: 180,
                    child: ListView(
                      padding: const EdgeInsets.symmetric(vertical: 4),
                      children: [
                        for (var i = 0; i < titles.length; i++) ...[
                          if (i == 0 || tabs[i].group != tabs[i - 1].group)
                            Padding(
                              padding: EdgeInsets.only(
                                left: 16,
                                right: 8,
                                top: i == 0 ? 4 : 14,
                                bottom: 4,
                              ),
                              child: Text(
                                tabs[i].group.toUpperCase(),
                                style: TextStyle(
                                  fontSize: 11,
                                  fontWeight: FontWeight.w600,
                                  letterSpacing: 0.6,
                                  color: MicaTheme.of(context).text.faint,
                                ),
                              ),
                            ),
                          _navRow(context, i, titles[i], icons[i]),
                        ],
                        const Divider(height: 1),
                        // About isn't a content tab — it pops the version dialog.
                        ListTile(
                          dense: true,
                          leading: const Icon(Icons.info_outline, size: 20),
                          title: Text(context.l10n.aboutTitle),
                          subtitle: const Text('v$kAppVersion'),
                          onTap: () => _showAboutDialog(context),
                        ),
                      ],
                    ),
                  ),
                  const VerticalDivider(width: 1),
                  Expanded(
                    child: SingleChildScrollView(
                      padding: const EdgeInsets.fromLTRB(20, 16, 20, 16),
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        mainAxisSize: MainAxisSize.min,
                        children: sections[_tab],
                      ),
                    ),
                  ),
                ],
              ),
      ),
      // No Save button. Everything here applies as you touch it — toggles and
      // sliders always did, the connection switches on pick, and the AI fields
      // commit when they lose focus. AppFlowy, AFFiNE and Notion all settle in
      // the same place, and the button we had was worse than redundant: it only
      // ever saved the AI section, from under every page.
      //
      // The spinner rides here so a commit in flight is visible without a
      // button to host it.
      actions: [
        if (_saving)
          const Padding(
            padding: EdgeInsets.only(right: 12),
            child: SizedBox(
              width: 16,
              height: 16,
              child: CircularProgressIndicator(strokeWidth: 2),
            ),
          ),
        TextButton(
          onPressed: _saving ? null : () => Navigator.of(context).pop(),
          child: Text(context.l10n.commonClose),
        ),
      ],
    );
  }
}

/// Global AI dialog: type an instruction and pick where the generated content
/// goes — a new page, the current page, or a brand-new workspace.
class _AiDialog extends StatefulWidget {
  const _AiDialog({
    required this.canEdit,
    required this.hasWorkspace,
    required this.onStream,
    required this.onNewPage,
    required this.onCurrentPage,
    required this.onNewWorkspace,
  });

  final bool canEdit;
  final bool hasWorkspace;
  final Stream<String> Function(String prompt, {String? system}) onStream;
  final Future<void> Function(String markdown) onNewPage;
  final Future<void> Function(String markdown)? onCurrentPage;
  final Future<void> Function(String markdown) onNewWorkspace;

  @override
  State<_AiDialog> createState() => _AiDialogState();
}

class _AiDialogState extends State<_AiDialog> {
  final _prompt = TextEditingController();
  final _scroll = ScrollController();
  late _AiTarget _target = widget.hasWorkspace
      ? _AiTarget.newPage
      : _AiTarget.newWorkspace;
  StreamSubscription<String>? _sub;
  bool _streaming = false;
  bool _applying = false;
  bool _done = false;
  final StringBuffer _buffer = StringBuffer();
  String? _error;

  bool get _busy => _streaming || _applying;

  @override
  void dispose() {
    _sub?.cancel();
    _prompt.dispose();
    _scroll.dispose();
    super.dispose();
  }

  void _generate() {
    final prompt = _prompt.text.trim();
    if (prompt.isEmpty) return;
    // New page / new workspace want a document with a title line.
    final system = _target == _AiTarget.currentPage ? null : kAiDocSystem;
    setState(() {
      _streaming = true;
      _done = false;
      _error = null;
      _buffer.clear();
    });
    _sub = widget
        .onStream(prompt, system: system)
        .listen(
          (delta) {
            setState(() => _buffer.write(delta));
            WidgetsBinding.instance.addPostFrameCallback((_) {
              if (_scroll.hasClients) {
                _scroll.jumpTo(_scroll.position.maxScrollExtent);
              }
            });
          },
          onError: (Object error) {
            if (mounted) {
              setState(() {
                _streaming = false;
                _error = error.toString();
              });
            }
          },
          onDone: () {
            if (mounted) {
              setState(() {
                _streaming = false;
                _done = true;
              });
            }
          },
        );
  }

  Future<void> _apply() async {
    final markdown = _buffer.toString().trim();
    if (markdown.isEmpty) return;
    setState(() {
      _applying = true;
      _error = null;
    });
    try {
      switch (_target) {
        case _AiTarget.newPage:
          await widget.onNewPage(markdown);
        case _AiTarget.currentPage:
          await widget.onCurrentPage?.call(markdown);
        case _AiTarget.newWorkspace:
          await widget.onNewWorkspace(markdown);
      }
      if (mounted) Navigator.of(context).pop();
    } catch (error) {
      if (mounted) setState(() => _error = error.toString());
    } finally {
      if (mounted) setState(() => _applying = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final canWriteCurrent = widget.onCurrentPage != null;
    final hasOutput = _buffer.isNotEmpty;
    return AlertDialog(
      title: Row(
        children: [
          Icon(
            Icons.auto_awesome,
            size: 22,
            color: MicaTheme.of(context).editor.alertAccents['important'],
          ),
          const SizedBox(width: 8),
          Text(context.l10n.aiAskTitle),
        ],
      ),
      content: SizedBox(
        width: 520,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            TextField(
              controller: _prompt,
              autofocus: true,
              minLines: 2,
              maxLines: 5,
              enabled: !_busy,
              decoration: InputDecoration(
                hintText: context.l10n.aiPromptHint,
                border: const OutlineInputBorder(),
              ),
            ),
            const SizedBox(height: 12),
            Wrap(
              spacing: 8,
              children: [
                if (widget.canEdit)
                  ChoiceChip(
                    label: Text(context.l10n.aiTargetNewPage),
                    selected: _target == _AiTarget.newPage,
                    onSelected: widget.hasWorkspace && !_busy
                        ? (_) => setState(() => _target = _AiTarget.newPage)
                        : null,
                  ),
                if (widget.canEdit && canWriteCurrent)
                  ChoiceChip(
                    label: Text(context.l10n.aiTargetCurrentPage),
                    selected: _target == _AiTarget.currentPage,
                    onSelected: _busy
                        ? null
                        : (_) =>
                              setState(() => _target = _AiTarget.currentPage),
                  ),
                ChoiceChip(
                  label: Text(context.l10n.aiTargetNewWorkspace),
                  selected: _target == _AiTarget.newWorkspace,
                  onSelected: _busy
                      ? null
                      : (_) => setState(() => _target = _AiTarget.newWorkspace),
                ),
              ],
            ),
            if (hasOutput || _streaming) ...[
              const SizedBox(height: 12),
              Container(
                height: 220,
                width: double.infinity,
                padding: const EdgeInsets.all(12),
                decoration: BoxDecoration(
                  color: MicaTheme.of(context).surface.raised,
                  borderRadius: BorderRadius.circular(8),
                  border: Border.all(
                    color: MicaTheme.of(context).border.normal,
                  ),
                ),
                child: SingleChildScrollView(
                  controller: _scroll,
                  child: Text(
                    _buffer.isEmpty ? '…' : _buffer.toString(),
                    style: const TextStyle(fontSize: 13, height: 1.5),
                  ),
                ),
              ),
              const SizedBox(height: 6),
              Row(
                children: [
                  if (_streaming) ...[
                    const SizedBox(
                      width: 14,
                      height: 14,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    ),
                    const SizedBox(width: 8),
                    Text(context.l10n.aiGenerating),
                  ] else if (_done)
                    Text(
                      context.l10n.aiDoneReview,
                      style: TextStyle(color: MicaTheme.of(context).text.muted),
                    ),
                ],
              ),
            ],
            if (_error != null) ...[
              const SizedBox(height: 12),
              ErrorBanner(_error!),
            ],
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: _busy ? null : () => Navigator.of(context).pop(),
          child: Text(context.l10n.commonCancel),
        ),
        if (!_done)
          FilledButton.icon(
            onPressed: _busy ? null : _generate,
            icon: const Icon(Icons.auto_awesome, size: 18),
            label: Text(
              hasOutput ? context.l10n.aiRegenerate : context.l10n.aiGenerate,
            ),
          )
        else ...[
          TextButton.icon(
            onPressed: _applying ? null : _generate,
            icon: const Icon(Icons.refresh, size: 18),
            label: Text(context.l10n.aiRegenerate),
          ),
          FilledButton.icon(
            onPressed: _applying ? null : _apply,
            icon: _applying
                ? const SizedBox(
                    width: 16,
                    height: 16,
                    child: CircularProgressIndicator(strokeWidth: 2),
                  )
                : const Icon(Icons.check, size: 18),
            label: Text(context.l10n.aiInsert),
          ),
        ],
      ],
    );
  }
}

/// Recycle bin: lists soft-deleted pages and offers restore / delete-forever.
/// Only the roots of each deleted subtree are shown; restoring a root brings its
/// whole subtree back.
/// Publish the open (cloud) document to a public read-only link. Toggle on to
/// mint/return the link, off to revoke it (the public URL 404s at once).
class _ShareDialog extends StatefulWidget {
  const _ShareDialog({
    required this.onLoad,
    required this.onEnable,
    required this.onDisable,
    required this.buildUrl,
  });

  final Future<({bool shared, String? token})> Function() onLoad;
  final Future<String> Function() onEnable; // returns the token
  final Future<void> Function() onDisable;
  final String Function(String token) buildUrl;

  @override
  State<_ShareDialog> createState() => _ShareDialogState();
}

class _ShareDialogState extends State<_ShareDialog> {
  bool _loading = true;
  bool _busy = false;
  String? _error;
  bool _shared = false;
  String? _url;

  @override
  void initState() {
    super.initState();
    _refresh();
  }

  Future<void> _refresh() async {
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final status = await widget.onLoad();
      if (!mounted) return;
      setState(() {
        _shared = status.shared;
        _url = status.token == null ? null : widget.buildUrl(status.token!);
        _loading = false;
      });
    } catch (error) {
      if (!mounted) return;
      setState(() {
        _error = error.toString();
        _loading = false;
      });
    }
  }

  Future<void> _toggle(bool on) async {
    final l10n = context.l10n;
    setState(() => _busy = true);
    try {
      if (on) {
        final token = await widget.onEnable();
        if (!mounted) return;
        setState(() {
          _shared = true;
          _url = widget.buildUrl(token);
        });
      } else {
        await widget.onDisable();
        if (!mounted) return;
        setState(() {
          _shared = false;
          _url = null;
        });
      }
    } catch (error) {
      _snack(l10n.shareActionFailed(error.toString()));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<bool> _copy() async {
    final url = _url;
    if (url == null) return false;
    await Clipboard.setData(ClipboardData(text: url));
    return true;
  }

  void _snack(String message) {
    if (!mounted) return;
    ScaffoldMessenger.of(
      context,
    ).showSnackBar(SnackBar(content: Text(message)));
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: Text(context.l10n.shareTitle),
      content: SizedBox(
        width: 440,
        child: _loading
            ? const SizedBox(
                height: 80,
                child: Center(child: CircularProgressIndicator()),
              )
            : _error != null
            ? SizedBox(
                height: 80,
                child: Center(
                  child: Text(context.l10n.shareLoadFailed(_error!)),
                ),
              )
            : Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  SwitchListTile(
                    contentPadding: EdgeInsets.zero,
                    value: _shared,
                    onChanged: _busy ? null : _toggle,
                    title: Text(context.l10n.sharePublicAccess),
                    subtitle: Text(context.l10n.sharePublicAccessSub),
                  ),
                  if (_shared && _url != null) ...[
                    const SizedBox(height: 12),
                    Container(
                      padding: const EdgeInsets.symmetric(
                        horizontal: 12,
                        vertical: 8,
                      ),
                      decoration: BoxDecoration(
                        color: MicaTheme.of(context).surface.raised,
                        borderRadius: BorderRadius.circular(6),
                      ),
                      child: Row(
                        children: [
                          Expanded(
                            child: SelectableText(
                              _url!,
                              maxLines: 1,
                              style: const TextStyle(fontSize: 13),
                            ),
                          ),
                          InlineCopyButton(
                            tooltip: context.l10n.shareCopyLink,
                            size: 18,
                            onCopy: _copy,
                          ),
                        ],
                      ),
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
}

/// Move / copy a page-or-folder subtree into another cloud workspace. Picking a
/// destination runs a server dry-run (report only, no mutation) so the user
/// sees the counts and any breaking links BEFORE committing. Confirm runs the
/// real transfer and pops with the report + destination name for the caller to
/// refresh + snackbar.
///
/// v1 has no destination-folder picker: everything lands at the destination
/// ROOT (`parent_view_id: null`). See [_WorkspaceShellState._openTransfer].
class _TransferDialog extends StatefulWidget {
  const _TransferDialog({
    required this.copy,
    required this.destinations,
    required this.onTransfer,
  });

  /// true = copy (source kept); false = move (source soft-deleted after copy).
  final bool copy;

  /// Cloud workspaces the subtree can go to — the source is already excluded.
  final List<({String id, String name})> destinations;

  /// Runs one transfer against the picked destination. [dryRun] true = preview
  /// (no mutation); false = commit. Source workspace + view + move/copy are
  /// bound by the caller.
  final Future<TransferReport> Function({
    required String destWorkspaceId,
    required bool dryRun,
  })
  onTransfer;

  @override
  State<_TransferDialog> createState() => _TransferDialogState();
}

class _TransferDialogState extends State<_TransferDialog> {
  String? _destId;
  TransferReport? _preview; // dry-run result for the current destination
  bool _loadingPreview = false;
  bool _submitting = false;
  String? _error;

  ({String id, String name})? get _dest {
    final id = _destId;
    if (id == null) return null;
    return widget.destinations.where((d) => d.id == id).firstOrNull;
  }

  Future<void> _selectDest(String? id) async {
    if (id == null || id == _destId) return;
    setState(() {
      _destId = id;
      _preview = null;
      _error = null;
      _loadingPreview = true;
    });
    try {
      final report = await widget.onTransfer(destWorkspaceId: id, dryRun: true);
      if (!mounted || _destId != id) return; // superseded by a newer pick
      setState(() {
        _preview = report;
        _loadingPreview = false;
      });
    } catch (error) {
      if (!mounted || _destId != id) return;
      setState(() {
        _error = context.l10n.transferFailed(error.toString());
        _loadingPreview = false;
      });
    }
  }

  Future<void> _confirm() async {
    final dest = _dest;
    if (dest == null || _submitting) return;
    setState(() {
      _submitting = true;
      _error = null;
    });
    try {
      final report = await widget.onTransfer(
        destWorkspaceId: dest.id,
        dryRun: false,
      );
      if (!mounted) return;
      Navigator.of(context).pop((report: report, destName: dest.name));
    } catch (error) {
      if (!mounted) return;
      setState(() {
        _error = context.l10n.transferFailed(error.toString());
        _submitting = false;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final report = _preview;
    return AlertDialog(
      title: Text(
        widget.copy ? l10n.transferCopyTitle : l10n.transferMoveTitle,
      ),
      content: SizedBox(
        width: 440,
        child: widget.destinations.isEmpty
            ? Text(l10n.transferNoDestinations)
            : Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  DropdownButtonFormField<String>(
                    initialValue: _destId,
                    isExpanded: true,
                    decoration: InputDecoration(
                      labelText: l10n.transferPickWorkspace,
                      border: const OutlineInputBorder(),
                    ),
                    items: widget.destinations
                        .map(
                          (d) => DropdownMenuItem(
                            value: d.id,
                            child: Text(
                              d.name,
                              overflow: TextOverflow.ellipsis,
                            ),
                          ),
                        )
                        .toList(),
                    onChanged: _submitting ? null : _selectDest,
                  ),
                  const SizedBox(height: 16),
                  if (_loadingPreview)
                    const Padding(
                      padding: EdgeInsets.symmetric(vertical: 8),
                      child: Center(
                        child: SizedBox(
                          height: 20,
                          width: 20,
                          child: CircularProgressIndicator(strokeWidth: 2),
                        ),
                      ),
                    )
                  else if (report != null) ...[
                    Text(
                      // One preview string was shared by both modes, so a move
                      // confirmed itself with 「将复制 …」 — the exact opposite
                      // of what the button was about to do to the source.
                      widget.copy
                          ? l10n.transferPreview(
                              report.documents,
                              report.folders,
                              report.images,
                            )
                          : l10n.transferMovePreview(
                              report.documents,
                              report.folders,
                              report.images,
                            ),
                      style: const TextStyle(fontSize: 14),
                    ),
                    if (report.danglingLinks.isNotEmpty) ...[
                      const SizedBox(height: 10),
                      _TransferNotice(
                        icon: Icons.link_off,
                        color: MicaTheme.of(context).status.warning,
                        text: l10n.transferDanglingWarning(
                          report.danglingLinks.length,
                        ),
                      ),
                    ],
                  ],
                  // A move can't carry version history (checkpoints stay on the
                  // source; the copy starts fresh) — always warn before a move.
                  if (!widget.copy) ...[
                    const SizedBox(height: 10),
                    _TransferNotice(
                      icon: Icons.info_outline,
                      color: MicaTheme.of(context).text.muted,
                      text: l10n.transferVersionNotice,
                    ),
                  ],
                  if (_error != null) ...[
                    const SizedBox(height: 12),
                    Text(
                      _error!,
                      style: TextStyle(
                        color: MicaTheme.of(context).status.danger,
                        fontSize: 13,
                      ),
                    ),
                  ],
                ],
              ),
      ),
      actions: [
        TextButton(
          onPressed: _submitting ? null : () => Navigator.of(context).pop(),
          child: Text(l10n.commonCancel),
        ),
        FilledButton(
          // Enabled only once a destination is picked and no request is in
          // flight — the confirm re-runs the transfer for real (dryRun: false).
          onPressed: (_dest == null || _submitting || _loadingPreview)
              ? null
              : _confirm,
          child: _submitting
              ? const SizedBox(
                  height: 16,
                  width: 16,
                  child: CircularProgressIndicator(strokeWidth: 2),
                )
              : Text(widget.copy ? l10n.transferCopy : l10n.transferMove),
        ),
      ],
    );
  }
}

/// A small icon + text row for the transfer dialog's inline notices (broken
/// links, version-history caveat). Kept tiny and local — not worth a shared
/// widget.
class _TransferNotice extends StatelessWidget {
  const _TransferNotice({
    required this.icon,
    required this.color,
    required this.text,
  });

  final IconData icon;
  final Color color;
  final String text;

  @override
  Widget build(BuildContext context) {
    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Icon(icon, size: 16, color: color),
        const SizedBox(width: 8),
        Expanded(
          child: Text(text, style: TextStyle(fontSize: 13, color: color)),
        ),
      ],
    );
  }
}

/// Version history for the open (cloud) document: list named checkpoints, pin
/// the current state as a new one, or roll back to an old one. Restore reflects
/// live in the editor via the normal sync path (the server broadcasts it as an
/// update), so there is no manual reload here.
/// Version history as a two-pane modal (AFFiNE/Notion shape): a read-only
/// preview of the selected version on the left, the timeline on the right, a
/// restore action at the bottom. The preview reuses the SAME editor in
/// `canEdit: false` — no separate renderer, no HTML (see version-history-plan).
class _VersionHistoryDialog extends StatefulWidget {
  const _VersionHistoryDialog({
    required this.onList,
    required this.onCreate,
    required this.onRestore,
    required this.onLoadContent,
    this.onLoadImageBytes,
    this.onResolveImageUrls,
    this.authorNames = const {},
  });

  /// User id → display name, for [DocVersion.createdBy].
  ///
  /// Empty is the normal case and means "don't show an author column": local
  /// history has no user ids at all, and in a one-person workspace a column
  /// repeating your own name down every row is noise, not information.
  final Map<String, String> authorNames;

  final Future<List<DocVersion>> Function() onList;
  final Future<void> Function(String name) onCreate;
  final Future<void> Function(String versionId) onRestore;

  /// A version's blocks (tree order) + root id, for the read-only preview.
  final Future<({String rootBlockId, List<Map<String, dynamic>> blocks})>
  Function(String versionId)
  onLoadContent;

  /// Image handlers passed straight to the preview editor so pictures render.
  final Future<Uint8List?> Function(String fileId)? onLoadImageBytes;
  final Future<Map<String, String>> Function(List<String> fileIds)?
  onResolveImageUrls;

  @override
  State<_VersionHistoryDialog> createState() => _VersionHistoryDialogState();
}

class _VersionHistoryDialogState extends State<_VersionHistoryDialog> {
  bool _loading = true;
  bool _busy = false; // a create/restore is in flight
  String? _error;
  List<DocVersion> _versions = const [];

  // Preview pane state, keyed to the selected version.
  String? _selectedId;
  bool _previewLoading = false;
  String? _previewError;
  ({String rootBlockId, List<Map<String, dynamic>> blocks})? _content;
  // The previous (older) version's content, for the diff. Null = no predecessor
  // (oldest version) → the preview shows no diff tint.
  ({String rootBlockId, List<Map<String, dynamic>> blocks})? _prevContent;

  @override
  void initState() {
    super.initState();
    _refresh();
  }

  Future<void> _refresh() async {
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final vs = await widget.onList();
      if (!mounted) return;
      setState(() {
        _versions = vs;
        _loading = false;
      });
      // Auto-open the newest version so the pane is never blank.
      if (_selectedId == null && vs.isNotEmpty) {
        _select(vs.first);
      }
    } catch (error) {
      if (!mounted) return;
      setState(() {
        _error = error.toString();
        _loading = false;
      });
    }
  }

  /// Load a version's content (and its predecessor's, for the diff) into the
  /// read-only preview pane. The predecessor is the next-OLDER entry in the
  /// timeline (versions are newest-first).
  Future<void> _select(DocVersion v) async {
    setState(() {
      _selectedId = v.id;
      _previewLoading = true;
      _previewError = null;
      _content = null;
      _prevContent = null;
    });
    final idx = _versions.indexWhere((x) => x.id == v.id);
    final prev = (idx >= 0 && idx + 1 < _versions.length)
        ? _versions[idx + 1]
        : null;
    try {
      final content = await widget.onLoadContent(v.id);
      // Predecessor is best-effort: a failure just drops the diff, not the
      // preview.
      ({String rootBlockId, List<Map<String, dynamic>> blocks})? prevContent;
      if (prev != null) {
        try {
          prevContent = await widget.onLoadContent(prev.id);
        } catch (_) {}
      }
      if (!mounted || _selectedId != v.id) return;
      setState(() {
        _content = content;
        _prevContent = prevContent;
        _previewLoading = false;
      });
    } catch (error) {
      if (!mounted || _selectedId != v.id) return;
      setState(() {
        _previewError = error.toString();
        _previewLoading = false;
      });
    }
  }

  /// A block equals its predecessor if kind + text + data all match (a cheap
  /// structural compare; data is order-insensitive via jsonEncode of a sorted
  /// view is overkill here — the block data is small and written consistently).
  bool _blockEqual(Map<String, dynamic> a, Map<String, dynamic> b) {
    return a['type'] == b['type'] &&
        (a['text'] ?? '') == (b['text'] ?? '') &&
        jsonEncode(a['data']) == jsonEncode(b['data']);
  }

  /// The top-level blocks of a version, in tree order (root's children — the
  /// flat shape the editor mounts).
  List<Map<String, dynamic>> _topBlocks(
    ({String rootBlockId, List<Map<String, dynamic>> blocks}) content,
  ) {
    final byId = {for (final b in content.blocks) (b['id'] as String): b};
    final childIds =
        ((byId[content.rootBlockId]?['children'] as List?) ?? const [])
            .cast<String>();
    return [
      for (final id in childIds)
        if (byId[id] != null) byId[id]!,
    ];
  }

  EditorNode _toNode(Map<String, dynamic> b, String? diff) => EditorNode(
    id: b['id'] as String,
    kind: b['type'] as String? ?? 'paragraph',
    text: b['text'] as String? ?? '',
    data: Map<String, dynamic>.from((b['data'] as Map?) ?? const {}),
    diffStatus: diff,
  );

  /// Build read-only editor nodes for the selected version, tagged with a
  /// block-level diff vs the predecessor: added (in this version, not before),
  /// changed (same id, different content), deleted (in the predecessor, gone
  /// now — spliced back in at its old position as a struck-through ghost). No
  /// predecessor → plain nodes, no tint.
  List<EditorNode> _previewNodes(
    ({String rootBlockId, List<Map<String, dynamic>> blocks}) content,
  ) {
    final current = _topBlocks(content);
    final prev = _prevContent;
    if (prev == null) {
      return [for (final b in current) _toNode(b, null)];
    }
    final prevBlocks = _topBlocks(prev);
    final prevById = {for (final b in prevBlocks) (b['id'] as String): b};
    final currentIds = {for (final b in current) b['id'] as String};

    // Group deleted blocks (in prev, not in current) by the surviving block they
    // follow, so they render at roughly their old position ('' = before all).
    final deletedAfter = <String, List<Map<String, dynamic>>>{};
    var lastSurviving = '';
    for (final p in prevBlocks) {
      final pid = p['id'] as String;
      if (currentIds.contains(pid)) {
        lastSurviving = pid;
      } else {
        (deletedAfter[lastSurviving] ??= []).add(p);
      }
    }

    final nodes = <EditorNode>[];
    for (final d in deletedAfter[''] ?? const []) {
      nodes.add(_toNode(d, 'deleted'));
    }
    for (final b in current) {
      final id = b['id'] as String;
      final before = prevById[id];
      final status = before == null
          ? 'added'
          : (_blockEqual(b, before) ? null : 'changed');
      nodes.add(_toNode(b, status));
      for (final d in deletedAfter[id] ?? const []) {
        nodes.add(_toNode(d, 'deleted'));
      }
    }
    return nodes;
  }

  Future<void> _createCheckpoint() async {
    final l10n = context.l10n;
    final name = await _promptName();
    if (name == null || name.trim().isEmpty) return;
    setState(() => _busy = true);
    try {
      await widget.onCreate(name.trim());
      await _refresh();
      _snack(l10n.versionCheckpointSaved);
    } catch (error) {
      _snack(l10n.versionSaveFailed(error.toString()));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  /// A version's display name: named checkpoints show their label; auto
  /// snapshots (label null) read as "Auto-save" and lean on the timestamp.
  String _displayName(DocVersion v) =>
      v.isAuto ? context.l10n.versionAutoSnapshot : v.label!.trim();

  /// One timeline row, card-shaped (design 08): active = accent border + wash,
  /// which at this width reads far better than Material's tinted-title `selected`.
  Widget _versionRow(BuildContext context, DocVersion v) {
    final selected = v.id == _selectedId;
    return Padding(
      padding: const EdgeInsets.only(bottom: 6),
      child: Material(
        color: selected
            ? MicaTheme.of(context).accent.wash
            : Colors.transparent,
        borderRadius: BorderRadius.circular(10),
        child: InkWell(
          borderRadius: BorderRadius.circular(10),
          onTap: _busy ? null : () => _select(v),
          child: Container(
            decoration: BoxDecoration(
              borderRadius: BorderRadius.circular(10),
              border: Border.all(
                color: selected
                    ? MicaTheme.of(
                        context,
                      ).accent.primary.withValues(alpha: 0.35)
                    : Colors.transparent,
              ),
            ),
            padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 9),
            child: Row(
              children: [
                Icon(
                  v.isAuto ? Icons.schedule : Icons.bookmark,
                  size: 16,
                  color: selected
                      ? MicaTheme.of(context).accent.primary
                      : MicaTheme.of(context).text.faint,
                ),
                const SizedBox(width: 9),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        v.isAuto ? _stamp(v) : _displayName(v),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontSize: 14,
                          fontWeight: v.isAuto
                              ? FontWeight.w400
                              : FontWeight.w600,
                          color: selected
                              ? MicaTheme.of(context).accent.primary
                              : MicaTheme.of(context).text.primary,
                        ),
                      ),
                      ?_rowSubtitle(v),
                    ],
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  /// A version's timestamp in the design-08 relative form: `15:08` today,
  /// 「昨天 22:03」 yesterday, a bare date further back.
  ///
  /// Falls back to the raw string when the stamp does not parse — showing the
  /// unhelpful original beats inventing a date, and beats a blank where a time
  /// belongs.
  String _stamp(DocVersion v) {
    final when = versionTime(v);
    if (when == null) return v.createdAt;
    final l10n = context.l10n;
    final s = versionStamp(when);
    return switch (s.day) {
      VersionDay.today => s.time,
      VersionDay.yesterday => l10n.versionYesterdayAt(s.time),
      VersionDay.earlier => l10n.versionDateShort(s.month, s.dayOfMonth),
    };
  }

  /// Who saved this version, or null when that can't be said usefully.
  ///
  /// Deliberately quiet: an unknown id renders nothing rather than a placeholder
  /// like 「未知用户」 — a row that admits it doesn't know who did this is worse
  /// than a row that doesn't raise the question. Ids go stale legitimately (a
  /// member who left is no longer in the map).
  String? _authorName(DocVersion v) {
    final id = v.createdBy;
    if (id == null) return null;
    final name = widget.authorNames[id]?.trim();
    return (name == null || name.isEmpty) ? null : name;
  }

  /// The second line: an author when there is one to name, the timestamp for a
  /// named checkpoint (whose title is its label, not its time), or both.
  Widget? _rowSubtitle(DocVersion v) {
    final author = _authorName(v);
    final time = v.isAuto ? null : _stamp(v);
    final parts = [if (time != null) time, if (author != null) author];
    if (parts.isEmpty) return null;
    return Text(
      parts.join('  ·  '),
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
    );
  }

  Future<void> _restore(DocVersion version) async {
    final l10n = context.l10n;
    final ok = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(context.l10n.versionRestoreConfirmTitle),
        content: Text(
          context.l10n.versionRestoreConfirmBody(_displayName(version)),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: Text(context.l10n.commonCancel),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: Text(context.l10n.versionRestore),
          ),
        ],
      ),
    );
    if (ok != true) return;
    setState(() => _busy = true);
    try {
      await widget.onRestore(version.id);
      if (!mounted) return;
      _snack(l10n.versionRestored(_displayName(version)));
      Navigator.of(context).pop();
    } catch (error) {
      _snack(l10n.versionRestoreFailed(error.toString()));
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<String?> _promptName() {
    final controller = TextEditingController();
    return showDialog<String>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(context.l10n.versionSaveCheckpoint),
        content: TextField(
          controller: controller,
          autofocus: true,
          decoration: InputDecoration(hintText: context.l10n.versionNameHint),
          onSubmitted: (v) => Navigator.pop(context, v),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: Text(context.l10n.commonCancel),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, controller.text),
            child: Text(context.l10n.commonSave),
          ),
        ],
      ),
    );
  }

  void _snack(String message) {
    if (!mounted) return;
    ScaffoldMessenger.of(
      context,
    ).showSnackBar(SnackBar(content: Text(message)));
  }

  DocVersion? get _selected =>
      _versions.where((v) => v.id == _selectedId).firstOrNull;

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    return Dialog(
      insetPadding: const EdgeInsets.symmetric(horizontal: 40, vertical: 24),
      clipBehavior: Clip.antiAlias,
      child: SizedBox(
        width: 980,
        height: 640,
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Expanded(child: _buildPreview(l10n)),
            const VerticalDivider(width: 1),
            SizedBox(width: 300, child: _buildTimeline(l10n)),
          ],
        ),
      ),
    );
  }

  Widget _buildPreview(AppLocalizations l10n) {
    if (_selectedId == null) {
      return Center(
        child: Text(l10n.versionEmpty, textAlign: TextAlign.center),
      );
    }
    if (_previewLoading) {
      return const Center(child: CircularProgressIndicator());
    }
    if (_previewError != null) {
      return Center(child: Text(l10n.versionLoadFailed(_previewError!)));
    }
    final content = _content;
    if (content == null) return const SizedBox.shrink();
    // The SAME editor in canEdit:false — reused, not re-rendered (P-A hardening
    // hides caret/IME/toolbars). Isolated: it renders the version's own blocks,
    // never the live document, so it can't affect the open page.
    return Column(
      children: [
        // Diff legend — shown only when there's a predecessor to compare against.
        if (_prevContent != null)
          Container(
            width: double.infinity,
            padding: const EdgeInsets.symmetric(horizontal: 32, vertical: 6),
            color: Theme.of(
              context,
            ).colorScheme.surfaceContainerHighest.withValues(alpha: 0.4),
            child: Row(
              children: [
                _legendDot(
                  MicaTheme.of(context).status.success,
                  l10n.versionDiffAdded,
                ),
                const SizedBox(width: 14),
                _legendDot(
                  MicaTheme.of(context).status.warning,
                  l10n.versionDiffChanged,
                ),
                const SizedBox(width: 14),
                _legendDot(
                  MicaTheme.of(context).status.danger,
                  l10n.versionDiffRemoved,
                ),
              ],
            ),
          ),
        Expanded(child: _buildPreviewBody(content)),
      ],
    );
  }

  Widget _legendDot(Color color, String label) => Row(
    mainAxisSize: MainAxisSize.min,
    children: [
      Container(
        width: 10,
        height: 10,
        decoration: BoxDecoration(
          color: color,
          borderRadius: BorderRadius.circular(2),
        ),
      ),
      const SizedBox(width: 5),
      Text(label, style: const TextStyle(fontSize: 12)),
    ],
  );

  Widget _buildPreviewBody(
    ({String rootBlockId, List<Map<String, dynamic>> blocks}) content,
  ) {
    return SingleChildScrollView(
      padding: const EdgeInsets.symmetric(horizontal: 32, vertical: 24),
      child: Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 720),
          child: MicaEditor(
            key: ValueKey('version-preview-$_selectedId'),
            rootBlockId: content.rootBlockId,
            nodes: _previewNodes(content),
            version: 0,
            canEdit: false,
            onApplyOperations: (_) async {},
            onLoadImageBytes: widget.onLoadImageBytes,
            onResolveImageUrls: widget.onResolveImageUrls,
          ),
        ),
      ),
    );
  }

  Widget _buildTimeline(AppLocalizations l10n) {
    return Column(
      children: [
        // Header: title + save-checkpoint + close.
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 12, 8, 8),
          child: Row(
            children: [
              Text(
                l10n.versionHistoryTitle,
                style: const TextStyle(fontWeight: FontWeight.w600),
              ),
              const Spacer(),
              IconButton(
                tooltip: l10n.versionSaveCheckpoint,
                onPressed: _busy ? null : _createCheckpoint,
                icon: const Icon(Icons.bookmark_add_outlined, size: 20),
              ),
              IconButton(
                tooltip: l10n.commonClose,
                onPressed: () => Navigator.of(context).pop(),
                icon: const Icon(Icons.close, size: 20),
              ),
            ],
          ),
        ),
        const Divider(height: 1),
        Expanded(
          child: _loading
              ? const Center(child: CircularProgressIndicator())
              : _error != null
              ? Center(child: Text(l10n.versionLoadFailed(_error!)))
              : _versions.isEmpty
              ? Center(
                  child: Text(l10n.versionEmpty, textAlign: TextAlign.center),
                )
              : ListView(
                  padding: const EdgeInsets.symmetric(horizontal: 8),
                  children: [
                    // 今天 / 更早 (design 08). A flat list of bare timestamps
                    // gave no sense of when you were looking at.
                    for (final section in groupVersions(_versions)) ...[
                      Padding(
                        padding: const EdgeInsets.only(
                          left: 6,
                          top: 10,
                          bottom: 4,
                        ),
                        child: Text(
                          section.day == VersionDay.today
                              ? context.l10n.versionGroupToday
                              : context.l10n.versionGroupEarlier,
                          style: TextStyle(
                            fontSize: 11,
                            fontWeight: FontWeight.w600,
                            letterSpacing: 0.6,
                            color: MicaTheme.of(context).text.faint,
                          ),
                        ),
                      ),
                      for (final v in section.items) _versionRow(context, v),
                    ],
                  ],
                ),
        ),
        const Divider(height: 1),
        // Restore bar — acts on the previewed version (bottom-right, AFFiNE/
        // Notion shape).
        Padding(
          padding: const EdgeInsets.all(12),
          child: Row(
            mainAxisAlignment: MainAxisAlignment.end,
            children: [
              FilledButton(
                onPressed: (_busy || _selected == null)
                    ? null
                    : () => _restore(_selected!),
                child: Text(l10n.versionRestore),
              ),
            ],
          ),
        ),
      ],
    );
  }
}

class _RecycleBinDialog extends StatefulWidget {
  const _RecycleBinDialog({
    required this.onLoad,
    required this.onRestore,
    required this.onPurge,
    required this.canEdit,
    required this.liveViews,
    required this.relativeStrings,
    this.onPurgeAll,
  });

  /// Empty the whole bin, returning how many views went. **Null in 本地模式**:
  /// the on-device store has no bulk purge, and a button that cannot work is
  /// worse than no button — same null-means-absent rule as everything else here.
  /// Local users still purge one row at a time.
  final Future<int> Function()? onPurgeAll;

  /// The workspace's live tree, used only to name where a restore will land.
  final List<DocumentView> liveViews;

  /// Localized relative-time wording for the deletion time.
  final RelativeTimeStrings relativeStrings;

  final Future<List<DocumentView>> Function() onLoad;
  final Future<void> Function(DocumentView view) onRestore;
  final Future<void> Function(DocumentView view) onPurge;

  /// Both restore and purge are gated on the editor role server-side
  /// (`ensure_workspace_editor`). The buttons were drawn regardless, so a viewer
  /// could press either and collect a 403 — the same dead-control shape the
  /// null-means-absent rule elsewhere in this file exists to prevent. The list
  /// itself stays visible: knowing what was deleted is useful read-only.
  final bool canEdit;

  @override
  State<_RecycleBinDialog> createState() => _RecycleBinDialogState();
}

class _RecycleBinDialogState extends State<_RecycleBinDialog> {
  bool _loading = true;
  String? _error;
  List<TrashEntry> _entries = const [];

  @override
  void initState() {
    super.initState();
    _refresh();
  }

  Future<void> _refresh() async {
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final all = await widget.onLoad();
      // Roots only, plus what each restore will actually bring back — see
      // `buildTrashEntries`.
      final entries = buildTrashEntries(deleted: all, live: widget.liveViews);
      if (!mounted) return;
      setState(() {
        _entries = entries;
        _loading = false;
      });
    } catch (error) {
      if (!mounted) return;
      setState(() {
        _error = error.toString();
        _loading = false;
      });
    }
  }

  /// A folder must LOOK like a folder. Every row used to wear the same page
  /// glyph, so a deleted folder — the one case where restore reaches past the row
  /// you clicked — was indistinguishable from a single page.
  Widget _trashLeading(DocumentView view) {
    final emoji = view.icon?.trim();
    if (emoji != null && emoji.isNotEmpty) {
      return SizedBox(
        width: 18,
        child: Center(child: Text(emoji, style: const TextStyle(fontSize: 15))),
      );
    }
    return Icon(
      view.isFolder ? Icons.folder_outlined : Icons.description_outlined,
      size: 18,
      color: MicaTheme.of(context).text.muted,
    );
  }

  /// Where it came from · what comes back with it · when it was deleted.
  ///
  /// Each part is dropped when it isn't known rather than filled with a
  /// placeholder: the local world carries no `icon` and no deletion timestamp
  /// (they never reach `DocumentView` through `_viewFromData`), and a top-level
  /// page has no path. An empty subtree says nothing rather than 「含 0 个页面」.
  Widget? _trashSubtitle(BuildContext context, TrashEntry entry) {
    final l10n = context.l10n;
    final counts = <String>[
      if (entry.folders > 0) l10n.folderCount(entry.folders),
      if (entry.pages > 0) l10n.pageCount(entry.pages),
    ];
    // `delete_view` stamps `updated_at = now()` when trashing, so for a deleted
    // row this IS the deletion time. Null in the local world.
    final deletedAt = entry.view.updatedAt;
    final parts = <String>[
      if (entry.path.isNotEmpty) entry.path,
      if (counts.isNotEmpty)
        l10n.recycleSubtree(counts.join(l10n.importListSeparator)),
      if (deletedAt != null) relativeMeta(deletedAt, widget.relativeStrings),
    ];
    if (parts.isEmpty) return null;
    return Text(
      parts.join('  ·  '),
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
      style: TextStyle(fontSize: 12, color: MicaTheme.of(context).text.faint),
    );
  }

  /// Permanent delete is the only irreversible action in this dialog, and it
  /// reaches far wider than the row you clicked: the server drops the whole
  /// subtree's views *and* their backing `documents` rows (`purge_view_subtree`
  /// in `routes/documents.rs`), which cascades version history and comments
  /// away with them. It used to fire straight off the icon button, so one
  /// mis-click next to 「恢复」 destroyed a subtree with nothing to undo it.
  Future<bool> _confirmPurge(DocumentView view) {
    final l10n = context.l10n;
    return showDestructiveConfirm(
      context,
      title: l10n.recyclePurgeConfirmTitle(view.name),
      body: l10n.recyclePurgeConfirmBody,
      confirmLabel: l10n.recycleDeleteForever,
      cancelLabel: l10n.commonCancel,
    );
  }

  /// Empty the bin, behind the same gate as a single permanent delete — this one
  /// is every row at once, so the confirmation names the count.
  Future<void> _confirmPurgeAll() async {
    final purgeAll = widget.onPurgeAll;
    if (purgeAll == null) return;
    final l10n = context.l10n;
    final ok = await showDestructiveConfirm(
      context,
      title: l10n.recycleEmptyAllConfirmTitle(_entries.length),
      body: l10n.recyclePurgeConfirmBody,
      confirmLabel: l10n.recycleEmptyAll,
      cancelLabel: l10n.commonCancel,
    );
    if (!ok) return;
    try {
      final removed = await purgeAll();
      if (!mounted) return;
      ScaffoldMessenger.maybeOf(context)?.showSnackBar(
        SnackBar(content: Text(context.l10n.recycleEmptiedAll(removed))),
      );
    } catch (error) {
      if (!mounted) return;
      setState(() => _error = error.toString());
    }
    await _refresh();
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: Row(
        children: [
          const Icon(Icons.delete_outline, size: 22),
          const SizedBox(width: 8),
          Text(context.l10n.recycleBinTitle),
          const Spacer(),
          IconButton(
            tooltip: context.l10n.recycleRefresh,
            onPressed: _loading ? null : _refresh,
            icon: const Icon(Icons.refresh, size: 20),
          ),
        ],
      ),
      content: SizedBox(width: 420, height: 360, child: _buildBody(context)),
      actions: [
        // Only with something to empty, and only where a bulk purge exists.
        if (widget.onPurgeAll != null && widget.canEdit && _entries.isNotEmpty)
          TextButton(
            style: TextButton.styleFrom(
              foregroundColor: kDestructiveRed(context),
            ),
            onPressed: _confirmPurgeAll,
            child: Text(context.l10n.recycleEmptyAll),
          ),
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(context.l10n.commonClose),
        ),
      ],
    );
  }

  Widget _buildBody(BuildContext context) {
    if (_loading) {
      return const Center(child: CircularProgressIndicator());
    }
    if (_error != null) {
      return Center(child: ErrorBanner(_error!));
    }
    if (_entries.isEmpty) {
      return EmptyState(
        icon: Icons.delete_outline,
        title: context.l10n.recycleEmpty,
        detail: context.l10n.recycleEmptyDetail,
      );
    }
    return ListView.separated(
      itemCount: _entries.length,
      separatorBuilder: (_, _) => const Divider(height: 1),
      itemBuilder: (context, i) {
        final entry = _entries[i];
        final view = entry.view;
        return ListTile(
          contentPadding: const EdgeInsets.symmetric(horizontal: 4),
          leading: _trashLeading(view),
          title: Text(view.name, overflow: TextOverflow.ellipsis),
          subtitle: _trashSubtitle(context, entry),
          trailing: !widget.canEdit
              ? null
              : Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    IconButton(
                      tooltip: context.l10n.recycleRestore,
                      icon: const Icon(Icons.restore, size: 20),
                      onPressed: () async {
                        await widget.onRestore(view);
                        await _refresh();
                      },
                    ),
                    IconButton(
                      tooltip: context.l10n.recycleDeleteForever,
                      color: MicaTheme.of(context).status.danger,
                      icon: const Icon(Icons.delete_forever, size: 20),
                      onPressed: () async {
                        if (!await _confirmPurge(view)) return;
                        await widget.onPurge(view);
                        await _refresh();
                      },
                    ),
                  ],
                ),
        );
      },
    );
  }
}
