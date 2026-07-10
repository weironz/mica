import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';

/// Double-click selects the word under the caret; triple-click selects the whole
/// block. Driven through the real gesture pipeline (the multi-tap counting lives
/// in _onTapDown). The focused node's selection surfaces through the editor's
/// TextInputClient editing value, which we read to assert the range.
void main() {
  Future<TextInputClient> pumpEditor(
      WidgetTester tester, List<EditorNode> nodes) async {
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: MicaEditor(
            rootBlockId: 'root',
            nodes: nodes,
            version: 0,
            canEdit: true,
            onApplyOperations: (_) async {},
          ),
        ),
      ),
    );
    await tester.pump();
    return tester.state(find.byType(MicaEditor)) as TextInputClient;
  }

  testWidgets('double-tap selects the word, triple-tap the whole block',
      (tester) async {
    const line = 'hello brave world';
    // Pad the top so the target line sits well below y=0: the floating format
    // toolbar appears ~44px ABOVE a ranged selection, and we must tap on the
    // line (not on the toolbar) for the second/third tap to reach the canvas.
    final state = await pumpEditor(tester, [
      EditorNode(id: 'pad0', kind: 'paragraph', text: 'pad line one'),
      EditorNode(id: 'pad1', kind: 'paragraph', text: 'pad line two'),
      EditorNode(id: 'pad2', kind: 'paragraph', text: 'pad line three'),
      EditorNode(id: 'pad3', kind: 'paragraph', text: 'pad line four'),
      EditorNode(id: 'a', kind: 'paragraph', text: line),
    ]);
    final box = tester.getTopLeft(find.byType(MicaEditor));
    // Inside the 5th prose line. NB: flutter_test has no Roboto, so prose falls
    // to a bundled fallback font — its line height sets where each line sits, so
    // this y is calibrated to that (with the CJK-fallback fonts the 5th line is
    // at ~y165–180; recalibrate if the bundled fallback ever changes).
    final p = box + const Offset(40, 170);

    // One tap, fresh pointer id each time so the gesture arena fully resolves
    // between taps; small pumps keep them inside the multi-tap window (400ms).
    Future<void> tapOnce(int pointer) async {
      final g = await tester.startGesture(p, pointer: pointer);
      await g.up();
      await tester.pump(const Duration(milliseconds: 30));
    }

    await tapOnce(11);
    var sel = state.currentTextEditingValue!.selection;
    expect(sel.isCollapsed, isTrue,
        reason: 'a lone tap places a caret, not a selection');
    final caret = sel.baseOffset;

    // Second tap at the same point within the window → word selection.
    await tapOnce(12);
    sel = state.currentTextEditingValue!.selection;
    expect(sel.isCollapsed, isFalse,
        reason: 'a double-tap selects the word under the caret');
    expect(sel.start <= caret && sel.end >= caret, isTrue,
        reason: 'the selected word brackets the original caret');
    expect(sel.end - sel.start < line.length, isTrue,
        reason: 'a word is shorter than the whole line');

    // Third tap → whole-block selection.
    await tapOnce(13);
    sel = state.currentTextEditingValue!.selection;
    expect(sel.start, 0);
    expect(sel.end, line.length,
        reason: 'a triple-tap selects the entire block text');
  });

  testWidgets('a second tap far away does not escalate to a word selection',
      (tester) async {
    const line = 'alpha beta gamma delta epsilon';
    final state = await pumpEditor(
        tester, [EditorNode(id: 'a', kind: 'paragraph', text: line)]);
    final box = tester.getTopLeft(find.byType(MicaEditor));

    await tester.tapAt(box + const Offset(40, 20));
    await tester.pump();
    // A second tap well beyond the slop radius is a separate single click.
    await tester.tapAt(box + const Offset(220, 20));
    await tester.pump();
    expect(state.currentTextEditingValue!.selection.isCollapsed, isTrue,
        reason: 'taps at different spots stay separate single clicks');
  });

  testWidgets('double-tap on a math block opens its editor, not text selection',
      (tester) async {
    await pumpEditor(tester, [
      EditorNode(id: 'm', kind: 'math_block', text: r'E = mc^2'),
      EditorNode(id: 'p', kind: 'paragraph', text: ''),
    ]);

    final box = tester.getTopLeft(find.byType(MicaEditor));
    final p = box + const Offset(40, 20);
    // A tap on a math block opens its LaTeX editor (atomic behavior, checked
    // BEFORE the word-selection path). Let the async showDialog route settle.
    await tester.tapAt(p);
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));

    expect(find.byType(AlertDialog), findsOneWidget,
        reason: 'a tap on a math block opens the LaTeX editor');
  });
}
