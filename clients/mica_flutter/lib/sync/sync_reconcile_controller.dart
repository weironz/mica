import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';

import '../api/models.dart';
import '../api/sync_client.dart';
import '../api/views_events.dart';
import '../cloud/cloud_sync_session.dart';
import '../doc_tab.dart';
import '../session/session_controller.dart';

/// The sync domain of `_WorkspaceShellState` — the reconcile loop that keeps
/// the document sockets (presence + CRDT replica) tracking the open tab, the
/// sidebar's tree-change socket, the cross-document outbox sweep, the
/// sync-paused fault banner and the op-path remote-seq refetch — extracted
/// verbatim (god-class seventh step; see
/// docs/reviews/code-optimization/OPTIMIZATION_PLAN_ROUND3.md §7).
///
/// The TABS themselves are not owned here: they arrive through the injected
/// [tabs] / [activeIndex] accessors, and the controller touches each tab only
/// through the [TabSockets] interface — a tab's socket pair, its document id
/// and its activation stamp. The tab model (view, bootstrap, workspace stamp)
/// stays the State's. What the controller owns is the SOCKET lifecycle and its
/// bookkeeping: the tree-change socket, the refetch debounce timer, the sweep
/// guard, the banner-once flag, and the device client id.
///
/// The sessions it builds ([DocumentSyncClient] / [CloudSyncSession]) are
/// constructed directly — they are already injectable models — while
/// everything the SHELL knows (base URI, sign-in, the on-device stores,
/// `mounted`) arrives as injected functions. User feedback (the banner pair,
/// the presence row, the editor-body rebuild) is reported back through hooks
/// implemented by the State; the controller never touches a context.
class SyncReconcileController extends ChangeNotifier {
  SyncReconcileController({
    required this.session,
    required this.tabs,
    required this.activeTab,
    required this.workspaceForActiveTab,
    required this.activeDocumentId,
    required this.activeBootstrap,
    required this.baseUri,
    required this.activeIsLocal,
    required this.isMounted,
    required this.isWeb,
    required this.deviceClientId,
    required this.cloudDocStore,
    required this.openWebDocStore,
    required this.ensureFreshSession,
    required this.readPref,
    required this.savePref,
    required this.refreshTree,
    required this.applyCloudBlocks,
    required this.loadComments,
    required this.onCloudOnline,
    required this.onReadyReconcileUploads,
    required this.onSyncPhase,
    required this.onFaultBanner,
    required this.onHideFaultBanner,
    required this.onRefreshBootstrap,
    required this.onPresence,
    required this.presenceIsEmpty,
    required this.onPresenceCleared,
  });

  /// The session/workspace hub — read for the sign-in the sockets authorize
  /// with and the trees the sweep walks. Never written here.
  final SessionController session;

  /// The shell's tab list — the controller reads it fresh at the exact moments
  /// the original code read `_tabs`.
  final List<TabSockets> Function() tabs;

  /// The ACTIVE tab — what the shell's `_sync` / `_cloudSession` proxies
  /// resolve to. Read fresh each time; tab switches must be seen immediately.
  final TabSockets Function() activeTab;

  /// The workspace the ACTIVE tab's document lives in (the shell's
  /// `_workspaceForTab` fallback rule — deliberately not re-owned here).
  final Workspace? Function() workspaceForActiveTab;

  /// The ACTIVE tab's loaded document id, or null while it holds none — read
  /// before AND after awaits that may change it.
  final String? Function() activeDocumentId;

  /// The ACTIVE tab's bootstrap, or null — read for the presence filter and
  /// the remote-seq path; never rebuilt here (the editor body is the State's).
  final DocumentBootstrap? Function() activeBootstrap;

  final Uri Function() baseUri;
  final bool Function() activeIsLocal;

  /// The State's `mounted` — the guards below protect socket writes and
  /// durable outbox drains, not just setState.
  final bool Function() isMounted;

