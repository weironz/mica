import 'package:flutter/foundation.dart';

import '../api/models.dart';
import '../api/tree_fetch.dart';
import '../api/tree_request_seq.dart';
import '../cloud/body_ownership.dart';
import '../local/local_offline_api.dart'
    show WorkspaceData, ViewData;
import '../perf/switch_trace.dart';
import '../session/session_controller.dart';

/// The cloud page-tree domain of `_WorkspaceShellState` — the write operations
/// (rename / create / delete / reorder / clone / restore), the workspace-switch
/// tree load, the tree-change-bell refresh, and the on-device page-tree mirror
/// — extracted verbatim (god-class fifth step; see
/// docs/code-optimization/OPTIMIZATION_PLAN_ROUND3.md §7).
///
/// The TREE itself is not stored here: it lives in the injected
/// [SessionController] (third step), and every mutation goes through its
/// `replaceViews` transition — which is also why this controller carries no
/// notifications of its own; the session store is the notifier the State
/// already listens to. The state it does own is the [TreeRequestSeq], the log
/// that decides which of several overlapping fetches for one workspace may
/// commit.
///
/// All remote calls, prefs IO and the mirror write arrive as injected
/// functions (no `MicaApi` dependency — tests script them directly); the tab /
/// selection state the load ends in is reported back through [onSelection] so
/// the controller never touches a context. Mid-flight liveness checks use the
/// injected [isMounted] because the original code's `mounted` guards protect
/// DURABLE writes (the mirror), not just setState.
class CloudViewsController extends ChangeNotifier {
  CloudViewsController({
    required this.session,
    required Future<DocumentCreateResult> Function(
      String token,
      String workspaceId,
      String name, {
      String? parentViewId,
    }) createDocument,
    required Future<DocumentBootstrap> Function(
      String token,
      String workspaceId,
      String documentId,
    ) bootstrapDocument,
    required Future<DocumentView> Function(
      String token,
      String workspaceId,
      String name, {
      String? parentViewId,
    }) createFolder,
    required Future<DocumentView> Function(
      String token,
      String workspaceId,
      String viewId,
      String name, {
      String? icon,
    }) updateView,
    required Future<List<DocumentView>> Function(
      String token,
      String workspaceId,
      String viewId,
    ) deleteView,
    required Future<({int affected, List<String> skipped})> Function({
      required String token,
      required String workspaceId,
      required List<String> viewIds,
    }) batchTrashViews,
    required Future<CloneReport> Function({
      required String token,
      required String workspaceId,
      required String viewId,
      String? name,
      required bool dryRun,
    }) cloneView,
    required Future<void> Function(
      String token,
      String workspaceId, {
      required String? parentViewId,
      required List<String> orderedViewIds,
    }) reorderViews,
    required Future<List<DocumentView>> Function(
      String token,
      String workspaceId,
      String viewId,
    ) restoreView,
    required Future<({List<DocumentView>? views, String? etag})> Function(
      String token,
      String workspaceId, {
      String? etag,
    }) listViewsIfChanged,
    required this.readPref,
    required this.savePref,
    required this.mirrorPageTree,
    required this.origin,
    required this.isMounted,
    required this.openViewId,
    required this.crdtSessionDocumentId,
  }) : _createDocument = createDocument,
       _bootstrapDocument = bootstrapDocument,
       _createFolder = createFolder,
       _updateView = updateView,
       _deleteView = deleteView,
       _batchTrashViews = batchTrashViews,
       _cloneView = cloneView,
       _reorderViews = reorderViews,
       _restoreView = restoreView,
       _listViewsIfChanged = listViewsIfChanged;

  /// The session/workspace hub — the tree and the selection live here.
  final SessionController session;

