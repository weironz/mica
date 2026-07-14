import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';

// Ctrl/Cmd+Alt+1…6 sets the focused block's heading level; Ctrl/Cmd+Alt+0
// returns it to plain text (Notion/Word convention — bare Ctrl+digit is owned
// by the browser's tab switching on the web build). Observed through the
// ActiveBlockHook, the same feed the toolbar highlight uses.
void main() {
  Future<EditorActiveBlockHook> pumpEditor(
    WidgetTester tester,
    List<EditorNode> nodes,
  ) async {
    final hook = EditorActiveBlockHook();
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: MicaEditor(
            rootBlockId: 'root',
            nodes: nodes,
            version: 0,
            canEdit: true,
            onApplyOperations: (_) async {},
            activeBlockHook: hook,
          ),
        ),
      ),
    );
    await tester.pump();
    return hook;
  }

  Future<void> pressHeadingShortcut(WidgetTester tester, int digit) async {
    final key = switch (digit) {
      0 => LogicalKeyboardKey.digit0,
      1 => LogicalKeyboardKey.digit1,
      2 => LogicalKeyboardKey.digit2,
      _ => LogicalKeyboardKey.digit3,
    };
    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
    await tester.sendKeyEvent(key);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await tester.pump();
  }

  testWidgets('Ctrl+Alt+2 makes the focused paragraph an H2; '
      'Ctrl+Alt+0 returns it to text', (tester) async {
    final hook = await pumpEditor(tester, [
      EditorNode(id: 'p', kind: 'paragraph', text: 'hello world'),
    ]);

    // Focus the editor + place the caret in the paragraph.
    await tester.tapAt(
      tester.getTopLeft(find.byType(MicaEditor)) + const Offset(80, 14),
    );
    await tester.pump();

    await pressHeadingShortcut(tester, 2);
    expect(hook.kind, 'heading');
    expect(hook.level, 2);

    await pressHeadingShortcut(tester, 0);
    expect(hook.kind, 'paragraph');
  });

  testWidgets('plain Ctrl+digit (no Alt) does NOT change the block',
      (tester) async {
    final hook = await pumpEditor(tester, [
      EditorNode(id: 'p', kind: 'paragraph', text: 'hello world'),
    ]);
    await tester.tapAt(
      tester.getTopLeft(find.byType(MicaEditor)) + const Offset(80, 14),
    );
    await tester.pump();

    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.digit1);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await tester.pump();

    expect(hook.kind, 'paragraph');
  });
}
