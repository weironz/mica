// P2 §7 upstream differ: the pure pending-upload queue + the content-addressed
// image-id rewrite that reconcile replays once a blob finally uploads.
@TestOn('vm')
library;

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
}
