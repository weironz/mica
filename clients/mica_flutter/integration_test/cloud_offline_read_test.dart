// P2 option C — Phase 1: a cloud doc mirrored to the on-device store reads
// offline. No server needed — the session seeds its replica from local storage
// and fires onReady with that content before (and independent of) any socket.
//
//   flutter test integration_test/cloud_offline_read_test.dart -d windows
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:mica_flutter/cloud/cloud_sync.dart';
import 'package:mica_flutter/cloud/store_cloud_doc_store.dart';
import 'package:mica_flutter/src/rust/api/document.dart';
import 'package:mica_flutter/src/rust/api/store.dart';
import 'package:mica_flutter/src/rust/frb_generated.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async => RustLib.init());
  tearDownAll(() async => RustLib.dispose());

  test('StoreCloudDocStore mirrors a cloud doc (bytes + cursor) via MicaStore', () {
    final dir = Directory.systemTemp.createTempSync('mica_cds');
    final store = MicaStore.open(path: '${dir.path}/s.db')!;
    final adapter = StoreCloudDocStore(store, 'cloud-uuid-1');
    expect(adapter.load(), isNull, reason: 'not mirrored yet');

    final doc = MicaDocument.fromBlocksJson(
      rootId: 'r',
      blocksJson: jsonEncode([
        {'id': 'r', 'type': 'page', 'children': ['a']},
        {'id': 'a', 'type': 'paragraph', 'text': 'cloud note'},
      ]),
    );
    adapter.save(doc.encodeState(), 42);

    final loaded = adapter.load();
    expect(loaded, isNotNull);
    expect(loaded!.cursor, 42);
    final replay = MicaDocument.fromState(bytes: loaded.state)!;
    final blocks =
        (jsonDecode(replay.toBlocksJson()) as List).cast<Map<String, dynamic>>();
    expect(blocks.firstWhere((b) => b['id'] == 'a')['text'], 'cloud note');
    _bestEffortDelete(dir);
  });

  test('StoreCloudDocStore outbox round-trips + trims monotonically via FFI (P2a)',
      () {
    final dir = Directory.systemTemp.createTempSync('mica_outbox');
    final store = MicaStore.open(path: '${dir.path}/s.db')!;
    final ob = StoreCloudDocStore(store, 'doc-ob');
    Uint8List b(int n) => Uint8List.fromList([n]);

    // Fresh doc: empty outbox, zeroed cursor.
    expect(ob.outboxAfter(0), isEmpty);
    expect(ob.cursor(), (lastSyncedRid: 0, pushedClock: 0));

    // Append three local edits — monotonic clocks 1,2,3.
    expect([ob.appendOutbox(b(1)), ob.appendOutbox(b(2)), ob.appendOutbox(b(3))],
        [1, 2, 3]);
    expect(ob.outboxAfter(0).map((e) => e.clock), [1, 2, 3]);
    expect(ob.outboxAfter(0).first.bytes, b(1));

    // Ack through clock 2, then trim the acked prefix.
    ob.advance(lastSyncedRid: 5, pushedClock: 2);
    expect(ob.cursor(), (lastSyncedRid: 5, pushedClock: 2));
    ob.trimOutboxThrough(2);
    // Only the un-pushed tail (clock 3) remains; it's still the outbox.
    expect(ob.outboxAfter(ob.cursor().pushedClock).map((e) => e.clock), [3]);

    // A new edit continues past the trim, never reused below pushed_clock.
    expect(ob.appendOutbox(b(4)), 4, reason: 'clock monotonic across trim');
    expect(ob.outboxAfter(2).map((e) => e.clock), [3, 4]);

    // advance() preserves the field not passed (pushed_clock here).
    ob.advance(lastSyncedRid: 9);
    expect(ob.cursor(), (lastSyncedRid: 9, pushedClock: 2));

    _bestEffortDelete(dir);
  });

  test('cloud session reads a mirrored doc offline — onReady fires with no server',
      () {
    final dir = Directory.systemTemp.createTempSync('mica_offr');
    final store = MicaStore.open(path: '${dir.path}/s.db')!;
    final adapter = StoreCloudDocStore(store, 'doc-x');
    // Pre-mirror a doc, as a prior online session would have.
    final doc = MicaDocument.fromBlocksJson(
      rootId: 'r',
      blocksJson: jsonEncode([
        {'id': 'r', 'type': 'page', 'children': ['a']},
        {'id': 'a', 'type': 'paragraph', 'text': 'offline readable'},
      ]),
    );
    adapter.save(doc.encodeState(), 7);

    List<Map<String, dynamic>>? seeded;
    String? seededRoot;
    final session = CloudSyncSession(
      // A dead port: the socket never connects, so onReady can ONLY come from the
      // local seed — proving offline read needs no server.
      uri: Uri.parse('ws://127.0.0.1:1/nope'),
      clientId: store.clientId(),
      onReady: (root, blocks) {
        seededRoot = root;
        seeded = blocks;
      },
      onRemoteBlocks: (_) {},
      persistence: adapter,
    );
    session.connect();

    // The seed runs synchronously inside connect(), before the socket attempt.
    expect(session.isReady, isTrue, reason: 'seeded ready with no network');
    expect(seededRoot, 'r');
    expect(seeded, isNotNull);
    expect(
      seeded!.any((b) => b['id'] == 'a' && b['text'] == 'offline readable'),
      isTrue,
      reason: 'onReady rendered the locally-mirrored content, no server',
    );
    session.dispose();
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
