import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

// Regression for issue #158: Enter was recognized only by a NET +1 newline
// count. Replacing a selection that already contains a soft newline deletes one
// newline and inserts another, so the count is unchanged and the block split was
// skipped — the document silently stayed ONE quote with a stale soft break.
void main() {
  testWidgets('Enter replacing a selection with an old soft break splits', (
    tester,
  ) async {
    final writes = <DocOp>[];
    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: Scaffold(
          body: SingleChildScrollView(
            child: MicaEditor(
              rootBlockId: 'root',
              nodes: [
                EditorNode(id: 'q', kind: 'quote', text: 'a\nbc'),
              ],
              version: 0,
              canEdit: true,
              onApplyOperations: (ops) async => writes.addAll(ops),
            ),
          ),
        ),
      ),
    );
    await tester.pump();
    await tester.tapAt(
      tester.getTopLeft(find.byType(DocumentSurface)) + const Offset(40, 15),
    );
    await tester.pump();

    // Park the caret at offset 3, then extend the selection back to [1,3):
    // the selection now covers the old soft newline and 'b'.
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'a\nbc',
        selection: TextSelection.collapsed(offset: 3),
      ),
    );
    await tester.pump();
    await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowLeft);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
    await tester.pump();

    // OS Enter: replace the selection with a newline ('a\nc', caret 2).
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'a\nc',
        selection: TextSelection.collapsed(offset: 2),
      ),
    );
    await tester.testTextInput.receiveAction(TextInputAction.newline);
    await tester.pump();

    final surface = tester.widget<DocumentSurface>(
      find.byType(DocumentSurface),
    );
    expect(surface.nodes.length, 2, reason: 'Enter must split the quote block');
    expect(surface.nodes.map((n) => n.text).toList(), ['a', 'c']);
    expect(surface.nodes.every((n) => n.kind == 'quote'), isTrue);
    expect(
      writes.map((op) => op['type']).toList(),
      contains('insert_block'),
    );
    await tester.pumpWidget(const SizedBox());
    await tester.pump();
  });
}