  /// `kIsWeb` — the cross-doc sweep is desktop-only.
  final bool Function() isWeb;

  /// The device's stable CRDT actor id (desktop: from the local store; web: a
  /// per-session placeholder). The FIRST ask opens the on-device store, so it
  /// is an injected async rather than a value.
  final Future<BigInt?> Function() deviceClientId;

  /// The desktop on-device doc store for a doc id, or null.
  final CloudDocStore? Function(String documentId) cloudDocStore;

  /// The web (IndexedDB) doc store for a doc id, or null.
  final Future<CloudDocStore?> Function(String documentId) openWebDocStore;

  /// Renew the access token before building a socket URI — the URI builders
  /// run per connection attempt, never with a baked-in token.
  final Future<void> Function() ensureFreshSession;

  final String? Function(String key) readPref;
  final void Function(String key, String value) savePref;

  // ── Hooks (implemented by the State) ──────────────────────────────────────

  /// Re-ask for a workspace's tree (the views-events bell).
  final Future<void> Function(String workspaceId) refreshTree;

  /// Rebuild the selected bootstrap from the replica's blocks (onReady /
  /// onRemoteBlocks) — stays in the State: a setState over the ACTIVE tab's
  /// editor body.
  final void Function(String documentId) applyCloudBlocks;

  /// Comments anchor into the CRDT; resolvable only once bootstrapped.
  final Future<void> Function(String documentId) loadComments;

  /// Reaching the server means online again — the State refetches the
  /// authoritative nav and drains this doc's pending image uploads.
  final void Function() onCloudOnline;

  /// A cold bootstrap completed for [documentId] — the State drains that
  /// doc's offline-inserted images (§7.1). Distinct from [onCloudOnline]:
  /// this one is per-document and fires on ready, not on server contact.
  final Future<void> Function(String documentId) onReadyReconcileUploads;

  /// The sync phase badge on the open document.
  final void Function(SyncPhase phase)? onSyncPhase;

  /// The "cloud sync paused" banner pair — needs a context, so the State's.
  final void Function()? onFaultBanner;
  final void Function()? onHideFaultBanner;

  /// The op-path refetch of the open document (the debounced
  /// `_refreshSelectedBootstrap`, still the State's — it awaits the api).
  final void Function(String documentId) onRefreshBootstrap;

  /// The presence row write for the ACTIVE tab's document (setState).
  final void Function(String documentId, List<PresenceUser> users) onPresence;

  /// Whether the presence row is empty — the close path's clear is conditional
  /// in the original (`if (_presence.isNotEmpty)` guards a pointless setState),
  /// and only the State knows.
  final bool Function() presenceIsEmpty;

  /// The presence row clear (setState).
  final void Function() onPresenceCleared;

  // ── Owned state ───────────────────────────────────────────────────────────

  /// The tree-change socket for the workspace whose SIDEBAR is showing, plus
  /// which workspace that is — so reconcile can tell "same one" from "moved".
  ViewsEventsChannel? _viewsEvents;
  String? _viewsEventsWorkspaceId;

  /// Debounced refetch timer for the op-path remote-seq notice.
  Timer? _syncRefetchTimer;

  /// Awareness: debounce broadcasting the local caret as presence (P2).
  Timer? _cursorTimer;

  /// Guards the cross-document drain so only one runs at a time (it fires on
  /// every online/reconnect).
  bool _sweepingOutboxes = false;

  /// B3: whether the "cloud sync paused" banner is up, so it shows once per
  /// stuck episode and clears on recovery.
  bool _syncBannerShown = false;

  /// Cached [deviceClientId] — the second and later sessions (and the sweep)
  /// need it synchronously.
  BigInt? _clientId;

  // ── Reconcile loop ────────────────────────────────────────────────────────

