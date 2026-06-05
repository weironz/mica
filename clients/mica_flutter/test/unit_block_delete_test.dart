import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';

/// Unit blocks (atomic kinds, rendered ```mermaid diagrams) are consumed
/// whole by a selection delete — merging their source text into neighbors
/// left stray code on the page.
void main() {
  EditorNode mermaid(String id) => EditorNode(
        id: id,
        kind: 'code_block',
        text: 'graph LR\n  A --> B',
        data: {'language': 'mermaid'},
      );

  EditorController fresh(List<EditorNode> nodes) {
    final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
    c.load(nodes);
    return c;
  }

  test('selection ending ON a diagram deletes it whole', () {
    final c = fresh([
      EditorNode(id: 'a', kind: 'paragraph', text: 'above the diagram'),
      mermaid('m'),
      EditorNode(id: 'b', kind: 'paragraph', text: 'below'),
    ]);
    // Drag from inside "above" onto the diagram (its caret stop is offset 0).
    c.selection = const DocSelection(
      anchor: DocPosition(0, 6),
      focus: DocPosition(1, 0),
    );
    expect(c.deleteSelection(), isTrue);

    expect(c.nodes.map((n) => n.id), ['a', 'b']);
    expect(c.nodes[0].text, 'above ',
        reason: 'the diagram source must NOT spill into the paragraph');
  });

  test('selection starting ON a diagram deletes it whole', () {
    final c = fresh([
      mermaid('m'),
      EditorNode(id: 'b', kind: 'paragraph', text: 'below the diagram'),
    ]);
    c.selection = const DocSelection(
      anchor: DocPosition(0, 0),
      focus: DocPosition(1, 5),
    );
    expect(c.deleteSelection(), isTrue);

    expect(c.nodes, hasLength(1));
    expect(c.nodes[0].kind, 'paragraph',
        reason: 'the surviving carrier is a plain paragraph, not a '
            'mermaid code_block holding leftover prose');
    expect(c.nodes[0].text, ' the diagram');
    expect(c.nodes[0].data['language'], isNull);
  });

  test('diagram fully inside the range is removed with it', () {
    final c = fresh([
      EditorNode(id: 'a', kind: 'paragraph', text: 'head'),
      mermaid('m'),
      EditorNode(id: 'b', kind: 'paragraph', text: 'tail'),
    ]);
    c.selection = const DocSelection(
      anchor: DocPosition(0, 2),
      focus: DocPosition(2, 2),
    );
    expect(c.deleteSelection(), isTrue);
    expect(c.nodes, hasLength(1));
    expect(c.nodes[0].text, 'heil');
  });

  test('a code-view diagram still merges like a normal code block', () {
    final c = fresh([
      EditorNode(id: 'a', kind: 'paragraph', text: 'above'),
      EditorNode(
        id: 'm',
        kind: 'code_block',
        text: 'graph LR',
        data: {'language': 'mermaid', 'view': 'code'},
      ),
    ]);
    c.selection = const DocSelection(
      anchor: DocPosition(0, 5),
      focus: DocPosition(1, 5),
    );
    expect(c.deleteSelection(), isTrue);
    expect(c.nodes[0].text, 'above LR',
        reason: 'source-form editing keeps ordinary text-merge semantics');
  });
}
