import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';

// The format toolbar highlights the current block type (which heading level
// you're in, etc.). It reads EditorActiveBlockHook, which the editor publishes
// from the focused block. These lock in the hook contract + that a tap into a
// heading reports its level.
void main() {
  test('EditorActiveBlockHook notifies on change, dedupes identical publishes', () {
    final hook = EditorActiveBlockHook();
    var n = 0;
    hook.addListener(() => n++);
    hook.publish('heading', 2);
    expect((hook.kind, hook.level), ('heading', 2));
    expect(n, 1);
    hook.publish('heading', 2); // identical → no notify
    expect(n, 1);
    hook.publish('heading', 3); // level change → notify
    expect(n, 2);
    hook.publish('paragraph', null); // kind change → notify
    expect((hook.kind, hook.level), ('paragraph', null));
    expect(n, 3);
  });

  Future<EditorActiveBlockHook> pump(WidgetTester tester, EditorNode node) async {
    final hook = EditorActiveBlockHook();
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: MicaEditor(
            rootBlockId: 'root',
            nodes: [node],
            version: 0,
            canEdit: true,
            onApplyOperations: (_) async {},
            activeBlockHook: hook,
          ),
        ),
      ),
    );
    await tester.pump();
    final box = tester.getTopLeft(find.byType(MicaEditor));
    await tester.tapAt(box + const Offset(40, 20));
    await tester.pump();
    await tester.pump();
    return hook;
  }

  testWidgets('tapping into a heading reports its level', (tester) async {
    final hook = await pump(
      tester,
      EditorNode(id: 'h', kind: 'heading', text: 'Title', data: {'level': 2}),
    );
    expect(hook.kind, 'heading');
    expect(hook.level, 2);
  });

  testWidgets('tapping into a paragraph reports kind with no level', (tester) async {
    final hook = await pump(
      tester,
      EditorNode(id: 'p', kind: 'paragraph', text: 'body text'),
    );
    expect(hook.kind, 'paragraph');
    expect(hook.level, isNull);
  });
}
