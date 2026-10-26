import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

EditorNode paragraph(String text) => EditorNode(id: 'a', kind: 'paragraph', text: text);
EditorHistoryState result() => EditorHistoryState(nodes: [paragraph('a')], canUndo: false, canRedo: true, undoDepth: 0, redoDepth: 1);
void main() {
  test('replace all waits while controller undo is pending', () async {
    final gate = Completer<EditorHistoryState>(); final entered = Completer<void>(); final ops = <DocOp>[];
    final c = EditorController(rootBlockId: 'root', onOps: (batch) async => ops.addAll(batch), onHistory: (action) async {
      if (action == HistoryAction.undo) { entered.complete(); return gate.future; }
      return result();
    });
    addTearDown(c.dispose);
    c.load([paragraph('a')]); c.selection = const DocSelection.collapsed(DocPosition(0, 1));
    c.setFocusedText('ab', 2, 2); await c.flushPending();
    final undo = c.undo(); await entered.future;
    final count = c.replaceAll('b', 'X');
    final during = c.nodes.single.text;
    gate.complete(result()); await undo; await c.flushPending();
    expect(count, 0); expect(during, 'ab'); expect(ops.map((o) => o['text']), ['ab']);
  });
  testWidgets('replace all button is disabled during pending undo', (t) async {
    final gate = Completer<EditorHistoryState>(); var entered = false; final ops = <DocOp>[]; final hook = EditorFindHook();
    await t.pumpWidget(MaterialApp(localizationsDelegates: AppLocalizations.localizationsDelegates, supportedLocales: AppLocalizations.supportedLocales,
      home: Scaffold(body: MicaEditor(rootBlockId: 'root', nodes: [paragraph('a')], version: 0, canEdit: true, findHook: hook,
        onApplyOperations: (batch) async => ops.addAll(batch), onApplyHistory: (action) async {
          if (action == HistoryAction.undo) { entered = true; return gate.future; }
          return result();
        }))));
    await t.pump();
    final r = t.renderObject<RenderDocument>(find.byType(DocumentSurface));
    final origin = t.getTopLeft(find.byType(DocumentSurface));
    await t.tapAt(origin + r.caretRectFor(const DocPosition(0, 0))!.center); await t.pump();
    t.testTextInput.updateEditingValue(const TextEditingValue(text: 'ab', selection: TextSelection.collapsed(offset: 2)));
    await t.pump(const Duration(milliseconds: 500));
    hook.open(); await t.pump(); await t.pump();
    await t.tap(find.byIcon(Icons.keyboard_arrow_right)); await t.pump();
    await t.enterText(find.byType(TextField).at(0), 'b'); await t.pump();
    await t.enterText(find.byType(TextField).at(1), 'X'); await t.pump();
    await t.tapAt(origin + r.caretRectFor(const DocPosition(0, 0))!.center); await t.pump();
    await t.sendKeyDownEvent(LogicalKeyboardKey.controlLeft); await t.sendKeyEvent(LogicalKeyboardKey.keyZ); await t.sendKeyUpEvent(LogicalKeyboardKey.controlLeft); await t.pump();
    expect(entered, isTrue);
    final button = t.widget<IconButton>(find.widgetWithIcon(IconButton, Icons.change_circle_outlined));
    final disabled = button.onPressed == null;
    await t.tap(find.byIcon(Icons.change_circle_outlined)); await t.pump();
    gate.complete(result()); await t.pump(const Duration(milliseconds: 500));
    expect(disabled, isTrue); expect(ops.map((o) => o['text']), ['ab']);
    await t.pumpWidget(const SizedBox()); await t.pump();
  });
}
