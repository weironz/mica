import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

// Regression for issue #157: after Ctrl+A the platform only sees the FOCUS
// block's text, so ordinary typing and IME composition edited that block alone
// instead of replacing the document selection. The widget must route the
// platform diff through the controller's document-range splice.
Future<List<List<DocOp>>> _pump(WidgetTester tester) async {
  final writes = <List<DocOp>>[];
  await tester.pumpWidget(
    MaterialApp(
      localizationsDelegates: AppLocalizations.localizationsDelegates,
      supportedLocales: AppLocalizations.supportedLocales,
      home: Scaffold(
        body: SingleChildScrollView(
          child: MicaEditor(
            rootBlockId: 'root',
            nodes: [
              EditorNode(id: 'a', kind: 'paragraph', text: 'alpha'),
              EditorNode(id: 'b', kind: 'paragraph', text: 'beta'),
            ],
            version: 0,
            canEdit: true,
            onApplyOperations: (ops) async => writes.add(ops),
          ),
        ),
      ),
    ),
  );
  await tester.pump();
  await tester.tapAt(
    tester.getTopLeft(find.byType(DocumentSurface)) + const Offset(60, 15),
  );
  await tester.pump();
  return writes;
}

List<EditorNode> _nodes(WidgetTester tester) =>
    tester.widget<DocumentSurface>(find.byType(DocumentSurface)).nodes;

Future<void> _selectAll(WidgetTester tester) async {
  await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
  await tester.sendKeyEvent(LogicalKeyboardKey.keyA);
  await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
  await tester.pump();
}

void main() {
  testWidgets('typing after Ctrl+A replaces the whole document selection', (
    tester,
  ) async {
    await _pump(tester);
    await _selectAll(tester);
    expect(find.byType(DocumentSurface), findsOneWidget);

    // The OS only knows the focus block ("beta") and appends X at its caret.
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'betaX',
        selection: TextSelection.collapsed(offset: 5),
      ),
    );
    await tester.pump();

    final nodes = _nodes(tester);
    expect(nodes.length, 1, reason: 'the selection must be replaced, not edited');
    expect(nodes.single.text, 'X');
    await tester.pumpWidget(const SizedBox());
    await tester.pump();
  });

  testWidgets('IME composition commit after Ctrl+A also replaces the selection', (
    tester,
  ) async {
    await _pump(tester);
    await _selectAll(tester);

    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'betaX',
        selection: TextSelection.collapsed(offset: 5),
        composing: TextRange(start: 4, end: 5),
      ),
    );
    await tester.pump();
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'betaX',
        selection: TextSelection.collapsed(offset: 5),
      ),
    );
    await tester.pump();

    final nodes = _nodes(tester);
    expect(nodes.length, 1);
    expect(nodes.single.text, 'X');
    await tester.pumpWidget(const SizedBox());
    await tester.pump();
  });

  testWidgets('multi-line mobile paste keeps a soft break, no hard split', (
    tester,
  ) async {
    await _pump(tester);
    final render = tester.renderObject<RenderDocument>(
      find.byType(DocumentSurface),
    );
    await tester.tapAt(
      tester.getTopLeft(find.byType(DocumentSurface)) +
          render.caretRectFor(const DocPosition(0, 0))!.center,
    );
    await tester.pump();
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'alX\nYpha',
        selection: TextSelection.collapsed(offset: 6),
      ),
    );
    await tester.pump();
    final nodes = _nodes(tester);
    expect(nodes.length, 2, reason: 'only the two original paragraphs');
    expect(nodes.first.text, 'alX\nYpha');
    await tester.pumpWidget(const SizedBox());
    await tester.pump();
  });

  testWidgets('focus loss drops a stale prose capture', (tester) async {
    await _pump(tester);
    await _selectAll(tester);
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'betaX',
        selection: TextSelection.collapsed(offset: 5),
        composing: TextRange(start: 4, end: 5),
      ),
    );
    await tester.pump();
    // Leave the editor and return: the captured cross-block range must die.
    // Tapping the canvas keeps editor focus; unfocus is what detaches the IME.
    FocusManager.instance.primaryFocus?.unfocus();
    await tester.pump();
    await tester.tapAt(
      tester.getTopLeft(find.byType(DocumentSurface)) + const Offset(60, 15),
    );
    await tester.pump();
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'betaX',
        selection: TextSelection.collapsed(offset: 5),
      ),
    );
    await tester.pump();
    expect(_nodes(tester).length, 2, reason: 'no stale whole-document splice');
    await tester.pumpWidget(const SizedBox());
    await tester.pump();
  });
}