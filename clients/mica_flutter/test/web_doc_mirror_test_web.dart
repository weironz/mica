// The web mirror reader (`openWebDocMirror`) takes a [WebIdbDocStore] — which
// holds the doc's single-writer Web Lock — only to READ the mirror. Every exit
// path (never mirrored, undecodable, success) must hand the lock back, or the
// NEXT `WebIdbDocStore.open` of the same doc (the real sync setup, right here
// in this tab) is refused and sync silently degrades to no-IndexedDB
// persistence (issue #149). Browser test: real IndexedDB + real Web Locks.
import 'dart:typed_data';

import 'dart:js_interop';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/cloud/doc_store_platform_web.dart';
import 'package:mica_flutter/cloud/web_idb_doc_store.dart';
import 'package:mica_flutter/web/yjs_interop.dart';

var _dbSeq = 0;
var _docSeq = 0;

/// Inject the yjs bundle so the two "valid mirror" cases can build a real
/// yjs document. `flutter test` serves the `test/` directory but does not
/// load `web/yjs_bundle.js` on its own — the standalone `.html` harness did.
@JS('eval')
external JSAny? _eval(JSString code);

Future<void> _ensureYjs() async {
  if (yjsAvailable) return;
  _eval(
    '''
    var s = document.createElement('script');
    s.src = 'support/yjs_bundle.js';
    document.body.appendChild(s);
  '''
        .toJS,
  );
  for (var i = 0; i < 100 && !yjsAvailable; i++) {
    await Future<void>.delayed(const Duration(milliseconds: 20));
  }
  if (!yjsAvailable) fail('yjs bundle did not load in the flutter test browser');
}

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
  setUpAll(_ensureYjs);
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
      final doc = micaYjs.newDoc();
      micaYjs.mapSet(micaYjs.getMap(doc, 'meta'), 'root', 'r'.toJS);
      final block = micaYjs.newMap();
      micaYjs.mapSet(block, 'ty', 'page'.toJS);
      micaYjs.mapSet(block, 'text', micaYjs.newText('Mirror text'));
      micaYjs.mapSet(block, 'children', micaYjs.newArray());
      micaYjs.mapSet(micaYjs.getMap(doc, 'blocks'), 'r', block);
      final seed = await openWhenFree(db, id);
      seed.save(micaYjs.encodeState(doc).toDart, 0);
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
    seed.save(micaYjs.encodeState(micaYjs.newDoc()).toDart, 0);
    await seed.flush();
    seed.dispose();
    expect(await openWebDocMirror('https://cloud.example', id, dbName: db), isNull);
    expect(await openWebDocMirror('https://cloud.example', id, dbName: db), isNull);
    final next = await openWhenFree(db, id);
    next.dispose();
  });
}
