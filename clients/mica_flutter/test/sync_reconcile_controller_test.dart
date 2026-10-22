// The sync domain's characterization tests — written BEFORE the State wiring,
// pinning the behaviors the doc comments in main_shell.dart record as lessons
// (each pin below names the incident it guards).
//
// The controller under test is the reconcile loop + the outbox sweep + the
// fault banner + the views-events socket: the socket half of
// _WorkspaceShellState (god-class seventh step; see
// docs/reviews/code-optimization/OPTIMIZATION_PLAN_ROUND3.md §7). Everything
// remote/stateful arrives injected; a fake tab trio stands in for the tab
// list (the controller sees tabs only through TabSockets).
import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/api/sync_client.dart';
import 'package:mica_flutter/cloud/cloud_sync_session.dart';
import 'package:mica_flutter/doc_tab.dart';
import 'package:mica_flutter/session/session_controller.dart';
import 'package:mica_flutter/sync/sync_reconcile_controller.dart';

// ── Fakes ──────────────────────────────────────────────────────────────────

/// A mutable stand-in for DocTab, seen through the TabSockets slice.
class _Tab implements TabSockets {
  _Tab({this.documentId, this.lastActivated = 0});

  @override
  String? documentId;
  @override
  int lastActivated;
  @override
  DocumentSyncClient? sync;
  @override
  CloudSyncSession? cloudSession;

  @override
  bool get isLive => sync != null;
}

/// A doc-sync client that never opens a socket — the shell-side behavior
/// under test (park/close/liveness) is bookkeeping, not protocol.
DocumentSyncClient _docSync(String documentId) => DocumentSyncClient(
      documentId: documentId,
      uri: () async => Uri.parse('wss://unused.example/doc'),
      selfName: 'test',
      onRemoteSeq: (_, _) {},
      onPresence: (_) {},
    );

/// A cloud session that never connects — safe to construct and dispose.
CloudSyncSession _cloudSession(String documentId) => CloudSyncSession(
      uri: () async => Uri.parse('wss://unused.example/doc'),
      clientId: BigInt.one,
      onReady: (_, _) {},
      onRemoteBlocks: (_) {},
    );

/// A cloud session that reports READY — only the `isReady` guard under test
/// reads it; the session is never connected.
class _ReadySession extends CloudSyncSession {
  _ReadySession()
      : super(
          uri: () async => Uri.parse('wss://unused.example/doc'),
          clientId: BigInt.one,
          onReady: (_, _) {},
          onRemoteBlocks: (_) {},
        );

  @override
  bool get isReady => true;
}

AuthSession _session() => const AuthSession(
  accessToken: 'token',
  refreshToken: 'refresh',
  user: User(id: 'u1', email: 'u@x', displayName: 'U'),
);

Workspace _ws([String id = 'w1']) => Workspace(
  id: id,
  name: 'ws-$id',
  ownerId: 'u1',
  role: 'owner',
);

DocumentBootstrap _boot(String documentId, {int currentSeq = 0}) =>
    DocumentBootstrap(
      document: DocumentRecord(
        id: documentId,
        rootBlockId: 'r',
        currentSeq: currentSeq,
      ),
      view: DocumentView(
        id: 'v-$documentId',
        parentViewId: null,
        objectId: documentId,
        objectType: 'document',
        name: 'p',
        position: '0',
      ),
      snapshot: const DocumentSnapshot(
        versionSeq: 1,
        schemaVersion: 1,
        payload: {},
      ),
    );

DocumentView _viewOf(String documentId, {String type = 'document'}) =>
    DocumentView(
      id: 'v-$documentId',
      parentViewId: null,
      objectId: documentId,
      objectType: type,
      name: 'p',
      position: '0',
    );

/// A doc store with a scriptable outbox.
class _FakeDocStore implements CloudDocStore {
  _FakeDocStore({this.hasPending = false});

  final bool hasPending;
  bool disposed = false;
  final appended = <Uint8List>[];

  @override
  ({Uint8List state, int cursor})? load() => null;