  final Future<DocumentCreateResult> Function(
    String token,
    String workspaceId,
    String name, {
    String? parentViewId,
  }) _createDocument;
  final Future<DocumentBootstrap> Function(
    String token,
    String workspaceId,
    String documentId,
  ) _bootstrapDocument;
  final Future<DocumentView> Function(
    String token,
    String workspaceId,
    String name, {
    String? parentViewId,
  }) _createFolder;
  final Future<DocumentView> Function(
    String token,
    String workspaceId,
    String viewId,
    String name, {
    String? icon,
  }) _updateView;
  final Future<List<DocumentView>> Function(
    String token,
    String workspaceId,
    String viewId,
  ) _deleteView;
  final Future<({int affected, List<String> skipped})> Function({
    required String token,
    required String workspaceId,
    required List<String> viewIds,
  }) _batchTrashViews;
  final Future<CloneReport> Function({
    required String token,
    required String workspaceId,
    required String viewId,
    String? name,
    required bool dryRun,
  }) _cloneView;
  final Future<void> Function(
    String token,
    String workspaceId, {
    required String? parentViewId,
    required List<String> orderedViewIds,
  }) _reorderViews;
  final Future<List<DocumentView>> Function(
    String token,
    String workspaceId,
    String viewId,
  ) _restoreView;
  final Future<({List<DocumentView>? views, String? etag})> Function(
    String token,
    String workspaceId, {
    String? etag,
  }) _listViewsIfChanged;

  final String? Function(String key) readPref;
  final void Function(String key, String value) savePref;
  final void Function(
    String origin,
    List<WorkspaceData> workspaces,
    List<ViewData> views,
  ) mirrorPageTree;

  /// The server origin the mirror is scoped by (the live api base).
  final String Function() origin;

  /// The State's `mounted` — mid-flight guards below protect durable writes,
  /// not just setState, so they run inside the controller.
  final bool Function() isMounted;

  /// The tab state's currently open view id, read at the moment the original
  /// code read `_selectedView?.id` (timing is part of the contract: it is read
  /// before AND after the awaits that may change it).
  final String? Function() openViewId;

  /// The document the live CRDT session holds, if any — the REST snapshot must
  /// not land on a body it owns (see `restSnapshotMayReplaceBody`).
  final String? Function() crdtSessionDocumentId;

  /// Which of several overlapping tree fetches for a workspace may commit. See
  /// [TreeRequestSeq] for why the last-arriving response is not the right one
  /// to believe.
  final TreeRequestSeq _requests = TreeRequestSeq();

  // ── Presentation hooks (implemented by the State) ─────────────────────────
  //
  // The controller never touches a context; these are the only way it reaches
  // the user or the tab state.

  /// A failure string for the message bus (the old `_message` field).
  void Function(String message)? onError;

  /// The tree load's outcome for the SELECTION slot — the State applies the
  /// same writes the old inline setState block made.
  void Function(ViewsSelection selection)? onSelection;

  /// A batch-delete decided the open page was inside a trashed subtree.
  void Function()? onOpenPageGone;

  /// A clone finished and the tree was reloaded — the copy's deduped name is
  /// worth telling the user.
  void Function(CloneReport report)? onCloned;

  /// The page a workspace was last on, resolved against the tree ALREADY in
  /// memory — null when this workspace has not been opened in this session.
  ///
  /// Deliberately the same rule [loadWorkspaceViews] applies once the real
  /// tree arrives (remembered id, else the first openable page), so the page
  /// shown the instant you switch is the page you end up on. A different rule
  /// here would show one page and then swap it for another.
  DocumentView? rememberedViewOf(String workspaceId) {
    final views = session.viewsByWorkspace[workspaceId];
    if (views == null || views.isEmpty) return null;
    final remembered = readPref('lastViewId:$workspaceId');
    return views.where((view) => view.id == remembered).firstOrNull ??
        firstOpenableView(views);
  }

  /// Rename [viewId] in [workspaceId]'s tree. The renamed view REPLACES the old
  /// row by id (position in the list preserved), not appended — the sidebar
  /// must not jump. Returns the server's renamed view.
  Future<DocumentView> renameView(
    String token,
    String workspaceId,
    String viewId,
    String name,
  ) async {
    final renamed = await _updateView(token, workspaceId, viewId, name);
    session.replaceViews(
      workspaceId,
      (session.viewsByWorkspace[workspaceId] ?? const [])
          .map((item) => item.id == renamed.id ? renamed : item)
          .toList(),
    );
    return renamed;
  }

