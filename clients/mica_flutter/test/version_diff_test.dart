import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/version_diff.dart';

/// Version-history preview showed the diff legend (green/amber/red) over a page
/// where nothing was ever tinted. The diff itself was right the whole time —
/// `EditorController.load` copies every node it is handed, and `EditorNode.copy`
/// rebuilt the node WITHOUT `diffStatus`, so the tag was erased between the
/// dialog and the renderer. Same failure shape as `DocVersion.fromJson`
/// dropping `created_by`: the data was computed and then thrown away one layer
/// down.
void main() {
  VersionContent content(List<Map<String, dynamic>> blocks) => (
    rootBlockId: 'root',
    blocks: [
      {
        'id': 'root',
        'type': 'page',
        'text': '',
        'children': [for (final b in blocks) b['id']],
      },
      ...blocks,
    ],
  );

  Map<String, dynamic> block(
    String id,
    String text, {
    String type = 'paragraph',
    Map<String, dynamic>? data,
  }) {
    // `data` is absent, not empty, on most server blocks — the diff has to cope.
    final b = <String, dynamic>{
      'id': id,
      'type': type,
      'text': text,
      'children': const <String>[],
    };
    if (data != null) b['data'] = data;
    return b;
  }

  group('EditorNode.copy', () {
    test('carries diffStatus — the tint died here', () {
      final n = EditorNode(
        id: 'b1',
        kind: 'paragraph',
        text: 'hi',
        diffStatus: 'added',
      );
      expect(n.copy().diffStatus, 'added');
    });

    test('a live-editor node still copies as untagged', () {
      final n = EditorNode(id: 'b1', kind: 'paragraph', text: 'hi');
      expect(n.copy().diffStatus, isNull);
    });
  });

  group('versionDiffNodes', () {
    test('no predecessor → every node untagged', () {
      final nodes = versionDiffNodes(
        content([block('a', 'one'), block('b', 'two')]),
        null,
      );
      expect(nodes.map((n) => n.id), ['a', 'b']);
      expect(nodes.every((n) => n.diffStatus == null), isTrue);
    });

    test('added / changed / unchanged / deleted', () {
      final prev = content([
        block('a', 'one'),
        block('gone', 'removed me'),
        block('c', 'three'),
      ]);
      final now = content([
        block('a', 'one'),
        block('b', 'brand new'),
        block('c', 'three, edited'),
      ]);

      final nodes = versionDiffNodes(now, prev);

      // The deleted block is spliced back in after the block it used to follow.
      expect(nodes.map((n) => n.id), ['a', 'gone', 'b', 'c']);
      expect(nodes.map((n) => n.diffStatus), [
        null,
        'deleted',
        'added',
        'changed',
      ]);
    });

    test('a block deleted from the very top lands before everything', () {
      final prev = content([block('gone', 'header'), block('a', 'one')]);
      final nodes = versionDiffNodes(content([block('a', 'one')]), prev);
      expect(nodes.map((n) => n.id), ['gone', 'a']);
      expect(nodes.first.diffStatus, 'deleted');
    });

    test('a changed block kind counts as changed', () {
      final prev = content([block('a', 'title')]);
      final now = content([block('a', 'title', type: 'heading')]);
      expect(versionDiffNodes(now, prev).single.diffStatus, 'changed');
    });

    test('data key order is not a difference', () {
      // jsonEncode of two equal maps written in a different order differs, which
      // would paint an untouched document entirely amber.
      final prev = content([
        block('a', 'x', data: {'level': 2, 'indent': 1}),
      ]);
      final now = content([
        block('a', 'x', data: {'indent': 1, 'level': 2}),
      ]);
      expect(versionDiffNodes(now, prev).single.diffStatus, isNull);
    });

    test('a real data change is still a difference', () {
      final prev = content([
        block('a', 'x', data: {'level': 2}),
      ]);
      final now = content([
        block('a', 'x', data: {'level': 3}),
      ]);
      expect(versionDiffNodes(now, prev).single.diffStatus, 'changed');
    });

    test('two identical versions produce no tag at all', () {
      // What the legend must key off: a predecessor exists, yet there is nothing
      // to colour, so promising green/amber/red would be a lie.
      final same = content([block('a', 'one'), block('b', 'two')]);
      final nodes = versionDiffNodes(same, same);
      expect(nodes.any((n) => n.diffStatus != null), isFalse);
    });

    test('blocks not reachable from the root are ignored', () {
      final orphaned = (
        rootBlockId: 'root',
        blocks: [
          {
            'id': 'root',
            'type': 'page',
            'text': '',
            'children': ['a'],
          },
          block('a', 'one'),
          block('detached', 'not in the tree'),
        ],
      );
      expect(versionDiffNodes(orphaned, null).map((n) => n.id), ['a']);
    });
  });
}
