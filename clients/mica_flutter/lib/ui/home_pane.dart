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

/// Recently edited pages for ONE workspace, phrased for display.
///
/// The search dialog's pre-typing list. It lives here, beside the home pane,
/// because both need the same `RelativeTimeStrings` built from l10n — assembling
/// that a second time in main.dart is how "2 hours ago" ends up worded two ways.
///
/// Scoped to one workspace, unlike [buildHomePane]: search itself is
/// per-workspace, so offering recents from elsewhere would list pages no query
/// in that box could return.
List<HomeDocEntry> buildWorkspaceRecents(
  BuildContext context, {
  required String? workspaceId,
  required String workspaceName,
  required List<DocumentView> views,
  int limit = 6,
  DateTime? now,
}) {
  if (workspaceId == null) return const [];
  final l10n = context.l10n;
  return buildRecents(
    viewsByWorkspace: {workspaceId: views},
    workspaceNames: {workspaceId: workspaceName},
    strings: RelativeTimeStrings(
      justNow: l10n.homeJustNow,
      minutesAgo: l10n.homeMinutesAgo,
      hoursAgo: l10n.homeHoursAgo,
      yesterday: l10n.homeYesterday,
      daysAgo: l10n.homeDaysAgo,
    ),
    limit: limit,
    now: now,
  );
}

/// Build the start pane (what shows when no page is open) for the current state.
///
/// [viewsByWorkspace] and [workspaceNames] span EVERY workspace of the active
/// world, so "back to your last page" can point into another workspace. The
/// local and cloud worlds stay separate — the caller passes whichever is active.
Widget buildHomePane(
  BuildContext context, {
  required Map<String, List<DocumentView>> viewsByWorkspace,
  required Map<String, String> workspaceNames,
  required VoidCallback? onCreatePage,
  required VoidCallback? onSearch,
  required void Function(String id) onOpenView,
  DateTime? now,
}) {
  final l10n = context.l10n;
  final mac = defaultTargetPlatform == TargetPlatform.macOS;
  final recents = buildRecents(
    viewsByWorkspace: viewsByWorkspace,
    workspaceNames: workspaceNames,
    strings: RelativeTimeStrings(
      justNow: l10n.homeJustNow,
      minutesAgo: l10n.homeMinutesAgo,
      hoursAgo: l10n.homeHoursAgo,
      yesterday: l10n.homeYesterday,
      daysAgo: l10n.homeDaysAgo,
    ),
    limit: 1,
    now: now,
  );
  return HomeScreen(
    strings: HomeStrings(
      createTitle: l10n.homeCreateTitle,
      // Keyboard hints, not prose: the glyph differs by platform, and a macOS
      // user reads "Ctrl" as a different key entirely.
      createHint: mac ? '⌘N' : 'Ctrl+N',
      searchTitle: l10n.homeSearchTitle,
      searchHint: mac ? '⌘⇧F' : 'Ctrl+Shift+F',
      recentLabel: l10n.homeRecentLabel,
    ),
    recent: recents.isEmpty ? null : recents.first,
    onCreatePage: onCreatePage,
    onSearch: onSearch,
    onOpen: onOpenView,
  );
}