  /// Keep the tree-change socket pointed at the sidebar's workspace.
  ///
  /// Separate from the document socket reconcile below on purpose: that one
  /// keys off the OPEN DOCUMENT and bails when there is none, but an empty
  /// workspace still has a tree that MCP can grow into — the exact case this
  /// feature is for.
  void reconcileViewsEvents() {
    final auth = session.session;
    final workspaceId = activeIsLocal()
        ? null
        : session.selectedWorkspace?.id;
    if (auth == null || workspaceId == null) {
      disposeViewsEvents();
      return;
    }
    if (_viewsEventsWorkspaceId == workspaceId) return;
    _viewsEvents?.dispose();
    _viewsEventsWorkspaceId = workspaceId;
    _viewsEvents = ViewsEventsChannel(
      uri: () async {
        await ensureFreshSession();
        return viewsSocketUri(
          baseUri(),
          workspaceId,
          session.session?.accessToken ?? auth.accessToken,
        );
      },
      onChanged: () => refreshTree(workspaceId),
    )..connect();
  }

  /// The tree-change socket is a cloud thing too; without this it would keep
  /// reconnecting with a token that logout just revoked.
  void disposeViewsEvents() {
    _viewsEvents?.dispose();
    _viewsEvents = null;
    _viewsEventsWorkspaceId = null;
  }

  /// Open, switch, or close the document WebSocket so it always tracks the
  /// currently selected document.
  void reconcile() {
    reconcileViewsEvents();
    final session = this.session.session;
    // The socket belongs to the DOCUMENT, so it takes the workspace from the
    // tab holding it — not from `_selectedWorkspace`. Those differ for a moment
    // whenever a tab switch also switches workspaces (the shell catches up
    // asynchronously), and for that moment this would otherwise open
    // `/ws/workspaces/<the one we left>/documents/<this tab's doc>`, which the
    // server rejects because its document lookup is workspace-scoped too.
    final workspace = workspaceForActiveTab();
    final documentId = activeDocumentId();

    if (session == null || workspace == null || documentId == null) {
      closeActiveDocumentSync();
      return;
    }

    final active = activeTab();
    if (active.sync?.documentId == documentId) {
      return;
    }

    closeActiveDocumentSync();
    final sync = DocumentSyncClient(
      documentId: documentId,
      // Renewed per attempt, same as the CRDT session below.
      uri: () async {
        await ensureFreshSession();
        return documentSocketUri(
          baseUri(),
          workspace.id,
          documentId,
          this.session.session?.accessToken ?? session.accessToken,
        );
      },
      selfName: session.user.displayName,
      onRemoteSeq: handleRemoteSeq,
      onPresence: (users) {
        // Only the tab being LOOKED AT owns the presence row. A background tab
        // is still connected under the cap, and without this test its roster
        // would overwrite the visible one — the reader would see the avatars of
        // whoever is editing some other page. `handleRemoteSeq` already
        // self-filters on documentId; this is the same guard for presence.
        if (isMounted() && activeDocumentId() == documentId) {
          onPresence(documentId, users);
        }
      },
    );
    active.sync = sync;
    onPresenceCleared();
    sync.connect();

    // A new connection just came up — park the least-recently-used ones that no
    // longer fit under the cap.
    enforceSyncCap();

    // Open a yrs CRDT session for this doc (desktop = Rust FFI replica, web = JS
    // yjs replica — both wire-compatible). P3d: reconcile itself only runs
    // off cloud state (session + workspace + bootstrap — all cloud-world
    // fields), so the old mode guard is redundant; a local-world selection
    // never reaches here.
    unawaited(_setupCloudYrs(documentId, workspace, session));
  }

