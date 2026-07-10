// P2-M2: on-device persistence works end to end through FFI.
//
// Opens a local SQLite store, saves an edited document, then reopens the SAME
// db file in a fresh store and loads it back — proving offline edits survive a
// restart, and that the device identity (client id) is stable across reopen.
// This is the heart of "single-device pure-offline usable".
//
//   flutter test integration_test/frb_store_test.dart -d windows
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:mica_flutter/src/rust/api/document.dart';
import 'package:mica_flutter/src/rust/api/store.dart';
import 'package:mica_flutter/src/rust/frb_generated.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async => RustLib.init());
  tearDownAll(() async => RustLib.dispose());

  test('local store persists edits + stable identity across reopen', () {
    final dir = Directory.systemTemp.createTempSync('mica_store');
    final path = '${dir.path}/store.db';

    // First session: open, create + edit + save a document.
    final s1 = MicaStore.open(path: path)!;
    final clientId = s1.clientId();
    final deviceId = s1.deviceId();
    final doc = MicaDocument.fromBlocksJson(
      rootId: 'r',
      blocksJson: jsonEncode([
        {'id': 'r', 'type': 'page', 'children': ['a']},
        {'id': 'a', 'type': 'paragraph', 'text': 'Offline note'},
      ]),
    );
    doc.textInsert(id: 'a', at: 12, text: '!');
    s1.saveDoc(docId: 'doc1', doc: doc);
    expect(s1.listDocs(), ['doc1']);

    // Second session: reopen the SAME file in a fresh store, load it back.
    final s2 = MicaStore.open(path: path)!;
    expect(s2.clientId(), clientId, reason: 'client id stable across reopen');
    expect(s2.deviceId(), deviceId, reason: 'device id stable across reopen');

    final loaded = s2.loadDoc(docId: 'doc1');
    expect(loaded, isNotNull, reason: 'the saved doc is found after reopen');
    final blocks =
        (jsonDecode(loaded!.toBlocksJson()) as List).cast<Map<String, dynamic>>();
    final a = blocks.firstWhere((b) => b['id'] == 'a');
    expect(a['text'], 'Offline note!', reason: 'edits persisted to disk');
    expect(loaded.rootBlockId(), 'r');

    _bestEffortDelete(dir);
  });

  test('missing doc loads as null', () {
    final dir = Directory.systemTemp.createTempSync('mica_store2');
    final s = MicaStore.open(path: '${dir.path}/s.db')!;
    expect(s.loadDoc(docId: 'nope'), isNull);
    _bestEffortDelete(dir);
  });

  // P2 local-first (Phase 0): the base + append-log + sync-cursor primitives the
  // cloud path will adopt so cloud docs become offline-durable and resync-able.
  test('append-log + updates_after round-trip a doc through FFI', () {
    final dir = Directory.systemTemp.createTempSync('mica_log');
    final s = MicaStore.open(path: '${dir.path}/s.db')!;
    final cid = s.clientId();

    final doc = MicaDocument.fromBlocksJson(
      rootId: 'r',
      blocksJson: jsonEncode([
        {'id': 'r', 'type': 'page', 'children': ['a']},
        {'id': 'a', 'type': 'paragraph', 'text': 'hi'},
      ]),
    );
    s.saveDoc(docId: 'd', doc: doc);
    final base = doc.encodeState();

    // Two edits, each captured as a yrs diff and appended to the log; the clock
    // is monotonic per doc.
    var sv = doc.stateVector();
    doc.textInsert(id: 'a', at: 2, text: ' there');
    expect(
        s.appendUpdate(docId: 'd', update: doc.encodeDiffSince(stateVector: sv)),
        1);

    sv = doc.stateVector();
    doc.textInsert(id: 'a', at: 8, text: '!');
    expect(
        s.appendUpdate(docId: 'd', update: doc.encodeDiffSince(stateVector: sv)),
        2);

    // The full log, then only the tail past clock 1 (the "un-pushed outbox" query).
    final all = s.updatesAfter(docId: 'd', after: 0);
    expect(all.map((e) => e.clock).toList(), [1, 2]);
    expect(s.updatesAfter(docId: 'd', after: 1).map((e) => e.clock).toList(), [2]);

    // base + logged updates reconstruct the edited doc — the durable form works.
    final replay = MicaDocument.fromStateWithClientId(bytes: base, clientId: cid)!;
    for (final e in all) {
      replay.applyUpdate(update: e.payload);
    }
    final blocks =
        (jsonDecode(replay.toBlocksJson()) as List).cast<Map<String, dynamic>>();
    expect(blocks.firstWhere((b) => b['id'] == 'a')['text'], 'hi there!');

    _bestEffortDelete(dir);
  });

  test('sync cursor round-trips + persists across reopen', () {
    final dir = Directory.systemTemp.createTempSync('mica_cur');
    final path = '${dir.path}/s.db';
    final s1 = MicaStore.open(path: path)!;
    // Defaults to 0/0 for a doc that has never synced.
    var c = s1.syncCursor(docId: 'd');
    expect(c.lastSyncedRid, 0);
    expect(c.pushedClock, 0);
    s1.setSyncCursor(
        docId: 'd', cursor: const SyncCursor(lastSyncedRid: 42, pushedClock: 7));
    c = s1.syncCursor(docId: 'd');
    expect(c.lastSyncedRid, 42);
    expect(c.pushedClock, 7);
    // Persisted (not in-memory): survives reopening the same db file.
    final s2 = MicaStore.open(path: path)!;
    final c2 = s2.syncCursor(docId: 'd');
    expect(c2.lastSyncedRid, 42);
    expect(c2.pushedClock, 7);
    _bestEffortDelete(dir);
  });

  // P2 local-first (Phase 1b-2′): one store holds both the on-device page tree
  // (origin='local') and a cloud page tree mirrored for offline nav
  // (origin=<server URL>). Listing by origin must never leak one into the other,
  // and the origin column must be durable across a reopen. This exercises the
  // full Dart→FFI→SQLite origin round-trip the Rust unit tests can't reach.
  test('origin scopes views + workspaces through FFI, durable across reopen', () {
    final dir = Directory.systemTemp.createTempSync('mica_origin');
    final path = '${dir.path}/s.db';
    final s = MicaStore.open(path: path)!;
    const cloud = 'https://mica.example.com';

    // A default 'local' workspace always exists; the cloud origin starts empty.
    expect(
      s.listWorkspaces(origin: 'local').map((w) => w.id),
      contains('local'),
    );
    expect(s.listWorkspaces(origin: cloud), isEmpty);

    // Mirror a cloud workspace + view under the server origin, plus a local view.
    s.saveWorkspace(
      workspace: const LocalWorkspace(
        id: 'cw',
        name: 'Cloud WS',
        position: '0000000010',
        origin: cloud,
      ),
    );
    s.saveView(
      view: const LocalView(
        id: 'cv',
        workspaceId: 'cw',
        parentId: null,
        objectId: 'cd',
        name: 'Cloud Page',
        position: '0000000010',
        trashed: false,
        origin: cloud,
      ),
    );
    s.saveView(
      view: const LocalView(
        id: 'lv',
        workspaceId: 'local',
        parentId: null,
        objectId: 'ld',
        name: 'Local Page',
        position: '0000000010',
        trashed: false,
        origin: 'local',
      ),
    );

    // Isolation: each origin lists only its own rows.
    final localViews = s.listViews(origin: 'local');
    expect(localViews.map((v) => v.id), contains('lv'));
    expect(localViews.map((v) => v.id), isNot(contains('cv')));

    final cloudViews = s.listViews(origin: cloud);
    expect(cloudViews.map((v) => v.id), ['cv']);
    expect(cloudViews.single.origin, cloud);
    expect(s.listWorkspaces(origin: cloud).map((w) => w.id), ['cw']);

    // Durable: the origin column survives reopening the same db file.
    final s2 = MicaStore.open(path: path)!;
    expect(s2.listViews(origin: cloud).map((v) => v.id), ['cv']);
    expect(s2.listViews(origin: 'local').map((v) => v.id), contains('lv'));
    expect(s2.listWorkspaces(origin: cloud).map((w) => w.id), ['cw']);

    _bestEffortDelete(dir);
  });
}

// On Windows the open SQLite handle (held by the still-alive MicaStore opaque
// object) keeps the db file locked, so a recursive delete fails. Cleanup is not
// what we're testing — make it best-effort and let the OS temp reaper finish.
void _bestEffortDelete(Directory dir) {
  try {
    dir.deleteSync(recursive: true);
  } catch (_) {
    // ignore: locked db file on Windows
  }
}
