part of 'main.dart';

/// The rename dialog, as a widget that OWNS its controller.
///
/// It must own it: disposing a controller right after `showDialog` returns —
/// the obvious spelling, and what the workspace dialogs above still do — throws
/// in debug. The pop only *starts* the exit transition, and that transition
/// rebuilds the field, which re-`addListener`s the controller you just killed
/// (`ChangeNotifier.debugAssertNotDisposed`, seen as a red screen on cancel).
/// State.dispose runs after the route is really gone, so it is the safe hook.
class _RenameDialog extends StatefulWidget {
  const _RenameDialog({required this.initialName});

  final String initialName;

  @override
  State<_RenameDialog> createState() => _RenameDialogState();
}

class _RenameDialogState extends State<_RenameDialog> {
  late final TextEditingController _ctrl =
      TextEditingController(text: widget.initialName)
        ..selection = TextSelection(
          baseOffset: 0,
          extentOffset: widget.initialName.length,
        );

  @override
  void dispose() {
    _ctrl.dispose();
    super.dispose();
  }

  void _submit() => Navigator.of(context).pop(_ctrl.text);

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    return AlertDialog(
      title: Text(l10n.commonRename),
      content: SizedBox(
        // Wide enough for a real page name; the sidebar row it replaces is
        // ~180px and is pinned to the screen edge, which is the whole reason
        // this is a dialog.
        width: 400,
        child: TextField(
          controller: _ctrl,
          autofocus: true,
          decoration: InputDecoration(
            labelText: l10n.renameNameLabel,
            border: const OutlineInputBorder(),
          ),
          onSubmitted: (_) => _submit(),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l10n.commonCancel),
        ),
        FilledButton.icon(
          onPressed: _submit,
          icon: const Icon(Icons.save),
          label: Text(l10n.commonSave),
        ),
      ],
    );
  }
}

/// Reverse-reference panel under a cloud page: the pages that link TO it.
///
/// Lazy: fetches on mount and whenever [viewId] changes, so opening a page is
/// never blocked on the backlink scan (it lands a frame or two later). Renders
/// NOTHING while loading, on error, or when empty — a page with no backlinks
/// shows no panel at all, matching the "empty ⇒ hide" spec. Only when there are
/// results does the "N backlinks" header + tappable source list appear; a tap
/// reuses the same view-open path search results use ([onOpen]).
class _BacklinksPanel extends StatefulWidget {
  const _BacklinksPanel({
    required this.viewId,
    required this.load,
    required this.onOpen,
    required this.onLoaded,
    super.key,
  });

  final String viewId;
  final Future<List<Backlink>> Function(String viewId) load;
  final Future<void> Function(String viewId) onOpen;

  /// Reports how many links this page ended up showing (0 while loading, and 0
  /// forever if there are none — the panel renders nothing then). The page
  /// needs it to size the editor above: see [_WorkspaceViewState._backlinkCount].
  final void Function(String viewId, int count) onLoaded;

  @override
  State<_BacklinksPanel> createState() => _BacklinksPanelState();
}

class _BacklinksPanelState extends State<_BacklinksPanel> {
  List<Backlink> _links = const [];

  @override
  void initState() {
    super.initState();
    _fetch();
  }

  @override
  void didUpdateWidget(_BacklinksPanel old) {
    super.didUpdateWidget(old);
    // A ValueKey on viewId remounts this widget per page, so this rarely fires;
    // kept as a belt-and-braces refresh if the key strategy ever changes.
    if (old.viewId != widget.viewId) {
      setState(() => _links = const []);
      _fetch();
    }
  }

  Future<void> _fetch() async {
    final viewId = widget.viewId;
    try {
      final links = await widget.load(viewId);
      // Guard against a late response after the user navigated away.
      if (!mounted || widget.viewId != viewId) return;
      setState(() => _links = links);
      widget.onLoaded(viewId, links.length);
    } catch (_) {
      // Backlinks are a best-effort convenience; a failed scan just shows no
      // panel rather than surfacing an error over the page.
      if (!mounted || widget.viewId != viewId) return;
      setState(() => _links = const []);
      widget.onLoaded(viewId, 0);
    }
  }