  Future<void> _setupCloudYrs(
    String documentId,
    Workspace workspace,
    AuthSession session,
  ) async {
    // Desktop pins the CRDT actor to the device's stable client id; web has no
    // on-device store, so it uses a per-session yjs actor (placeholder id here).
    var clientId = _clientId;
    if (clientId == null) {
      clientId = await deviceClientId() ?? (isWeb() ? BigInt.zero : null);
      _clientId = clientId;
    }
    if (clientId == null || !isMounted()) return;
    // The selection may have moved while we awaited the device id.
    if (activeDocumentId() != documentId || _activeSyncDocumentId() != documentId) {
      return;
    }
    // C1: unacked local diffs persist per-document, so a crash / hard close
    // recovers edits the server never acked. Restored here; re-pushed on connect.
    final unackedKey = 'cloudUnacked:$documentId';
    // Local-first (Phase 1): mirror this cloud doc to the on-device store so it
    // reads offline across a restart. deviceClientId() above opened the store.
    // P4-2: on web the mirror is IndexedDB-backed instead (null only when
    // IndexedDB is unavailable → online-only, as before).
    var persistence = cloudDocStore(documentId);
    persistence ??= await openWebDocStore(documentId);
    // The selection may have moved while the browser store opened. If it did,
    // dispose the store we just opened — on web it holds a single-writer Web
    // Lock that would otherwise leak (orphaned, never disposed → that doc can
    // never take the writable mirror again this page load).
    if (!isMounted() ||
        activeDocumentId() != documentId ||
        _activeSyncDocumentId() != documentId) {
      persistence?.dispose();
      return;
    }
    if (persistence != null) {
      // P2b: the append-log is now the durable outbox. One-time migration — fold
      // any legacy prefs `cloudUnacked` queue into it (append THEN delete; the
      // reverse order would drop in-flight edits), so unpushed edits from before
      // P2b survive and get pushed on connect.
      final legacy = loadUnacked(unackedKey);
      if (legacy.isNotEmpty) {
        for (final diff in legacy) {
          persistence.appendOutbox(diff);
        }
        savePref(unackedKey, ''); // the log owns the outbox now
      }
    }
    final yrs = CloudSyncSession(
      // Rebuilt per connection attempt, and renewed first. This session lives as
      // long as the document stays open — hours — while the access token lapses
      // in one. Renewal is otherwise only driven by `_run()` (a user action), so
      // an idle window longer than the TTL used to leave every reconnect
      // replaying a dead token: 401, back off, repeat, silently forever.
      uri: () async {
        await ensureFreshSession();
        return documentSocketUri(
          baseUri(),
          workspace.id,
          documentId,
          this.session.session?.accessToken ?? session.accessToken,
        );
      },
      clientId: clientId,
      onReady: (_, _) {
        clearSyncBanner(); // B3: a fresh bootstrap means sync recovered.
        applyCloudBlocks(documentId);
        // Now online for this doc: drain any images inserted offline (§7.1).
        unawaited(onReadyReconcileUploads(documentId));
        // Comments are anchored into this document's CRDT, so they can only be
        // resolved once it is bootstrapped.
        unawaited(loadComments(documentId));
      },
      onRemoteBlocks: (_) => applyCloudBlocks(documentId),
      onFault: (reason, count) => onCloudSyncFault(documentId, reason, count),
      // Reaching the server means we're back online — leave the P1c offline-nav
      // fallback (refetch workspaces/roles) AND drain every other cloud doc's
      // un-pushed offline outbox, not just this active one.
      onServerConnected: onCloudOnline,
      onSyncPhase: onSyncPhase,
      // Desktop's durable outbox is the append-log (persistence); web / no
      // store keeps the in-memory queue + prefs crash-recovery (C1).
      restoreUnacked: persistence == null ? loadUnacked(unackedKey) : null,
      onPersistUnacked: persistence == null
          ? (unacked) => saveUnacked(unackedKey, unacked)
          : null,
      persistence: persistence,
    );
    activeTab().cloudSession = yrs;
    yrs.connect();
  }

  /// The ACTIVE tab's doc socket document id, if any (the liveness guard
  /// `_setupCloudYrs` re-checks after its awaits).
  String? _activeSyncDocumentId() => activeTab().sync?.documentId;

