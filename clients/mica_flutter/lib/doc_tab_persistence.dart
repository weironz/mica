// Open tabs survive a restart (the 2026-10-30 call: AFFiNE's model).
//
// What is saved is the strip's META only: for each tab, its page id, the
// workspace it belongs to, and the page name (so the strip can label a tab
// before anything loads), plus which tab was active. Never the document.
//
// Restore is LAZY, like AFFiNE's (`tabsStatus$`: a tab is listed from meta and
// only gets a view when it is first shown). The active tab is opened by the
// normal startup path (`lastWorkspaceId` / `lastViewId`); every other tab comes
// back as a label and fetches its document the first time it is activated. So
// restoring eight tabs costs eight entries in the strip, not eight bootstraps
// or eight sockets — which matters more here than in AFFiNE, because a Mica
// document is a WebSocket of its own.
//
// A tab whose page is gone, or whose workspace this account no longer sees,
// is dropped on restore rather than restored as a tab that can never load.
import 'dart:convert';

import 'api/models.dart';
import 'doc_tab.dart';

/// The prefs key. One per device, like `lastWorkspaceId`: the cloud world has
/// one signed-in account at a time and the local world has no tabs.
const String kOpenTabsPref = 'openTabs';

/// The strip as JSON, or null when there is nothing worth saving (a single
/// tab is what a fresh start gives anyway). Tabs with no page are skipped.
String? encodeOpenTabs(List<DocTab> tabs, int activeIndex) {
  final saved = <Map<String, String>>[];
  var active = 0;
  for (var i = 0; i < tabs.length; i++) {
    final view = tabs[i].view;
    final workspaceId = tabs[i].workspaceId;
    if (view == null || workspaceId == null) continue;
    if (i == activeIndex) active = saved.length;
    saved.add({'v': view.id, 'w': workspaceId, 'n': view.name});
  }
  if (saved.length < 2) return null;
  return jsonEncode({'tabs': saved, 'active': active});
}

/// Rebuild the strip from [saved].
///
/// [current] is the tab startup already opened (the last-viewed page). It
/// takes its saved slot when one matches, so the strip comes back in its old
/// order with the right tab active; when none matches it goes first and stays
/// active. [lookup] finds a page in the trees already in memory; a saved tab
/// it cannot find is dropped.
({List<DocTab> tabs, int active})? restoreOpenTabs(
  String? saved,
  DocTab current,
  DocumentView? Function(String workspaceId, String viewId) lookup,
) {
  if (saved == null || saved.isEmpty) return null;
  final List<dynamic> entries;
  try {
    final decoded = jsonDecode(saved);
    if (decoded is! Map || decoded['tabs'] is! List) return null;
    entries = decoded['tabs'] as List;
  } on FormatException {
    return null;
  }
  final tabs = <DocTab>[];
  var placedCurrent = false;
  for (final e in entries) {
    if (e is! Map) continue;
    final viewId = e['v'], workspaceId = e['w'];
    if (viewId is! String || workspaceId is! String) continue;
    if (!placedCurrent &&
        current.view?.id == viewId &&
        current.workspaceId == workspaceId) {
      tabs.add(current);
      placedCurrent = true;
      continue;
    }
    final view = lookup(workspaceId, viewId);
    if (view == null || view.isFolder) continue;
    tabs.add(DocTab(view: view)..workspaceId = workspaceId);
  }
  if (!placedCurrent) tabs.insert(0, current);
  if (tabs.length < 2) return null;
  return (tabs: tabs, active: tabs.indexOf(current));
}

/// Saves the strip when it changed since the last save, so the caller can ask
/// on every rebuild without writing prefs on every rebuild.
class OpenTabsSaver {
  String? _last;
  bool _primed = false;

  /// Remember [tabs] as already saved, without writing — call after a restore
  /// so the first rebuild does not write back what was just read.
  void prime(List<DocTab> tabs, int activeIndex) {
    _last = encodeOpenTabs(tabs, activeIndex);
    _primed = true;
  }

  /// Write through [save] when the encoded strip differs from the last write.
  /// Does nothing until [prime] ran, so the startup frames (one empty tab,
  /// before the restore) cannot overwrite the saved strip.
  void maybeSave(
    List<DocTab> tabs,
    int activeIndex,
    void Function(String value) save,
  ) {
    if (!_primed) return;
    final encoded = encodeOpenTabs(tabs, activeIndex) ?? '';
    if (encoded == (_last ?? '')) return;
    _last = encoded;
    save(encoded);
  }
}