  @override
  Widget build(BuildContext context) {
    if (_links.isEmpty) return const SizedBox.shrink();
    final tokens = MicaTheme.of(context);
    return Padding(
      padding: const EdgeInsets.only(top: 16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          // The hairline that says "the page ends here". Without it the panel
          // read as text floating in the margin — and every peer that puts a
          // link panel under the body anchors it with either a rule (Obsidian,
          // AFFiNE) or per-row fills (Logseq, Roam). None of them anchors with
          // whitespace alone, and none boxes the whole panel: the fill stays on
          // the individual row so the section still reads as the document
          // continuing, not as a separate component.
          Container(height: 1, color: tokens.border.subtle),
          const SizedBox(height: 12),
          Row(
            children: [
              Icon(
                Icons.link,
                size: 16,
                color: MicaTheme.of(context).text.muted,
              ),
              const SizedBox(width: 6),
              Text(
                context.l10n.backlinksHeading(_links.length),
                style: TextStyle(
                  fontSize: 13,
                  fontWeight: FontWeight.w600,
                  color: MicaTheme.of(context).text.muted,
                ),
              ),
            ],
          ),
          const SizedBox(height: 6),
          for (final link in _links)
            InkWell(
              onTap: () => widget.onOpen(link.viewId),
              borderRadius: BorderRadius.circular(6),
              child: Container(
                margin: const EdgeInsets.only(bottom: 4),
                decoration: BoxDecoration(
                  color: tokens.surface.sunken,
                  borderRadius: BorderRadius.circular(6),
                ),
                padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 7),
                child: Row(
                  children: [
                    Icon(
                      Icons.description_outlined,
                      size: 16,
                      color: MicaTheme.of(context).text.muted,
                    ),
                    const SizedBox(width: 8),
                    Expanded(
                      child: Text(
                        link.title.isEmpty ? 'Untitled' : link.title,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontSize: 14,
                          color: MicaTheme.of(context).text.primary,
                        ),
                      ),
                    ),
                  ],
                ),
              ),
            ),
        ],
      ),
    );
  }
}

/// The comments entry in the doc header: an outline bubble, with the count of
/// still-open threads when there are any. Quiet when the document has none.
/// The sidebar's Home row. Its own widget so the pane's build method stays a list
/// of parts rather than another nested block of decoration.
class _HomeNavRow extends StatelessWidget {
  const _HomeNavRow({
    required this.label,
    required this.selected,
    required this.onTap,
  });

  final String label;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return InkWell(
      onTap: onTap,
      borderRadius: BorderRadius.circular(8),
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 9),
        decoration: BoxDecoration(
          color: selected ? MicaTheme.of(context).accent.wash : null,
          borderRadius: BorderRadius.circular(8),
        ),
        child: Row(
          children: [
            Icon(
              Icons.home_outlined,
              size: 18,
              color: selected
                  ? MicaTheme.of(context).accent.primary
                  : MicaTheme.of(context).text.muted,
            ),
            const SizedBox(width: 10),
            Text(
              label,
              style: TextStyle(
                fontSize: 14,
                fontWeight: FontWeight.w600,
                color: selected
                    ? MicaTheme.of(context).accent.hover
                    : MicaTheme.of(context).text.primary,
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// The muted words·chars readout. Lives in the shell now, pinned to the pane's
/// bottom-right; it used to be drawn by the editor at the end of the CONTENT,
/// which on a short page put it in the middle of the screen with nothing around
/// it. `IgnorePointer` so it never eats a click meant for the page beneath.
class _WordCountBadge extends StatelessWidget {
  const _WordCountBadge({required this.counts});

  final DocCounts counts;

  @override
  Widget build(BuildContext context) {
    return IgnorePointer(
      child: DecoratedBox(
        decoration: BoxDecoration(
          color: MicaTheme.of(context).surface.raised.withValues(alpha: 0.95),
          borderRadius: BorderRadius.circular(6),
          border: Border.all(color: const Color(0x14000000)),
        ),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
          child: Text(
            context.l10n.editorWordCount(counts.words, counts.chars),
            style: TextStyle(
              fontSize: 12,
              height: 1.0,
              color: MicaTheme.of(context).text.muted,
            ),
          ),
        ),
      ),
    );
  }
}

/// The right sidebar's tabs.
enum _ToolsTab { outline, comments }

/// One tab in the right sidebar's header. Icon + label + optional count.
class _ToolsTabButton extends StatelessWidget {
  const _ToolsTabButton({
    required this.icon,
    required this.label,
    required this.active,
    required this.onTap,
    this.badge = 0,
  });

  final IconData icon;
  final String label;
  final bool active;
  final VoidCallback onTap;

  /// Unresolved count, shown on the tab so you do not have to open it to learn
  /// there is something in it. Zero draws nothing.
  final int badge;

  @override
  Widget build(BuildContext context) {
    final tokens = MicaTheme.of(context);
    final color = active ? tokens.accent.primary : tokens.text.muted;
    return InkWell(
      onTap: onTap,
      borderRadius: BorderRadius.circular(6),
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 5),
        decoration: BoxDecoration(
          color: active ? tokens.accent.wash : null,
          borderRadius: BorderRadius.circular(6),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(icon, size: 15, color: color),
            const SizedBox(width: 6),
            Text(
              label,
              style: TextStyle(
                fontSize: 12,
                fontWeight: active ? FontWeight.w600 : FontWeight.w400,
                color: color,
              ),
            ),
            if (badge > 0) ...[
              const SizedBox(width: 5),
              Text(
                '$badge',
                style: TextStyle(fontSize: 11, color: tokens.text.faint),
              ),
            ],
          ],
        ),
      ),
    );
  }
}

class _CommentsButton extends StatelessWidget {
  const _CommentsButton({
    required this.openCount,
    required this.onTap,
    this.active = false,
  });

