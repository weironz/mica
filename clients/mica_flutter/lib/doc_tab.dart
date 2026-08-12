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
