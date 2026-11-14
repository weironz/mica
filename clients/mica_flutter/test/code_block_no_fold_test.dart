// Long code blocks used to auto-fold past 40 lines to their first 20, behind
// an expand strip (issue #174). They now always show in full; `data.collapsed`
// only means something on a raw-HTML `<details>` block.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/render.dart';

final _source = List.generate(60, (i) => 'line $i').join('\n');

void main() {
  for (final collapsed in [null, true]) {
    testWidgets('a 60-line code block lays out every line '
        '(data.collapsed: $collapsed)', (tester) async {
      final blink = ValueNotifier(false);
      addTearDown(blink.dispose);
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: SingleChildScrollView(
              child: SizedBox(
                width: 480,
                child: DocumentSurface(
                  nodes: [
                    EditorNode(
                      id: 'code',
                      kind: 'code_block',
                      text: _source,
                      data: {'collapsed': ?collapsed},
                    ),
                  ],
                  contentRevision: 0,
                  selection: null,
                  showCaret: false,
                  caretBlink: blink,
                  appearance: const EditorAppearance(),
                ),
              ),
            ),
          ),
        ),
      );
      final r = tester.renderObject<RenderDocument>(
        find.byType(DocumentSurface),
      );
      final box = r.blockBox(0)!;
      final last = r.caretRectFor(DocPosition(0, _source.length))!;
      expect(last.bottom, lessThanOrEqualTo(box.bottom));
    });
  }

  test('toggleCollapsed leaves a plain code block alone', () {
    final ops = <List<Map<String, dynamic>>>[];
    final c = EditorController(
      rootBlockId: 'root',
      onOps: (o) async => ops.add(o),
    )..load([EditorNode(id: 'code', kind: 'code_block', text: _source)]);
    c.toggleCollapsed(0);
    expect(c.nodes.single.data.containsKey('collapsed'), isFalse);
    expect(ops, isEmpty);
  });
}