  /// Set [view]'s emoji. The NAME is passed through untouched (the picker's
  /// three-way icon result must never wipe anything else), and the updated row
  /// replaces by id exactly like a rename.
  Future<DocumentView> setViewIcon(
    String token,
    String workspaceId,
    DocumentView view,
    String icon,
  ) async {
    final updated = await _updateView(
      token,
      workspaceId,
      view.id,
      view.name,
      icon: icon,
    );
    session.replaceViews(
      workspaceId,
      (session.viewsByWorkspace[workspaceId] ?? const [])
          .map((item) => item.id == updated.id ? updated : item)
          .toList(),
    );
    return updated;
  }

  /// Create a page and its bootstrap in [workspaceId]. The new view is
  /// APPENDED to the tree (the caller opens it — tab state is the State's).
  /// Returns both so the caller can put the page straight into the editor.
  Future<({DocumentView view, DocumentBootstrap bootstrap})> createDocument(
    String token,
    String workspaceId,
    String name, {
    String? parentViewId,
  }) async {
    final created = await _createDocument(
      token,
      workspaceId,
      name,
      parentViewId: parentViewId,
    );
    final bootstrap = await _bootstrapDocument(
      token,
      workspaceId,
      created.document.id,
    );
    session.replaceViews(workspaceId, [
      ...session.viewsByWorkspace[workspaceId] ?? const [],
      created.view,
    ]);
    return (view: created.view, bootstrap: bootstrap);
  }

  /// Create a cloud folder (pure container) and add it to the tree. Unlike a
  /// document it is NOT opened in the editor (folders have no content); the
  /// user creates pages under it. Mirrored offline via [mirrorPageTree].
  ///
  /// Returns the new view's id (null when the State died mid-flight) so the
  /// caller can drop it into inline-rename — the sidebar name becomes editable
  /// immediately, no dialog.
  Future<String?> createFolder(
    String token,
    String workspaceId,
    String name, {
    String? parentViewId,
  }) async {
    final view = await _createFolder(
      token,
      workspaceId,
      name,
      parentViewId: parentViewId,
    );
    if (!isMounted()) return null;
    session.replaceViews(workspaceId, [
      ...session.viewsByWorkspace[workspaceId] ?? const [],
      view,
    ]);
    // This workspace only. Unqualified, it wrote every workspace the mirror
    // had preheated into memory — which on a real account is the 2.1 s the
    // mirror method's own comment measured, paid on every new folder.
    mirrorOneWorkspace(workspaceId: workspaceId);
    return view.id;
  }

  /// Trash one view. The server answers with the whole new tree, which
  /// REPLACES the held one.
  Future<void> deleteView(
    String token,
    String workspaceId,
    String viewId,
  ) async {
    final views = await _deleteView(token, workspaceId, viewId);
    session.replaceViews(workspaceId, views);
  }

  /// Trash a selection in ONE request, then reload the tree.
  ///
  /// Reloads rather than reading a tree back from the response: `batch-trash`
  /// reports what it TOUCHED (ids + a count), which is the honest thing for it
  /// to report and not a page tree. One extra GET for an operation the user
  /// invoked deliberately is a fair trade for not inventing a second shape of
  /// truth about the tree.
  Future<void> deleteViews({
    required String token,
    required String workspaceId,
    required List<String> viewIds,
  }) async {
    final result = await _batchTrashViews(
      token: token,
      workspaceId: workspaceId,
      viewIds: viewIds,
    );
    await loadWorkspaceViews(token, workspaceId);
    if (!isMounted()) return;
    // The open page may have been anywhere inside a trashed subtree, so the
    // reloaded tree — not the requested ids — decides whether it survived.
    final open = openViewId();
    final stillThere = (session.viewsByWorkspace[workspaceId] ?? const []).any(
      (v) => v.id == open,
    );
    if (open != null && !stillThere) {
      onOpenPageGone?.call();
    }
    // `skipped` is the server saying it did LESS than asked (already trashed,
    // or gone). Silent here on purpose: both causes mean the row is already
    // where the user wanted it, and the reloaded tree shows exactly that.
    if (result.skipped.isNotEmpty) {
      debugPrint('batch-trash skipped ${result.skipped.length} ids');
    }
  }

  /// Duplicate [viewId] in place (cloud). The server copies the subtree, shares
  /// blobs, and dedupes the name; we just reload the tree to show the copy.
  Future<void> cloneView({
    required String token,
    required String workspaceId,
    required String viewId,
    required String name,
  }) async {
    final report = await _cloneView(
      token: token,
      workspaceId: workspaceId,
      viewId: viewId,
      name: name,
      dryRun: false,
    );
    await loadWorkspaceViews(token, workspaceId);
    onCloned?.call(report);
  }

