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
