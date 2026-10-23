part of 'main.dart';

extension _WorkspaceTreeNav on _WorkspaceViewState {
  Widget _pageTree(BuildContext context, bool canEdit) {
    // Which of the four states this is, is decided by `pageTreeStateFor` in
    // ui/page_tree_state.dart, NOT by inline ifs here — that function is the
    // only part of this rule a test can reach, because `_pageTree` cannot be
    // built without the 97-parameter widget around it. Re-inlining a branch
    // here silently un-tests it.
    final treeState = pageTreeStateFor(
      hasWorkspace: widget.selectedWorkspace != null,
      viewsEmpty: widget.views.isEmpty,
      treePending: widget.treePending,
    );

    if (treeState == PageTreeState.noWorkspace) {
      return EmptyState(
        icon: Icons.ads_click,
        title: context.l10n.workspaceRowSelectWorkspace,
        detail: context.l10n.sidebarSelectWorkspaceDetail,
      );
    }

    if (treeState == PageTreeState.skeleton) {
      return const PageTreeSkeleton();
    }

    if (treeState == PageTreeState.empty) {
      return EmptyState(
        icon: Icons.note_add,
        title: context.l10n.sidebarNoPagesTitle,
        detail: context.l10n.sidebarNoPagesDetail,
        // The rule's other half (design 19): an empty tree should offer the way
        // OUT of being empty, not just describe it. Offered only when the user
        // may actually create — a dead button is worse than none.
        actionLabel: canEdit ? context.l10n.newPage : null,
        onAction: canEdit ? () => _createLocated(folder: false) : null,
      );
    }

    // The single highlighted row = the located node (last-tapped folder/page),
    // falling back to the open doc. So a focused folder highlights just like the
    // open page, and the two never light up at once.
    //
    // Released location = NO highlight, deliberately. The highlight's whole job
    // is to answer "where will the New buttons put this"; leaving it lit on the
    // open page after the location was released answers a question nobody asked
    // and lies about the one that was.
    //
    // (This was briefly the other way round — falling back to the open document
    // so the row stayed lit. It reads better in a screenshot and worse in use:
    // you cannot tell a located row from a merely-open one, which is the single
    // thing this highlight exists to tell you.)
    final activeId = _rootFocused
        ? null
        : (_focusedNavId ?? widget.selectedView?.id);
    // Tapping the tree's blank area (below/around the rows) deselects the
    // located node so the top New buttons create at the root. Rows keep their
    // own tap handlers (they win the gesture arena); only taps that miss a row
    // A reveal is requested before the row it wants exists — ancestors expand
    // in the same setState, and a cross-workspace hit waits on that tree
    // loading. Retrying after each frame is what makes both land.
    if (_pendingRevealId != null) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) _flushPendingReveal();
      });
    }
    // Once, not once per row: each row needs to know its NEIGHBOUR's depth, and
    // rebuilding the whole flattened tree inside the map would make drawing the
    // sidebar quadratic — on the 3700-page workspace that is not a nicety.
    final treeRows = _visibleDocumentTree();
    // reach this. Opaque so the empty space below the last row is hittable.
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: () {
        _clearTreeSelection();
        _focusRoot();
      },
      // Esc ALSO clears the selection — via [_escKeyEvent], a global
      // HardwareKeyboard handler, NOT any binding here. Three focus-based
      // bindings were built first and all three were dead (a local
      // `Shortcuts`/`Actions` pair, the same plus a requested FocusNode, and
      // `_appShortcuts` where Ctrl+N works); the probe story is in
      // `docs/features/shortcuts.md`. Short version: when Esc arrives, focus sits on the
      // page route's own scope — an ANCESTOR of this shell — and key dispatch
      // walks from the focused node UPWARD, so nothing registered in here is
      // ever on the path. The feature shipped without Esc for that reason
      // (a documented key that does nothing is worse than no key), until the
      // user asked for it (2026-08-27) — which is what the global handler's
      // dialog/rename caveats were traded against.
      child: _buildTreeList(treeRows, canEdit, activeId),
    );
  }

  Widget _buildTreeList(
    List<({DocumentView view, int depth, bool hasChildren})> treeRows,
    bool canEdit,
    String? activeId,
  ) {
    return
    // The auto-scroll region wraps the list EXACTLY, so its box is the
    // viewport the edge zones are measured against.
    DragAutoScrollRegion(
      enabled: _draggingTree,
      controller: _treeScroll,
      child: CustomScrollView(
        controller: _treeScroll,
        slivers: [
          SliverList(
            delegate: SliverChildBuilderDelegate(
              (context, index) {
                final item = treeRows[index];
                // The NEXT row's depth bounds how far left an `after` drop under this
                // row may pop out; null means "this is the last row", which is what
                // lets the last row reach the workspace root.
                final nextDepth = index + 1 < treeRows.length
                    ? treeRows[index + 1].depth
                    : null;
                final row = DocumentListItem(
                  key: ValueKey(item.view.id),
                  view: item.view,
                  depth: item.depth,
                  hasChildren: item.hasChildren,
                  revealToggle: _navHovered,
                  isCollapsed: !_expandedViewIds.contains(item.view.id),
                  isSelected: item.view.id == activeId,
                  isMultiSelected: _selection.contains(item.view.id),
                  // Read-only trees get no selection: nothing in the batch menu is
                  // something a viewer could do.
                  onSelectClick: canEdit
                      ? ({required extendRange}) => _handleSelectClick(
                          item.view,
                          extendRange: extendRange,
                        )
                      : null,
                  onPlainTap: () => _anchorTreeSelection(item.view),
                  batchActions:
                      canEdit && _selection.actsOnWholeSelection(item.view.id)
                      ? () => _batchMenuItems(item.view)
                      : null,
                  canEdit: canEdit,
                  isRenaming: item.view.id == _renamingViewId,
                  onToggle: () => _toggleViewExpand(item.view),
                  onPressed: () => _navigateToView(item.view),
                  onCreateChild: () =>
                      _createInFolder(item.view, folder: false),
                  onCreateChildFolder: () =>
                      _createInFolder(item.view, folder: true),
                  onExportFolder: widget.onExportFolderZip == null
                      ? null
                      : () => _exportFolderFile(item.view),
                  // Import md / images / a nested folder UNDER this folder — both worlds
                  // (server validates parent_view_id; local prefixes the tree).
                  onImportFilesIntoFolder: item.view.objectType == 'folder'
                      ? () => _importFilesIntoFolder(item.view)
                      : null,
                  onImportFolderIntoFolder: item.view.objectType == 'folder'
                      ? () => _importFolderIntoFolder(item.view)
                      : null,
                  // Cross-workspace move/copy — cloud-only (onTransfer is null in a
                  // local workspace, which hides both entries). Works for pages and
                  // folders alike; the folder carries its subtree server-side.
                  onTransferMove: widget.onTransfer == null
                      ? null
                      : () => widget.onTransfer!([item.view], false),
                  onTransferCopy: widget.onTransfer == null
                      ? null
                      : () => widget.onTransfer!([item.view], true),
                  onClone: () => widget.onCloneView(item.view),
                  onRename: () => _promptRenameView(item.view),
                  onSetIcon: widget.onSetViewIcon == null
                      ? null
                      : () => widget.onSetViewIcon!(item.view),
                  // Pages only — a folder has no document to open in a tab. Also null
                  // in the local world, where the host passes no tab callbacks.
                  onOpenInNewTab:
                      (widget.onOpenInNewTab == null ||
                          item.view.objectType == 'folder')
                      ? null
                      : () => widget.onOpenInNewTab!(item.view),
                  onRenameSubmit: (name) => _commitRename(item.view, name),
                  onRenameCancel: _cancelRename,
                  onDelete: () => widget.onDeleteView(item.view),
                );
                if (!canEdit) {
                  return Padding(
                    key: ValueKey(item.view.id),
                    padding: const EdgeInsets.only(bottom: 8),
                    child: row,
                  );
                }
                // `childWhenDragging` only fades the row under the pointer. The rest
                // of the selection is travelling too, so it fades with it — otherwise
                // four rows sit there looking like they stayed behind.
                final travelling =
                    _draggingTree &&
                    _selection.contains(item.view.id) &&
                    _selection.length > 1;
                return KeyedSubtree(
                  key: ValueKey(item.view.id),
                  child: _draggableTreeRow(
                    item.view,
                    item.depth,
                    nextDepth,
                    travelling ? Opacity(opacity: 0.4, child: row) : row,
                  ),
                );
              },
              childCount: treeRows.length,
              findChildIndexCallback: (key) => key is ValueKey<String>
                  ? _pageTreeCache.visibleIndexOf(key.value)
                  : null,
            ),
          ),
          // The root drop zone. Every other drop target takes its parent from the
          // ROW it sits on (`before`/`after` mean "sibling of that row"), so when
          // the bottom of the tree is a nested row there was no way to say "below
          // all of this, at the workspace root" — reported as 「放在整个页面树最底
          // 部，让他与根路径齐平，似乎做不到」.
          //
          // The blank area under the tree already MEANS root: tapping it releases
          // the located node, and the New buttons then create at the root.
          // Accepting a drop there says the same thing with a drag, so there is
          // nothing new to learn.
          //
          // `SliverFillRemaining(hasScrollBody: false)` and not a trailing box of
          // some chosen height: it takes the LEFTOVER viewport when the tree is
          // short — which is the whole blank area the user was aiming at — and
          // only its own height when the tree overflows, so it adds no phantom
          // scroll extent to a long tree. The first attempt was a 40px box and
          // the drop silently did nothing, because the pointer was below it in
          // blank space that still belonged to no one.
          if (_draggingTree)
            SliverFillRemaining(hasScrollBody: false, child: _rootDropZone()),
        ],
      ),
    );
  }

  /// "Move to the workspace root, at the end." Exists only during a drag.
  ///
  /// Deliberately NOT the full depth picker (drag left/right to choose the
  /// nesting level, the way Notion does). That is the general answer, and it is
  /// a rework rather than an addition: depth here is derived from which row's
  /// DragTarget the pointer is over, and a DragTarget cannot see the pointer's
  /// x — it would take one overlay tracking the pointer and computing (index,
  /// depth) itself. It would also have to respect this tree's invariant that
  /// only a folder may hold children, so the indicator must snap to the LEGAL
  /// depths rather than to every depth, or it draws a line you cannot drop on.
  /// Worth doing if picking a MIDDLE level turns out to be wanted too; the
  /// case actually reported is the root.
  Widget _rootDropZone() {
    return DragTarget<List<DocumentView>>(
      hitTestBehavior: HitTestBehavior.opaque,
      // A row already at the root, dropped here, is a reorder to the end —
      // a legitimate thing to want, so it is accepted rather than refused.
      onWillAcceptWithDetails: (_) => true,
      onAcceptWithDetails: (details) {
        _clearTreeSelection();
        widget.onReorderViews(null, rootDropOrder(widget.views, details.data));
      },
      builder: (context, candidate, rejected) {
        // No label any more. A caption explaining that this area means "root"
        // was a way of describing the model instead of making it operable —
        // and it only ever spoke about ONE level. Now the same dot that walks
        // out to level 0 under the last row does the explaining, and this zone
        // is just the rest of that gesture's surface.
        return Align(
          alignment: Alignment.topLeft,
          child: Padding(
            padding: const EdgeInsets.only(top: 6),
            child: _DropIndicator(
              inset: dropIndicatorInset(0),
              // This zone accepts everything, so `blocked` cannot arise here —
              // still routed through the same enum rather than a bool, so there
              // is one vocabulary for "what is the indicator saying".
              state: candidate.isNotEmpty
                  ? DropIndicatorState.allowed
                  : DropIndicatorState.idle,
            ),
          ),
        );
      },
    );
  }

  /// Placeholder rows for a tree that has not arrived yet.
  ///
  /// Static on purpose: an animated shimmer would be a second thing running
  /// during the very frames this is meant to keep cheap, and the one controlled
  /// comparison anyone cites found skeletons no better liked than a spinner.
  /// The job here is only to not lie about the workspace being empty.
  /// Tapping the tree's blank area deselects the located node: nothing is
  /// highlighted and the top New-page/New-folder buttons create at the
  /// workspace root (the only way to reach the root while a doc stays open).
  void _focusRoot() => _tree.focusRoot();

  /// Wrap a page row so it can be dragged to reorder among its siblings (its
  /// subtree follows, since children render under their parent). Touch moves
  /// scroll the tree; long-press then move reorders. Mouse drags start at once.
  /// The top/bottom half of each sibling row is a drop slot (before/after).
  Widget _draggableTreeRow(
    DocumentView view,
    int depth,
    int? nextDepth,
    Widget row,
  ) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 8),
      child: NavigationDraggable<List<DocumentView>>(
        // Grabbing a row inside a multi-selection drags the whole selection —
        // the same rule the context menu uses, so what you get is what the
        // highlight already showed you.
        data: _rowsActedOnBy(view.id),
        dragAnchorStrategy: pointerDragAnchorStrategy,
        // No onDragUpdate: it is mounted-guarded, and auto-scrolling this very
        // list unmounts the row being dragged. DragAutoScrollRegion reads the
        // raw pointer stream instead — see its doc comment.
        onDragStarted: () => setState(() => _draggingTree = true),
        onDragEnd: (_) => _endTreeDrag(),
        onDraggableCanceled: (_, _) => _endTreeDrag(),
        onDragCompleted: _endTreeDrag,
        feedback: _dragFeedback(_rowsActedOnBy(view.id)),
        childWhenDragging: Opacity(opacity: 0.4, child: row),
        child: Stack(
          children: [
            row,
            // Drop zones exist only during a drag, so they never block taps.
            if (_draggingTree)
              Positioned.fill(
                child: Column(
                  children: [
                    // The indicator is drawn at the depth the row would LAND
                    // at, which for before/after is this row's own depth (they
                    // mean "become its sibling").
                    Expanded(
                      flex: 3,
                      child: _dropSlot(view, _DropMode.before, depth),
                    ),
                    Expanded(
                      flex: 4,
                      child: _dropSlot(view, _DropMode.into, depth),
                    ),
                    // `nextDepth` is what bounds how far left this drop may
                    // pop out — see [reparentLevelFor].
                    Expanded(
                      flex: 3,
                      child: _dropSlot(
                        view,
                        _DropMode.after,
                        depth,
                        nextDepth: nextDepth,
                      ),
                    ),
                  ],
                ),
              ),
          ],
        ),
      ),
    );
  }

  /// The chip that follows the pointer during a tree drag.
  ///
  /// One row: name and icon, as it always was. Several: the SAME chip with two
  /// stacked cards peeking out behind it and a count underneath — a deck, not a
  /// list. The alternative (render all N rows) was rejected on sight: five rows
  /// is a panel that covers the drop indicator you are aiming with, and the
  /// indicator is the only thing telling you where the drop lands.
  ///
  /// The count is what makes the deck honest, because the cards behind are a
  /// fixed two whether you grabbed three rows or thirty.
  Widget _dragFeedback(List<DocumentView> rows) {
    final tokens = MicaTheme.of(context);
    final lead = rows.isEmpty ? null : rows.first;
    Widget card({double inset = 0, double opacity = 1, Widget? child}) =>
        Padding(
          padding: EdgeInsets.only(left: inset, top: inset),
          child: Opacity(
            opacity: opacity,
            child: Material(
              elevation: 6,
              borderRadius: BorderRadius.circular(6),
              child: Container(
                padding: const EdgeInsets.symmetric(
                  horizontal: 12,
                  vertical: 8,
                ),
                constraints: const BoxConstraints(maxWidth: 260, minWidth: 120),
                decoration: BoxDecoration(
                  color: tokens.surface.overlay,
                  borderRadius: BorderRadius.circular(6),
                ),
                child: child ?? const SizedBox(height: 20),
              ),
            ),
          ),
        );

    final top = card(
      inset: rows.length > 1 ? 8 : 0,
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(
            lead != null && lead.objectType == 'folder'
                ? Icons.folder_outlined
                : Icons.description_outlined,
            size: 18,
            color: tokens.accent.primary,
          ),
          const SizedBox(width: 8),
          Flexible(
            child: Text(lead?.name ?? '', overflow: TextOverflow.ellipsis),
          ),
        ],
      ),
    );
    if (rows.length <= 1) return top;
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Stack(
          children: [
            // Back to front, so the named card ends up on top.
            card(opacity: 0.45),
            card(inset: 4, opacity: 0.7),
            top,
          ],
        ),
        const SizedBox(height: 6),
        Material(
          color: Colors.transparent,
          child: Padding(
            padding: const EdgeInsets.only(left: 8),
            child: Text(
              context.l10n.batchDragLabel(rows.length),
              style: TextStyle(fontSize: 12, color: tokens.text.muted),
            ),
          ),
        ),
      ],
    );
  }

  /// Ends the drag. The auto-scroll stops with it: `DragAutoScrollRegion` is
  /// driven by `_draggingTree`, so clearing the flag is what turns it off.
  void _endTreeDrag() {
    setState(() => _draggingTree = false);
  }

  // ── Ctrl/Shift multi-selection ─────────────────────────────────────────────

  /// A Ctrl (toggle) or Shift (extend) click landed on [view].
  void _handleSelectClick(DocumentView view, {required bool extendRange}) {
    setState(() {
      _selection.click(
        view.id,
        extendRange: extendRange,
        // The tree AS DISPLAYED: a Shift range is only meaningful over rows the
        // user can see. A collapsed folder's children are simply absent.
        visibleRows: [
          for (final row in _visibleDocumentTree())
            (id: row.view.id, parentId: row.view.parentViewId),
        ],
      );
    });
  }

  /// An unmodified click landed on [view]: drop the selection, but leave the
  /// row as the anchor a following Shift-click measures from.
  ///
  /// A no-op when it is already the state, so it can sit in the hot path of
  /// every row tap without causing a rebuild per click.
  void _anchorTreeSelection(DocumentView view) {
    if (_selection.anchorOn(view.id)) setState(() {});
  }

  /// Drop the selection AND the anchor — for a click that hit no row (the blank
  /// area below the tree), or after a drop relocated the rows.
  void _clearTreeSelection() {
    if (_selection.clear()) setState(() {});
  }

  /// The rows a gesture on [id] acts on: the whole selection when [id] is part
  /// of a multi-selection, otherwise just [id]. Nested rows are dropped — see
  /// [independentSelection].
  ///
  /// Returned in TREE order, not selection order, so a batch lands at the
  /// destination looking the way it looked in the sidebar. (Selection order is
  /// whatever sequence the user happened to Ctrl-click in, which nobody
  /// remembers by the time they see the result.)
  List<DocumentView> _rowsActedOnBy(String id) {
    if (!_selection.actsOnWholeSelection(id)) {
      final one = _pageTreeCache.viewById(id);
      return one == null ? const [] : [one];
    }
    final inTreeOrder = [
      for (final row in _visibleDocumentTree())
        if (_selection.contains(row.view.id)) row.view.id,
    ];
    final kept = independentSelection(
      inTreeOrder,
      (child) => _pageTreeCache.viewById(child)?.parentViewId,
    );
    return [for (final k in kept) ?_pageTreeCache.viewById(k)];
  }

  /// The context menu shown when a right-click lands inside a multi-selection.
  ///
  /// Counts come from [_rowsActedOnBy], not from `_selection.length`:
  /// selecting a folder and a page inside it is two highlighted rows but ONE
  /// thing to move, and a menu that says 3 while doing 2 is the kind of lie
  /// that only shows up after the deed.
  List<PopupMenuEntry<VoidCallback>> _batchMenuItems(DocumentView row) {
    final rows = _rowsActedOnBy(row.id);
    final l10n = context.l10n;
    final transfer = widget.onTransfer;
    return [
      if (transfer != null) ...[
        PopupMenuItem<VoidCallback>(
          value: () => transfer(rows, false),
          child: _MenuRow(
            icon: Icons.drive_file_move_outlined,
            label: l10n.batchTransferMove(rows.length),
          ),
        ),
        PopupMenuItem<VoidCallback>(
          value: () => transfer(rows, true),
          child: _MenuRow(
            icon: Icons.copy_all_outlined,
            label: l10n.batchTransferCopy(rows.length),
          ),
        ),
        const PopupMenuDivider(),
      ],
      PopupMenuItem<VoidCallback>(
        value: () => _deleteSelection(rows),
        child: _MenuRow(
          icon: Icons.delete_outline,
          label: l10n.batchDelete(rows.length),
          danger: true,
        ),
      ),
    ];
  }

  /// Trash a whole selection after confirming, in ONE request.
  ///
  /// Confirmed even though the destination is the recycle bin: a per-row delete
  /// is one row you are looking at, while this one can carry folders whose
  /// contents are off screen — the count in the prompt is the only place those
  /// are ever mentioned.
  Future<void> _deleteSelection(List<DocumentView> rows) async {
    if (rows.isEmpty) return;
    final l10n = context.l10n;
    final ok = await showDestructiveConfirm(
      context,
      title: l10n.batchDelete(rows.length),
      body: l10n.batchDeleteConfirm(rows.length),
      confirmLabel: l10n.batchDelete(rows.length),
      cancelLabel: l10n.commonCancel,
    );
    if (!ok || !mounted) return;
    _clearTreeSelection();
    await widget.onDeleteViews([for (final r in rows) r.id]);
  }

  Widget _dropSlot(
    DocumentView target,
    _DropMode mode,
    int depth, {
    int? nextDepth,
  }) {
    // A Builder so `onMove` can measure against THIS SLOT's box. Using the
    // State's own `context` measured against the whole pane instead, which put
    // every pointer position past the last level and pinned the indicator at
    // the row's own depth — the level never moved, in either direction.
    return Builder(
      builder: (slotContext) => DragTarget<List<DocumentView>>(
        // The whole zone must be droppable; DragTarget defaults to deferToChild,
        // which would limit the hit area to the thin indicator. These overlays
        // only exist during a drag, so opaque is safe (no taps to intercept).
        hitTestBehavior: HitTestBehavior.opaque,
        // Refused if the target sits inside ANY of the dragged rows: dropping a
        // batch into one of its own folders is the same impossibility as with
        // one row, and only needs to be true of one of them to be true.
        onWillAcceptWithDetails: (details) => TreeDropPlanner(
          widget.views,
        ).canDrop(details.data, target, into: mode == _DropMode.into),
        // Only `after` re-parents, and only sideways. `details.offset` IS the
        // pointer here because the Draggable uses `pointerDragAnchorStrategy`;
        // with the default strategy it would be the feedback's top-left corner
        // and every level would read one chip-width to the right.
        onMove: mode != _DropMode.after
            ? null
            : (details) {
                final box = slotContext.findRenderObject() as RenderBox?;
                if (box == null || !box.hasSize) return;
                final local = box.globalToLocal(details.offset);
                final level = reparentLevelFor(
                  pointerX: local.dx,
                  rowDepth: depth,
                  nextRowDepth: nextDepth,
                );
                if (level != _dropLevel) setState(() => _dropLevel = level);
              },
        onLeave: (_) {
          if (_dropLevel != null) setState(() => _dropLevel = null);
        },
        onAcceptWithDetails: (details) {
          // A level is just a different target for the SAME `after` drop:
          // "sibling of my ancestor at depth L".
          final resolved = TreeDropPlanner(widget.views).resolveAfterTarget(
            target,
            depth,
            mode == _DropMode.after ? _dropLevel : null,
          );
          setState(() => _dropLevel = null);
          _handleDrop(details.data, resolved, mode);
          // The rows have landed somewhere new; keeping them highlighted would
          // leave a selection describing where they used to be.
          _clearTreeSelection();
        },
        builder: (context, candidate, rejected) {
          // `rejected` is how a DragTarget says "something is over me and I
          // refused it" — the case that used to draw nothing at all.
          final state = candidate.isNotEmpty
              ? DropIndicatorState.allowed
              : rejected.isNotEmpty
              ? DropIndicatorState.blocked
              : DropIndicatorState.idle;
          if (mode == _DropMode.into) {
            // Nesting: highlight the whole target row. Warning-coloured when the
            // nest is refused — dropping a folder inside its own subtree is the
            // one people actually try, and it is exactly where silence was worst.
            final tokens = MicaTheme.of(context);
            final edge = switch (state) {
              DropIndicatorState.idle => Colors.transparent,
              DropIndicatorState.allowed => tokens.accent.primary,
              DropIndicatorState.blocked => tokens.status.warning,
            };
            return Container(
              decoration: BoxDecoration(
                borderRadius: BorderRadius.circular(6),
                border: Border.all(color: edge, width: 2),
                color: edge == Colors.transparent
                    ? Colors.transparent
                    : edge.withValues(alpha: 0.08),
              ),
            );
          }
          // Indented to the level this drop lands at, with a terminal dot at the
          // head — AFFiNE's tree indicator, which is Atlassian's
          // pragmatic-drag-and-drop shape: an 8px hollow circle, and a 2px line
          // leaving its centre.
          //
          // The dot is not decoration. It is the thing you aim with: while
          // dragging under the last row of a subtree, moving the pointer left
          // steps the dot out one level at a time, and where it lands is where
          // the row lands.
          //
          // When blocked, the mark is drawn at the level you were ASKING for, in
          // the warning colour — the desired instruction, not nothing.
          return Align(
            alignment: mode == _DropMode.before
                ? Alignment.topLeft
                : Alignment.bottomLeft,
            child: _DropIndicator(
              inset: dropIndicatorInset(
                state == DropIndicatorState.idle ? depth : _dropLevel ?? depth,
              ),
              state: state,
            ),
          );
        },
      ),
    );
  }

  void _handleDrop(
    List<DocumentView> dragged,
    DocumentView target,
    _DropMode mode,
  ) {
    if (dragged.isEmpty) return;
    final moving = {for (final d in dragged) d.id};
    if (mode == _DropMode.into) {
      final children =
          widget.views
              .where(
                (v) => v.parentViewId == target.id && !moving.contains(v.id),
              )
              .toList()
            ..sort((a, b) => a.position.compareTo(b.position));
      children.addAll(dragged);
      setState(() => _expandForChildOf(target.id)); // reveal the drop target
      widget.onReorderViews(target.id, children);
      return;
    }

    final parentId = target.parentViewId;
    final siblings =
        widget.views.where((v) => v.parentViewId == parentId).toList()
          ..sort((a, b) => a.position.compareTo(b.position));
    // The ordering itself is a tested pure function: getting "remove first,
    // then read the index" the wrong way round puts a block one slot off,
    // which is invisible for one row and obvious for five.
    final order = siblingOrderAfterDrop(
      siblings: [for (final s in siblings) s.id],
      dragged: [for (final d in dragged) d.id],
      targetId: target.id,
      before: mode == _DropMode.before,
    );
    if (order == null) return;
    final byId = {
      for (final v in widget.views) v.id: v,
      for (final d in dragged) d.id: d,
    };
    widget.onReorderViews(parentId, [for (final id in order) ?byId[id]]);
  }

  List<({DocumentView view, int depth, bool hasChildren})>
  _visibleDocumentTree() {
    _pageTreeCache.updateViews(widget.views);
    return _pageTreeCache.visibleRows(_expandedViewIds);
  }

  /// Toggling a folder also locates it (the "target" for the top
  /// New-page/New-folder buttons); expansion lives in the tree controller.
  void _toggleViewExpand(DocumentView view) {
    _tree.toggle(view.id);
    _pruneSelectionToVisible();
  }

  /// Drop selected rows that are no longer on screen.
  ///
  /// Collapsing a folder hides its children, and a selection is only honest
  /// while you can see what is in it: without this, "删除 5 项" would delete
  /// three, because everything downstream reads the VISIBLE tree. Keeping the
  /// hidden ones instead is worse — that is a batch acting on rows the user
  /// folded away and stopped thinking about.
  void _pruneSelectionToVisible() {
    if (_selection.isEmpty) return;
    final visible = {for (final row in _visibleDocumentTree()) row.view.id};
    if (_selection.retainVisible(visible)) setState(() {});
  }

  /// `workspace/folder/.../page` for the open page — GitHub's "copy path", for
  /// telling someone (or an agent) WHERE something is.
  ///
  /// The workspace leads because a path without it is ambiguous the moment
  /// there is more than one, and whoever you paste it to cannot guess which you
  /// meant. Segments and separator rationale live on [pagePathSegments], which
  /// the breadcrumb reads too.
  String? _pagePath() => pagePathSegments(
    workspaceName: widget.selectedWorkspace?.name,
    view: widget.selectedView,
    views: widget.views,
  )?.join('/');

  /// Put [_pagePath] on the clipboard.
  Future<bool> _copyPagePath() async {
    final l10n = context.l10n;
    final path = _pagePath();
    if (path == null) {
      ScaffoldMessenger.of(
        context,
      ).showSnackBar(SnackBar(content: Text(l10n.pageOpenFirst)));
      return false;
    }
    final ok = await copyTextToClipboard(path);
    if (!mounted) return ok;
    // SUCCESS says nothing here. The button that was just pressed turns into a
    // green check where the cursor already is (GitHub's "Copied!" affordance) —
    // a snackbar for a working copy throws a black bar across the bottom of the
    // window to report that the expected thing happened, which is the loudest
    // possible way to say the least.
    //
    // FAILURE still speaks: it is unexpected, it needs words, and there is no
    // icon state that can carry "the clipboard refused".
    if (!ok) {
      ScaffoldMessenger.of(
        context,
      ).showSnackBar(SnackBar(content: Text(l10n.pageCopyFailed)));
    }
    return ok;
  }

  /// Expand a view's ancestors and scroll it into view in the sidebar.
  ///
  /// The expand half already existed for folder hits; the scroll half did not,
  /// so "locate this" quietly did nothing whenever the row sat below the fold —
  /// which on a 434-page workspace is most of them.
  ///
  /// Kept as a PENDING id rather than scrolling here, because the row usually
  /// does not exist yet at this moment: its ancestors are expanded in the same
  /// `setState`, and a cross-workspace hit has to wait for the other
  /// workspace's tree to load at all. [_flushPendingReveal] retries per frame
  /// until the row is on screen.
  /// Locate [viewId] when its tree has it, and remember it either way —
  /// the row may belong to a workspace whose views have not arrived yet.
  void _revealInTree(String viewId) => _tree.revealInTree(widget.views, viewId);

  /// Scroll the pending row into view, then stop asking.
  ///
  /// Computed from the row's INDEX, not from its widget. The first version
  /// asked `Scrollable.ensureVisible` for the row's `BuildContext` via a
  /// GlobalKey — which is null for exactly the rows that need scrolling: the
  /// tree is a lazy `ListView`, so a row below the fold has no element at all.
  /// It worked only for rows already on screen, i.e. never usefully. Reported
  /// as "看着没效果".
  ///
  /// Rows are a uniform height inside one workspace ([_treeRowExtent]), so
  /// index × extent lands within a pixel or two, and centring absorbs that.
  void _flushPendingReveal() {
    final id = _tree.pendingRevealId;
    if (id == null) return;
    if (!_tree.bumpRevealAttempt()) {
      // Gave up after about 2s of frames (deleted page, a workspace that
      // failed to load); the controller stopped rather than run a
      // callback every frame forever.
      return;
    }
    // The tree may only have arrived now (workspace switch): expand ancestors,
    // then WAIT A FRAME before scrolling.
    //
    // Scrolling in the same pass lands short, which is what the user saw:
    // expanding adds rows, so the list gets taller — but `maxScrollExtent`
    // still describes the layout measured BEFORE those rows existed, and the
    // clamp below pins the target to that stale, smaller maximum. The shortfall
    // is exactly the height of the rows just revealed, which is why it looked
    // like "scrolled, but you still have to drag a bit further".
    if (widget.views.any((v) => v.id == id) && !_tree.isExpanded(id)) {
      _revealFolder(id);
      return; // re-entered next frame, against the grown list
    }
    final rows = _visibleDocumentTree();
    final index = rows.indexWhere((r) => r.view.id == id);
    // Not in the tree yet, or still inside a parent this frame's expand has not
    // rendered through — keep waiting.
    if (index < 0 || !_treeScroll.hasClients) return;

    _tree.clearPendingReveal();
    final position = _treeScroll.position;
    unawaited(
      _treeScroll.animateTo(
        treeRevealOffset(
          index: index,
          rowExtent: _treeRowExtent,
          viewportHeight: position.viewportDimension,
          maxScrollExtent: position.maxScrollExtent,
          minScrollExtent: position.minScrollExtent,
        ),
        duration: const Duration(milliseconds: 200),
        curve: Curves.easeOut,
      ),
    );
  }

  /// Both editable drag wrappers and read-only rows add an 8px gap to the
  /// DocumentListItem's 38px floor. Keep locating distant rows in this same unit.
  double get _treeRowExtent => 46.0;

  /// Locate [viewId] and expand it plus its ancestors; an id not in this
  /// workspace's tree does nothing. Logic in the tree controller.
  void _revealFolder(String viewId) => _tree.revealFolder(widget.views, viewId);

  /// Expand every ANCESTOR of [id] so a (possibly nested) node is revealed in
  /// the sidebar — used on navigate/create so the active/new page is never
  /// hidden under a collapsed parent (AppFlowy/Notion "reveal current page").
  /// The node itself is not expanded (that would show ITS children).
  void _revealAncestors(String id) => _tree.revealAncestors(widget.views, id);

  /// Expand [id] itself (so a freshly-created/dropped child under it is visible)
  /// plus its ancestor chain.
  void _expandForChildOf(String id) => _tree.expandForChildOf(widget.views, id);
}