  Future<void> restoreView(
    String token,
    String workspaceId,
    String viewId,
  ) async {
    final views = await _restoreView(token, workspaceId, viewId);
    session.replaceViews(workspaceId, views);
  }

  /// Persist a new sibling order: assign evenly spaced, zero-padded positions
  /// to [orderedSiblings] (all sharing [parentViewId]) and push the ones that
  /// changed. Ordering is per-parent, so renumbering one group is
  /// self-contained.
  ///
  /// Show the new order NOW; tell the server after.
  ///
  /// This used to await one moveView round trip PER row and only then
  /// setState, so the rows sat visibly in their old places for N requests
  /// before jumping. For a drag that reads as lag; for a freshly created page
  /// being slid up beside the located row it read as a bug — "it appears at the
  /// bottom and then moves" — and no care in the caller could hide it, because
  /// the caller was already done.
  ///
  /// Optimistic, with the server as the correction: the tree already holds
  /// what actually came back (a reload), and a failure surfaces through the
  /// caller's busy wrapper like any other write. The local guess uses the
  /// SAME step-of-ten rule the request does, so guess and answer agree instead
  /// of racing.
  Future<void> reorderViews(
    String token,
    String workspaceId,
    String? parentViewId,
    List<DocumentView> orderedSiblings,
  ) async {
    String pad(int n) => n.toString().padLeft(10, '0');

    final optimisticWorkspace = session.selectedWorkspace;
    if (optimisticWorkspace != null) {
      final views = [...?session.viewsByWorkspace[optimisticWorkspace.id]];
      for (var i = 0; i < orderedSiblings.length; i++) {
        final v = orderedSiblings[i];
        final placed = DocumentView(
          id: v.id,
          parentViewId: parentViewId,
          objectId: v.objectId,
          objectType: v.objectType,
          name: v.name,
          position: pad((i + 1) * 10),
          icon: v.icon,
          updatedAt: v.updatedAt,
        );
        final idx = views.indexWhere((x) => x.id == v.id);
        // Not present yet happens on the create path: the row was added by the
        // create's own replace in this same turn, or is about to be.
        if (idx >= 0) {
          views[idx] = placed;
        } else {
          views.add(placed);
        }
      }
      session.replaceViews(optimisticWorkspace.id, views);
    }

    if (orderedSiblings.isEmpty) return;

    // ONE request for the whole group.
    //
    // This was a loop of `moveView`, awaited per sibling. It looked like it
    // only paid for the rows that actually moved — `if (view.position !=
    // desired)` — but a view is created with a `Uuid::now_v7()` position, so
    // that test is TRUE for every sibling that has not been reordered before.
    // Creating a page or folder beside a located row therefore fired one
    // round trip per existing child of that folder, in series, with the busy
    // wrapper holding the whole app busy throughout. That is the lag: it
    // grows with the size of the folder you are creating in.
    //
    // `/views/reorder` exists for exactly this and has since 0.13.19 — its own
    // comment names the per-view loop as the thing not to do — but no client
    // code had ever called it. It validates every id before writing and does
    // the renumber in one transaction, so it also removes the half-renumbered
    // group a failure used to leave behind.
    //
    // Nothing to reconcile afterwards: the optimistic write above assigned
    // `pad((i + 1) * 10)`, which is the same value the server assigns
    // (`format!("{:010}", (i + 1) * 10)`), for the same ids in the same order.
    // The reply is a count, not views, so there is nothing to correct from
    // either.
    //
    // One field DOES drift until the next tree load: the server bumps
    // `updated_at` on every reordered row, and the per-view reply used to
    // carry that back. Left as is on purpose — "recently edited" jumping to
    // the top because you dragged a row is the questionable behaviour here,
    // and quietly re-teaching the client to display it is not the place to
    // decide that.
    await _reorderViews(
      token,
      workspaceId,
      parentViewId: parentViewId,
      orderedViewIds: [for (final v in orderedSiblings) v.id],
    );
  }