  // ── Socket teardown ───────────────────────────────────────────────────────

  /// Drop [tab]'s sockets while keeping everything the reader can see.
  ///
  /// The tab keeps its `view` and `bootstrap`, so it still renders the page it
  /// was showing — it just stops receiving remote edits until it is activated
  /// again, which reconnects it through [reconcile].
  ///
  /// `drainAndDispose` rather than `dispose`: the outgoing replica may hold
  /// edits the server has not acked, and dropping the session without draining
  /// them loses content. This is the same call [closeActiveDocumentSync] makes,
  /// for the same reason.
  void parkTab(TabSockets tab) {
    tab.sync?.dispose();
    tab.sync = null;
    final cloud = tab.cloudSession;
    tab.cloudSession = null;
    if (cloud != null) unawaited(cloud.drainAndDispose());
  }

  /// Park live tabs past [kMaxLiveSyncTabs]; see [tabsToPark] for the choice.
  void enforceSyncCap() {
    final tabs = this.tabs();
    // The ACTIVE tab is by definition the most recently activated, but it is
    // excluded explicitly so a misconfigured cap cannot disconnect the page
    // the user is typing into.
    for (final tab in tabsToPark(tabs, activeTab())) {
      parkTab(tab);
    }
  }

  /// Tear down EVERY tab's sockets, not just the active one.
  ///
  /// For sign-out, server switch and app dispose. [closeActiveDocumentSync] is
  /// active-tab-scoped by design (it is the doc-switch path), so those callers
  /// would otherwise leave background tabs holding open sockets — still
  /// authenticated with the credentials that were just discarded.
  void closeAllDocumentSync() {
    final active = activeTab();
    for (final tab in tabs()) {
      if (!identical(tab, active)) parkTab(tab);
    }
    closeActiveDocumentSync();
  }

  void closeActiveDocumentSync() {
    final active = activeTab();
    _syncRefetchTimer?.cancel();
    _syncRefetchTimer = null;
    _cursorTimer?.cancel();
    active.sync?.dispose();
    active.sync = null;
    final cloud = active.cloudSession;
    active.cloudSession = null;
    // C2: let the outgoing session flush + drain its outbox before closing, so a
    // doc switch / workspace change / sign-out doesn't hard-drop unacked edits.
    // Fire-and-forget — the page-switch path (_selectView) still awaits an
    // explicit drainOutbox first; the app-close hard case needs C1 (outbox
    // persistence) for a full guarantee.
    if (cloud != null) unawaited(cloud.drainAndDispose());
    if (!presenceIsEmpty()) {
      onPresenceCleared();
    }
  }

  // ── Awareness ─────────────────────────────────────────────────────────────

  /// The editor's caret moved — broadcast it (debounced) as awareness so other
  /// collaborators see this user's cursor.
  void onEditorSelection(String? blockId, int? offset) {
    _cursorTimer?.cancel();
    _cursorTimer = Timer(const Duration(milliseconds: 120), () {
      activeTab().sync?.sendCursor(blockId, offset);
    });
  }

  // ── Fault banner (B3) ─────────────────────────────────────────────────────

  /// A cloud replica hit an integrity fault it wouldn't silently absorb (red
  /// line #1). The session already self-heals with a capped re-bootstrap; here
  /// we log it and raise the banner once past the threshold. The banner UI
  /// itself (l10n + ScaffoldMessenger) is the State's, via [onFaultBanner].
  void onCloudSyncFault(String documentId, String reason, int count) {
    debugPrint(
      '[cloud-sync] integrity fault ($reason) on $documentId — #$count',
    );
    // The session auto-heals (capped re-bootstrap) for the first few consecutive
    // faults; past that it's genuinely stuck (B3). Surface it once so the user
    // knows edits may not be reaching the cloud — with a one-tap retry — instead
    // of failing silently (red line #1).
    if (count <= 3 || _syncBannerShown || !isMounted()) return;
    _syncBannerShown = true;
    onFaultBanner?.call();
  }

