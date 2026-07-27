// Assembles the workspace/folder overview: breadcrumb + localized copy + derived
// items + the widget. Standalone library so main.dart keeps only wiring.
//
// The breadcrumb lives here rather than inside WorkspaceOverview because that
// widget is deliberately a pure content view (items + mode + callbacks). Without a
// trail the overview would be a one-way trip — you could descend into folders with
// no way back up except the sidebar, which is the kind of dead end the design's
// empty-state rule exists to prevent.

import 'package:flutter/material.dart';

import '../api/models.dart';
import '../editor/render.dart' show EditorTheme;
import '../l10n/locale_controller.dart';
import 'home_data.dart' show RelativeTimeStrings, relativeMeta;
import 'overview_data.dart';
import 'workspace_overview.dart';

/// Build the overview of [folderId] (null = the workspace root).
///
/// [onOpen] receives a PAGE id; folder taps go to [onEnterFolder], because
/// descending is navigation within this pane, not opening a document.
Widget buildOverviewPane(
  BuildContext context, {
  required List<DocumentView> views,
  required String? folderId,
  required WorkspaceOverviewMode mode,
  required void Function(WorkspaceOverviewMode mode) onModeChanged,
  required void Function(String pageId) onOpen,
  required void Function(String? folderId) onEnterFolder,
  VoidCallback? onCreatePage,
  DateTime? now,
}) {
  final l10n = context.l10n;
  final clock = now ?? DateTime.now();
  final relative = RelativeTimeStrings(
    justNow: l10n.homeJustNow,
    minutesAgo: l10n.homeMinutesAgo,
    hoursAgo: l10n.homeHoursAgo,
    yesterday: l10n.homeYesterday,
    daysAgo: l10n.homeDaysAgo,
  );

  final items = buildOverviewItems(
    views: views,
    parentViewId: folderId,
    formatPageMeta: (view) =>
        relativeMeta(view.updatedAt, relative, now: clock),
    formatFolderMeta: l10n.itemCount,
  );
  final byId = {for (final v in views) v.id: v};

  return Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      _Breadcrumb(
        rootLabel: l10n.overviewRootLabel,
        trail: overviewTrail(views, folderId),
        onTap: onEnterFolder,
      ),
      // The content view brings no outer margin of its own (it is meant to be
      // droppable anywhere), and the breadcrumb above is inset 24 — without this
      // the cards sat flush against the window edge, misaligned with the trail.
      Expanded(
        child: Padding(
          padding: const EdgeInsets.fromLTRB(24, 0, 24, 24),
          child: WorkspaceOverview(
            items: items,
            mode: mode,
            onModeChanged: onModeChanged,
            // A folder is not a document: descend into it instead of trying to open
            // it (which is what a folder tap on the home screen used to do — i.e.
            // nothing at all).
            onOpen: (id) {
              if (byId[id]?.isFolder ?? false) {
                onEnterFolder(id);
              } else {
                onOpen(id);
              }
            },
            onEmptyAction: onCreatePage,
            strings: WorkspaceOverviewStrings(
              sectionLabel: l10n.overviewSectionLabel,
              cardsModeLabel: l10n.overviewCardsMode,
              listModeLabel: l10n.overviewListMode,
              moreActionsLabel: l10n.sidebarMoreActions,
              emptyTitle: l10n.overviewEmptyTitle,
              emptyBody: l10n.overviewEmptyBody,
              emptyActionLabel: onCreatePage == null ? null : l10n.newPage,
            ),
          ),
        ),
      ),
    ],
  );
}

/// Root › folder › folder. Every crumb but the last is tappable, so descending is
/// always reversible.
class _Breadcrumb extends StatelessWidget {
  const _Breadcrumb({
    required this.rootLabel,
    required this.trail,
    required this.onTap,
  });

  final String rootLabel;
  final List<DocumentView> trail;
  final void Function(String? folderId) onTap;

  @override
  Widget build(BuildContext context) {
    final crumbs = <Widget>[
      _crumb(rootLabel, trail.isEmpty, () => onTap(null)),
    ];
    for (var i = 0; i < trail.length; i++) {
      final view = trail[i];
      final isLast = i == trail.length - 1;
      crumbs
        ..add(
          const Padding(
            padding: EdgeInsets.symmetric(horizontal: 6),
            child: Icon(
              Icons.chevron_right,
              size: 14,
              color: EditorTheme.faint,
            ),
          ),
        )
        ..add(
          _crumb(
            view.icon == null ? view.name : '${view.icon} ${view.name}',
            isLast,
            () => onTap(view.id),
          ),
        );
    }
    return Padding(
      padding: const EdgeInsets.fromLTRB(24, 18, 24, 10),
      child: SingleChildScrollView(
        scrollDirection: Axis.horizontal,
        reverse: true, // deep trails keep the CURRENT folder in view
        child: Row(children: crumbs),
      ),
    );
  }

  Widget _crumb(String label, bool current, VoidCallback onPressed) {
    final text = Text(
      label,
      style: TextStyle(
        fontSize: 13,
        fontWeight: current ? FontWeight.w600 : FontWeight.w400,
        color: current ? EditorTheme.text : EditorTheme.muted,
      ),
    );
    // The current crumb is where you already are — a button that would do nothing.
    if (current) return text;
    return InkWell(
      onTap: onPressed,
      borderRadius: BorderRadius.circular(6),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 2),
        child: text,
      ),
    );
  }
}
