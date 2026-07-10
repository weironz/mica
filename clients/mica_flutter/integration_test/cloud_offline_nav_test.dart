// P1c offline read: the full mirror → "restart" → read cycle through real FFI.
//
// Simulates an online cloud session that mirrored a page tree + a document to
// the on-device store, then a FRESH LocalOffline instance (a process restart)
// reading it all back offline — the path _applyOfflineCloudNav / _selectView
// drive when the server is unreachable. Crucially it proves the store is
// readable in a fresh instance once opened via deviceClientId() (the fix for the
// "store never opened on cold offline restart" bug).
//
//   flutter test integration_test/cloud_offline_nav_test.dart -d windows
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:mica_flutter/local/local_offline.dart';
import 'package:mica_flutter/src/rust/api/document.dart';
import 'package:mica_flutter/src/rust/frb_generated.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async => RustLib.init());
  tearDownAll(() async => RustLib.dispose());

  test('cloud page tree + doc mirror survive a fresh LocalOffline (offline read)',
      () async {
    final dir = Directory.systemTemp.createTempSync('mica_offnav');
    const server = 'https://mica.example.com';

    // ── "online" session: open the store, mirror the page tree + a doc ────────
    final online = LocalOffline(rootDirOverride: dir.path);
    await online.open();

    const ws = (id: 'w1', name: 'Cloud WS', position: '0000000010', role: 'editor');
    const view = (
      id: 'v1',
      workspaceId: 'w1',
      parentId: null,
      objectId: 'doc-uuid-1',
      name: 'Cloud Page',
      position: '0000000010',
      trashed: false,
    );
    online.mirrorCloudPageTree(server, const [ws], const [view]);

    // The cloud session's write-through mirrors the doc replica keyed by its UUID.
    final doc = MicaDocument.fromBlocksJson(
      rootId: 'r',
      blocksJson: jsonEncode([
        {'id': 'r', 'type': 'page', 'children': ['a']},
        {'id': 'a', 'type': 'paragraph', 'text': 'Offline body'},
      ]),
    );
    online.cloudDocStore('doc-uuid-1')!.save(doc.encodeState(), 7);

    // ── "restart": a FRESH LocalOffline over the same data dir ────────────────
    final offline = LocalOffline(rootDirOverride: dir.path);
    // The cloud cold-start path opens the store lazily via deviceClientId() —
    // without this the mirror reads as empty (the bug this test guards).
    await offline.deviceClientId();

    // Page tree reads back, origin-scoped.
    final cache = offline.cachedCloudPageTree(server);
    expect(cache, isNotNull, reason: 'mirror survives a fresh instance');
    expect(cache!.workspaces.map((w) => w.id), ['w1']);
    expect(cache.views.single.objectId, 'doc-uuid-1');
    expect(cache.views.single.name, 'Cloud Page');
    // A different origin sees nothing (isolation holds across the restart).
    expect(offline.cachedCloudPageTree('https://other.example.com'), isNull);

    // The doc mirror opens synchronously with the correct root + blocks — the
    // data _offlineCloudBootstrap turns into a complete DocumentBootstrap.
    final opened = offline.openCloudDocMirror('doc-uuid-1');
    expect(opened, isNotNull, reason: 'a previously-synced doc opens offline');
    expect(opened!.rootBlockId, 'r');
    final body = opened.blocks.firstWhere((b) => b['id'] == 'a');
    expect(body['text'], 'Offline body');

    // A never-synced doc has no mirror → null (tree lists it, opening is empty).
    expect(offline.openCloudDocMirror('never-synced'), isNull);

    _bestEffortDelete(dir);
  });

  test('detachCloudWorkspace forks an independent local copy (P3f)', () async {
    final dir = Directory.systemTemp.createTempSync('mica_detach');
    const server = 'https://mica.example.com';
    final local = LocalOffline(rootDirOverride: dir.path);
    await local.open();

    // Mirror a cloud workspace: two pages (parent + child), one with content.
    const ws = (id: 'cw', name: 'Team', position: '0000000010', role: 'editor');
    const parent = (
      id: 'v1',
      workspaceId: 'cw',
      parentId: null,
      objectId: 'doc-a',
      name: 'Parent',
      position: '0000000010',
      trashed: false,
    );
    const child = (
      id: 'v2',
      workspaceId: 'cw',
      parentId: 'v1',
      objectId: 'doc-b', // never opened online → no mirror content
      name: 'Child',
      position: '0000000010',
      trashed: false,
    );
    local.mirrorCloudPageTree(server, const [ws], const [parent, child]);
    final doc = MicaDocument.fromBlocksJson(
      rootId: 'r',
      blocksJson: jsonEncode([
        {'id': 'r', 'type': 'page', 'children': ['a']},
        {'id': 'a', 'type': 'paragraph', 'text': 'team notes'},
      ]),
    );
    local.cloudDocStore('doc-a')!.save(doc.encodeState(), 3);

    final result = local.detachCloudWorkspace(server, 'cw', 'Team');
    expect(result, isNotNull);
    expect(result!.docs, 1, reason: 'one mirrored doc had content');

    // The fork: a NEW local workspace, role owner, both pages present with the
    // parent link remapped, and FRESH doc ids (independence).
    final forkWs = local
        .listWorkspaces()
        .where((w) => w.id == result.workspaceId)
        .single;
    expect(forkWs.role, 'owner');
    final forkViews = [
      for (final v in local.listViews())
        if (v.workspaceId == result.workspaceId) v,
    ];
    expect(forkViews.length, 2);
    final forkParent = forkViews.singleWhere((v) => v.name == 'Parent');
    final forkChild = forkViews.singleWhere((v) => v.name == 'Child');
    expect(forkChild.parentId, forkParent.id, reason: 'tree remapped');
    expect(forkParent.objectId, isNot('doc-a'), reason: 'fresh doc id');

    // Content copied; the copy is INDEPENDENT of the mirror.
    final copied = local.openCloudDocMirror(forkParent.objectId);
    expect(copied, isNotNull);
    expect(copied!.blocks.any((b) => b['text'] == 'team notes'), isTrue);
    // The cloud mirror rows are untouched (the cloud original keeps working).
    expect(
      local.cachedCloudPageTree(server)!.views.map((v) => v.id),
      containsAll(['v1', 'v2']),
    );

    _bestEffortDelete(dir);
  });
}

void _bestEffortDelete(Directory dir) {
  try {
    dir.deleteSync(recursive: true);
  } catch (_) {
    // ignore: locked db file on Windows
  }
}
