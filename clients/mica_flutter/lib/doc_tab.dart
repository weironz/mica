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

  /// The document id this tab is synced to, or null when it holds no document.
  /// The sync layer is keyed by document id, so this is the tab's identity as
  /// far as connections are concerned.
  String? get documentId => bootstrap?.document.id;
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
