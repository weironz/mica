// One open page in the tab strip.
//
// This exists because the shell's navigation state was *singular*: one
// `_selectedView` and one `_selectedBootstrap` field on the app state. That is
// the actual obstacle to multiple tabs — not the tab strip, which is a row of
// buttons. AppFlowy carries the same state as a per-tab `PageManager` object
// (`workspace/presentation/home/home_stack.dart`), and adding tabs there is
// natural for exactly this reason.
//
// The fields start as the ones that were previously singular. Per-tab sync
// (`DocumentSyncClient` / `CloudSyncSession`) moves in when the connection cap
// lands — a tab beyond the live limit keeps its `view`/`bootstrap` but gives up
// its sockets, so those have to be nullable *per tab*, not per app.

import 'api/models.dart';
import 'api/sync_client.dart';
import 'cloud/cloud_sync_session.dart';

/// How many tabs may hold live sockets at once.
///
/// Mica opens one WebSocket PER DOCUMENT
/// (`/ws/workspaces/{workspace_id}/documents/{document_id}`), so tabs cost real
/// connections — unlike either reference product. AppFlowy syncs through a
/// local Rust backend, and AFFiNE multiplexes every doc over ONE socket per
/// renderer with `docId` as a message field, so neither pays this and neither
/// caps anything.
///
/// Three is "the tabs a person is actually working between", not a measured
/// limit. The honest fix is to change Mica's connection granularity to match
/// AFFiNE's; this cap is the cheap stand-in until then, which is why it is one
/// integer and one comparison rather than a cache framework.
const int kMaxLiveSyncTabs = 3;

class DocTab {
  DocTab({this.view, this.bootstrap});

  /// The page this tab points at.
  ///
  /// Non-null while [bootstrap] is still null — that gap IS the loading state,
  /// which is why these are two independently settable fields and not one
  /// `bootstrap.view`. Collapsing them would erase "this tab is opening".
  DocumentView? view;

  /// The loaded document, once it arrives. Also null when the page was trashed
  /// or deleted out from under an open tab.
  DocumentBootstrap? bootstrap;

  /// Which workspace this tab's page belongs to.
  ///
  /// Tabs outlive a workspace switch — switching clears only the ACTIVE tab, so
  /// the others go on holding pages from wherever they were opened. Without
  /// this the strip becomes a mix of workspaces that all claim to be in the
  /// current one, and acting on such a tab bootstraps its document against the
  /// wrong workspace: a 404 that reads as "this page is broken".
  ///
  /// AFFiNE keeps the same thing in `WorkbenchMeta.basename` (`/workspace/<id>`)
  /// — a tab there carries its workspace in its URL for exactly this reason.
  ///
  /// Null while the tab holds no document. That is NOT "the current workspace":
  /// a fresh tab must not force a switch to anywhere.
  String? workspaceId;

  /// The document id this tab is synced to, or null when it holds no document.
  /// The sync layer is keyed by document id, so this is the tab's identity as
  /// far as connections are concerned.
  String? get documentId => bootstrap?.document.id;

  /// This tab's presence/op socket, and its CRDT replica. Non-null exactly when
  /// the tab is LIVE — connected and receiving remote edits.
  ///
  /// Per tab, not per app, so a background tab can keep syncing. The shell's
  /// `_sync` / `_cloudSession` are proxies onto the ACTIVE tab's pair, which is
  /// what lets the ~40 call sites that mean "the open document's session" keep
  /// reading a single field.
  DocumentSyncClient? sync;
  CloudSyncSession? cloudSession;

  /// Monotonic stamp of when this tab was last activated, for the LRU choice of
  /// what to park when more than [kMaxLiveSyncTabs] tabs want to be live.
  ///
  /// A counter rather than a clock: it only has to ORDER activations, and a
  /// wall clock would make the tests depend on timing and could go backwards.
  int lastActivated = 0;

  /// Whether this tab currently holds sockets.
  ///
  /// A parked tab is not broken — it keeps [view] and [bootstrap], so it still
  /// renders and still reads. It just stops receiving other people's edits
  /// until it is activated again.
  bool get isLive => sync != null;
}

/// The workspace a tab switch must land in, or null when no switch is needed.
///
/// The subtle case is [tabWorkspaceId] being null — an empty tab, or one still
/// loading. That is not "the current workspace" and it is not "unknown, go find
/// out": it means the tab has no page yet, so switching anywhere on its behalf
/// would move the user somewhere they did not ask to go.
String? workspaceToSwitchTo({
  required String? tabWorkspaceId,
  required String? currentWorkspaceId,
}) {
  if (tabWorkspaceId == null) return null;
  if (tabWorkspaceId == currentWorkspaceId) return null;
  return tabWorkspaceId;
}

/// Which live tabs must give up their sockets so that at most [max] stay live.
///
/// Least-recently-activated first. [active] is never returned: it is by
/// definition the most recently activated, but it is excluded explicitly so a
/// misconfigured [max] cannot disconnect the page the user is typing into.
///
/// Pure and separate from the shell for the same reason as
/// [activeIndexAfterClose] — this is the decision, and the caller is only the
/// disposal. A wrong answer here silently stops a tab from receiving other
/// people's edits, which is invisible until someone loses work.
List<DocTab> tabsToPark(
  List<DocTab> tabs,
  DocTab active, {
  int max = kMaxLiveSyncTabs,
}) {
  final live = tabs.where((t) => t.isLive && !identical(t, active)).toList()
    ..sort((a, b) => b.lastActivated.compareTo(a.lastActivated));
  // The active tab holds one of the slots whether or not it is live yet, so the
  // others compete for max - 1. Without this the cap would admit max + 1.
  final keep = max - 1;
  return live.length <= keep ? const [] : live.sublist(keep);
}

/// Which tab is active after the one at [closing] is removed, given [active]
/// now and [count] tabs BEFORE the removal.
///
/// Pure, and separate from the widget, because this is the only arithmetic in
/// the tab model that can be wrong in a way the user notices — closing a tab
/// and landing on the wrong page. Everything else is a list mutation.
///
/// Closing the active tab lands on the tab that slides into its place (the one
/// to its right), which is what every browser does; closing the last tab in the
/// row falls back to the new last tab. Closing a tab to the LEFT of the active
/// one shifts the active index down so the same page stays open — the bug this
/// function exists to prevent is that shift being forgotten.
int activeIndexAfterClose({
  required int closing,
  required int active,
  required int count,
}) {
  // Callers refuse to close the last tab; this mirrors that rather than
  // inventing an answer for a state that cannot occur.
  if (count <= 1) return active;
  if (closing > active) return active;
  if (closing < active) return active - 1;
  return closing.clamp(0, count - 2);
}
