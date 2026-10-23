part of 'main.dart';

extension _WorkspaceEditorPane on _WorkspaceViewState {
  /// The editor pane, with the tab strip on top of whatever it renders.
  ///
  /// The strip wraps [_editorPaneBody] rather than living inside it because the
  /// body returns early for every empty state — no workspace, no page open. A
  /// strip nested below those returns would vanish the moment one of its tabs
  /// held nothing, which is exactly when the user needs it to switch away.
  Widget _editorPane(BuildContext context) {
    final onSelectTab = widget.onSelectTab;
    final onCloseTab = widget.onCloseTab;
    final body = _editorPaneBody(context);
    // Only the local world skips the strip now — it has no tab model at all.
    // The "fewer than two tabs" case used to skip it too; the strip is resident
    // since 2026-08-12 (user's call), because hiding it also hid the `+`, and
    // that left a single-tab window with no visible way to open a second one.
    if (onSelectTab == null || onCloseTab == null || widget.tabs.isEmpty) {
      return body;
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        DocTabStrip(
          tabs: widget.tabs,
          activeIndex: widget.activeTabIndex,
          onSelect: onSelectTab,
          onClose: onCloseTab,
          untitledLabel: context.l10n.untitledPage,
          onNewTab: widget.onNewTabPage == null ? null : _showNewTabMenu,
          newTabTooltip: context.l10n.tabNewTooltip,
          foreignWorkspaceName: _foreignWorkspaceName,
        ),
        Expanded(child: body),
      ],
    );
  }

  /// The workspace name to show on a tab that does not belong to the workspace
  /// on screen — null for every ordinary tab.
  ///
  /// Named rather than merely marked: "this tab is from somewhere else" leaves
  /// you having to click it to find out where, and clicking is the thing that
  /// moves you.
  String? _foreignWorkspaceName(DocTab tab) {
    final id = tab.workspaceId;
    if (id == null || id == widget.selectedWorkspace?.id) return null;
    for (final w in widget.workspaces) {
      if (w.id == id) return w.name;
    }
    // Stamped with a workspace this account can no longer see. Say nothing
    // rather than invent a name — the tab is already in the state
    // `_workspaceForTab` refuses to sync.
    return null;
  }

  /// The tab strip's `+`: create a page, or pick an existing one — both into a
  /// new tab.
  ///
  /// The picker is the app's EXISTING search dialog, not a list built here.
  /// AppFlowy's `+` opens its command palette (search + recents + "new page")
  /// and Mica already has the search half of that; a second page list would be
  /// the two-representations trap this repo keeps paying for. The visible
  /// difference from AppFlowy is that ours has no recents — search shows
  /// nothing until you type. Adding recents is a change to search itself, and
  /// it would then apply everywhere search is opened from, which is the right
  /// way to get it and not something to bolt onto this menu.
  Future<void> _showNewTabMenu(Offset globalPosition) async {
    final onNewPage = widget.onNewTabPage;
    if (onNewPage == null) return;
    final overlay =
        Overlay.of(context).context.findRenderObject() as RenderBox?;
    if (overlay == null) return;
    final local = overlay.globalToLocal(globalPosition);
    final picked = await showMenu<String>(
      context: context,
      position: RelativeRect.fromLTRB(
        local.dx,
        local.dy,
        overlay.size.width - local.dx,
        0,
      ),
      items: [
        PopupMenuItem(
          value: 'new',
          child: _MenuRow(icon: Icons.add, label: context.l10n.tabNewPage),
        ),
        PopupMenuItem(
          value: 'open',
          child: _MenuRow(
            icon: Icons.search,
            label: context.l10n.tabOpenExisting,
          ),
        ),
      ],
    );
    if (!mounted) return;
    switch (picked) {
      case 'new':
        onNewPage();
      case 'open':
        _openSearch(inNewTab: true);
    }
  }

  Widget _editorPaneBody(BuildContext context) {
    final workspace = widget.selectedWorkspace;
    if (workspace == null) {
      return EmptyState(
        icon: Icons.ads_click,
        title: context.l10n.editorSelectWorkspaceTitle,
        detail: context.l10n.editorSelectWorkspaceDetail,
      );
    }

    final bootstrap = widget.selectedBootstrap;
    if (bootstrap == null) {
      // Nothing open → HOME (design 03). This used to be a "pick a page" empty
      // state, which told the user to do something instead of helping them do
      // it; home offers the create action and the recent pages right there.
      // Falls back to the old copy only if the host supplies no home pane.
      return widget.overviewPane ??
          widget.homePane ??
          EmptyState(
            icon: Icons.description_outlined,
            title: context.l10n.editorSelectPageTitle,
            detail: context.l10n.editorSelectPageDetail,
          );
    }

    final canEdit = matchesEditRole(workspace.role);

    return ColoredBox(
      color: MicaTheme.of(context).surface.base,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          // ONE top row for the pane, so the page tools line up with the left
          // sidebar's collapse button instead of sitting 37px below it in the
          // document column. Notion puts the same cluster here; the row exists
          // whether or not the format bar is on, because the tools do.
          Padding(
            padding: const EdgeInsets.fromLTRB(8, 4, 8, 0),
            child: LayoutBuilder(
              builder: (context, header) => Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Row(
                    children: [
                      // The path comes first and takes what it needs, inside two
                      // bounds — see [_pathBudget]. It used to be a flat 260, which
                      // dropped ancestors on paths that had most of the row sitting
                      // empty beside them: an ellipsis with no visible cause.
                      //
                      // The pixel-based collapse in PageBreadcrumb stays as the last
                      // resort. It is NOT a depth rule ("collapse past 3 segments") —
                      // this repo tried that and recorded why it failed: `tools ›
                      // 笔记软件 › mica › 单机手动部署（IP 直连，不用 Traefik）` is
                      // three segments and still overruns. AppFlowy gets away with a
                      // depth rule because their breadcrumb has no width cap at all
                      // and simply scrolls, which a shared row cannot do.
                      if (widget.selectedView != null)
                        ConstrainedBox(
                          constraints: BoxConstraints(
                            maxWidth: _pathBudget(
                              header.maxWidth,
                              barOn:
                                  widget.showFormatBar &&
                                  canEdit &&
                                  header.maxWidth >= 720,
                            ),
                          ),
                          child: PageBreadcrumb(
                            views: widget.views,
                            current: widget.selectedView!,
                            onSelect: widget.onSelectView,
                            // Renaming from the breadcrumb tail (AppFlowy does this).
                            // Gated on the editor role: a viewer's rename would 403,
                            // and an edit affordance that cannot succeed is worse than
                            // none — same rule as everywhere else here.
                            onRename: canEdit ? widget.onRenameView : null,
                            onCopyPath: _copyPagePath,
                            workspaceName: widget.selectedWorkspace?.name,
                            trailing: Row(
                              mainAxisSize: MainAxisSize.min,
                              children: [
                                // Quiet sync status (nothing when synced). It STAYS
                                // on the breadcrumb while the tools moved to the pane
                                // header: it reports on this document, not on the app
                                // — same reason the breadcrumb itself lives here.
                                if (widget.syncPhase != null)
                                  _SyncBadge(widget.syncPhase!),
                              ],
                            ),
                          ),
                        ),
                      // The format bar takes whatever the path left. It is already a
                      // horizontal SingleChildScrollView, so a narrow share does not
                      // break it — it scrolls. [_pathBudget] guarantees that share
                      // never drops below _formatBarFloor.
                      Expanded(
                        child:
                            (widget.showFormatBar &&
                                canEdit &&
                                header.maxWidth >= 720)
                            ? ListenableBuilder(
                                listenable: _activeBlockHook,
                                builder: (context, _) => _formatBar(context),
                              )
                            : const SizedBox.shrink(),
                      ),
                      // The right-hand twin of the sidebar collapse button.
                      // It used to live one row lower, on the title row, so
                      // the "symmetric pair" the old comment claimed was
                      // visibly off — different row, different size. Same
                      // line, same density, same icon size as the left one.
                      IconButton(
                        tooltip: _toolsExpanded
                            ? context.l10n.pageHideSidePanel
                            : context.l10n.pageShowSidePanel,
                        visualDensity: VisualDensity.compact,
                        onPressed: () =>
                            setState(() => _toolsExpanded = !_toolsExpanded),
                        icon: Transform.flip(
                          flipX: true,
                          child: const Icon(
                            Icons.view_sidebar_outlined,
                            size: 20,
                          ),
                        ),
                      ),
                      if (widget.onAddComment != null)
                        _CommentsButton(
                          openCount: widget.commentThreads
                              .where((t) => !t.isResolved)
                              .length,
                          active:
                              _toolsExpanded && _toolsTab == _ToolsTab.comments,
                          onTap: () {
                            final showing =
                                _toolsExpanded &&
                                _toolsTab == _ToolsTab.comments;
                            setState(() {
                              // From anywhere else this means "show me the
                              // comments", not "toggle the sidebar" — so it
                              // opens the sidebar AND selects the tab. Only
                              // a second press on an already-showing comment
                              // tab closes it.
                              _toolsExpanded = !showing;
                              _toolsTab = _ToolsTab.comments;
                            });
                            // Closing drops the emphasis: it means "the
                            // thread I am reading", and with the panel gone
                            // there is no such thing — leaving it on would
                            // strand a stronger wash with nothing to explain
                            // it.
                            if (showing) widget.onFocusCommentThread(null);
                          },
                        ),
                      _PropertiesToggle(
                        active: _showProperties,
                        hasProperties: bootstrap.rootFrontMatter
                            .trim()
                            .isNotEmpty,
                        onTap: () =>
                            setState(() => _showProperties = !_showProperties),
                      ),
                      PopupMenuButton<String>(
                        tooltip: context.l10n.pageMenu,
                        // Same density as its neighbours — it used to sit alone
                        // on the title row at the default size.
                        padding: EdgeInsets.zero,
                        icon: const Icon(Icons.more_horiz, size: 20),
                        onSelected: _onPageMenu,
                        itemBuilder: (context) => [
                          // Copy sits above the exports and gets its own group:
                          // it is the cheap, frequent one (grab the text, paste
                          // it somewhere), while everything below produces a
                          // file. Grouping it with the exports would bury it.
                          PopupMenuItem(
                            value: 'copy-md',
                            child: ListTile(
                              dense: true,
                              contentPadding: EdgeInsets.zero,
                              leading: const Icon(Icons.copy_all_outlined),
                              title: Text(context.l10n.pageCopyContent),
                            ),
                          ),
                          const PopupMenuDivider(),
                          // One export, always a ZIP. The old "Export Markdown"
                          // handed back a lone .md whose images pointed at
                          // `![](photo.png)` — a file that was nowhere in the
                          // download. A zip can carry them; a .md cannot.
                          PopupMenuItem(
                            value: 'export-zip',
                            child: ListTile(
                              dense: true,
                              contentPadding: EdgeInsets.zero,
                              leading: const Icon(Icons.folder_zip_outlined),
                              title: Text(context.l10n.rowExportZipImages),
                            ),
                          ),
                          PopupMenuItem(
                            value: 'export-html',
                            child: ListTile(
                              dense: true,
                              contentPadding: EdgeInsets.zero,
                              leading: const Icon(Icons.html_outlined),
                              title: Text(context.l10n.rowExportHtml),
                            ),
                          ),
                          // PDF export: desktop drives the OS WebView2 runtime's
                          // headless print (native bytes → download); web hands
                          // the same self-contained HTML to the browser's own
                          // print dialog ("Save as PDF"). Available on both.
                          PopupMenuItem(
                            value: 'export-pdf',
                            child: ListTile(
                              dense: true,
                              contentPadding: EdgeInsets.zero,
                              leading: const Icon(
                                Icons.picture_as_pdf_outlined,
                              ),
                              title: Text(context.l10n.rowExportPdf),
                            ),
                          ),
                          const PopupMenuDivider(),
                          PopupMenuItem(
                            value: 'import-md',
                            child: ListTile(
                              dense: true,
                              contentPadding: EdgeInsets.zero,
                              leading: const Icon(Icons.upload_file_outlined),
                              title: Text(context.l10n.pageImportMarkdown),
                            ),
                          ),
                          if (widget.onShare != null) ...[
                            const PopupMenuDivider(),
                            PopupMenuItem(
                              value: 'share',
                              child: ListTile(
                                dense: true,
                                contentPadding: EdgeInsets.zero,
                                leading: const Icon(Icons.public),
                                title: Text(context.l10n.shareTitle),
                              ),
                            ),
                          ],
                          if (widget.onVersionHistory != null) ...[
                            const PopupMenuDivider(),
                            PopupMenuItem(
                              value: 'version-history',
                              child: ListTile(
                                dense: true,
                                contentPadding: EdgeInsets.zero,
                                leading: const Icon(Icons.history),
                                title: Text(context.l10n.versionHistoryTitle),
                              ),
                            ),
                          ],
                          if (widget.onRestoreCheckpoint != null) ...[
                            const PopupMenuDivider(),
                            PopupMenuItem(
                              value: 'restore-checkpoint',
                              child: ListTile(
                                dense: true,
                                contentPadding: EdgeInsets.zero,
                                leading: const Icon(Icons.restore_outlined),
                                title: Text(context.l10n.pageRestoreCheckpoint),
                              ),
                            ),
                          ],
                        ],
                      ),
                    ],
                  ),
                  if (widget.showFormatBar && canEdit && header.maxWidth < 720)
                    ListenableBuilder(
                      listenable: _activeBlockHook,
                      builder: (context, _) => _formatBar(context),
                    ),
                ],
              ),
            ),
          ),
          // The editor column is capped at widget.pageWidth (a fixed page-width
          // step) inside _editorScroll, and the window caps it below that on a
          // narrow pane — so no measured max is needed here.
          Expanded(
            child: Stack(
              children: [
                Positioned.fill(
                  child: _editorScroll(context, canEdit, bootstrap),
                ),
                // Bottom-right of the PANE, so it reads as a status
                // readout that is always where you left it — not a chip
                // that lands wherever the text happens to stop.
                Positioned(
                  right: 12,
                  bottom: 8,
                  child: ValueListenableBuilder<DocCounts>(
                    valueListenable: _counts,
                    builder: (context, counts, _) => counts.chars == 0
                        ? const SizedBox.shrink()
                        : _WordCountBadge(counts: counts),
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  /// Enter in the page title: split at the caret — the remainder becomes a
  /// new first body line (pushing the body down), the caret follows it.
  void _titleEnter() {
    final text = _pageTitle.text;
    final sel = _pageTitle.selection;
    final at = sel.isValid ? sel.start.clamp(0, text.length) : text.length;
    final rest = text.substring(at);
    if (rest.isNotEmpty) {
      _pageTitle.text = text.substring(0, at);
      _schedulePageTitleSave();
    }
    _commandHook.insertTopParagraph(rest);
  }

  /// The optional formatting toolbar (Settings -> Appearance, off by
  /// default): one-click access to the high-frequency Markdown actions,
  /// driven through [_commandHook] so focus/selection semantics stay in the
  /// editor.
  /// Narrowest share of the header the format bar may be squeezed to.
  ///
  /// MEASURED, not chosen: the full strip is 19 buttons over 606px. 360 keeps
  /// roughly the first eleven — undo/redo, the three heading levels, bold /
  /// italic / strike, code, link — visible without scrolling, which is the set
  /// you reach for without looking. Past that the bar scrolls, which it has
  /// always been able to do (it is a horizontal SingleChildScrollView).
  static const _formatBarFloor = 360.0;

  /// The right-hand cluster (side-panel toggle, comments, properties, page
  /// menu) shares this row too, and is not flexible. Reserved alongside the
  /// bar's floor so the floor is a floor on what the BAR gets, not on what is
  /// left over before the icons take their cut.
  static const _headerIconsWidth = 160.0;

  /// How wide the breadcrumb may get.
  ///
  /// "Path first" — it takes what it needs — under two bounds, and each one is
  /// the answer to a question that was actually asked:
  ///
  ///  * A ceiling at 70% of the header, so a pathological title cannot span the
  ///    whole thing. Real names and real nesting stay well under it; the one
  ///    that reached it was a hand-typed `asdfsadfasfdasfds…`.
  ///  * With the bar on, a reserve for the bar: the path may not eat the last
  ///    [_formatBarFloor] + [_headerIconsWidth] of the row. That is a bound on
  ///    what the PATH may take — not a promise about the bar's rendered width,
  ///    which `_formatBar` narrows again with its own `Center(maxWidth:
  ///    pageWidth)`. Measured on a 1050-wide window the bar ends up around
  ///    317px and shows ten buttons before it scrolls.
  ///
  /// The 200 floor at the end is for a genuinely narrow pane, where the two
  /// bounds cross and something has to give; there the path keeps enough to
  /// show the page name and the bar scrolls from the start.
  ///
  /// Measured, bar on, this page's path needing 221px:
  ///
  ///   window 1600 -> path 221, bar 606 (everything fits, no scroll)
  ///   window 1400 -> path 221, bar 562
  ///   window 1280 -> path 221, bar 442
  ///   window 1050 -> path yields, bar 317
  ///
  /// The path holding its natural 221 rather than its whole budget is the point
  /// of `MainAxisSize.min` on PageBreadcrumb's Row — a cap is not a demand.
  double _pathBudget(double header, {required bool barOn}) {
    final ceiling = header * 0.7;
    if (!barOn)
      return math.max(0, math.min(ceiling, header - _headerIconsWidth));
    return math.max(
      math.min(ceiling, header - _formatBarFloor - _headerIconsWidth),
      200.0,
    );
  }

  Widget _formatBar(BuildContext context) {
    Widget btn(
      IconData icon,
      String tip,
      VoidCallback onTap, {
      bool active = false,
    }) {
      return Tooltip(
        message: tip,
        waitDuration: const Duration(milliseconds: 500),
        child: InkWell(
          borderRadius: BorderRadius.circular(6),
          onTap: onTap,
          child: Container(
            decoration: active
                ? BoxDecoration(
                    color: MicaTheme.of(
                      context,
                    ).accent.primary.withValues(alpha: 0.08),
                    borderRadius: BorderRadius.circular(6),
                  )
                : null,
            padding: const EdgeInsets.all(6),
            child: Icon(
              icon,
              size: 18,
              color: active
                  ? MicaTheme.of(context).accent.primary
                  : MicaTheme.of(context).text.muted,
            ),
          ),
        ),
      );
    }

    Widget divider() => Container(
      width: 1,
      height: 18,
      margin: const EdgeInsets.symmetric(horizontal: 4),
      color: MicaTheme.of(context).border.normal,
    );

    final h = _commandHook;
    final k = _activeBlockHook.kind;
    final lvl = _activeBlockHook.level;
    bool onHeading(int n) => k == 'heading' && lvl == n;

    /// A heading button TOGGLES: pressing the level you are already on puts the
    /// block back to body text.
    ///
    /// Reported 2026-08-12. Each level used to re-apply itself unconditionally,
    /// which made the lit button the only control on this bar that did nothing
    /// when pressed — every neighbour (bold, quote, list) has always toggled.
    ///
    /// One builder rather than three near-identical `btn(...)` calls, because
    /// the toggle has to be identical across the levels and this bar and the
    /// floating one already disagreed once by being written out twice.
    Widget headingBtn(IconData icon, String label, int level) =>
        btn(icon, label, () {
          final target = headingButtonTarget(
            kind: k,
            // Null level on a non-heading block; 0 is never a heading level, so
            // it can only ever answer "not the one you pressed".
            level: lvl ?? 0,
            pressed: level,
          );
          h.setBlock(target.kind, target.data);
        }, active: onHeading(level));
    // TextFieldTapRegion: clicking the toolbar must not read as a tap OUTSIDE
    // the table-cell editor's TextField — onTapOutside would unfocus/commit the
    // cell on pointer-down, so the command would land after the cell closed.
    return TextFieldTapRegion(
      child: Container(
        decoration: BoxDecoration(
          color: MicaTheme.of(context).surface.base,
          border: Border(
            bottom: BorderSide(color: MicaTheme.of(context).border.normal),
          ),
        ),
        padding: const EdgeInsets.symmetric(vertical: 3),
        // Keep the compact toolbar centered and let its controls scroll within
        // the available header width; narrow panes give it a separate row.
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 4),
          child: Center(
            child: ConstrainedBox(
              constraints: BoxConstraints(maxWidth: widget.pageWidth),
              child: Padding(
                padding: EdgeInsets.zero,
                child: SingleChildScrollView(
                  scrollDirection: Axis.horizontal,
                  child: Row(
                    children: [
                      btn(Icons.undo, context.l10n.pageFormatUndo, h.undo),
                      btn(Icons.redo, context.l10n.pageFormatRedo, h.redo),
                      divider(),
                      btn(
                        Icons.notes,
                        context.l10n.pageFormatText,
                        () => h.setBlock('paragraph'),
                        active: k == 'paragraph',
                      ),
                      headingBtn(
                        Icons.looks_one_outlined,
                        context.l10n.pageFormatHeading1,
                        1,
                      ),
                      headingBtn(
                        Icons.looks_two_outlined,
                        context.l10n.pageFormatHeading2,
                        2,
                      ),
                      headingBtn(
                        Icons.looks_3_outlined,
                        context.l10n.pageFormatHeading3,
                        3,
                      ),
                      divider(),
                      btn(
                        Icons.format_bold,
                        context.l10n.pageFormatBold,
                        () => h.toggleMark('bold'),
                      ),
                      btn(
                        Icons.format_italic,
                        context.l10n.pageFormatItalic,
                        () => h.toggleMark('italic'),
                      ),
                      btn(
                        Icons.format_strikethrough,
                        context.l10n.pageFormatStrikethrough,
                        () => h.toggleMark('strike'),
                      ),
                      btn(
                        Icons.code,
                        context.l10n.pageFormatInlineCode,
                        () => h.toggleMark('code'),
                      ),
                      btn(Icons.link, context.l10n.pageFormatLink, h.editLink),
                      divider(),
                      btn(
                        Icons.format_list_bulleted,
                        context.l10n.pageFormatBulletedList,
                        () => h.setBlock('bulleted_list'),
                        active: k == 'bulleted_list',
                      ),
                      btn(
                        Icons.format_list_numbered,
                        context.l10n.pageFormatNumberedList,
                        () => h.setBlock('numbered_list'),
                        active: k == 'numbered_list',
                      ),
                      btn(
                        Icons.check_box_outlined,
                        context.l10n.pageFormatTodoList,
                        () => h.setBlock('todo', {'checked': false}),
                        active: k == 'todo',
                      ),
                      btn(
                        Icons.format_quote,
                        context.l10n.pageFormatQuote,
                        () => h.setBlock('quote'),
                        active: k == 'quote',
                      ),
                      btn(
                        Icons.terminal,
                        context.l10n.pageFormatCodeBlock,
                        () => h.setBlock('code_block'),
                        active: k == 'code_block',
                      ),
                      divider(),
                      btn(
                        Icons.horizontal_rule,
                        context.l10n.pageFormatDivider,
                        () => h.insert('divider'),
                      ),
                      btn(
                        Icons.grid_on,
                        context.l10n.pageFormatTable,
                        () => h.insert('table'),
                      ),
                      btn(
                        Icons.image_outlined,
                        context.l10n.pageFormatImage,
                        () => h.insert('image'),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }

  Widget _editorScroll(
    BuildContext context,
    bool canEdit,
    DocumentBootstrap bootstrap,
  ) {
    return Listener(
      // A press anywhere on the page that is NOT inside the editor canvas
      // (margins beside/above the page column) resets diagram zoom/pan —
      // clicks INSIDE the canvas are judged by the editor itself.
      behavior: HitTestBehavior.translucent,
      onPointerDown: (e) {
        final box =
            _editorSurfaceKey.currentContext?.findRenderObject() as RenderBox?;
        if (box == null || !box.attached) return;
        final local = box.globalToLocal(e.position);
        if (!(Offset.zero & box.size).contains(local)) {
          _commandHook.resetDiagramViews();
        }
      },
      child: LayoutBuilder(
        builder: (context, constraints) => SingleChildScrollView(
          padding: EdgeInsets.fromLTRB(
            constraints.maxWidth < 600 ? 12 : 28,
            8,
            constraints.maxWidth < 600 ? 12 : 28,
            28,
          ),
          child: Center(
            child: ConstrainedBox(
              constraints: BoxConstraints(maxWidth: widget.pageWidth),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  // The editor canvas reserves EditorTheme.gutter on the left
                  // for the block drag handles, so its text starts at x=gutter.
                  // Inset the title + meta rows by the same amount to keep the
                  // page's text column aligned (handles float in the margin).
                  Padding(
                    padding: const EdgeInsets.only(left: EditorTheme.gutter),
                    child: Row(
                      crossAxisAlignment: CrossAxisAlignment.center,
                      children: [
                        // Settings can hide the title; the row itself stays for
                        // the page menu + side-panel toggle.
                        if (!widget.showPageTitle)
                          const Spacer()
                        else
                          Expanded(
                            child: PageTitleField(
                              controller: _pageTitle,
                              focusNode: _pageTitleFocus,
                              appearance: _editorAppearance,
                              hintText: context.l10n.untitledPage,
                              readOnly: !canEdit,
                              onChanged: (_) => _schedulePageTitleSave(),
                              onEnterBody: _titleEnter,
                              onArrowDown: _commandHook.focusFirstLine,
                            ),
                          ),
                      ],
                    ),
                  ),
                  const SizedBox(height: 4),
                  // Collaborator presence, right-aligned under the title row —
                  // shown ONLY when someone else is here. Solo, this was an
                  // "Only you" line that just widened the title↔body gap for no
                  // reason (the space the user flagged).
                  if (widget.presence.isNotEmpty) ...[
                    Padding(
                      padding: const EdgeInsets.only(left: EditorTheme.gutter),
                      child: Row(
                        children: [
                          const Spacer(),
                          _PresenceBar(presence: widget.presence),
                        ],
                      ),
                    ),
                    const SizedBox(height: 4),
                  ],
                  // Page properties (front-matter) — a structured, editable view
                  // of the document's YAML front matter, above the body. Reads the
                  // root block's `data['front_matter']`; commits write it back
                  // through the same self-dispatching op sink the editor uses
                  // (local / cloud-CRDT / cloud-REST all handled). See
                  // docs/features/page-properties.md.
                  if (_showProperties)
                    PropertyPanel(
                      frontMatter: bootstrap.rootFrontMatter,
                      canEdit: canEdit,
                      // Clicking a tag opens workspace search for `#tag` — the
                      // precise token the index stores for list/tag values, so it
                      // finds pages that CARRY the tag, not ones that merely
                      // mention the word in their body (M2).
                      onOpenTag: (v) => _openSearch(initialQuery: '#$v'),
                      onCommit: (fm) {
                        final data = Map<String, dynamic>.from(
                          bootstrap.rootData,
                        );
                        if (fm.isEmpty) {
                          data.remove('front_matter');
                        } else {
                          data['front_matter'] = fm;
                        }
                        return (widget.onApplyMaintenanceOperations ??
                            widget.onApplyOperations)([
                          {
                            'type': 'update_block',
                            'block_id': bootstrap.document.rootBlockId,
                            'data': data,
                          },
                        ]);
                      },
                    ),
                  Padding(
                    key: _editorSurfaceKey,
                    padding: const EdgeInsets.only(top: 4),
                    child: MicaEditor(
                      countsSink: _counts,
                      key: ValueKey(
                        '${bootstrap.document.id}#${widget.editorEpoch}',
                      ),
                      focusNode: _editorFocus,
                      rootBlockId: bootstrap.document.rootBlockId,
                      nodes: _editorNodes(bootstrap.childBlocks),
                      version: bootstrap.snapshot.versionSeq,
                      canEdit: canEdit,
                      onSelectionChanged: widget.onCursorChanged,
                      commentHighlights: widget.commentHighlights,
                      onAddComment: widget.onAddComment == null
                          ? null
                          : (sb, so, eb, eo, quote) async {
                              await widget.onAddComment!(sb, so, eb, eo, quote);
                              // You just wrote it; showing it should not cost
                              // another click. Only for a comment THIS user made:
                              // opening because a collaborator commented would be
                              // an interruption, not a convenience.
                              if (mounted) {
                                setState(() {
                                  _toolsExpanded = true;
                                  _toolsTab = _ToolsTab.comments;
                                });
                              }
                            },
                      remoteCursors: _editorCursors(widget.presence),
                      onApplyOperations: widget.onApplyOperations,
                      onApplyHistory: widget.onApplyHistory,
                      onOpFault: widget.onEditorFault,
                      onUploadImage: widget.onUploadImage,
                      onImportImageUrl: widget.onImportImageUrl,
                      onLoadImageBytes: widget.onLoadImageBytes,
                      onResolveImageUrls: widget.onResolveImageUrls,
                      onAiStream: widget.showAi ? widget.onAiStream : null,
                      reHostImages: widget.reHostImages,
                      scrollHook: _scrollHook,
                      commandHook: _commandHook,
                      outlineHook: _outlineHook,
                      findHook: _findHook,
                      activeBlockHook: _activeBlockHook,
                      onExitTop: _focusPageTitle,
                      appearance: _editorAppearance,
                      onOpenPage: _openPageLink,
                      pageLinks: () => [
                        for (final v in widget.views)
                          if (v.objectType == 'document')
                            PageLinkTarget(id: v.id, title: v.name),
                      ],
                    ),
                  ),
                  // Reverse references — the cloud pages that link to this one.
                  // Cloud + documents only (folders have no body); lazy-loaded so
                  // it never blocks the page opening. Hidden entirely in 本地模式
                  // (onLoadBacklinks null there).
                  if (widget.onLoadBacklinks != null &&
                      widget.selectedView?.objectType == 'document')
                    Padding(
                      padding: const EdgeInsets.only(left: EditorTheme.gutter),
                      child: _BacklinksPanel(
                        key: ValueKey('backlinks#${widget.selectedView!.id}'),
                        viewId: widget.selectedView!.id,
                        load: widget.onLoadBacklinks!,
                        onOpen: widget.onOpenSearchResult,
                        onLoaded: _onBacklinksLoaded,
                      ),
                    ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }

  /// The outline row that stands for the page title. Not a block id — the title
  /// is not a block — so it is a sentinel no document can produce.
  static const String _outlineTitleId = '\u0000outline-page-title';

  /// Put the caret at the end of the page-title field.
  ///
  /// Two callers: pressing Up on the document's first line (`onExitTop`), and
  /// tapping the title row in the outline. Shared rather than duplicated because
  /// of the web quirk below, which took a wrong attempt to find.
  void _focusPageTitle() {
    if (!widget.showPageTitle) return;
    _pageTitleFocus.requestFocus();
    // The web TextField select-alls when focused programmatically; that happens
    // in the focus microtask, so queue ours right behind it — the caret is
    // collapsed before the next frame paints (a post-frame callback here
    // flashed the selection for one frame).
    Future.microtask(() {
      if (!mounted) return;
      _pageTitle.selection = TextSelection.collapsed(
        offset: _pageTitle.text.length,
      );
    });
  }

  /// Tappable outline entries for the current page's headings (no section
  /// header). Tapping scrolls the editor to that heading. Fed the LIVE heading
  /// list from [_outlineHook] (republished by the editor on every edit), so it
  /// tracks typing — not just navigation.
  List<Widget> _pageOutlineItems(
    BuildContext context,
    List<OutlineEntry> headings,
  ) {
    // The page's own title leads its table of contents.
    //
    // It used to be missing entirely, which reads as a bug once you notice it:
    // a document's outline that never mentions the document. The title is not a
    // heading BLOCK — it lives outside the editor's document (see
    // `docs/features/page-title-plan.md`) — so the editor's live heading feed cannot
    // carry it. It is prepended here, where the page is known.
    //
    // Level 1 on purpose: every heading the author writes then indents under it,
    // which is what "this is the root of the page" should look like. An empty or
    // still-untitled name contributes nothing rather than a blank row.
    final title = widget.selectedView?.name.trim() ?? '';
    final entries = <OutlineEntry>[
      if (title.isNotEmpty && !isUntitledPageName(title))
        OutlineEntry(id: _outlineTitleId, text: title, level: 1),
      ...headings,
    ];
    return [
      for (final h in entries)
        if (h.text.trim().isNotEmpty)
          InkWell(
            // The title row has no block to scroll to — it is not in the
            // document — so it puts the caret in the title field instead, which
            // is both "go to the top" and "here is where you rename it".
            onTap: () => h.id == _outlineTitleId
                ? _focusPageTitle()
                : _scrollHook.scrollToBlock(h.id),
            borderRadius: BorderRadius.circular(6),
            child: Padding(
              padding: EdgeInsets.only(
                left: 4.0 + 14 * ((h.level.clamp(1, 6) - 1).clamp(0, 5)),
                top: 5,
                bottom: 5,
                right: 4,
              ),
              child: Text(
                h.text.trim(),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  fontSize: h.level <= 1 ? 14 : 13,
                  fontWeight: h.level <= 1 ? FontWeight.w600 : FontWeight.w400,
                  color: MicaTheme.of(context).text.primary,
                ),
              ),
            ),
          ),
    ];
  }

  int _headingLevel(DocumentBlock b) {
    final level = b.data['level'];
    if (level is int) return level.clamp(1, 6);
    if (level is num) return level.toInt().clamp(1, 6);
    return 1;
  }
}