  /// The editor's op pipeline (controller `_send`) failed to commit a batch —
  /// most importantly a StoreCloudDocStore.appendOutbox StateError, which used to
  /// be swallowed by a bare `catchError`, silently dropping the edit from the
  /// durable outbox (red line #1: never lose an edit without a trace). Count and
  /// log it; when a cloud session is what's failing, reuse the "sync paused"
  /// banner+retry once past its threshold. In local-only mode there's nothing to
  /// re-bootstrap, so we just log/count (still better than the silent swallow).
  void onEditorFault(Object error, int count) {
    debugPrint('[editor-op] commit failed — #$count: $error');
    if (activeTab().cloudSession != null) {
      onCloudSyncFault(
        activeDocumentId() ?? 'editor',
        'editor_op_failed',
        count,
      );
    }
  }

  /// Clear the sync-paused banner (B3) — recovery succeeded, doc switched, or
  /// the user dismissed it.
  void clearSyncBanner() {
    if (!_syncBannerShown) return;
    _syncBannerShown = false;
    onHideFaultBanner?.call();
  }

  /// Retry a stuck cloud sync (B3): tear the session down and reconcile, which
  /// cold-bootstraps a fresh replica (unacked edits persist and replay).
  void retryCloudSync() {
    clearSyncBanner();
    closeActiveDocumentSync();
    reconcile();
  }

  // ── Unacked prefs (C1 legacy path) ────────────────────────────────────────

  /// Load a document's persisted unacked-diff queue (C1 crash recovery). Stored
  /// as a JSON array of base64 diffs under `cloudUnacked:<docId>`.
  List<Uint8List> loadUnacked(String key) {
    final raw = readPref(key);
    if (raw == null || raw.isEmpty) return const [];
    try {
      final list = jsonDecode(raw) as List;
      return [for (final s in list) base64.decode(s as String)];
    } catch (_) {
      return const [];
    }
  }

  void saveUnacked(String key, List<Uint8List> unacked) {
    savePref(
      key,
      unacked.isEmpty
          ? ''
          : jsonEncode([for (final b in unacked) base64.encode(b)]),
    );
  }

  // ── Op-path remote seq ────────────────────────────────────────────────────

  /// A remote (or our own, echoed) accepted update advanced the server
  /// sequence. If it is ahead of what we hold, pull the latest snapshot. Our
  /// own edits already updated `currentSeq` via their REST response, so their
  /// echo is ignored here.
  void handleRemoteSeq(String documentId, int serverSeq) {
    // When the yrs session owns this doc, op-model seq notifications are stale
    // noise — remote changes arrive as CRDT updates instead.
    if (activeTab().cloudSession?.isReady ?? false) return;
    final bootstrap = activeBootstrap();
    if (bootstrap == null ||
        bootstrap.document.id != documentId ||
        serverSeq <= bootstrap.document.currentSeq) {
      return;
    }

    _syncRefetchTimer?.cancel();
    _syncRefetchTimer = Timer(const Duration(milliseconds: 120), () {
      onRefreshBootstrap(documentId);
    });
  }

  // ── Cross-document outbox sweep ────────────────────────────────────────────

