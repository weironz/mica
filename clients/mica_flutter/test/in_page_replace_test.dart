import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/model.dart';

// The in-page find bar's Replace / Replace-all. Both go through the controller's
// single edit path (`replaceRange` / `replaceAll` → `update_block`), so these
// lock in that a splice keeps text, inline marks, and the op count correct — no
// second editing representation.
EditorController _doc(List<EditorNode> nodes) {
  final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
  c.load(nodes);
  return c;
}

void main() {
  group('replaceRange', () {
    test('replaces one range and lands the caret after the insert', () {
      final c = _doc([EditorNode(id: 'a', kind: 'paragraph', text: 'hello world')]);
      expect(c.replaceRange(0, 6, 11, 'there'), isTrue);
      expect(c.nodes.single.text, 'hello there');
      expect(c.selection!.isCollapsed, isTrue);
      expect(c.selection!.focus.offset, 11); // 6 + 'there'.length
    });

    test('out-of-range request is refused (stale match)', () {
      final c = _doc([EditorNode(id: 'a', kind: 'paragraph', text: 'hi')]);
      expect(c.replaceRange(0, 5, 9, 'x'), isFalse);
      expect(c.nodes.single.text, 'hi');
    });

    test('inline marks ride the splice', () {
      // "ab CD ef" with bold over "CD"; replacing "ab" (shorter) shifts the mark.
      final c = _doc([
        EditorNode(
          id: 'a',
          kind: 'paragraph',
          text: 'ab CD ef',
          data: {'marks': marksToJson([Mark(3, 5, 'bold')])},
        ),
      ]);
      expect(c.replaceRange(0, 0, 2, 'X'), isTrue);
      expect(c.nodes.single.text, 'X CD ef');
      final m = marksFromData(c.nodes.single.data).single;
      // "CD" moved left by one (2 → 1 shorter prefix): 3→2, 5→4.
      expect((m.start, m.end, m.type), (2, 4, 'bold'));
    });
  });

  group('replaceAll', () {
    test('replaces every case-insensitive match across nodes, one batch',
        () async {
      var opBatches = 0;
      final c = EditorController(
        rootBlockId: 'root',
        onOps: (_) async => opBatches++,
      );
      c.load([
        EditorNode(id: 'a', kind: 'paragraph', text: 'foo Foo foo'),
        EditorNode(id: 'b', kind: 'paragraph', text: 'no match here'),
        EditorNode(id: 'c', kind: 'heading', text: 'FOO', data: {'level': 1}),
      ]);
      final n = c.replaceAll('foo', 'bar');
      expect(n, 4);
      expect(c.nodes[0].text, 'bar bar bar');
      expect(c.nodes[1].text, 'no match here');
      expect(c.nodes[2].text, 'bar');
      // A single committed op batch → a single undo step. onOps runs on the
      // controller's async commit chain, so flush microtasks before counting.
      expect(c.canUndo, isTrue);
      await Future<void>.delayed(Duration.zero);
      expect(opBatches, 1);
    });

    test('empty query replaces nothing', () {
      final c = _doc([EditorNode(id: 'a', kind: 'paragraph', text: 'abc')]);
      expect(c.replaceAll('', 'x'), 0);
      expect(c.nodes.single.text, 'abc');
    });

    test('marks survive a multi-match splice within one node', () {
      // "x a y a z" bold over the trailing "z"; replacing "a"→"aa" grows text,
      // and the bold must still cover "z" afterward.
      final c = _doc([
        EditorNode(
          id: 'a',
          kind: 'paragraph',
          text: 'x a y a z',
          data: {'marks': marksToJson([Mark(8, 9, 'bold')])},
        ),
      ]);
      expect(c.replaceAll('a', 'aa'), 2);
      expect(c.nodes.single.text, 'x aa y aa z');
      final m = marksFromData(c.nodes.single.data).single;
      expect(c.nodes.single.text.substring(m.start, m.end), 'z');
    });
  });
}
