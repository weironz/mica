import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';

EditorController _fresh(List<EditorNode> nodes) {
  final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
  c.load(nodes);
  return c;
}

EditorNode _p(String id, String text) =>
    EditorNode(id: id, kind: 'paragraph', text: text);

void main() {
  // ---------------------------------------------------------------------------
  // Word / segment boundaries (double / triple click)
  // ---------------------------------------------------------------------------
  group('wordBoundsAt', () {
    test('ASCII identifier run', () {
      // caret inside "bar" of "foo bar baz"
      expect(wordBoundsAt('foo bar baz', 5), (4, 7));
    });

    test('underscore and digits stay in one word', () {
      expect(wordBoundsAt('a foo_bar2 b', 5), (2, 10));
    });

    test('contiguous CJK run selected together', () {
      // "你好 世界": click inside the first CJK run grabs 你好 only (space is a
      // boundary), not 世界.
      const text = '你好 世界';
      expect(wordBoundsAt(text, 1), (0, 2));
      // click in the second run grabs 世界.
      expect(wordBoundsAt(text, 4), (3, 5));
    });

    test('a click on punctuation selects only that character', () {
      // "a, b": offset 1 is the comma.
      expect(wordBoundsAt('a, b', 1), (1, 2));
    });

    test('a click on whitespace selects only that character', () {
      expect(wordBoundsAt('a b', 1), (1, 2));
    });

    test('caret at end falls back to the last glyph', () {
      expect(wordBoundsAt('foo', 3), (0, 3));
    });

    test('CJK and ASCII are different classes (boundary between them)', () {
      // "你a": a class change is a boundary, so each is its own word.
      const text = '你a';
      expect(wordBoundsAt(text, 0), (0, 1)); // 你
      expect(wordBoundsAt(text, 1), (1, 2)); // a
    });
  });

  test('selectWordAt selects the word and selectBlockText the whole block', () {
    final c = _fresh([_p('a', 'hello brave world')]);
    c.selection = const DocSelection.collapsed(DocPosition(0, 8)); // inside brave
    expect(c.selectWordAt(const DocPosition(0, 8)), isTrue);
    expect(c.selection!.start.offset, 6);
    expect(c.selection!.end.offset, 11);

    expect(c.selectBlockText(0), isTrue);
    expect(c.selection!.start.offset, 0);
    expect(c.selection!.end.offset, 'hello brave world'.length);
  });

  test('word/block selection is a no-op on atomic blocks', () {
    final c = _fresh([
      EditorNode(id: 'm', kind: 'math_block', text: r'E = mc^2'),
    ]);
    expect(c.selectWordAt(const DocPosition(0, 2)), isFalse);
    expect(c.selectBlockText(0), isFalse);
  });

  // ---------------------------------------------------------------------------
  // Tab / Shift+Tab list indent (validation — user reported it "not working")
  // ---------------------------------------------------------------------------
  test('Tab deepens a list item, Shift+Tab makes it shallower', () {
    final c = _fresh([
      EditorNode(id: 'a', kind: 'bulleted_list', text: 'one'),
      EditorNode(id: 'b', kind: 'bulleted_list', text: 'two'),
    ]);
    // Indent the SECOND item — the first above caps it at level 1.
    c.selection = const DocSelection.collapsed(DocPosition(1, 0));
    expect(c.indentSelection(1), isTrue);
    expect(c.nodes[1].indent, 1);

    expect(c.indentSelection(-1), isTrue);
    expect(c.nodes[1].indent, 0);
  });

  test('Tab cannot create an orphan level (max one deeper than the item above)',
      () {
    final c = _fresh([
      EditorNode(id: 'a', kind: 'bulleted_list', text: 'one'),
      EditorNode(id: 'b', kind: 'bulleted_list', text: 'two'),
    ]);
    c.selection = const DocSelection.collapsed(DocPosition(1, 0));
    // First Tab: 0 → 1 (allowed). Second Tab: capped at 1 (no level-2 orphan).
    expect(c.indentSelection(1), isTrue);
    expect(c.nodes[1].indent, 1);
    expect(c.indentSelection(1), isFalse,
        reason: 'no change — the item above is only level 0');
    expect(c.nodes[1].indent, 1);
  });

  test('Shift+Tab at level 0 does nothing', () {
    final c = _fresh([
      EditorNode(id: 'a', kind: 'bulleted_list', text: 'top'),
    ]);
    c.selection = const DocSelection.collapsed(DocPosition(0, 0));
    expect(c.indentSelection(-1), isFalse);
    expect(c.nodes[0].indent, 0);
  });

  test('Tab indents every list item across a ranged selection', () {
    final c = _fresh([
      EditorNode(id: 'a', kind: 'bulleted_list', text: 'one'),
      EditorNode(id: 'b', kind: 'bulleted_list', text: 'two'),
      EditorNode(id: 'c', kind: 'bulleted_list', text: 'three'),
    ]);
    // Select items b and c; a (above) caps them at level 1.
    c.selection = const DocSelection(
      anchor: DocPosition(1, 0),
      focus: DocPosition(2, 5),
    );
    expect(c.indentSelection(1), isTrue);
    expect(c.nodes[1].indent, 1);
    expect(c.nodes[2].indent, 1);
  });
}