  @override
  void save(Uint8List state, int cursor) {}

  @override
  int appendOutbox(Uint8List diff) {
    appended.add(diff);
    return 1;
  }

  @override
  List<({int clock, Uint8List bytes})> outboxAfter(int pushedClock) => hasPending
      ? [(clock: 2, bytes: Uint8List.fromList([1]))]
      : const [];

  @override
  ({int lastSyncedRid, int pushedClock}) cursor() =>
      (lastSyncedRid: 0, pushedClock: 1);

  @override
  void advance({int? lastSyncedRid, int? pushedClock}) {}

  @override
  void trimOutboxThrough(int pushedClock) {}

  @override
  bool appendRemote(int rid, Uint8List update) => true;

  @override
  bool appendRemoteBatch(List<({int rid, Uint8List update})> items) => true;

  @override
  ({int local, int remote}) logSizes() => (local: 0, remote: 0);

  @override
  void compact() {}

  @override
  void dispose() => disposed = true;
}

/// Everything the controller needs from the shell, scripted per test.
class _Harness {
  _Harness({List<_Tab> tabs = const [], int activeIndex = 0})
      : tabs = tabs.isEmpty ? [_Tab()] : List.of(tabs),
        _activeIndex = activeIndex {
    controller = SyncReconcileController(
      session: sessions,
      tabs: () => this.tabs,
      activeTab: () => this.tabs[_activeIndex],
      workspaceForActiveTab: () {
        final id = this.tabs[_activeIndex].documentId == null
            ? null
            : workspaceForDoc;
        if (id == null) return sessions.selectedWorkspace;
        for (final w in sessions.workspaces) {
          if (w.id == id) return w;
        }
        return null;
      },
      activeDocumentId: () => this.tabs[_activeIndex].documentId,
      activeBootstrap: () => activeBoot,
      baseUri: () => Uri.parse('https://cloud.example/api'),
      activeIsLocal: () => isLocal,
      isMounted: () => mounted,
      isWeb: () => web,
      deviceClientId: () async => deviceClientId,
      cloudDocStore: (docId) => stores[docId],
      openWebDocStore: (docId) async => webStores[docId],
      ensureFreshSession: () async {
        freshCalls++;
      },
      readPref: (key) => prefs[key],
      savePref: (key, value) => prefs[key] = value,
      refreshTree: (workspaceId) async {
        refreshTreeCalls.add(workspaceId);
      },
      applyCloudBlocks: (documentId) => applyCloudBlocksCalls.add(documentId),
      loadComments: (documentId) async {},
      onCloudOnline: () => cloudOnlineCalls++,
      onReadyReconcileUploads: (documentId) async {
        readyUploadCalls.add(documentId);
      },
      onSyncPhase: null,
      onFaultBanner: () => faultBannerCalls++,
      onHideFaultBanner: () => hideFaultBannerCalls++,
      onRefreshBootstrap: (documentId) => refreshBootstrapCalls.add(documentId),
      onPresence: (documentId, users) {
        presenceCalls.add((documentId, users));
        presenceEmpty = false;
      },
      presenceIsEmpty: () => presenceEmpty,
      onPresenceCleared: () {
        presenceCleared++;
        presenceEmpty = true;
      },
    );
  }

  final SessionController sessions = SessionController();
  final List<_Tab> tabs;
  final int _activeIndex;
  bool mounted = true;
  bool web = false;
  bool isLocal = false;
  BigInt? deviceClientId = BigInt.from(7);
  String? workspaceForDoc = 'w1';
  DocumentBootstrap? activeBoot;
  final stores = <String, CloudDocStore>{};
  final webStores = <String, CloudDocStore>{};
  final prefs = <String, String>{};
  int freshCalls = 0;

  late final SyncReconcileController controller;

  int cloudOnlineCalls = 0;
  final readyUploadCalls = <String>[];
  int faultBannerCalls = 0;
  int hideFaultBannerCalls = 0;
  int presenceCleared = 0;
  bool presenceEmpty = true;
  final refreshTreeCalls = <String>[];
  final applyCloudBlocksCalls = <String>[];
  final refreshBootstrapCalls = <String>[];
  final presenceCalls = <(String, List<PresenceUser>)>[];

