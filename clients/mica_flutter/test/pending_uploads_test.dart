// P2 §7 upstream differ: the pure pending-upload queue + the content-addressed
// image-id rewrite that reconcile replays once a blob finally uploads.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/cloud/pending_uploads.dart';

final _sha = 'a' * 64;
final _sha2 = 'b' * 64;
const _uuid = 'a1b2c3d4-1111-2222-3333-444455556666';

PendingUpload _e(String sha, {String ws = 'ws1', String doc = 'doc1', String name = 'pic.png'}) =>
    (sha: sha, workspaceId: ws, docId: doc, name: name);

Map<String, dynamic> _img(String id, String fileId) => {
      'id': id,
      'type': 'image',
      'text': '',
      'data': {'file_id': fileId, 'name': 'pic.png'},
      'children': const <String>[],
    };

void main() {
  group('PendingUploads CRUD', () {
    test('add is idempotent on (sha, workspace, doc)', () {
      final q = PendingUploads();
      expect(q.add(_e(_sha)), isTrue);
      expect(q.add(_e(_sha)), isFalse); // same triple → no dup
      expect(q.all.length, 1);
      // Same sha but a different doc is a distinct entry.
      expect(q.add(_e(_sha, doc: 'doc2')), isTrue);
      expect(q.all.length, 2);
    });

    test('remove targets the exact (sha, workspace, doc) entry', () {
      final q = PendingUploads([_e(_sha), _e(_sha2)]);
      expect(q.remove('ws1', 'doc1', _sha), isTrue);
      expect(q.remove('ws1', 'doc1', _sha), isFalse); // already gone
      expect(q.all.single.sha, _sha2);
    });

    test('forDoc filters by workspace + doc', () {
      final q = PendingUploads([
        _e(_sha, doc: 'doc1'),
        _e(_sha2, doc: 'doc2'),
        _e('c' * 64, ws: 'ws2', doc: 'doc1'),
      ]);
      final got = q.forDoc('ws1', 'doc1');
      expect(got.length, 1);
      expect(got.single.sha, _sha);
    });

    test('isEmpty / isNotEmpty reflect contents', () {
      final q = PendingUploads();
      expect(q.isEmpty, isTrue);
      q.add(_e(_sha));
      expect(q.isNotEmpty, isTrue);
    });
  });

  group('PendingUploads JSON round-trip', () {
    test('toJson → fromJson preserves entries', () {
      final q = PendingUploads([_e(_sha, name: 'a.png'), _e(_sha2, doc: 'doc2', name: 'b.png')]);
      final back = PendingUploads.fromJson(q.toJson());
      expect(back.all.length, 2);
      expect(back.all[0], (sha: _sha, workspaceId: 'ws1', docId: 'doc1', name: 'a.png'));
      expect(back.all[1].sha, _sha2);
      expect(back.all[1].docId, 'doc2');
    });

    test('null / empty / corrupt input yields an empty queue', () {
      expect(PendingUploads.fromJson(null).isEmpty, isTrue);
      expect(PendingUploads.fromJson('').isEmpty, isTrue);
      expect(PendingUploads.fromJson('not json').isEmpty, isTrue);
      expect(PendingUploads.fromJson('{"not":"a list"}').isEmpty, isTrue);
      // A list with a malformed element drops just that element.
      expect(PendingUploads.fromJson('[{"s":"$_sha"}]').isEmpty, isTrue); // missing w/d
    });
  });

  group('buildImageIdRewriteOps', () {
    test('rewrites every image referencing the placeholder, preserving data', () {
      final blocks = [
        {'id': 'p', 'type': 'paragraph', 'text': 'hi', 'data': <String, dynamic>{}, 'children': <String>[]},
        _img('img1', _sha),
        _img('img2', _sha), // same blob pasted twice → both rewritten
        _img('img3', _sha2), // a different blob → untouched
      ];
      final ops = buildImageIdRewriteOps(blocks: blocks, fromId: _sha, toId: _uuid);
      expect(ops.length, 2);
      expect(ops.map((o) => o['block_id']), ['img1', 'img2']);
      for (final o in ops) {
        expect(o['type'], 'update_block');
        final data = o['data'] as Map;
        expect(data['file_id'], _uuid);
        expect(data['name'], 'pic.png'); // sibling props preserved
      }
    });

    test('no match → no ops', () {
      final ops = buildImageIdRewriteOps(
        blocks: [_img('img1', _sha2)],
        fromId: _sha,
        toId: _uuid,
      );
      expect(ops, isEmpty);
    });
  });

  group('reconcileDocPendingUploads', () {
    final bytes = Uint8List.fromList([1, 2, 3, 4]);

    test('uploads with inferred MIME, rewrites active doc, and removes entry', () async {
      final q = PendingUploads([_e(_sha, name: 'photo.jpg')]);
      final mirrored = <String, Uint8List>{};
      final applied = <List<Map<String, dynamic>>>[];
      String? seenMime;

      final res = await reconcileDocPendingUploads(
        pending: q,
        workspaceId: 'ws1',
        docId: 'doc1',
        isSessionStillActive: () => true,
        readBlocks: () => [_img('img1', _sha)],
        loadBlob: (sha) => sha == _sha ? bytes : null,
        mirrorBlob: (id, data) => mirrored[id] = data,
        uploadImage: (data, name, mime) async {
          seenMime = mime;
          return _uuid;
        },
        applyOps: applied.add,
      );

      expect(seenMime, 'image/jpeg');
      expect(mirrored[_uuid], bytes);
      expect(applied, hasLength(1));
      expect((applied.single.single['data'] as Map)['file_id'], _uuid);
      expect(res.blocksRewritten, isTrue);
      expect(res.queueChanged, isTrue);
      expect(q.isEmpty, isTrue);
    });

    test('switching doc mid-upload keeps queue entry and skips stale applyOps', () async {
      final q = PendingUploads([_e(_sha)]);
      final uploadGate = Completer<String>();
      var active = true;
      final applied = <List<Map<String, dynamic>>>[];
      final mirrored = <String, Uint8List>{};

      final future = reconcileDocPendingUploads(
        pending: q,
        workspaceId: 'ws1',
        docId: 'doc1',
        isSessionStillActive: () => active,
        readBlocks: () => [_img('img1', _sha)],
        loadBlob: (_) => bytes,
        mirrorBlob: (id, data) => mirrored[id] = data,
        uploadImage: (_, _, _) => uploadGate.future,
        applyOps: applied.add,
      );

      // User switches pages while the 6 MB image upload is still in flight.
      active = false;
      uploadGate.complete(_uuid);
      final res = await future;

      expect(mirrored[_uuid], bytes, reason: 'global CAS mirror is still safe');
      expect(applied, isEmpty, reason: 'must never write ops into a stale session');
      expect(res.blocksRewritten, isFalse);
      expect(res.queueChanged, isFalse);
      expect(
        q.forDoc('ws1', 'doc1'),
        hasLength(1),
        reason: 'queue entry must stay until the live doc is actually rewritten',
      );
    });

    test('self-heals orphaned 64-hex sha256 placeholder even when queue is empty', () async {
      final q = PendingUploads();
      final applied = <List<Map<String, dynamic>>>[];

      final res = await reconcileDocPendingUploads(
        pending: q,
        workspaceId: 'ws1',
        docId: 'doc1',
        isSessionStillActive: () => true,
        readBlocks: () => [_img('img1', _sha), _img('img2', _uuid)],
        loadBlob: (sha) => sha == _sha ? bytes : null,
        mirrorBlob: (_, _) {},
        uploadImage: (_, _, _) async => _uuid,
        applyOps: applied.add,
      );

      expect(res.blocksRewritten, isTrue);
      expect(res.queueChanged, isFalse);
      expect(applied, hasLength(1));
      expect(applied.single.single['block_id'], 'img1');
      expect((applied.single.single['data'] as Map)['file_id'], _uuid);
    });

    test('drops stale queue entry without uploading when block was deleted', () async {
      final q = PendingUploads([_e(_sha)]);
      var uploads = 0;

      final res = await reconcileDocPendingUploads(
        pending: q,
        workspaceId: 'ws1',
        docId: 'doc1',
        isSessionStillActive: () => true,
        readBlocks: () => [_img('img2', _uuid)],
        loadBlob: (_) => bytes,
        mirrorBlob: (_, _) {},
        uploadImage: (_, _, _) async {
          uploads++;
          return _uuid;
        },
        applyOps: (_) {},
      );

      expect(uploads, 0);
      expect(res.queueChanged, isTrue);
      expect(res.blocksRewritten, isFalse);
      expect(q.isEmpty, isTrue);
    });
  });
}
