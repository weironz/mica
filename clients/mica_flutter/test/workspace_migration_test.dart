// P2 §6: the pure migration core — which blobs to reconcile, and replaying a
// local block tree as ops under the cloud root (strategy (c), no meta.root push).
@TestOn('vm')
library;

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/cloud/workspace_migration.dart';

final _sha = '0' * 64; // a valid sha256-shaped local id
final _sha2 = 'a' * 64;
const _uuid = 'a1b2c3d4-1111-2222-3333-444455556666';

Map<String, dynamic> _blk(
  String id,
  String type, {
  String text = '',
  Map<String, dynamic>? data,
  List<String> children = const [],
}) => {
  'id': id,
  'type': type,
  'text': text,
  'data': data ?? <String, dynamic>{},
  'children': children,
};

void main() {
  group('isLocalBlobId', () {
    test('sha256 hex is a local id; UUID and junk are not', () {
      expect(isLocalBlobId(_sha), isTrue);
      expect(isLocalBlobId(_uuid), isFalse);
      expect(isLocalBlobId(''), isFalse);
      expect(isLocalBlobId('0' * 63), isFalse); // too short
      expect(isLocalBlobId('Z' * 64), isFalse); // not hex
    });
  });

  group('imageBlobIds', () {
    test('collects unique sha-keyed image file_ids in first-seen order', () {
      final blocks = [
        _blk('root', 'page', children: ['p', 'i1', 'i2', 'i3', 'i4']),
        _blk('p', 'paragraph', text: 'hi'),
        _blk('i1', 'image', data: {'file_id': _sha}),
        _blk('i2', 'image', data: {'file_id': _sha2}),
        _blk('i3', 'image', data: {'file_id': _sha}), // dup → skipped
        _blk('i4', 'image', data: {'file_id': _uuid}), // already cloud → skipped
      ];
      expect(imageBlobIds(blocks), equals([_sha, _sha2]));
    });

    test('ignores non-image blocks, empty/missing file_id', () {
      final blocks = [
        _blk('p', 'paragraph', data: {'file_id': _sha}), // not an image
        _blk('i1', 'image', data: {'file_id': ''}),
        _blk('i2', 'image'), // no file_id
      ];
      expect(imageBlobIds(blocks), isEmpty);
    });
  });

  group('buildMigrationOps', () {
    test('skips local root, reparents its children onto the cloud root', () {
      final blocks = [
        _blk('lroot', 'page', text: 'Title', children: ['p1', 'p2']),
        _blk('p1', 'paragraph', text: 'one'),
        _blk('p2', 'paragraph', text: 'two'),
      ];
      final ops = buildMigrationOps(
        blocks: blocks,
        localRootId: 'lroot',
        cloudRootId: 'cloudRoot',
        idMap: const {},
      );

      // First op carries the local root's kind/text onto the cloud root.
      expect(ops.first['type'], 'update_block');
      expect(ops.first['block_id'], 'cloudRoot');
      expect(ops.first['kind'], 'page');
      expect(ops.first['text'], 'Title');

      // The local root is never inserted as a block.
      final insertedIds = ops
          .where((o) => o['type'] == 'insert_block')
          .map((o) => (o['block'] as Map)['id'])
          .toList();
      expect(insertedIds, isNot(contains('lroot')));
      expect(insertedIds, equals(['p1', 'p2']));

      // Children are parented to the cloud root at their sibling indices.
      final i1 = ops.firstWhere(
        (o) => o['type'] == 'insert_block' && (o['block'] as Map)['id'] == 'p1',
      );
      expect(i1['parent_id'], 'cloudRoot');
      expect(i1['index'], 0);
      final i2 = ops.firstWhere(
        (o) => o['type'] == 'insert_block' && (o['block'] as Map)['id'] == 'p2',
      );
      expect(i2['parent_id'], 'cloudRoot');
      expect(i2['index'], 1);
    });

    test('emits parents before children (mirror-safe DFS order)', () {
      // root → a → b → c (deep nesting)
      final blocks = [
        _blk('lroot', 'page', children: ['a']),
        _blk('a', 'list', children: ['b']),
        _blk('b', 'list', children: ['c']),
        _blk('c', 'paragraph', text: 'leaf'),
      ];
      final ops = buildMigrationOps(
        blocks: blocks,
        localRootId: 'lroot',
        cloudRootId: 'cloudRoot',
        idMap: const {},
      );
      final order = ops
          .where((o) => o['type'] == 'insert_block')
          .map((o) => (o['block'] as Map)['id'] as String)
          .toList();
      expect(order, equals(['a', 'b', 'c']));
      // 'a' parents to cloud root; 'b' parents to 'a'; 'c' parents to 'b'.
      expect(
        ops.firstWhere((o) => (o['block'] as Map?)?['id'] == 'a')['parent_id'],
        'cloudRoot',
      );
      expect(
        ops.firstWhere((o) => (o['block'] as Map?)?['id'] == 'b')['parent_id'],
        'a',
      );
      expect(
        ops.firstWhere((o) => (o['block'] as Map?)?['id'] == 'c')['parent_id'],
        'b',
      );
    });

    test('rewrites image file_id via idMap, preserves marks + other props', () {
      final blocks = [
        _blk('lroot', 'page', children: ['img']),
        _blk('img', 'image', data: {
          'file_id': _sha,
          'name': 'pic.png',
          'align': 'center',
        }),
      ];
      final ops = buildMigrationOps(
        blocks: blocks,
        localRootId: 'lroot',
        cloudRootId: 'cloudRoot',
        idMap: {_sha: _uuid},
      );
      final insert = ops.firstWhere((o) => o['type'] == 'insert_block');
      final data = (insert['block'] as Map)['data'] as Map;
      expect(data['file_id'], _uuid, reason: 'sha256 rewritten to cloud UUID');
      expect(data['name'], 'pic.png', reason: 'other props preserved');
      expect(data['align'], 'center');
    });

    test('leaves an image file_id alone when not in the idMap (dangling blob)', () {
      final blocks = [
        _blk('lroot', 'page', children: ['img']),
        _blk('img', 'image', data: {'file_id': _sha}),
      ];
      final ops = buildMigrationOps(
        blocks: blocks,
        localRootId: 'lroot',
        cloudRootId: 'cloudRoot',
        idMap: const {}, // upload failed/skipped → no mapping
      );
      final insert = ops.firstWhere((o) => o['type'] == 'insert_block');
      expect((insert['block'] as Map)['data']['file_id'], _sha);
    });

    test('empty doc (root only) yields just the root update', () {
      final blocks = [_blk('lroot', 'page')];
      final ops = buildMigrationOps(
        blocks: blocks,
        localRootId: 'lroot',
        cloudRootId: 'cloudRoot',
        idMap: const {},
      );
      expect(ops.length, 1);
      expect(ops.single['type'], 'update_block');
    });

    test('missing local root yields no ops', () {
      final ops = buildMigrationOps(
        blocks: [_blk('p', 'paragraph')],
        localRootId: 'nope',
        cloudRootId: 'cloudRoot',
        idMap: const {},
      );
      expect(ops, isEmpty);
    });
  });
}
