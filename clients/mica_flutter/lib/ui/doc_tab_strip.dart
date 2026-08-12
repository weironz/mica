// The row of open-page tabs above the editor.
//
// PLACEMENT — inside the editor pane, not spanning the window.
//
// The two reference products differ here, and the choice is deliberate:
// AFFiNE puts its strip in a full-width row ABOVE the sidebar
// (`desktop/components/app-container/index.tsx`: `desktopTabsHeader` then
// `desktopAppViewMain`, which holds sidebar + main), while AppFlowy keeps its
// `TabsManager` INSIDE the content column with the sidebar at full height
// (`workspace/presentation/home/home_stack.dart`).
//
// Mica follows AppFlowy: the full-width form would have to restructure the
// shell's `Row` into a `Column`, and the narrow shell — where the sidebar
// slides OVER the editor rather than sitting beside it — has no row that spans
// both. AFFiNE can afford the full-width form because it earns the height back
// by moving the sidebar toggle and the nav buttons INTO the strip; Mica's
// editor pane already has those in its own top row, so there is nothing to
// reclaim and the restructure would buy nothing.
//
// The strip therefore costs vertical space and nothing horizontal — it does not
// compete with the formatting toolbar, which was the worry when this was
// specced.

import 'package:flutter/material.dart';

import '../doc_tab.dart';
import 'theme_tokens.dart';

/// Height of the strip when it is showing. Callers that need to reserve space
/// should read this rather than repeating the number.
const double kDocTabStripHeight = 34;

class DocTabStrip extends StatelessWidget {
  const DocTabStrip({
    super.key,
    required this.tabs,
    required this.activeIndex,
    required this.onSelect,
    required this.onClose,
    required this.untitledLabel,
    this.onNewTab,
    this.newTabTooltip,
  });

  final List<DocTab> tabs;
  final int activeIndex;
  final void Function(int index) onSelect;
  final void Function(int index) onClose;

  /// The trailing `+`. Receives the button's global position so the host can
  /// anchor a menu under it — the same shape the sidebar row's context menu
  /// uses, rather than a second way of locating a popup.
  ///
  /// Null hides the button entirely.
  final void Function(Offset globalPosition)? onNewTab;
  final String? newTabTooltip;

  /// `l10n.untitledPage`, passed in rather than read here so this file stays
  /// free of the localization import and can be widget-tested on its own.
  final String untitledLabel;

  @override
  Widget build(BuildContext context) {
    // Always on, including for a single tab.
    //
    // This was the other way round first, matching both references — AppFlowy
    // returns `SizedBox.shrink()` at one tab (`tabs_manager.dart`), Notion
    // hides it too — on the reasoning that a strip with nothing to switch
    // between is a permanent height tax. The user overruled it (2026-08-12),
    // and the reason is one the references do not face: their `+` is not the
    // only way to reach a second tab, and hiding the strip hides the `+` with
    // it, so a single-tab window had no visible affordance for opening one.
    //
    // The cost is real and accepted: [kDocTabStripHeight] of editor height,
    // always.
    final theme = MicaTheme.of(context);
    return Container(
      height: kDocTabStripHeight,
      decoration: BoxDecoration(
        color: theme.surface.sunken,
        border: Border(bottom: BorderSide(color: theme.border.subtle)),
      ),
      child: Row(
        children: [
          Expanded(
            child: ListView.builder(
              scrollDirection: Axis.horizontal,
              padding: const EdgeInsets.symmetric(horizontal: 4),
              itemCount: tabs.length,
              itemBuilder: (context, i) => _Tab(
                // The tab's own title, NOT the active document's — every tab
                // renders its own `view`, which is what makes the strip
                // readable while a background tab is still loading.
                label: _labelFor(tabs[i]),
                active: i == activeIndex,
                onTap: () => onSelect(i),
                // No close button on the only tab: the host refuses to close
                // the last one (`_tabs` is invariant-non-empty), so drawing an
                // X there is an affordance that does nothing when clicked.
                onClose: tabs.length < 2 ? null : () => onClose(i),
              ),
            ),
          ),
          // OUTSIDE the scroll view, so it stays reachable once the tabs
          // overflow. Inside, it would scroll off the right edge exactly when
          // there are enough tabs to want another one.
          if (onNewTab != null) _NewTabButton(onTap: onNewTab!, tooltip: newTabTooltip),
        ],
      ),
    );
  }

  String _labelFor(DocTab tab) {
    final name = tab.view?.name.trim() ?? '';
    return name.isEmpty ? untitledLabel : name;
  }
}

class _NewTabButton extends StatelessWidget {
  const _NewTabButton({required this.onTap, this.tooltip});

  final void Function(Offset globalPosition) onTap;
  final String? tooltip;

  @override
  Widget build(BuildContext context) {
    final theme = MicaTheme.of(context);
    final button = GestureDetector(
      // onTapDown, not onTap: the menu is anchored to where the pointer landed,
      // and onTap does not carry a position.
      onTapDown: (d) => onTap(d.globalPosition),
      child: Container(
        width: 30,
        height: 26,
        margin: const EdgeInsets.symmetric(horizontal: 4),
        decoration: BoxDecoration(borderRadius: BorderRadius.circular(6)),
        child: Icon(Icons.add, size: 16, color: theme.text.muted),
      ),
    );
    return tooltip == null ? button : Tooltip(message: tooltip!, child: button);
  }
}

class _Tab extends StatefulWidget {
  const _Tab({
    required this.label,
    required this.active,
    required this.onTap,
    required this.onClose,
  });

  final String label;
  final bool active;
  final VoidCallback onTap;

  /// Null on the only open tab — see the call site.
  final VoidCallback? onClose;

  @override
  State<_Tab> createState() => _TabState();
}

class _TabState extends State<_Tab> {
  bool _hovered = false;

  @override
  Widget build(BuildContext context) {
    final theme = MicaTheme.of(context);
    // The close button appears on hover or on the active tab. Showing it on
    // every tab at rest turns the strip into a row of X's and makes the titles
    // — the thing being read — narrower for no gain.
    final onClose = widget.onClose;
    final showClose = onClose != null && (_hovered || widget.active);
    return MouseRegion(
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: GestureDetector(
        onTap: widget.onTap,
        child: Container(
          margin: const EdgeInsets.symmetric(vertical: 4, horizontal: 2),
          padding: const EdgeInsets.only(left: 10, right: 4),
          constraints: const BoxConstraints(maxWidth: 180),
          decoration: BoxDecoration(
            color: widget.active
                ? theme.surface.base
                : (_hovered ? theme.surface.hover : Colors.transparent),
            borderRadius: BorderRadius.circular(6),
          ),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Flexible(
                child: Text(
                  widget.label,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    fontSize: 12,
                    color: widget.active
                        ? theme.text.primary
                        : theme.text.muted,
                  ),
                ),
              ),
              SizedBox(
                width: 20,
                child: showClose
                    ? IconButton(
                        icon: const Icon(Icons.close, size: 13),
                        padding: EdgeInsets.zero,
                        constraints: const BoxConstraints(),
                        splashRadius: 10,
                        color: theme.text.muted,
                        // Tooltip omitted on purpose: it would cover the
                        // neighbouring tab in a 34px row.
                        onPressed: onClose,
                      )
                    : null,
              ),
            ],
          ),
        ),
      ),
    );
  }
}