  /// Resolve a view for a background tab: from the tree in memory, or by
  /// fetching that workspace's tree when this device has never opened it.
  ///
  /// The tree of a workspace this device has never opened is not in memory —
  /// `viewsByWorkspace` fills per workspace as you visit them. Fetching it is
  /// one request, and it is the tree the tab needs anyway the moment you click
  /// it. A fetch failure is reported through [onError] and resolves null.
  Future<DocumentView?> ensureView(
    String token,
    String workspaceId,
    String viewId,
  ) async {
    var view = session.viewsByWorkspace[workspaceId]
        ?.where((v) => v.id == viewId)
        .firstOrNull;
    if (view == null) {
      try {
        final answer = await _listViewsIfChanged(token, workspaceId);
        final views = answer.views;
        if (views != null && isMounted()) {
          session.replaceViews(workspaceId, views);
          view = views.where((v) => v.id == viewId).firstOrNull;
        }
      } on ApiException catch (error) {
        onError?.call(error.message);
        return null;
      }
    }
    return view;
  }

  /// Re-ask for [workspaceId]'s tree because the server said it changed.
  ///
  /// The ETag turns a ping about something this client already knows (its own
  /// action's refetch, a rename it just made) into a 304. Errors are swallowed:
  /// this runs off a background bell, there is nothing for a user to act on,
  /// and the next bell — or the manual refresh button, which stays — retries.
  Future<void> refreshTree(String token, String workspaceId) async {
    // Claim a sequence number for THIS request. Anything older that lands later
    // is answering a question we have already replaced.
    final seq = _requests.claim(workspaceId);
    final held = session.viewsByWorkspace[workspaceId];
    final answer = await fetchBackgroundTree(
      () => _listViewsIfChanged(
        token,
        workspaceId,
        etag: (held == null || held.isEmpty)
            ? null
            : readPref('viewsEtag:$workspaceId'),
      ),
    );
    if (answer == null) return; // Transient failure; another bell retries.
    final views = answer.views;
    if (views == null || !isMounted()) return; // 304 — already current.
    // Superseded while we were in flight: a newer request exists, so this
    // response describes a state that has already been replaced. Writing it
    // would install the older tree AND its older ETag — and a stale ETag is
    // the durable half of the damage, because the next cold start sends it,
    // gets a 304, and keeps a tree the server has moved past.
    if (!_requests.isCurrent(workspaceId, seq)) return;
    _commitTree(workspaceId: workspaceId, views: views, etag: answer.etag);
    mirrorOneWorkspace(workspaceId: workspaceId);
  }

