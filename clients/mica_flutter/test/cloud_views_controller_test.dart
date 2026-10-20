import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/session/session_controller.dart';
import 'package:mica_flutter/views/cloud_views_controller.dart';

/// Characterization tests for [CloudViewsController], pinned against the
/// behavior the views-domain methods had as `_WorkspaceShellState` methods
/// (docs/reviews/code-optimization/OPTIMIZATION_PLAN_ROUND3.md §7, fifth step).
/// Every rule here existed in main_shell.dart before the move; the test names
/// cite the original method.
void main() {
  AuthSession session(String token) => AuthSession(
        accessToken: token,
        user: User(id: 'u', email: 'u@t.dev', displayName: 'U'),
      );

  Workspace ws(String id, {String name = 'W', String role = 'owner'}) =>
      Workspace(id: id, name: name, ownerId: 'o', role: role);

  DocumentView view(
    String id, {
    String? parentViewId,
    String objectType = 'document',
    String name = '',
    String position = '0',
    String? icon,
  }) => DocumentView(
        id: id,
        parentViewId: parentViewId,
        objectId: 'obj-$id',
        objectType: objectType,
        name: name,
        position: position,
        icon: icon,
      );

  DocumentBootstrap boot(String id) => DocumentBootstrap(
        document: DocumentRecord(id: 'doc-$id', rootBlockId: 'r', currentSeq: 0),
        view: view(id),
        snapshot: DocumentSnapshot(
          versionSeq: 1,
          schemaVersion: 1,
          payload: const {},
        ),
      );

  /// Scripted answers. Defaults are "impossible" so a test that forgets to
  /// script the call it exercises fails loudly instead of hanging.
  final calls = <String>[];
  final prefs = <String, String>{};
  List<String>? mirrored;
  var viewsAnswer = <String, ({List<DocumentView>? views, String? etag})>{};
  var createAnswer = <String, DocumentCreateResult>{};
  DocumentView? updateViewResult;
  List<DocumentView>? deleteViewResult;
  CloneReport? cloneReport;
  List<DocumentView>? restoreResult;
  String? openViewId;
  String? crdtDocId;
  bool mounted = true;
  Object? listViewsError;

  late SessionController store;
  late CloudViewsController c;

  void resetWorld() {
    calls.clear();
    prefs.clear();
    mirrored = null;
    viewsAnswer = {};
    createAnswer = {};
    updateViewResult = null;
    deleteViewResult = null;
    cloneReport = null;
    restoreResult = null;
    openViewId = null;
    crdtDocId = null;
    mounted = true;
    listViewsError = null;
    store = SessionController();
    store.signIn(session('t'), [ws('w1'), ws('w2')]);
  }

  CloudViewsController buildController({
    Future<({List<DocumentView>? views, String? etag})> Function()? fetchTree,
    Future<DocumentBootstrap> Function()? fetchBootstrap,
  }) => CloudViewsController(
        session: store,
        createDocument: (token, workspaceId, name, {parentViewId}) async {
          calls.add('createDocument:$workspaceId/$name/$parentViewId');
          return createAnswer[name] ??
              (throw StateError('no scripted createDocument for $name'));
        },
        bootstrapDocument: (token, workspaceId, documentId) async {
          calls.add('bootstrap:$workspaceId/$documentId');
          if (fetchBootstrap != null) return fetchBootstrap();
          return boot('v-$documentId');
        },
        createFolder: (token, workspaceId, name, {parentViewId}) async {
          calls.add('createFolder:$workspaceId/$name/$parentViewId');
          return view('folder-$name', objectType: 'folder', name: name);
        },
        updateView: (token, workspaceId, viewId, name, {icon}) async {
          calls.add('updateView:$viewId/$name/icon=$icon');
          return updateViewResult ?? view(viewId, name: name, icon: icon);
        },
        deleteView: (token, workspaceId, viewId) async {
          calls.add('deleteView:$viewId');
          return deleteViewResult ?? const [];
        },
        batchTrashViews: ({
          required token,
          required workspaceId,
          required viewIds,
        }) async {
          calls.add('batchTrash:$viewIds');
          return (affected: viewIds.length, skipped: const <String>[]);
        },
        cloneView: ({
          required token,
          required workspaceId,
          required viewId,
          name,
          required dryRun,
        }) async {
          calls.add('clone:$viewId/$name/dry=$dryRun');
          return cloneReport ??
              const CloneReport(
                newRootViewId: 'copy',
                newName: 'X 副本',
                documents: 1,
                folders: 0,
                dryRun: false,
              );
        },
        reorderViews: (token, workspaceId,
            {required parentViewId, required orderedViewIds}) async {
          calls.add('reorder:$parentViewId/$orderedViewIds');
        },
        restoreView: (token, workspaceId, viewId) async {
          calls.add('restore:$viewId');
          return restoreResult ?? const [];
        },
        listViewsIfChanged: (token, workspaceId, {etag}) async {
          calls.add('listViews:$workspaceId/etag=${etag ?? '-'}');
          if (fetchTree != null) return fetchTree();
          final error = listViewsError;
          if (error != null) throw error;
          return viewsAnswer[workspaceId] ?? (views: null, etag: etag);
        },
        readPref: (key) => prefs[key],
        savePref: (key, value) => prefs[key] = value,
        mirrorPageTree: (origin, workspaces, views) {
          mirrored = [
            for (final v in views)
              '${v.workspaceId}:${v.id}:${v.position}:trashed=${v.trashed}',
          ];
        },
        origin: () => 'https://srv',
        isMounted: () => mounted,
        openViewId: () => openViewId,
        crdtSessionDocumentId: () => crdtDocId,
      );

  /// Capture [CloudViewsController.onSelection] into [selections].
  void listen(CloudViewsController ctrl, List<ViewsSelection> selections) {
    ctrl.onSelection = selections.add;
  }

  setUp(() {
    resetWorld();
    c = buildController();
  });

  group('renameView (was _renameView)', () {
    test('replaces the row by id in place — not appended, order preserved',
        () async {
      store.replaceViews('w1', [
        view('a', name: 'A'),
        view('b', name: 'B'),
        view('c', name: 'C'),
      ]);
      updateViewResult = view('b', name: 'B renamed');
      final renamed = await c.renameView('t', 'w1', 'b', 'B renamed');
      expect(renamed.name, 'B renamed');
      expect(store.viewsByWorkspace['w1']!.map((v) => v.id), ['a', 'b', 'c'],
          reason: 'rename must not move the row in the sidebar');
      expect(
        store.viewsByWorkspace['w1']!.firstWhere((v) => v.id == 'b').name,
        'B renamed',
      );
    });

    test('setViewIcon passes the name through untouched (was _promptSetViewIcon)',
        () async {
      store.replaceViews('w1', [view('a', name: 'A')]);
      updateViewResult = view('a', name: 'A', icon: '😀');
      await c.setViewIcon('t', 'w1', view('a', name: 'A'), '😀');
      expect(calls, contains('updateView:a/A/icon=😀'),
          reason: 'the picker must never wipe the name by omission');
      expect(store.viewsByWorkspace['w1']!.single.icon, '😀');
    });
  });

  group('createDocument / createFolder (was _createDocument / _createFolder)',
      () {
    test('created document is appended and the bootstrap fetched after create',
        () async {
      store.replaceViews('w1', [view('a', name: 'A')]);
      createAnswer['New page'] = DocumentCreateResult(
        document: const DocumentRecord(id: 'd1', rootBlockId: 'r', currentSeq: 0),
        view: view('new', name: 'New page'),
      );
      final created = await c.createDocument('t', 'w1', 'New page');
      expect(created.view.id, 'new');
      expect(calls, [
        'createDocument:w1/New page/null',
        'bootstrap:w1/d1',
      ], reason: 'the bootstrap follows the create, in order');
      expect(
        store.viewsByWorkspace['w1']!.map((v) => v.id),
        ['a', 'new'],
        reason: 'the new view is APPENDED, at the bottom of the tree',
      );
    });

    test('createDocument under a parent passes parentViewId through',
        () async {
      createAnswer['Child'] = DocumentCreateResult(
        document: const DocumentRecord(id: 'd2', rootBlockId: 'r', currentSeq: 0),
        view: view('child', parentViewId: 'f1', name: 'Child'),
      );
      await c.createDocument('t', 'w1', 'Child', parentViewId: 'f1');
      expect(calls.first, 'createDocument:w1/Child/f1');
    });

    test('createFolder appends, mirrors THIS workspace only, returns its id',
        () async {
      store.replaceViews('w1', [view('a', name: 'A')]);
      store.replaceViews('w2', [view('z', name: 'Z')]);
      final id = await c.createFolder('t', 'w1', 'New folder');
      expect(id, 'folder-New folder');
      expect(store.viewsByWorkspace['w1']!.map((v) => v.id),
          ['a', 'folder-New folder']);
      // The mirror write is scoped: w2's view must NOT appear in it.
      expect(mirrored, isNotNull);
      expect(mirrored!, [
        'w1:a:0:trashed=false',
        'w1:folder-New folder:0:trashed=false',
      ], reason: '_cacheCloudPageTree writes one workspace, not all of memory');
    });

    test('createFolder returns null when the State died mid-flight', () async {
      mounted = false;
      final id = await c.createFolder('t', 'w1', 'New folder');
      expect(id, isNull, reason: 'the old `if (!mounted) return null` guard');
    });
  });

  group('deleteView / deleteViews (was _deleteView / _deleteViews)', () {
    test('deleteView replaces the tree with the server answer', () async {
      store.replaceViews('w1', [view('a'), view('b')]);
      deleteViewResult = [view('b')];
      await c.deleteView('t', 'w1', 'a');
      expect(store.viewsByWorkspace['w1']!.map((v) => v.id), ['b']);
    });

    test('deleteViews reloads the tree; the RELOADED tree decides if the open page survived',
        () async {
      store.replaceViews('w1', [view('a'), view('child', parentViewId: 'a')]);
      // The reload after the batch call drops both the trashed root and its
      // subtree — the open page ('child') is gone even though it was not in
      // the requested ids.
      viewsAnswer['w1'] = (views: [view('x')], etag: 'e2');
      openViewId = 'child';
      var gone = false;
      c.onOpenPageGone = () => gone = true;
      await c.deleteViews(token: 't', workspaceId: 'w1', viewIds: ['a']);
      expect(calls, contains('batchTrash:[a]'));
      expect(calls, contains('listViews:w1/etag=-'),
          reason: 'the reload after the batch request');
      expect(gone, isTrue,
          reason: 'the open page was inside the trashed subtree');
      expect(store.viewsByWorkspace['w1']!.single.id, 'x');
    });

    test('an open page that survived the reload keeps its editor', () async {
      store.replaceViews('w1', [view('a'), view('keep')]);
      viewsAnswer['w1'] = (views: [view('keep'), view('a')], etag: 'e2');
      openViewId = 'keep';
      var gone = false;
      c.onOpenPageGone = () => gone = true;
      await c.deleteViews(token: 't', workspaceId: 'w1', viewIds: ['a']);
      expect(gone, isFalse);
    });
  });

  group('reorderViews (was _reorderViews)', () {
    test('optimistic write lands immediately with step-of-ten positions',
        () async {
      store.replaceViews('w1', [
        view('a', position: '0000000010'),
        view('b', position: '0000000020'),
      ]);
      await c.reorderViews('t', 'w1', null, [view('b'), view('a')]);
      final after = store.viewsByWorkspace['w1']!;
      // The rows keep their LIST slots (`views[idx] = placed` in place) — the
      // POSITION field is what orders the sidebar visually, and that is what
      // the drag changed: b now sorts before a.
      expect(after.map((v) => '${v.id}@${v.position}'),
          ['a@0000000020', 'b@0000000010'],
          reason: 'pad((i + 1) * 10) — the SAME rule the server assigns');
      expect(calls, contains('reorder:null/[b, a]'));
    });

    test('a row not yet in the tree is ADDED by the optimistic pass', () async {
      store.replaceViews('w1', [view('a')]);
      // The create path: createDocument's replace has not landed yet.
      await c.reorderViews('t', 'w1', 'f', [view('a'), view('fresh')]);
      final after = store.viewsByWorkspace['w1']!;
      expect(after.map((v) => v.id), ['a', 'fresh']);
      expect(after.last.parentViewId, 'f');
      expect(after.last.position, '0000000020');
    });

    test('an empty sibling list fires NO server call (the original early-out)',
        () async {
      store.replaceViews('w1', [view('a')]);
      await c.reorderViews('t', 'w1', null, []);
      expect(calls, isEmpty,
          reason: 'the original `_reorderViews` returned before `_run` when '
              'the sibling list was empty');
      expect(store.viewsByWorkspace['w1']!.single.id, 'a');
    });
  });

  group('cloneView (was _cloneView)', () {
    test('dryRun is always false and the tree is reloaded after', () async {
      store.replaceViews('w1', [view('a')]);
      viewsAnswer['w1'] = (views: [view('a'), view('a-copy')], etag: 'e1');
      CloneReport? reported;
      c.onCloned = (report) => reported = report;
      await c.cloneView(
        token: 't',
        workspaceId: 'w1',
        viewId: 'a',
        name: 'a 副本',
      );
      expect(calls, contains('clone:a/a 副本/dry=false'));
      expect(reported, isNotNull,
          reason: 'the snackbar reads the server-deduped name');
      expect(store.viewsByWorkspace['w1']!.map((v) => v.id), ['a', 'a-copy']);
    });
  });

  group('ensureView (was _openElsewhereInBackgroundTab, tree half)', () {
    test('resolves from memory without any request when the tree is held',
        () async {
      store.replaceViews('w1', [view('a')]);
      final v = await c.ensureView('t', 'w1', 'a');
      expect(v?.id, 'a');
      expect(calls, isEmpty,
          reason: 'a visited workspace must not re-fetch its tree');
    });

    test('fetches the unvisited workspace\'s tree and files it', () async {
      viewsAnswer['w9'] = (views: [view('a9'), view('b9')], etag: 'e9');
      final v = await c.ensureView('t', 'w9', 'b9');
      expect(v?.id, 'b9');
      expect(store.viewsByWorkspace['w9']!.map((x) => x.id), ['a9', 'b9'],
          reason: 'the fetched tree is kept for the tab click to come');
    });

    test('an ApiException is reported and resolves null', () async {
      listViewsError = const ApiException('nope', statusCode: 500);
      String? err;
      c.onError = (m) => err = m;
      final v = await c.ensureView('t', 'w1', 'a');
      expect(v, isNull);
      expect(err, 'nope',
          reason: 'the old catch put the message on the bus, then returned');
    });
  });

  group('loadWorkspaceViews (was _loadSelectedWorkspaceViews)', () {
    test(
      'unmount during tree fetch cannot commit tree, etag or mirror',
      () async {
        store.replaceViews('w1', [view('held')]);
        prefs['viewsEtag:w1'] = 'held-etag';
        final tree = Completer<({List<DocumentView>? views, String? etag})>();
        final body = Completer<DocumentBootstrap>();
        final controller = buildController(
          fetchTree: () => tree.future,
          fetchBootstrap: () => body.future,
        );
        final selections = <ViewsSelection>[];
        listen(controller, selections);
        final loading = controller.loadWorkspaceViews('t', 'w1');
        mounted = false;
        tree.complete((views: [view('late')], etag: 'late-etag'));
        await Future<void>.delayed(Duration.zero);
        // The speculative body now has no consumer, but its error must still be
        // handled rather than reaching the application's uncaught-error zone.
        body.completeError(const ApiException('speculative body failed'));
        await loading;
        await Future<void>.delayed(Duration.zero);
        expect(store.viewsByWorkspace['w1']!.single.id, 'held');
        expect(prefs['viewsEtag:w1'], 'held-etag');
        expect(mirrored, isNull);
        expect(selections, isEmpty);
      },
    );

    test(
      'unmount during bootstrap cannot publish selection or write mirror',
      () async {
        final body = Completer<DocumentBootstrap>();
        final bootstrapStarted = Completer<void>();
        viewsAnswer['w1'] = (views: [view('fresh')], etag: 'fresh-etag');
        final controller = buildController(
          fetchBootstrap: () {
            bootstrapStarted.complete();
            return body.future;
          },
        );
        final selections = <ViewsSelection>[];
        listen(controller, selections);
        final loading = controller.loadWorkspaceViews('t', 'w1');
        await bootstrapStarted.future;
        mounted = false;
        body.complete(boot('fresh'));
        await loading;
        expect(selections, isEmpty);
        expect(mirrored, isNull);
      },
    );

    test('auto-open: remembered id wins, folders are never opened', () async {
      viewsAnswer['w1'] = (
        views: [view('folder', objectType: 'folder'), view('doc1')],
        etag: 'e1',
      );
      prefs['lastViewId:w1'] = 'doc1';
      final selections = <ViewsSelection>[];
      listen(c, selections);
      await c.loadWorkspaceViews('t', 'w1');
      final got = selections.single;
      expect(got.view!.id, 'doc1');
      expect(got.bootstrap, isNotNull,
          reason: 'a document target is bootstrapped');
    });

    test('falls back to the first OPENABLE page when the remembered id is gone',
        () async {
      viewsAnswer['w1'] = (
        views: [view('folder', objectType: 'folder'), view('doc2')],
        etag: 'e1',
      );
      prefs['lastViewId:w1'] = 'deleted-id';
      final selections = <ViewsSelection>[];
      listen(c, selections);
      await c.loadWorkspaceViews('t', 'w1');
      expect(selections.single.view!.id, 'doc2',
          reason: 'firstOpenableView skips folders');
    });

    test('a remembered FOLDER is selected but never bootstrapped', () async {
      viewsAnswer['w1'] = (
        views: [view('folder', objectType: 'folder', name: 'F'), view('doc3')],
        etag: 'e1',
      );
      prefs['lastViewId:w1'] = 'folder';
      final selections = <ViewsSelection>[];
      listen(c, selections);
      await c.loadWorkspaceViews('t', 'w1');
      // `selectedView ?? firstOpenableView` — a remembered id that resolves
      // WINS even when it is a folder; the guard only skips its BOOTSTRAP
      // (a folder has no content — opening one would 404 on its unbacked
      // object_id). The row highlights, the editor stays empty.
      final got = selections.single;
      expect(got.view!.id, 'folder');
      expect(got.bootstrap, isNull);
      expect(calls.where((s) => s.startsWith('bootstrap:')), isEmpty,
          reason: 'no bootstrap request fires for a folder target');
    });

    test('etag is sent only when a tree is already held; a 304 keeps the held tree',
        () async {
      store.replaceViews('w1', [view('held')]);
      prefs['viewsEtag:w1'] = 'e0';
      viewsAnswer['w1'] = (views: null, etag: 'e0');
      final selections = <ViewsSelection>[];
      listen(c, selections);
      await c.loadWorkspaceViews('t', 'w1');
      expect(calls, contains('listViews:w1/etag=e0'));
      expect(store.viewsByWorkspace['w1']!.single.id, 'held',
          reason: 'a 304 keeps the tree in hand, not an empty one');
      expect(selections.single.view!.id, 'held');
      expect(prefs['viewsEtag:w1'], 'e0',
          reason: 'a 304 has no tag — the old one still describes the tree');
    });

    test('a fresh tree installs its OWN etag in the same step', () async {
      store.replaceViews('w1', [view('held')]);
      prefs['viewsEtag:w1'] = 'e0';
      viewsAnswer['w1'] = (views: [view('fresh')], etag: 'e1');
      await c.loadWorkspaceViews('t', 'w1');
      expect(prefs['viewsEtag:w1'], 'e1',
          reason: '_commitTree: the tag is written with the tree it came with');
      expect(store.viewsByWorkspace['w1']!.single.id, 'fresh');
    });

    test('an etag-less response keeps the old tag (the header-less 200 case)',
        () async {
      store.replaceViews('w1', [view('held')]);
      prefs['viewsEtag:w1'] = 'e0';
      viewsAnswer['w1'] = (views: [view('fresh')], etag: null);
      await c.loadWorkspaceViews('t', 'w1');
      expect(prefs['viewsEtag:w1'], 'e0',
          reason: 'only advance the tag when the server actually sent one');
    });

    test('superseded response installs NOTHING — neither tree nor etag',
        () async {
      store.replaceViews('w1', [view('held')]);
      prefs['viewsEtag:w1'] = 'e0';
      final oldTree =
          Completer<({List<DocumentView>? views, String? etag})>();
      var answerCount = 0;
      final controller = buildController(fetchTree: () {
        answerCount++;
        return answerCount == 1
            ? oldTree.future
            : Future.value((views: [view('new')], etag: 'fresh'));
      });
      final selections = <ViewsSelection>[];
      listen(controller, selections);
      final first = controller.loadWorkspaceViews('t', 'w1');
      await controller.loadWorkspaceViews('t', 'w1');
      // Complete the stale response only AFTER the new tree and body landed.
      // A zero-duration delay with immediate answers does not model this race.
      oldTree.complete((views: [view('old')], etag: 'stale'));
      await first;
      expect(store.viewsByWorkspace['w1']!.single.id, 'new');
      expect(prefs['viewsEtag:w1'], 'fresh');
      expect(selections, hasLength(1));
      expect(selections.single.view!.id, 'new');
      expect(mirrored!.single, contains(':new:'));
    });

    test('stillCurrent=false when the switch moved on — the selection is not written',
        () async {
      viewsAnswer['w1'] = (views: [view('doc1')], etag: 'e1');
      final selections = <ViewsSelection>[];
      listen(c, selections);
      // The user switched to w2 while the load was in flight.
      store.select(ws('w2'));
      await c.loadWorkspaceViews('t', 'w1');
      final got = selections.single;
      expect(got.stillCurrent, isFalse,
          reason: 'the slower load must not open the workspace the user left');
      expect(got.view!.id, 'doc1',
          reason: 'the tree still installs; only the selection is guarded');
    });

    test('the auto-open target failing says so instead of a silent empty page',
        () async {
      viewsAnswer['w1'] = (views: [view('gone')], etag: 'e1');
      prefs['lastViewId:w1'] = 'gone';
      // Rebuild with a bootstrap that 404s.
      final failing = CloudViewsController(
        session: store,
        createDocument: (t, w, n, {parentViewId}) async =>
            throw StateError('unused'),
        bootstrapDocument: (t, w, d) async =>
            throw const ApiException('404 from server', statusCode: 404),
        createFolder: (t, w, n, {parentViewId}) async =>
            view('f', objectType: 'folder'),
        updateView: (t, w, i, n, {icon}) async => view(i, name: n),
        deleteView: (t, w, i) async => const [],
        batchTrashViews: ({
          required token,
          required workspaceId,
          required viewIds,
        }) async => (affected: 0, skipped: const <String>[]),
        cloneView: ({
          required token,
          required workspaceId,
          required viewId,
          name,
          required dryRun,
        }) async => throw StateError('unused'),
        reorderViews: (t, w,
            {required parentViewId, required orderedViewIds}) async {},
        restoreView: (t, w, i) async => const [],
        listViewsIfChanged: (t, w, {etag}) async =>
            viewsAnswer[w] ?? (views: null, etag: etag),
        readPref: (key) => prefs[key],
        savePref: (key, value) => prefs[key] = value,
        mirrorPageTree: (o, w, v) {},
        origin: () => 'https://srv',
        isMounted: () => true,
        openViewId: () => null,
        crdtSessionDocumentId: () => null,
      );
      final selections = <ViewsSelection>[];
      listen(failing, selections);
      await failing.loadWorkspaceViews('t', 'w1');
      final got = selections.single;
      expect(got.view!.id, 'gone', reason: 'the tree still shows the row');
      expect(got.bootstrap, isNull, reason: 'the body did not arrive');
      expect(got.bootstrapError, contains('404 from server'),
          reason: 'SAY SO — a silent empty page reads as data loss');
    });

    test('the REST snapshot stands down when a CRDT session owns the body',
        () async {
      viewsAnswer['w1'] = (views: [view('doc1')], etag: 'e1');
      crdtDocId = 'obj-doc1';
      final selections = <ViewsSelection>[];
      listen(c, selections);
      await c.loadWorkspaceViews('t', 'w1');
      expect(selections.single.writeBody, isFalse,
          reason: 'restSnapshotMayReplaceBody: a REST snapshot cannot merge');
    });

    test('a fresh tree is mirrored; a 304 is not (the 2.1s rule)', () async {
      viewsAnswer['w1'] = (views: [view('doc1')], etag: 'e1');
      await c.loadWorkspaceViews('t', 'w1');
      expect(mirrored, isNotNull,
          reason: 'a changed tree lands in the on-device mirror');
      expect(mirrored!.single, startsWith('w1:doc1:'));

      // Now a 304: the mirror must not be written again.
      mirrored = null;
      prefs['viewsEtag:w1'] = 'e1';
      viewsAnswer['w1'] = (views: null, etag: 'e1');
      await c.loadWorkspaceViews('t', 'w1');
      expect(mirrored, isNull,
          reason: 'a 304 means the mirror already holds this tree — '
              'writing it anyway is the whole cost');
    });
  });

  group('refreshTree (was _refreshTreeFromServer)', () {
    test('a changed tree installs and mirrors', () async {
      store.replaceViews('w1', [view('old')]);
      prefs['viewsEtag:w1'] = 'e0';
      viewsAnswer['w1'] = (views: [view('new')], etag: 'e1');
      await c.refreshTree('t', 'w1');
      expect(calls, contains('listViews:w1/etag=e0'),
          reason: 'the bell asks conditionally');
      expect(store.viewsByWorkspace['w1']!.single.id, 'new');
      expect(prefs['viewsEtag:w1'], 'e1');
      expect(mirrored, isNotNull);
    });

    test('a transient network failure is swallowed silently', () async {
      store.replaceViews('w1', [view('old')]);
      prefs['viewsEtag:w1'] = 'e0';
      listViewsError = const ApiException('boom', statusCode: 500);
      await c.refreshTree('t', 'w1');
      expect(store.viewsByWorkspace['w1']!.single.id, 'old',
          reason: 'nothing changed; the next bell retries');
      expect(prefs['viewsEtag:w1'], 'e0');
    });

    test('superseded bell answer is dropped', () async {
      store.replaceViews('w1', [view('old')]);
      prefs['viewsEtag:w1'] = 'e0';
      var answerCount = 0;
      final slow = CloudViewsController(
        session: store,
        createDocument: (t, w, n, {parentViewId}) async =>
            throw StateError('unused'),
        bootstrapDocument: (t, w, d) async => boot('v'),
        createFolder: (t, w, n, {parentViewId}) async =>
            view('f', objectType: 'folder'),
        updateView: (t, w, i, n, {icon}) async => view(i, name: n),
        deleteView: (t, w, i) async => const [],
        batchTrashViews: ({
          required token,
          required workspaceId,
          required viewIds,
        }) async => (affected: 0, skipped: const <String>[]),
        cloneView: ({
          required token,
          required workspaceId,
          required viewId,
          name,
          required dryRun,
        }) async => throw StateError('unused'),
        reorderViews: (t, w,
            {required parentViewId, required orderedViewIds}) async {},
        restoreView: (t, w, i) async => const [],
        listViewsIfChanged: (t, w, {etag}) async {
          answerCount++;
          return answerCount == 1
              ? (views: [view('stale')], etag: 'stale')
              : (views: [view('current')], etag: 'current');
        },
        readPref: (key) => prefs[key],
        savePref: (key, value) => prefs[key] = value,
        mirrorPageTree: (o, w, v) {
          mirrored = const [];
        },
        origin: () => 'https://srv',
        isMounted: () => true,
        openViewId: () => null,
        crdtSessionDocumentId: () => null,
      );
      final first = slow.refreshTree('t', 'w1');
      await Future<void>.delayed(Duration.zero);
      await slow.refreshTree('t', 'w1');
      await first;
      expect(store.viewsByWorkspace['w1']!.single.id, 'current');
      expect(prefs['viewsEtag:w1'], 'current');
    });

    test('no etag is sent when nothing is held (a cold workspace)', () async {
      viewsAnswer['w1'] = (views: [view('first')], etag: 'e1');
      await c.refreshTree('t', 'w1');
      expect(calls, contains('listViews:w1/etag=-'));
    });
  });

  group('rememberedViewOf (was _rememberedViewOf)', () {
    test('null when the workspace has no tree in memory', () {
      expect(c.rememberedViewOf('w1'), isNull);
    });

    test('remembered id wins; a stale id falls to the first openable page', () {
      store.replaceViews('w1', [
        view('folder', objectType: 'folder'),
        view('doc'),
      ]);
      prefs['lastViewId:w1'] = 'doc';
      expect(c.rememberedViewOf('w1')!.id, 'doc');

      prefs['lastViewId:w1'] = 'deleted';
      expect(c.rememberedViewOf('w1')!.id, 'doc',
          reason: 'firstOpenableView skips the folder');
    });
  });
}