  /// After coming online, drain the durable outbox of every cloud document
  /// that has un-pushed edits — not only the one currently open.
  ///
  /// The cloud session is a singleton bound to the ACTIVE doc, and each doc's
  /// outbox is only ever read by that doc's own session. So editing pages A/B/C
  /// offline and reconnecting while viewing C pushed only C; A and B's edits sat
  /// in the on-device append-log forever, and other devices kept showing stale
  /// content with no hint (the same gap the image-blob differ closed for images,
  /// never for text). This sweep closes it: for each other cloud doc with a
  /// non-empty outbox, spin up a short-lived headless [CloudSyncSession], let it
  /// flush + ack, and dispose it.
  ///
  /// Desktop only — the cross-doc durable outbox is the FFI append-log; web's
  /// store is per-tab IndexedDB with a single active doc and no enumeration.
  /// Best-effort and bounded: a doc with nothing pending is never even connected;
  /// a fault just leaves the edits durable for the next sweep. Sequential, so a
  /// backlog can't open a burst of sockets.
  Future<void> sweepPendingOutboxes() async {
    if (isWeb() || _sweepingOutboxes) return;
    final session = this.session.session;
    final clientId = _clientId;
    if (session == null || clientId == null) return;
    _sweepingOutboxes = true;
    try {
      // Snapshot the cloud (workspace, doc) pairs up front so we don't read
      // widget state across the awaits below. The active doc is drained by its
      // own live session — skip it here.
      final activeDoc = activeDocumentId();
      final targets = <({String workspaceId, String docId})>[];
      for (final entry in this.session.viewsByWorkspace.entries) {
        for (final v in entry.value) {
          if (v.objectType != 'document' || v.objectId == activeDoc) continue;
          targets.add((workspaceId: entry.key, docId: v.objectId));
        }
      }
      for (final t in targets) {
        if (!isMounted()) break;
        // Re-check: a doc opened mid-sweep is now the active one, drained live.
        if (t.docId == activeDocumentId()) continue;
        final store = cloudDocStore(t.docId);
        if (store == null) continue;
        if (store.outboxAfter(store.cursor().pushedClock).isEmpty) {
          store.dispose();
          continue;
        }
        await _drainDocOutbox(
          t.workspaceId,
          t.docId,
          store,
          session.accessToken,
          clientId,
        );
      }
    } finally {
      _sweepingOutboxes = false;
    }
  }

  /// The headless drain step, injectable so tests can observe WHICH docs a
  /// sweep touched without opening sockets. Production runs the default
  /// (`null`) — the real short-lived session below.
  Future<void> Function(
    String workspaceId,
    String docId,
    CloudDocStore store,
  )? _sweepDrain;

  /// Test seam for [_sweepDrain]; see its doc.
  @visibleForTesting
  void debugSetSweepDrain(
    Future<void> Function(
      String workspaceId,
      String docId,
      CloudDocStore store,
    )? drain,
  ) {
    _sweepDrain = drain;
  }

  /// Push one non-active cloud doc's outbox through a short-lived headless
  /// session, then dispose it. The session owns [store] (its `dispose` releases
  /// it). Best-effort: any failure leaves the edits durable for a later sweep.
  Future<void> _drainDocOutbox(
    String workspaceId,
    String docId,
    CloudDocStore store,
    String token,
    BigInt clientId,
  ) async {
    final injected = _sweepDrain;
    if (injected != null) {
      await injected(workspaceId, docId, store);
      return;
    }
    final session = CloudSyncSession(
      // Short-lived (drain one outbox, then dispose).
      uri: () async =>
          documentSocketUri(baseUri(), workspaceId, docId, token),
      clientId: clientId,
      onReady: (_, _) {},
      onRemoteBlocks: (_) {},
      persistence: store,
    );
    try {
      session.connect();
      await session.drainOutbox();
    } catch (_) {
      // Durable append-log keeps the edits; the next online sweep retries.
    } finally {
      session.dispose(); // also disposes `store` (persistence)
    }
  }

  // ── Introspection (tests; the State's dispose path) ────────────────────────

  /// The workspace the tree-change socket currently watches (null when the
  /// socket is down).
  String? get viewsEventsWorkspaceId => _viewsEventsWorkspaceId;

  /// The device client id once learned — the shell reads it back to seed the
  /// §6 migrator; null before the first CRDT session.
  BigInt? get knownClientId => _clientId;

  @override
  void dispose() {
    _syncRefetchTimer?.cancel();
    _cursorTimer?.cancel();
    disposeViewsEvents();
    super.dispose();
  }
}