  /// Load the tree of [workspaceId] and decide what the selection should be.
  ///
  /// The workspace-switch load: a speculative bootstrap of the remembered page
  /// races the tree fetch, a conditional (ETag) request avoids re-downloading
  /// an unchanged tree, and the auto-open lands on a DOCUMENT (never a folder).
  Future<void> loadWorkspaceViews(String token, String workspaceId) async {
    // Which page we are about to open is usually already known — it is the one
    // this workspace was last on, and its tree is in memory. Asking for its
    // content only AFTER the tree comes back spends a second round trip
    // establishing something we could have assumed; ask for both at once and
    // throw the guess away if the tree disagrees.
    final guess = rememberedViewOf(workspaceId);
    Future<DocumentBootstrap>? guessed;
    if (guess != null && guess.objectType != 'folder') {
      guessed = _bootstrapDocument(token, workspaceId, guess.objectId);
    }

    // Claim a sequence number for THIS request. Anything older that lands later
    // is answering a question we have already replaced.
    final seq = _requests.claim(workspaceId);

    // Ask conditionally, but only while actually holding a tree to fall back
    // on: a 304 answered to a client with nothing would leave the sidebar
    // empty. The tag is stored beside the tree it describes, so a mirror
    // preheated from disk and its tag stay in step across restarts.
    final held = session.viewsByWorkspace[workspaceId];
    final answer = await _listViewsIfChanged(
      token,
      workspaceId,
      etag: (held == null || held.isEmpty)
          ? null
          : readPref('viewsEtag:$workspaceId'),
    );
    // Superseded while we were in flight: a newer request for this workspace
    // exists, so this response describes a state that has already been
    // replaced. Writing it would install the older tree AND its older ETag —
    // and the stale ETag is the durable half, because the next cold start
    // sends it, gets a 304, and keeps a tree the server has moved past. The
    // selection block below guards itself separately (`stillCurrent`), but the
    // TREE and its tag had no such guard: a slower load landing last
    // overwrote the newer one.
    if (!_requests.isCurrent(workspaceId, seq)) return;

    final views = answer.views ?? held ?? const <DocumentView>[];
    _commitTree(
      workspaceId: workspaceId,
      views: views,
      etag: answer.views == null ? null : answer.etag,
    );
    SwitchTrace.current?.mark(answer.views == null ? 'tree(304)' : 'tree');
    // Reopen the page last viewed IN THIS workspace (AppFlowy remembers the
    // last view per-workspace). On a restore or a workspace switch the open
    // view is null, so fall back to the per-workspace saved id; a
    // deleted/absent id just falls through to firstOpenableView below.
    final wantViewId = openViewId() ?? readPref('lastViewId:$workspaceId');
    final selectedView = views.where((view) => view.id == wantViewId).firstOrNull;
    // Auto-open a DOCUMENT (folders have no content to bootstrap — opening one
    // would 404 on its unbacked object_id).
    final viewToOpen = selectedView ?? firstOpenableView(views);
    DocumentBootstrap? bootstrap;
    String? bootstrapError;
    if (viewToOpen != null && viewToOpen.objectType != 'folder') {
      final inFlight = guessed != null && guess?.objectId == viewToOpen.objectId
          ? guessed
          : null;
      if (inFlight == null) guessed?.ignore(); // guessed wrong — drop it quietly
      try {
        bootstrap = await (inFlight ??
            _bootstrapDocument(token, workspaceId, viewToOpen.objectId));
      } on ApiException catch (error) {
        // The auto-open target answered with an error (404/403 — deleted
        // server-side, access revoked, or an unbacked folder object_id from an
        // older client that didn't guard). listViews already succeeded, so we
        // are demonstrably ONLINE: show the tree with nothing opened rather
        // than letting one bad view cascade into a full offline downgrade at
        // startup.
        //
        // But SAY SO. Swallowing this silently rendered an open page as empty
        // — indistinguishable from "this page has no content", so a
        // server-side fault (e.g. a document whose root block was lost) looked
        // exactly like data loss: pressing refresh appeared to erase the page.
        bootstrap = null;
        bootstrapError = '$error';
      }
    } else {
      // Nothing to open (empty workspace, or a folder was remembered) — the
      // speculative fetch has no taker. Unignored, its failure would surface
      // as an unhandled async error with no user-visible cause.
      guessed?.ignore();
    }

    SwitchTrace.current?.mark('body');
    // Two clicks in quick succession run two of these. The tree is keyed by
    // workspace so a late arrival is harmless there, but the SELECTION is one
    // slot: without this the slower load lands last and opens the workspace
    // the user already switched away from. Nothing guards the entry point —
    // there is no busy latch on the workspace rows — so it has to be caught
    // here.
    final stillCurrent = session.selectedWorkspace?.id == workspaceId;
    // The REST snapshot cannot merge — it is the yrs base materialised into
    // plain JSON — so it must not land on a body a CRDT session already
    // holds. See `restSnapshotMayReplaceBody` for what that costs when it
    // does.
    final mayWriteBody =
        viewToOpen == null ||
        restSnapshotMayReplaceBody(
          arrivingDocumentId: viewToOpen.objectId,
          crdtSessionDocumentId: crdtSessionDocumentId(),
        );
    // The tree itself was installed by `_commitTree` (with its ETag, in one
    // step). The selection block owns the SELECTION and the body only.
    onSelection?.call(
      ViewsSelection(
        stillCurrent: stillCurrent,
        view: viewToOpen,
        bootstrap: bootstrap,
        writeBody: mayWriteBody,
        bootstrapError: bootstrapError,
      ),
    );
    SwitchTrace.current?.mark('paint');
    // A 304 means the mirror ALREADY holds this tree — there is nothing to
    // write, and writing it anyway is the whole cost. Measured on a real
    // account: 2.1s per warm switch, against 130–320ms for everything the user
    // waits on. (Narrowing it to one workspace was not enough on its own: the
    // store's purge step still scans every view of the origin, and that scan
    // is most of the cost. Not doing it at all is both faster and more
    // obviously correct.)
    if (answer.views != null) {
      mirrorOneWorkspace(workspaceId: workspaceId);
    }
    SwitchTrace.current?.mark('mirror-write');
  }

