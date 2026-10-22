// Issue #51:远端编辑到达(reconcile)时,本地选区要跟着文本走,而不是钉在原来的数字偏移上。
//
// 修复前 `_remapSelection` 只保留 focus 的数字偏移并把选区折叠:
//   - 协作者在我光标**之前**打字 → 光标相对文本向后退,下一个键落错位置;
//   - 拖选中途来一次远端更新 → 选区被折叠(AI「替换选区」随之失效)。
//
// 规则(controller.dart `_mapThroughRemoteEdit` 的文档):每个端点各自穿过该块
// 「旧文本 → 新文本」的前后缀 diff 映射;变化之前的不动、之后的平移;恰好插在端点上
// 的远端文本**永远不被吸进选区、也不推动光标**(光标与选区终点粘左,选区起点粘右);
// 端点所在文本被删 → 落到变化的边界上。
@TestOn('vm')
library;

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';

EditorNode _p(String id, String text) =>
    EditorNode(id: id, kind: 'paragraph', text: text);

EditorController _ctrl(List<EditorNode> doc) {
  final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
  c.load(doc);
  return c;
}

DocSelection _sel(int an, int ao, int fn, int fo) =>
    DocSelection(anchor: DocPosition(an, ao), focus: DocPosition(fn, fo));

DocSelection _caret(int n, int o) => DocSelection.collapsed(DocPosition(n, o));

/// The text the (single-node) selection currently covers.
String _selected(EditorController c) {
  final s = c.selection!;
  expect(s.isMultiNode, isFalse);
  return c.nodes[s.start.node].text.substring(s.start.offset, s.end.offset);
}

void main() {
  group('collapsed caret', () {
    test('remote insert BEFORE the caret shifts it right', () {
      final c = _ctrl([_p('a', 'hello world')]);
      c.setSelection(_caret(0, 6)); // before "world"
      c.reconcile([_p('a', 'XYZhello world')]);
      expect(c.selection, _caret(0, 9));
      expect(c.nodes[0].text.substring(9), 'world');
    });

    test('remote delete BEFORE the caret shifts it left', () {
      final c = _ctrl([_p('a', 'hello world')]);
      c.setSelection(_caret(0, 6));
      c.reconcile([_p('a', 'lo world')]); // "hel" removed
      expect(c.selection, _caret(0, 3));
      expect(c.nodes[0].text.substring(3), 'world');
    });

    test('remote edit AFTER the caret leaves it alone', () {
      final c = _ctrl([_p('a', 'hello world')]);
      c.setSelection(_caret(0, 5));
      c.reconcile([_p('a', 'hello brave new world')]);
      expect(c.selection, _caret(0, 5));
    });

    test('remote delete SPANNING the caret lands it on the deletion point', () {
      final c = _ctrl([_p('a', 'hello world')]);
      c.setSelection(_caret(0, 7)); // inside "world"
      c.reconcile([_p('a', 'hello!')]); // "world" -> "!"... replaced [5,11)
      expect(c.selection, _caret(0, 5));
    });

    test('remote insert EXACTLY at the caret does not push it', () {
      // Documented rule: the caret sticks to the character on its left, so two
      // people typing at one spot don't interleave each other's characters.
      final c = _ctrl([_p('a', 'ab')]);
      c.setSelection(_caret(0, 1));
      c.reconcile([_p('a', 'aXYb')]);
      expect(c.selection, _caret(0, 1));
    });

    test('caret block moved by a remote block insert keeps its offset map', () {
      final c = _ctrl([_p('a', 'first'), _p('b', 'second')]);
      c.setSelection(_caret(1, 3));
      c.reconcile([
        _p('new', 'inserted'),
        _p('a', 'first'),
        _p('b', '>>second'),
      ]);
      expect(c.selection, _caret(2, 5));
    });
  });

  group('expanded selection', () {
    test(
      'survives a remote insert before it, still covering the same text',
      () {
        final c = _ctrl([_p('a', 'hello world')]);
        c.setSelection(_sel(0, 6, 0, 11)); // "world"
        c.reconcile([_p('a', 'oh, hello world')]);
        expect(c.selection!.isCollapsed, isFalse);
        expect(_selected(c), 'world');
        expect(c.selection, _sel(0, 10, 0, 15));
      },
    );

    test('keeps its direction (backward drag)', () {
      final c = _ctrl([_p('a', 'hello world')]);
      c.setSelection(_sel(0, 11, 0, 6)); // anchor after focus
      c.reconcile([_p('a', '>>hello world')]);
      expect(c.selection, _sel(0, 13, 0, 8));
    });

    test('text inserted exactly at the START boundary stays outside', () {
      final c = _ctrl([_p('a', 'aXb')]);
      c.setSelection(_sel(0, 1, 0, 2)); // "X"
      c.reconcile([_p('a', 'a12Xb')]);
      expect(_selected(c), 'X');
    });

    test('text inserted exactly at the END boundary stays outside', () {
      final c = _ctrl([_p('a', 'aXb')]);
      c.setSelection(_sel(0, 1, 0, 2)); // "X"
      c.reconcile([_p('a', 'aX34b')]);
      expect(_selected(c), 'X');
    });

    test('a remote edit inside it keeps it expanded around the survivors', () {
      final c = _ctrl([_p('a', 'one two three')]);
      c.setSelection(_sel(0, 0, 0, 13));
      c.reconcile([_p('a', 'one 2 three')]);
      expect(_selected(c), 'one 2 three');
    });

    test('collapses only when every selected character was deleted', () {
      final c = _ctrl([_p('a', 'keep DROP keep')]);
      c.setSelection(_sel(0, 5, 0, 9)); // "DROP"
      c.reconcile([_p('a', 'keep  keep')]);
      expect(c.selection, _caret(0, 5));
    });

    test('multi-node: each endpoint maps through its own block', () {
      final c = _ctrl([_p('a', 'alpha'), _p('b', 'bravo'), _p('c', 'charlie')]);
      c.setSelection(_sel(0, 2, 2, 4));
      c.reconcile([
        _p('a', '__alpha'), // +2 before the anchor
        _p('b', 'BRAVO!!'), // middle block: irrelevant to the endpoints
        _p('c', 'charlie'),
      ]);
      expect(c.selection, _sel(0, 4, 2, 4));
    });
  });

  test('a selection in ANOTHER block is untouched by a remote edit', () {
    final c = _ctrl([_p('a', 'mine here'), _p('b', 'theirs')]);
    c.setSelection(_sel(0, 0, 0, 4)); // "mine"
    c.reconcile([_p('a', 'mine here'), _p('b', 'XXXX theirs')]);
    expect(c.selection, _sel(0, 0, 0, 4));
    expect(_selected(c), 'mine');
  });
}
