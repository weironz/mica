import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

void main() {
  testWidgets('structural shortcuts wait while engine undo is pending', (
    tester,
  ) async {
    final result = Completer<EditorHistoryState>();
    final writes = <List<DocOp>>[];
    var undoEntered = false;
    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: Scaffold(
          body: SingleChildScrollView(
            child: MicaEditor(
              rootBlockId: 'root',
              nodes: [EditorNode(id: 'a', kind: 'paragraph', text: 'a')],
              version: 0,
              canEdit: true,
              onApplyOperations: (ops) async => writes.add(ops),
              onApplyHistory: (action) async {
                if (action == HistoryAction.undo) {
                  undoEntered = true;
                  return result.future;
                }
                return EditorHistoryState(
                  nodes: [EditorNode(id: 'a', kind: 'paragraph', text: 'a')],
                  canUndo: false,
                  canRedo: false,
                  undoDepth: 0,
                  redoDepth: 0,
                );
              },
            ),
          ),
        ),
      ),
    );
    await tester.tapAt(
      tester.getTopLeft(find.byType(DocumentSurface)) + const Offset(60, 15),
    );
    await tester.pump();
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'ab',
        selection: TextSelection.collapsed(offset: 2),
      ),
    );
    await tester.pump(const Duration(milliseconds: 450));
    expect(writes, hasLength(1));

    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyZ);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await tester.pump();
    expect(undoEntered, isTrue);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.digit1);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await tester.pump();
    final surface = tester.widget<DocumentSurface>(
      find.byType(DocumentSurface),
    );
    final kindDuringUndo = surface.nodes.single.kind;
    final writesDuringUndo = writes.length;

    result.complete(
      EditorHistoryState(
        nodes: [EditorNode(id: 'a', kind: 'paragraph', text: 'a')],
        canUndo: false,
        canRedo: true,
        undoDepth: 0,
        redoDepth: 1,
      ),
    );
    await tester.pump();
    expect(kindDuringUndo, 'paragraph');
    expect(writesDuringUndo, 1);
    expect(
      tester
          .widget<DocumentSurface>(find.byType(DocumentSurface))
          .nodes
          .single
          .text,
      'a',
    );
    await tester.pumpWidget(const SizedBox());
    await tester.pump();
  });
}
