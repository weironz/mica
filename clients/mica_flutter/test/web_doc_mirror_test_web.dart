// The web mirror reader (`openWebDocMirror`) takes a [WebIdbDocStore] — which
// holds the doc's single-writer Web Lock — only to READ the mirror. Every exit
// path (never mirrored, undecodable, success) must hand the lock back, or the
// NEXT `WebIdbDocStore.open` of the same doc (the real sync setup, right here
// in this tab) is refused and sync silently degrades to no-IndexedDB
// persistence (issue #149). Browser test: real IndexedDB + real Web Locks.
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/cloud/doc_store_platform_web.dart';
import 'package:mica_flutter/cloud/web_idb_doc_store.dart';
import 'package:mica_flutter/web/mica_doc_wasm.dart';

import 'support/doc_engine_web.dart';

var _dbSeq = 0;
var _docSeq = 0;

/// The lock release is async (dispose waits for the write-behind tail), so
/// poll like `reopen` does in the store test.
Future<WebIdbDocStore> openWhenFree(String db, String doc) async {
  for (var attempt = 0; attempt < 100; attempt++) {
    final s = await WebIdbDocStore.open(
      'https://cloud.example',
      doc,
      dbName: db,
    );
    if (s != null) return s;
    await Future<void>.delayed(const Duration(milliseconds: 10));
  }
  fail(
    'WebIdbDocStore.open never reacquired the lock — a mirror read leaked it',
  );
}

void main_() {
  setUpAll(loadBrowserDocEngine);
  test(
    'two mirror reads of a NEVER-mirrored doc leave the lock acquirable',
    () async {
      final db = 'mica-mirror-test-${_dbSeq++}';
      final doc = 'doc-${_docSeq++}';

      // First read: the doc was never mirrored → load() == null → early return.
      expect(
        await openWebDocMirror('https://cloud.example', doc, dbName: db),
        isNull,
      );
      // Second read must still reach the store (old bug: the first read's lock
      // leak made this open return null inside the helper).
      expect(
        await openWebDocMirror('https://cloud.example', doc, dbName: db),
        isNull,
      );

      // …and the real sync setup that follows must still get the writable store.
      final store = await openWhenFree(db, doc);
      store.dispose();
    },
  );

  test(
    'a mirror read of an UNDECODABLE doc leaves the lock acquirable',
    () async {
      final db = 'mica-mirror-test-${_dbSeq++}';
      final doc = 'doc-${_docSeq++}';

      // Garbage base bytes the default replay/decoders choke on → the mirror
      // helper's catch path (corrupt → behave as never-mirrored).
      final seed = await openWhenFree(db, doc);
      seed.save(Uint8List.fromList([9, 9, 9, 9, 9]), 0);
      await seed.flush();
      seed.dispose();

      expect(
        await openWebDocMirror('https://cloud.example', doc, dbName: db),
        isNull,
        reason: 'undecodable mirror reads as never-mirrored',
      );
      expect(
        await openWebDocMirror('https://cloud.example', doc, dbName: db),
        isNull,
      );

      final store = await openWhenFree(db, doc);
      store.dispose();
    },
  );

  test(
    'a valid mirror returns its blocks and releases the writer lock',
    () async {
      final db = 'mica-mirror-test-${_dbSeq++}';
      final id = 'doc-${_docSeq++}';
      final doc = MicaDocWasm.fromBlocks('r', [
        {'id': 'r', 'type': 'page', 'text': 'Mirror text', 'data': <String, dynamic>{}, 'children': <String>[]},
      ])!;
      final seed = await openWhenFree(db, id);
      seed.save(doc.encodeState(), 0);
      doc.free();
      await seed.flush();
      seed.dispose();
      final mirror = await openWebDocMirror(
        'https://cloud.example',
        id,
        dbName: db,
      );
      expect(mirror, isNotNull);
      expect(mirror!.rootBlockId, isNotEmpty);
      expect(mirror.blocks.single['text'], 'Mirror text');
      expect(
        await openWebDocMirror('https://cloud.example', id, dbName: db),
        isNotNull,
      );
      final next = await openWhenFree(db, id);
      next.dispose();
    },
  );

  test('a mirror with an empty root releases its writer lock', () async {
    final db = 'mica-mirror-test-${_dbSeq++}';
    final id = 'doc-${_docSeq++}';
    final seed = await openWhenFree(db, id);
    // The lib0 v1 empty update: a document with no root at all.
    seed.save(Uint8List.fromList(const [0, 0]), 0);
    await seed.flush();
    seed.dispose();
    expect(await openWebDocMirror('https://cloud.example', id, dbName: db), isNull);
    expect(await openWebDocMirror('https://cloud.example', id, dbName: db), isNull);
    final next = await openWhenFree(db, id);
    next.dispose();
  });
}
