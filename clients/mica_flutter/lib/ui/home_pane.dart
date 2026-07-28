// Assembles the home screen: localized copy + derived data + the widget.
//
// A standalone library on purpose, so main.dart keeps only one line of wiring.
// The split is: `home_data.dart` derives (pure, unit-tested), `home_screen.dart`
// draws (widget, unit-tested), and this file is the seam that knows about
// BuildContext, l10n and the clock. None of it belongs in main.dart, which is
// already 8600 lines for exactly this reason.

import 'package:flutter/foundation.dart'
    show TargetPlatform, defaultTargetPlatform;
import 'package:flutter/material.dart';

import '../api/models.dart';
import '../l10n/locale_controller.dart';
import 'home_data.dart';
import 'home_screen.dart';

/// Build the home pane for the current state.
///
/// [viewsByWorkspace] and [workspaceNames] span EVERY workspace of the active
/// world: home is a cross-workspace entry point, which is what makes it different
/// from the sidebar (that shows the one workspace you are in). The local and cloud
/// worlds stay separate — the caller passes whichever is active.
Widget buildHomePane(
  BuildContext context, {
  required Map<String, List<DocumentView>> viewsByWorkspace,
  required Map<String, String> workspaceNames,
  required VoidCallback onCreatePage,
  required void Function(String viewId) onOpenView,
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

  return HomeScreen(
    strings: HomeStrings(
      createTitle: l10n.homeCreateTitle,
      createSubtitle: l10n.homeCreateSubtitle,
      // A keyboard hint, not prose: the glyph differs by platform, and a macOS
      // user reads "Ctrl" as a different key entirely.
      createHint: defaultTargetPlatform == TargetPlatform.macOS
          ? '⌘N'
          : 'Ctrl+N',
      recentLabel: l10n.homeRecentLabel,
      directoriesLabel: l10n.homeDirectoriesLabel,
      recentEmptyTitle: l10n.homeRecentEmptyTitle,
      recentEmptyBody: l10n.homeRecentEmptyBody,
      directoriesEmptyTitle: l10n.homeDirectoriesEmptyTitle,
      directoriesEmptyBody: l10n.homeDirectoriesEmptyBody,
    ),
    recents: buildRecents(
      viewsByWorkspace: viewsByWorkspace,
      workspaceNames: workspaceNames,
      strings: relative,
      now: clock,
    ),
    directories: buildDirectories(
      viewsByWorkspace: viewsByWorkspace,
      workspaceNames: workspaceNames,
      formatChildCount: l10n.itemCount,
    ),
    onOpen: onOpenView,
    onCreatePage: onCreatePage,
  );
}
