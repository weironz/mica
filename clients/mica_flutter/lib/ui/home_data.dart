// Home-screen data derivation — pure functions, no widgets, no services.
//
// Deliberately a standalone library (NOT `part of main.dart`): main.dart is
// 8600+ lines because logic kept landing there, and the two earlier splits
// (10445→9031→5967) used `part`, which shares privates and so never actually
// held a boundary — a later widget could not even reuse those files. Everything
// here is testable without pumping a widget or touching the network.
//
// "Recently edited" and the per-workspace page counts need no new endpoint: the
// shell already holds every workspace's views, and the server has always sent
// `views.updated_at` (the client just never read it).

import '../api/models.dart';
import 'home_screen.dart' show HomeDocEntry;

/// Copy for [relativeMeta], supplied by the caller so this file stays free of
/// hardcoded strings.
///
/// The counted forms are FUNCTIONS, not templates with a `{n}` to substitute:
/// languages that inflect by count (English "1 minute" vs "2 minutes") need the
/// number at format time, and hand-substituting a placeholder would lock every
/// locale into one plural form. The caller passes the generated l10n accessors.
class RelativeTimeStrings {
  const RelativeTimeStrings({
    required this.justNow,
    required this.minutesAgo,
    required this.hoursAgo,
    required this.yesterday,
    required this.daysAgo,
  });

  final String justNow;
  final String Function(int n) minutesAgo;
  final String Function(int n) hoursAgo;
  final String yesterday;
  final String Function(int n) daysAgo;
}

/// A short "when was this touched" label.
///
/// Falls back to an absolute `y/m/d` past a week, because "37 天前" tells nobody
/// anything useful. Returns an empty string for a null timestamp (the local world
/// and the offline page-tree mirror build views without one) — the caller then
/// simply renders no meta rather than a fake "just now".
String relativeMeta(
  DateTime? when,
  RelativeTimeStrings strings, {
  DateTime? now,
}) {
  if (when == null) return '';
  final ref = now ?? DateTime.now();
  final diff = ref.difference(when);
  if (diff.isNegative || diff.inMinutes < 1) return strings.justNow;
  if (diff.inMinutes < 60) return strings.minutesAgo(diff.inMinutes);
  if (diff.inHours < 24) return strings.hoursAgo(diff.inHours);
  if (diff.inDays == 1) return strings.yesterday;
  if (diff.inDays < 7) return strings.daysAgo(diff.inDays);
  return '${when.year}/${when.month}/${when.day}';
}

/// The most recently edited PAGES across every workspace, newest first.
///
/// Folders are excluded: "recently edited" is about documents you were writing,
/// and a folder's timestamp moves when its children are rearranged, which is not
/// what anyone is looking for here. Views without a timestamp sort last rather
/// than being dropped — they are real pages, just from a source with no mtime.
List<HomeDocEntry> buildRecents({
  required Map<String, List<DocumentView>> viewsByWorkspace,
  required Map<String, String> workspaceNames,
  required RelativeTimeStrings strings,
  int limit = 6,
  DateTime? now,
}) {
  final pages = <({DocumentView view, String workspaceId})>[];
  for (final entry in viewsByWorkspace.entries) {
    for (final view in entry.value) {
      if (view.isFolder) continue;
      pages.add((view: view, workspaceId: entry.key));
    }
  }
  pages.sort((a, b) {
    final at = a.view.updatedAt;
    final bt = b.view.updatedAt;
    if (at == null && bt == null) return a.view.name.compareTo(b.view.name);
    if (at == null) return 1; // no timestamp → last, not dropped
    if (bt == null) return -1;
    return bt.compareTo(at);
  });
  return [
    for (final page in pages.take(limit))
      (
        id: page.view.id,
        icon: page.view.icon,
        name: page.view.name,
        workspaceName: workspaceNames[page.workspaceId] ?? '',
        meta: relativeMeta(page.view.updatedAt, strings, now: now),
      ),
  ];
}

/// Top-level folders across every workspace, alphabetical.
///
/// Only top-level (`parentViewId == null`): the home screen is an entry point, and
/// listing nested folders would just reproduce the sidebar tree. `meta` is the
/// child count, formatted by [formatChildCount] so this file needs no plural rules
/// of its own.
List<HomeDocEntry> buildDirectories({
  required Map<String, List<DocumentView>> viewsByWorkspace,
  required Map<String, String> workspaceNames,
  required String Function(int count) formatChildCount,
  int limit = 8,
}) {
  final folders = <({DocumentView view, String workspaceId, int children})>[];
  for (final entry in viewsByWorkspace.entries) {
    final views = entry.value;
    for (final view in views) {
      if (!view.isFolder || view.parentViewId != null) continue;
      final children = views.where((v) => v.parentViewId == view.id).length;
      folders.add((view: view, workspaceId: entry.key, children: children));
    }
  }
  folders.sort((a, b) => a.view.name.compareTo(b.view.name));
  return [
    for (final folder in folders.take(limit))
      (
        id: folder.view.id,
        icon: folder.view.icon,
        name: folder.view.name,
        workspaceName: workspaceNames[folder.workspaceId] ?? '',
        meta: formatChildCount(folder.children),
      ),
  ];
}

/// How many PAGES a workspace holds — the sidebar switcher's subtitle.
///
/// Folders are not pages, so they are not counted: a switcher that says "12 个
/// 页面" when four of them are folders is lying in a way the user can check.
int countPages(List<DocumentView>? views) =>
    views?.where((v) => !v.isFolder).length ?? 0;
