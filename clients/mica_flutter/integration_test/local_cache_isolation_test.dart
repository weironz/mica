// Real FFI regression for issues #102 / #109 / #110 / #111, through the SAME
// Dart seams the app uses (MicaStore + the LocalOffline facade), against a
// real SQLite file.
//
//   flutter test integration_test/local_cache_isolation_test.dart -d windows
//
// These are behavioural: two origins legitimately holding the same UUID must
// not share a body/outbox/cursor/version; an account-scoped cache must not be
// readable by another account or by the legacy server-only key; clearing the
// mirror must keep un-pushed documents and quarantine legacy rows.
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:mica_flutter/local/cache_stats.dart';
import 'package:mica_flutter/local/local_offline.dart';
import '../test/support/native_build.dart';
import 'package:mica_flutter/src/rust/api/document.dart';
import 'package:mica_flutter/src/rust/api/store.dart';
import 'package:mica_flutter/src/rust/frb_generated.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(initCheckedNativeRust);
  tearDownAll(() async => RustLib.dispose());

  MicaDocument docWith(String text) => MicaDocument.fromBlocksJson(
    rootId: 'r',
    blocksJson: jsonEncode([
      {'id': 'r', 'type': 'page', 'children': ['a']},
      {'id': 'a', 'type': 'paragraph', 'text': text},
    ]),
  );

  String bodyOf(MicaDocument doc) {
    final blocks = (jsonDecode(doc.toBlocksJson()) as List).cast<Map>();
    return blocks.firstWhere((b) => b['id'] == 'a')['text'] as String;
  }

  String bodyOfData(DocData data) {
    return data.blocks.firstWhere((b) => b['id'] == 'a')['text'] as String;
  }

  test('issue #102: same doc UUID under two origins is fully isolated', () {
    final dir = Directory.systemTemp.createTempSync('mica_origin_iso');
    final store = MicaStore.open(path: '${dir.path}/s.db')!;
    addTearDown(() {
      store.dispose();
      dir.deleteSync(recursive: true);
    });
    const a = 'origin-a';
    const b = 'origin-b';
    const id = 'same-uuid';

    store.saveDoc(origin: a, docId: id, doc: docWith('A private body'));
    store.saveDoc(origin: b, docId: id, doc: docWith('B body'));

    expect(store.listDocs(origin: a), [id]);
    expect(bodyOf(store.loadDoc(origin: a, docId: id)!), 'A private body');
    expect(bodyOf(store.loadDoc(origin: b, docId: id)!), 'B body');

    // Outbox + cursor are per origin too.
    final clock = store.appendUpdate(
      origin: a,
      docId: id,
      update: docWith('A private body plus edit').encodeState(),
    );
    expect(clock, greaterThan(0));
    expect(
      store.updatesAfter(origin: a, docId: id, after: 0),
      isNotEmpty,
    );
    expect(store.updatesAfter(origin: b, docId: id, after: 0), isEmpty);
    store.setSyncCursor(
      origin: a,
      docId: id,
      cursor: SyncCursor(lastSyncedRid: 12, pushedClock: 3),
    );
    expect(store.syncCursor(origin: b, docId: id).pushedClock, 0);

    // Version history is per origin.
    expect(
      store.createLocalVersion(origin: a, docId: id, label: 'A checkpoint'),
      isNotNull,
    );
    expect(
      store.listLocalVersions(origin: b, docId: id).map((v) => v.label),
      isNot(contains('A checkpoint')),
    );

    // Deleting A's mirror must not touch B.
    store.deleteDoc(origin: a, docId: id);
    expect(store.loadDoc(origin: a, docId: id), isNull);
    expect(bodyOf(store.loadDoc(origin: b, docId: id)!), 'B body');


  });

  test('issues #109/#110/#111: account scope, incremental mirror, safe clear',
      () async {
    final dir = Directory.systemTemp.createTempSync('mica_cache_iso');
    final store = MicaStore.open(path: '${dir.path}/store.db')!;
    final local = LocalOffline(rootDirOverride: dir.path)
      ..debugSetStoreForTesting(store);
    addTearDown(() {
      store.dispose();
      dir.deleteSync(recursive: true);
    });
    const server = 'https://mica.example.com';
    final a = cloudCacheOrigin(server, 'user-a');
    final b = cloudCacheOrigin(server, 'user-b');

    const wa = (id: 'wa', name: 'Team A', position: '0000000010', role: 'editor');
    const wb = (id: 'wb', name: 'Team B', position: '0000000020', role: 'viewer');
    const va = (
      id: 'va',
      workspaceId: 'wa',
      parentId: null,
      objectId: 'doc-a',
      name: 'A page',
      position: '0000000010',
      trashed: false,
      objectType: 'document',
    );
    const vb = (
      id: 'vb',
      workspaceId: 'wb',
      parentId: null,
      objectId: 'doc-b',
      name: 'B page',
      position: '0000000010',
      trashed: false,
      objectType: 'document',
    );

    // Account A mirrors two workspaces; account B mirrors its own.
    local.mirrorCloudPageTree(
      a,
      const [wa, wb],
      const [va, vb],
      viewWorkspaceIds: const {'wa', 'wb'},
    );
    const vbForB = (
      id: 'vb-b',
      workspaceId: 'wa',
      parentId: null,
      objectId: 'doc-b',
      name: 'B page',
      position: '0000000010',
      trashed: false,
      objectType: 'document',
    );
    local.mirrorCloudPageTree(b, const [wa], const [vbForB],
        viewWorkspaceIds: const {'wa'});

    // #109: refreshing ONE workspace keeps the other's offline tree.
    local.mirrorCloudPageTree(a, const [wa, wb], const [va],
        viewWorkspaceIds: const {'wa'});
    final afterA = local.cachedCloudPageTree(a)!;
    expect(afterA.views.map((v) => v.id), contains('vb'),
        reason: 'refreshing wa must keep wb');
    // A workspace deleted server-side is cleaned out.
    local.mirrorCloudPageTree(a, const [wa], const [va],
        viewWorkspaceIds: const {'wa'});
    expect(local.cachedCloudPageTree(a)!.workspaces.map((w) => w.id), ['wa']);
    expect(local.cachedCloudPageTree(a)!.views.map((v) => v.id), ['va']);

    // #111: A's body lives only under A's account scope.
    local.cloudDocStore(a, 'doc-a')!.save(docWith('A private').encodeState(), 1);
    expect(bodyOfData(local.openCloudDocMirror(a, 'doc-a')!), 'A private');
    expect(local.openCloudDocMirror(b, 'doc-a'), isNull,
        reason: 'another account cannot read A\'s mirror');

    // Legacy pre-#111 rows (bare server URL): never visible to either account,
    // listed for explicit recovery, and untouched by a cache clear.
    final legacy = legacyCloudCacheOrigin(server);
    local.mirrorCloudPageTree(legacy, const [wa], const [va]);
    local.cloudDocStore(legacy, 'doc-a')!.save(docWith('legacy').encodeState(), 1);
    expect(local.cachedCloudPageTree(a)!.views.map((v) => v.id), ['va']);
    expect(local.quarantinedWorkspaces(servers: const [server]), isNotEmpty);
    expect(
      local.exportWorkspaceZip('wa', origin: legacy),
      isNotEmpty,
      reason: 'explicit recovery export stays possible',
    );

    // #110: a document with un-pushed edits survives a clear; a clean one goes.
    final pending = local.cloudDocStore(a, 'doc-a')!;
    final edited = MicaDocument.fromState(bytes: pending.load()!.state)!;
    final beforeEdit = edited.stateVector();
    edited.textInsert(id: 'a', at: 9, text: ' edit');
    pending.appendOutbox(edited.encodeDiffSince(stateVector: beforeEdit));
    edited.dispose();
    local.cloudDocStore(a, 'doc-b')!.save(docWith('clean copy').encodeState(), 5);

    final cleared = await local.clearMirrorCache(
      servers: const [server],
      accountId: 'user-a',
    );
    expect(cleared.retainedDocs, 1);
    expect(cleared.quarantinedOrigins, contains(legacy));
    // The pending page keeps its content, outbox, cursor and view row.
    expect(bodyOfData(local.openCloudDocMirror(a, 'doc-a')!), 'A private edit');
    expect(pending.outboxAfter(0), isNotEmpty);
    expect(local.listViews(origin: a).map((v) => v.id), contains('va'));
    // The clean doc and its page are gone; B is untouched.
    expect(local.openCloudDocMirror(a, 'doc-b'), isNull);
    expect(local.listViews(origin: a).map((v) => v.id), isNot(contains('vb')));
    expect(local.cachedCloudPageTree(b), isNotNull);
    // Legacy rows were not deleted.
    expect(local.cachedCloudPageTree(legacy), isNotNull);

  });

  test('forgetOrigin deletes only that origin\'s copy of a shared UUID',
      () async {
    final dir = Directory.systemTemp.createTempSync('mica_forget');
    final store = MicaStore.open(path: '${dir.path}/store.db')!;
    final local = LocalOffline(rootDirOverride: dir.path)
      ..debugSetStoreForTesting(store);
    addTearDown(() {
      store.dispose();
      dir.deleteSync(recursive: true);
    });
    const server = 'https://mica.example.com';
    final a = cloudCacheOrigin(server, 'user-a');
    final b = cloudCacheOrigin(server, 'user-b');
    const ws = (id: 'w', name: 'W', position: '0000000010', role: 'owner');
    const view = (
      id: 'v',
      workspaceId: 'w',
      parentId: null,
      objectId: 'dup-doc',
      name: 'Page',
      position: '0000000010',
      trashed: false,
      objectType: 'document',
    );
    local.mirrorCloudPageTree(a, const [ws], const [view]);
    local.mirrorCloudPageTree(b, const [ws], const [view]);
    local.cloudDocStore(a, 'dup-doc')!.save(docWith('A body').encodeState(), 1);
    local.cloudDocStore(b, 'dup-doc')!.save(docWith('B body').encodeState(), 1);

    local.forgetOrigin(a);

    expect(local.openCloudDocMirror(a, 'dup-doc'), isNull);
    expect(bodyOfData(local.openCloudDocMirror(b, 'dup-doc')!), 'B body',
        reason: 'forgetOrigin must pass the origin to deleteDoc');
  });

  test('issue #146: vault import rewrites relative page links to view ids',
      () async {
    final dir = Directory.systemTemp.createTempSync('mica_links');
    final store = MicaStore.open(path: '${dir.path}/store.db')!;
    final local = LocalOffline(rootDirOverride: dir.path)
      ..debugSetStoreForTesting(store);
    addTearDown(() {
      local.flush();
      store.dispose();
      dir.deleteSync(recursive: true);
    });
    final result = await local.importVaultTree([
      // A -> B is a FORWARD reference (B is created later in the plan);
      // B -> A is the backward one. Both must resolve to the new view ids.
      (path: 'A.md', bytes: utf8.encode('[to B](B.md)')),
      (path: 'B.md', bytes: utf8.encode('[to A](A.md)')),
    ], 'links-ws');
    expect(result.errors, isEmpty);
    expect(result.viewIds, hasLength(2));

    final views = local.listViews(origin: 'local');
    final a = views.singleWhere((v) => v.name == 'A');
    final b = views.singleWhere((v) => v.name == 'B');

    List<String> hrefs(String docId) {
      final data = local.openDoc(docId)!;
      final out = <String>[];
      for (final block in data.blocks) {
        final marks = (block['data'] as Map?)?['marks'];
        if (marks is List) {
          for (final mark in marks) {
            final href = mark is Map ? mark['href'] as String? : null;
            if (href != null) out.add(href);
          }
        }
      }
      return out;
    }

    expect(hrefs(a.objectId), ['mica://page/${b.id}']);
    expect(hrefs(b.objectId), ['mica://page/${a.id}']);
  });
}