  /// Install a freshly fetched tree together with the ETag that describes it.
  ///
  /// One function so the two cannot drift: the invariant the offline mirror
  /// depends on is "_a saved ETag means the mirror already holds that tree_",
  /// and it only holds if the tag is written in the same step as the tree it
  /// came with. Splitting them — as the two call sites used to — is how a tree
  /// from one response ends up paired with an ETag from another.
  void _commitTree({
    required String workspaceId,
    required List<DocumentView> views,
    String? etag,
  }) {
    // Only advance the tag when the server actually sent one. A 304 has no
    // body and no tag, and keeping the old one is exactly right — it still
    // describes the tree we are holding.
    if ((etag ?? '').isNotEmpty) {
      savePref('viewsEtag:$workspaceId', etag!);
    }
    session.replaceViews(workspaceId, views);
  }

  /// Mirror the cloud page tree (workspace list + per-workspace views) into the
  /// on-device store so a future offline start can still list and navigate
  /// cloud content (P2 option C — Phase 1b/1c). Origin-scoped by server URL,
  /// so switching servers doesn't cross over. P4-2: on web the mirror is
  /// localStorage-backed (LocalOffline web variant). The cloud is authoritative
  /// — this is a clean replace after each successful online load.
  /// Write the page-tree mirror for the ONE workspace whose tree just changed.
  ///
  /// It used to write EVERY workspace held in memory, every time. That was
  /// nearly free while memory only held the two or three workspaces visited in
  /// this session — and stopped being free the moment the mirror was preheated
  /// at startup, because memory then held all of them. Measured on a real
  /// account (35 workspaces) right after that change: a warm switch spent
  /// **2.1 seconds** here, against 130–320ms for everything the user actually
  /// waits on.
  ///
  /// Safe to narrow because `mirrorCloudPageTree` replaces per workspace: the
  /// ones not named here keep their rows.
  ///
  /// [workspaceId] is REQUIRED, and that is the fix for this happening twice.
  /// It was optional, defaulting to "write everything", and the second caller —
  /// creating a folder — simply did not pass it. So the 2.1 s write came back on
  /// every new folder, against a mirror that by then held ~13k views. An
  /// optional parameter whose omission is a performance cliff is a footgun with
  /// a safety catch; required, the compiler asks the question.
  void mirrorOneWorkspace({required String workspaceId}) {
    final currentOrigin = origin();
    final workspaces = <WorkspaceData>[
      for (final (i, w) in session.workspaces.indexed)
        (
          id: w.id,
          name: w.name,
          position: ((i + 1) * 10).toString().padLeft(10, '0'),
          role: w
              .role, // mirror the real role so offline editing knows its rights
        ),
    ];
    final views = <ViewData>[
      for (final e in session.viewsByWorkspace.entries)
        if (e.key == workspaceId)
        for (final v in e.value)
          (
            id: v.id,
            workspaceId: e.key,
            parentId: v.parentViewId,
            objectId: v.objectId,
            name: v.name,
            position: v.position,
            trashed: false,
            objectType: v.objectType,
          ),
    ];
    mirrorPageTree(currentOrigin, workspaces, views);
  }
}

/// The selection outcome of a workspace tree load, handed to the State (the old
/// inline setState block): which page is open, the body that goes with it (when
/// a CRDT session does not already own that body), and the error to say out
/// loud when the auto-open target failed.
class ViewsSelection {
  const ViewsSelection({
    required this.stillCurrent,
    required this.view,
    required this.bootstrap,
    required this.writeBody,
    required this.bootstrapError,
  });

  /// Whether the workspace this load was for is still the selected one.
  final bool stillCurrent;

  /// The page to open (a document, or null for an empty workspace).
  final DocumentView? view;

  /// The body for [view], when one was fetched.
  final DocumentBootstrap? bootstrap;

  /// Whether [bootstrap] may be written over what the editor already shows.
  final bool writeBody;

  /// The auto-open target's failure, verbatim — or null.
  final String? bootstrapError;
}