  /// Seed a live document on the active tab through one reconcile, using the
  /// controller's own session factories but a dead socket target — enough for
  /// the liveness bookkeeping these tests assert on.
  Future<void> openDocument(String documentId) async {
    tabs[_activeIndex].documentId = documentId;
    activeBoot = _boot(documentId);
    controller.reconcile();
    await Future<void>.delayed(Duration.zero);
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test('no session closes the document socket instead of connecting', () {
    final h = _Harness(tabs: [_Tab(documentId: 'd1')]);
    h.controller.reconcile();
    expect(h.tabs[0].sync, isNull);
    expect(h.tabs[0].cloudSession, isNull);
  });

  test('a live document opens a doc socket and a cloud session', () async {
    final h = _Harness(tabs: [_Tab(documentId: 'd1')]);
    h.sessions.restore(_session(), [_ws()], _ws(), const {});
    h.controller.reconcile();
    await Future<void>.delayed(Duration.zero);
    expect(h.tabs[0].sync, isNotNull,
        reason: 'the doc socket follows the open document');
    expect(h.tabs[0].cloudSession, isNotNull,
        reason: 'the yrs session is set up for the doc');
    expect(h.tabs[0].cloudSession!.isReady, isFalse,
        reason: 'not connected to anything real');
  });

  test('already on this document reconnects nothing', () async {
    final h = _Harness(tabs: [_Tab(documentId: 'd1')]);
    h.sessions.restore(_session(), [_ws()], _ws(), const {});
    await h.openDocument('d1');
    final existingSync = h.tabs[0].sync;
    final existingCloud = h.tabs[0].cloudSession;

    h.controller.reconcile();

    expect(h.tabs[0].sync, same(existingSync));
    expect(h.tabs[0].cloudSession, same(existingCloud),
        reason: 'the "already on this document" guard must not rebuild');
  });

  test('switching documents drains the outgoing cloud session', () async {
    final h = _Harness(tabs: [_Tab(documentId: 'd1')]);
    h.sessions.restore(_session(), [_ws()], _ws(), const {});
    await h.openDocument('d1');
    final first = h.tabs[0].cloudSession!;

    await h.openDocument('d2');

    expect(h.tabs[0].cloudSession, isNot(same(first)),
        reason: 'a fresh session is built for the new doc');
    expect(first.isReady, isFalse, reason: 'the old one is torn down');
  });

  test('a tab whose workspace cannot be resolved closes its socket', () {
    final h = _Harness(tabs: [_Tab(documentId: 'd1')]);
    h.sessions.restore(_session(), [_ws()], _ws(), const {});
    // Stamped with a workspace we are no longer a member of.
    h.workspaceForDoc = 'gone';
    h.activeBoot = _boot('d1');
    h.controller.reconcile();
    expect(h.tabs[0].sync, isNull,
        reason: 'a workspace-less tab must not open a socket into the wrong '
            'workspace — the server rejects workspace-scoped doc lookups');
  });

  test('reconcile clears the presence row once per document change', () async {
    final h = _Harness(tabs: [_Tab(documentId: 'd1')]);
    h.sessions.restore(_session(), [_ws()], _ws(), const {});
    await h.openDocument('d1');
    // The unconditional clear that follows connecting a new doc socket.
    expect(h.presenceCleared, 1);
    h.controller.closeActiveDocumentSync();
    // The close path's clear is conditional on a non-empty row — already
    // emptied above, so no second clear fires.
    expect(h.presenceCleared, 1);
  });

  test('parkTab keeps the doc id but drops both sockets', () async {
    final h = _Harness(tabs: [_Tab(documentId: 'd1')]);
    h.sessions.restore(_session(), [_ws()], _ws(), const {});
    await h.openDocument('d1');
    h.controller.parkTab(h.tabs[0]);
    expect(h.tabs[0].sync, isNull);
    expect(h.tabs[0].cloudSession, isNull);
    expect(h.tabs[0].documentId, 'd1',
        reason: 'the tab keeps rendering; only its sockets are parked');
  });

  test('the sync cap parks least-recently-activated live tabs first', () async {
    final tabs = [
      _Tab(documentId: 'd0', lastActivated: 10), // active
      _Tab(documentId: 'd1', lastActivated: 1), // oldest
      _Tab(documentId: 'd2', lastActivated: 5),
      _Tab(documentId: 'd3', lastActivated: 3),
      _Tab(documentId: 'd4', lastActivated: 7),
    ];
    final h = _Harness(tabs: tabs, activeIndex: 0);
    // Real clients (never connected — dispose is safe without a socket).
    for (final t in tabs) {
      t.sync = _docSync(t.documentId!);
    }
    // 4 live background tabs compete for 2 slots (cap 3 minus the active
    // one): the two OLDEST activations park.
    h.controller.enforceSyncCap();
    expect(tabs[1].sync, isNull, reason: 'LRU (activated 1) parks first');
    expect(tabs[3].sync, isNull, reason: 'next LRU (activated 3) parks');
    expect(tabs[2].sync, isNotNull,
        reason: 'newer (activated 5) stays');
    expect(tabs[4].sync, isNotNull,
        reason: 'newest background (activated 7) stays');
    expect(tabs[0].sync, isNotNull,
        reason: 'the ACTIVE tab is never parked');
  });

  test('closeAllDocumentSync parks background tabs and closes the active one',
      () async {
    final tabs = [
      _Tab(documentId: 'd0'),
      _Tab(documentId: 'd1'),
    ];
    final h = _Harness(tabs: tabs, activeIndex: 0);
    h.sessions.restore(_session(), [_ws()], _ws(), const {});
    for (final t in tabs) {
      t.sync = _docSync(t.documentId!);
      t.cloudSession = _cloudSession(t.documentId!);
    }
    h.controller.closeAllDocumentSync();
    expect(tabs.every((t) => t.sync == null), isTrue);
    expect(tabs.every((t) => t.cloudSession == null), isTrue);
  });

  group('views events socket', () {
    test('signs in and opens the socket for the selected workspace', () {
      final h = _Harness();
      h.sessions.restore(_session(), [_ws()], _ws(), const {});
      h.controller.reconcileViewsEvents();
      expect(h.controller.viewsEventsWorkspaceId, 'w1');
    });

    test('the same workspace keeps the socket (no reconnect)', () {
      final h = _Harness();
      h.sessions.restore(_session(), [_ws()], _ws(), const {});
      h.controller.reconcileViewsEvents();
      final first = h.controller.viewsEventsWorkspaceId;
      h.controller.reconcileViewsEvents();
      expect(h.controller.viewsEventsWorkspaceId, same(first),
          reason: 'the idempotence guard: same workspace, same socket');
    });

    test('switching workspaces re-points the socket', () {
      final h = _Harness();
      h.sessions.restore(
        _session(),
        [_ws('w1'), _ws('w2')],
        _ws('w1'),
        const {},
      );
      h.controller.reconcileViewsEvents();
      expect(h.controller.viewsEventsWorkspaceId, 'w1');
      h.sessions.select(_ws('w2'));
      h.controller.reconcileViewsEvents();
      expect(h.controller.viewsEventsWorkspaceId, 'w2');
    });

    test('no session or a local world disposes the socket', () {
      final h = _Harness();
      h.sessions.restore(_session(), [_ws()], _ws(), const {});
      h.controller.reconcileViewsEvents();
      expect(h.controller.viewsEventsWorkspaceId, 'w1');
      h.isLocal = true;
      h.controller.reconcileViewsEvents();
      expect(h.controller.viewsEventsWorkspaceId, isNull,
          reason: 'a local world has no cloud tree to watch');
    });

    test('disposeViewsEvents tears the socket down for sign-out', () {
      final h = _Harness();
      h.sessions.restore(_session(), [_ws()], _ws(), const {});
      h.controller.reconcileViewsEvents();
      h.controller.disposeViewsEvents();
      expect(h.controller.viewsEventsWorkspaceId, isNull);
    });
  });

  group('the fault banner', () {
    test('faults 1..3 are absorbed; the 4th raises the banner once', () {
      final h = _Harness();
      for (var i = 1; i <= 3; i++) {
        h.controller.onCloudSyncFault('d1', 'push_rejected', i);
      }
      expect(h.faultBannerCalls, 0,
          reason: 'the session self-heals for the first few faults');
      h.controller.onCloudSyncFault('d1', 'push_rejected', 4);
      expect(h.faultBannerCalls, 1);
      h.controller.onCloudSyncFault('d1', 'push_rejected', 5);
      expect(h.faultBannerCalls, 1,
          reason: 'B3: the banner shows once per stuck episode');
    });

    test('an unmounted shell never raises the banner', () {
      final h = _Harness()..mounted = false;
      h.controller.onCloudSyncFault('d1', 'x', 4);
      expect(h.faultBannerCalls, 0);
    });

    test('clearSyncBanner is a no-op when no banner is up', () {
      final h = _Harness();
      h.controller.clearSyncBanner();
      expect(h.hideFaultBannerCalls, 0);
    });

    test('clearSyncBanner reports hiding exactly once per episode', () {
      final h = _Harness();
      h.controller.onCloudSyncFault('d1', 'x', 4);
      h.controller.clearSyncBanner();
      h.controller.clearSyncBanner();
      expect(h.hideFaultBannerCalls, 1);
    });

    test('an editor fault with a live cloud session routes to the same banner',
        () {
      final h = _Harness(tabs: [_Tab(documentId: 'd1')]);
      h.tabs[0].cloudSession = _cloudSession('d1');
      h.controller.onEditorFault(StateError('outbox append failed'), 4);
      expect(h.faultBannerCalls, 1);
    });

    test('an editor fault with no cloud session only logs', () {
      final h = _Harness();
      h.controller.onEditorFault(StateError('x'), 9);
      expect(h.faultBannerCalls, 0,
          reason: 'local-only mode: nothing to re-bootstrap, log only');
    });
  });

  group('remote seq (op path)', () {
    test('an op-model seq while the yrs session owns the doc is ignored', () {
      final h = _Harness(tabs: [_Tab(documentId: 'd1')]);
      // A READY cloud session means CRDT updates arrive; the op path is stale.
      h.tabs[0].cloudSession = _ReadySession();
      h.activeBoot = _boot('d1');
      h.controller.handleRemoteSeq('d1', 10);
      expect(h.refreshBootstrapCalls, isEmpty,
          reason: 'yrs owns the doc → op seq notifications are stale noise');
    });

    test('a burst of seq bumps collapses into ONE delayed refetch',
        () async {
      final h = _Harness(tabs: [_Tab(documentId: 'd1')]);
      h.sessions.restore(_session(), [_ws()], _ws(), const {});
      h.activeBoot = _boot('d1');
      h.controller.handleRemoteSeq('d1', 5);
      h.controller.handleRemoteSeq('d1', 6);
      h.controller.handleRemoteSeq('d1', 7);
      await Future<void>.delayed(const Duration(milliseconds: 250));
      expect(h.refreshBootstrapCalls, ['d1']);
    });

    test('the debounce holds the refetch for its 120ms window', () async {
      final h = _Harness(tabs: [_Tab(documentId: 'd1')]);
      h.activeBoot = _boot('d1');
      h.controller.handleRemoteSeq('d1', 5);
      await Future<void>.delayed(const Duration(milliseconds: 40));
      expect(h.refreshBootstrapCalls, isEmpty,
          reason: 'the 120ms debounce absorbs a burst before firing');
    });

    test('a seq at or below currentSeq is our own echo', () async {
      final h = _Harness(tabs: [_Tab(documentId: 'd1')]);
      h.activeBoot = _boot('d1', currentSeq: 5);
      h.controller.handleRemoteSeq('d1', 5);
      await Future<void>.delayed(const Duration(milliseconds: 250));
      expect(h.refreshBootstrapCalls, isEmpty,
          reason: 'our own edits already advanced currentSeq; echo ignored');
    });

    test('a seq for a different document is ignored', () async {
      final h = _Harness(tabs: [_Tab(documentId: 'd1')]);
      h.activeBoot = _boot('d1');
      h.controller.handleRemoteSeq('other', 5);
      await Future<void>.delayed(const Duration(milliseconds: 250));
      expect(h.refreshBootstrapCalls, isEmpty);
    });

    test('no bootstrap at all is ignored', () async {
      final h = _Harness(tabs: [_Tab()]);
      h.controller.handleRemoteSeq('d1', 5);
      await Future<void>.delayed(const Duration(milliseconds: 250));
      expect(h.refreshBootstrapCalls, isEmpty);
    });
  });

  group('the cross-document outbox sweep', () {
    test('sweeps every non-active document with a pending outbox', () async {
      final h = _Harness(tabs: [_Tab(documentId: 'd0')]);
      h.sessions.restore(_session(), [_ws()], _ws(), const {});
      // Learn the device client id the way the original did: through the
      // active doc's own CRDT session (the sweep reads it from there).
      await h.openDocument('d0');
      h.sessions.replaceViews('w1', [
        _viewOf('d1'),
        _viewOf('d0'),
        _viewOf('d2'),
        _viewOf('f1', type: 'folder'),
      ]);
      h.stores['d1'] = _FakeDocStore(hasPending: true);
      h.stores['d2'] = _FakeDocStore(hasPending: true);
      final swept = <String>[];
      h.controller.debugSetSweepDrain(
        (workspaceId, docId, store) async => swept.add(docId),
      );
      await h.controller.sweepPendingOutboxes();
      expect(swept, containsAll(['d1', 'd2']));
      expect(swept, isNot(contains('d0')),
          reason: 'the active doc is drained by its own live session');
      expect(swept, isNot(contains('f1')),
          reason: 'folders have no document to drain');
    });

    test('a store with nothing pending is never connected', () async {
      final h = _Harness(tabs: [_Tab(documentId: 'd0')]);
      h.sessions.restore(_session(), [_ws()], _ws(), const {});
      await h.openDocument('d0'); // learn the client id
      h.sessions.replaceViews('w1', [_viewOf('d1')]);
      h.stores['d1'] = _FakeDocStore(hasPending: false);
      final store = h.stores['d1'] as _FakeDocStore;
      h.controller.debugSetSweepDrain(
        (workspaceId, docId, s) async => fail('never connected'),
      );
      await h.controller.sweepPendingOutboxes();
      expect(store.disposed, isTrue,
          reason: 'a doc with nothing pending is never even connected');
    });

    test('is re-entrancy guarded (no double sweep)', () async {
      final h = _Harness(tabs: [_Tab(documentId: 'd0')]);
      h.sessions.restore(_session(), [_ws()], _ws(), const {});
      await h.openDocument('d0'); // learn the client id
      h.sessions.replaceViews('w1', [_viewOf('d1')]);
      h.stores['d1'] = _FakeDocStore(hasPending: true);
      var inFlight = 0;
      var maxInFlight = 0;
      h.controller.debugSetSweepDrain((workspaceId, docId, store) async {
        inFlight++;
        if (inFlight > maxInFlight) maxInFlight = inFlight;
        await Future<void>.delayed(const Duration(milliseconds: 10));
        inFlight--;
      });
      final a = h.controller.sweepPendingOutboxes();
      final b = h.controller.sweepPendingOutboxes();
      await Future.wait([a, b]);
      expect(maxInFlight, 1,
          reason: 'the sweep guard admits one cross-document drain at a time');
    });

    test('web never sweeps', () async {
      final h = _Harness(tabs: [_Tab(documentId: 'd0')])..web = true;
      h.sessions.restore(_session(), [_ws()], _ws(), const {});
      h.sessions.replaceViews('w1', [_viewOf('d1')]);
      h.stores['d1'] = _FakeDocStore(hasPending: true);
      h.controller.debugSetSweepDrain(
        (workspaceId, docId, store) async => fail('web never sweeps'),
      );
      await h.controller.sweepPendingOutboxes();
    });

    test('no client id yet means no sweep (nothing CRDT-side ever opened)',
        () async {
      final h = _Harness(tabs: [_Tab(documentId: 'd0')])
        ..deviceClientId = null;
      h.sessions.restore(_session(), [_ws()], _ws(), const {});
      h.sessions.replaceViews('w1', [_viewOf('d1')]);
      h.stores['d1'] = _FakeDocStore(hasPending: true);
      h.controller.debugSetSweepDrain(
        (workspaceId, docId, store) async => fail('no client id, no sweep'),
      );
      await h.controller.sweepPendingOutboxes();
    });

    test('a mid-sweep activation of a doc skips it (live session owns it)',
        () async {
      final h = _Harness(tabs: [_Tab(documentId: 'd0')]);
      h.sessions.restore(_session(), [_ws()], _ws(), const {});
      await h.openDocument('d0'); // learn the client id
      h.sessions.replaceViews('w1', [_viewOf('d1'), _viewOf('d2')]);
      h.stores['d1'] = _FakeDocStore(hasPending: true);
      h.stores['d2'] = _FakeDocStore(hasPending: true);
      final swept = <String>[];
      h.controller.debugSetSweepDrain((workspaceId, docId, store) async {
        // d1's drain flips the active tab onto d1 mid-sweep.
        if (docId == 'd1') {
          h.tabs[0].documentId = 'd2';
        }
        swept.add(docId);
      });
      await h.controller.sweepPendingOutboxes();
      expect(swept, isNot(contains('d2')),
          reason: 'a doc opened mid-sweep is now the active one, drained live');
    });
  });

  group('unacked prefs (legacy path)', () {
    test('loadUnacked parses the base64 JSON queue', () {
      final h = _Harness();
      h.prefs['cloudUnacked:d1'] = jsonEncode([
        base64.encode([1, 2]),
        base64.encode([3]),
      ]);
      final loaded = h.controller.loadUnacked('cloudUnacked:d1');
      expect(loaded.length, 2);
      expect(loaded[0], [1, 2]);
      expect(loaded[1], [3]);
    });

    test('loadUnacked answers empty on absent or corrupt values', () {
      final h = _Harness();
      expect(h.controller.loadUnacked('missing'), isEmpty);
      h.prefs['bad'] = 'not json';
      expect(h.controller.loadUnacked('bad'), isEmpty);
    });

    test('saveUnacked round-trips and clears on empty', () {
      final h = _Harness();
      h.controller.saveUnacked('k', [Uint8List.fromList([9])]);
      expect(h.prefs['k'], jsonEncode([base64.encode([9])]));
      h.controller.saveUnacked('k', const []);
      expect(h.prefs['k'], '');
    });

    test('a legacy prefs queue migrates into the store, then the key clears',
        () async {
      final h = _Harness(tabs: [_Tab(documentId: 'd1')]);
      h.sessions.restore(_session(), [_ws()], _ws(), const {});
      final store = _FakeDocStore();
      h.stores['d1'] = store;
      h.prefs['cloudUnacked:d1'] = jsonEncode([base64.encode([9])]);
      h.controller.reconcile();
      await Future<void>.delayed(Duration.zero);
      expect(store.appended.length, 1,
          reason: 'legacy prefs entries fold into the durable append-log');
      expect(h.prefs['cloudUnacked:d1'], '',
          reason: 'append THEN delete; the reverse drops in-flight edits');
    });
  });

  test('dispose cancels the timers and the views-events socket', () async {
    final h = _Harness(tabs: [_Tab(documentId: 'd1')]);
    h.sessions.restore(_session(), [_ws()], _ws(), const {});
    h.controller.reconcileViewsEvents();
    await h.openDocument('d1');
    h.controller.dispose();
    expect(h.controller.viewsEventsWorkspaceId, isNull);
  });
}