  final int openCount;
  final VoidCallback onTap;

  /// The rail is docked open. Shown because the button is now a TOGGLE, not a
  /// "show me" action — without the state it is the only control on the row
  /// whose effect you cannot see from the control itself.
  final bool active;

  @override
  Widget build(BuildContext context) {
    return Tooltip(
      message: context.l10n.commentsTitle,
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(4),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 4),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(
                Icons.chat_bubble_outline,
                size: 15,
                color: active
                    ? MicaTheme.of(context).accent.primary
                    : MicaTheme.of(context).text.muted,
              ),
              if (openCount > 0) ...[
                const SizedBox(width: 3),
                Text(
                  '$openCount',
                  style: TextStyle(
                    fontSize: 11,
                    color: MicaTheme.of(context).text.muted,
                  ),
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

/// The quiet sync-status affordance in the doc header. Calibrated minimal (see
/// sync_status.dart + the SiYuan/AFFiNE research): NOTHING when synced, a faint
/// slow spinner while syncing, a muted cloud-off glyph when offline. It earns
/// attention only when edits are not yet safely on the server — easy to miss
/// when healthy, by design.
class _SyncBadge extends StatelessWidget {
  const _SyncBadge(this.phase);

  final SyncPhase phase;

  @override
  Widget build(BuildContext context) {
    switch (phase) {
      case SyncPhase.synced:
        return const SizedBox.shrink();
      case SyncPhase.syncing:
        return Padding(
          padding: const EdgeInsets.symmetric(horizontal: 6),
          child: SizedBox(
            width: 13,
            height: 13,
            child: CircularProgressIndicator(
              strokeWidth: 1.6,
              color: MicaTheme.of(context).text.faint,
            ),
          ),
        );
      case SyncPhase.offline:
        return Padding(
          padding: const EdgeInsets.symmetric(horizontal: 6),
          child: Tooltip(
            message: context.l10n.syncOffline,
            child: Icon(
              Icons.cloud_off_outlined,
              size: 15,
              color: MicaTheme.of(context).text.muted,
            ),
          ),
        );
    }
  }
}

/// The tree's drop indicator: a hollow terminal dot with a line leaving its
/// centre, drawn at the level the drop would land at.
///
/// Copied in geometry from AFFiNE's sidebar, which takes it from Atlassian's
/// pragmatic-drag-and-drop: an 8px circle, transparent inside, 2px border, and
/// a 2px line starting at the circle's centre so the two read as one mark. The
/// dot is what you aim with — the line alone says "something lands here", the
/// dot says WHICH LEVEL it lands at.
class _DropIndicator extends StatelessWidget {
  const _DropIndicator({required this.inset, required this.state});

  final double inset;
  final DropIndicatorState state;

  static const _terminal = 8.0;

  @override
  Widget build(BuildContext context) {
    final tokens = MicaTheme.of(context);
    final color = switch (state) {
      DropIndicatorState.idle => Colors.transparent,
      DropIndicatorState.allowed => tokens.accent.primary,
      DropIndicatorState.blocked => tokens.status.warning,
    };
    return Padding(
      padding: EdgeInsets.only(left: inset, right: 4),
      child: SizedBox(
        height: _terminal,
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.center,
          children: [
            SizedBox(
              width: _terminal,
              height: _terminal,
              child: DecoratedBox(
                decoration: BoxDecoration(
                  shape: BoxShape.circle,
                  border: Border.all(color: color, width: 2),
                ),
              ),
            ),
            // Starts under the dot's centre, so there is no gap between them.
            Expanded(
              child: Transform.translate(
                offset: const Offset(-_terminal / 2, 0),
                child: Container(height: 2, color: color),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

