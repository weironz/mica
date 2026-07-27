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
import 'package:intl/intl.dart';

import '../api/models.dart';
import '../l10n/app_localizations.dart';
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
  required String userName,
  required Map<String, List<DocumentView>> viewsByWorkspace,
  required Map<String, String> workspaceNames,
  required VoidCallback onCreatePage,
  required void Function(String viewId) onOpenView,
  DateTime? now,
}) {
  final l10n = context.l10n;
  final clock = now ?? DateTime.now();
  final locale = Localizations.localeOf(context).toLanguageTag();

  final relative = RelativeTimeStrings(
    justNow: l10n.homeJustNow,
    minutesAgo: l10n.homeMinutesAgo,
    hoursAgo: l10n.homeHoursAgo,
    yesterday: l10n.homeYesterday,
    daysAgo: l10n.homeDaysAgo,
  );

  return HomeScreen(
    // Localized long date, e.g. "2026年7月27日星期一" / "Monday, July 27, 2026".
    dateText: DateFormat.yMMMMEEEEd(locale).format(clock),
    greeting: _greeting(l10n, userName, clock),
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

/// Time-of-day greeting. Boundaries at 05/12/18 local time — the point is that it
/// matches what the user would call the current part of their day, so it follows
/// the device clock rather than anything server-side.
String _greeting(AppLocalizations l10n, String userName, DateTime clock) {
  final hour = clock.hour;
  if (hour >= 5 && hour < 12) return l10n.homeGreetingMorning(userName);
  if (hour >= 12 && hour < 18) return l10n.homeGreetingAfternoon(userName);
  return l10n.homeGreetingEvening(userName);
}
