import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';

void main() {
  EditorController load(List<EditorNode> nodes) {
    final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
    c.load(nodes);
    return c;
  }

  test('Tab attaches a code block to the item above; Shift+Tab detaches', () {
    final c = load([
      EditorNode(id: 'a', kind: 'numbered_list', text: 'one'),
      EditorNode(id: 'b', kind: 'code_block', text: 'print(1)'),
      EditorNode(id: 'c', kind: 'numbered_list', text: 'two', data: {'start': 2}),
    ]);
    c.collapseTo(const DocPosition(1, 0));
    expect(c.canAttachToItem('b'), isTrue);
    expect(c.indentSelection(1), isTrue);
    expect(c.nodes[1].data['li'], 0);
    // Already attached: another Tab is a no-op.
    expect(c.indentSelection(1), isFalse);
    // Shift+Tab detaches.
    expect(c.indentSelection(-1), isTrue);
    expect(c.nodes[1].data.containsKey('li'), isFalse);
  });

  test('attach follows the owning item through its earlier children', () {
    final c = load([
      EditorNode(id: 'a', kind: 'bulleted_list', text: 'item', data: {'indent': 1}),
      EditorNode(id: 'b', kind: 'quote', text: 'quoted', data: {'li': 1}),
      EditorNode(id: 'p', kind: 'paragraph', text: 'tail'),
    ]);
    c.collapseTo(const DocPosition(2, 0));
    expect(c.indentSelection(1), isTrue);
    expect(c.nodes[2].data['li'], 1, reason: 'skips sibling children to the item');
  });

  test('nothing to attach to: Tab on a lone paragraph is a no-op', () {
    final c = load([
      EditorNode(id: 'p', kind: 'paragraph', text: 'alone'),
    ]);
    c.collapseTo(const DocPosition(0, 0));
    expect(c.canAttachToItem('p'), isFalse);
    expect(c.indentSelection(1), isFalse);
  });
}
