import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

// gh#169: Tab in a code block indents its text by two spaces, but Shift+Tab
// fell through to list outdent, which a plain code block ignores — so code
// that had been indented could never be outdented again.
Future<List<DocOp>> _pump(WidgetTester tester, List<EditorNode> nodes) async {
  final writes = <DocOp>[];
  await tester.pumpWidget(
    MaterialApp(
      localizationsDelegates: AppLocalizations.localizationsDelegates,
      supportedLocales: AppLocalizations.supportedLocales,
      home: Scaffold(
        body: SingleChildScrollView(
          child: MicaEditor(
            rootBlockId: 'root',
            nodes: nodes,
            version: 0,
            canEdit: true,
            onApplyOperations: (ops) async => writes.addAll(ops),
          ),
        ),
      ),
    ),
  );
  await tester.pump();
  return writes;
}

String? _lastText(List<DocOp> writes, String id) => writes
    .lastWhere((op) => op['block_id'] == id && op.containsKey('text'))['text']
    as String?;

Future<void> _shiftTab(WidgetTester tester) async {
  await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
  await tester.sendKeyEvent(LogicalKeyboardKey.tab);
  await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
  await tester.pump();
}

void main() {
  testWidgets('Shift+Tab takes back the indent Tab put on a code line', (
    tester,
  ) async {
    final writes = await _pump(tester, [
      EditorNode(id: 'c', kind: 'code_block', text: 'x = 1'),
    ]);
    final surface = tester.getTopLeft(find.byType(DocumentSurface));
    await tester.tapAt(surface + const Offset(60, 14));
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.home);
    await tester.sendKeyEvent(LogicalKeyboardKey.tab);
    await tester.pump();
    expect(_lastText(writes, 'c'), '  x = 1');
    await _shiftTab(tester);
    expect(_lastText(writes, 'c'), 'x = 1');
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('Shift+Tab outdents every selected code line by one step', (
    tester,
  ) async {
    final writes = await _pump(tester, [
      EditorNode(id: 'c', kind: 'code_block', text: 'if a:\n    b\n  c\nd'),
    ]);
    final surface = tester.getTopLeft(find.byType(DocumentSurface));
    await tester.tapAt(surface + const Offset(60, 14));
    await tester.pump();
    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyA);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await tester.pump();
    await _shiftTab(tester);
    expect(_lastText(writes, 'c'), 'if a:\n  b\nc\nd');
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('a code block attached to a list item detaches on Shift+Tab', (
    tester,
  ) async {
    final writes = await _pump(tester, [
      EditorNode(id: 'a', kind: 'bulleted_list', text: 'item'),
      EditorNode(
        id: 'c',
        kind: 'code_block',
        text: 'x = 1',
        data: {'li': 0},
      ),
    ]);
    final surface = tester.getTopLeft(find.byType(DocumentSurface));
    await tester.tapAt(surface + const Offset(80, 50));
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.home);
    await _shiftTab(tester);
    final op = writes.lastWhere((op) => op['block_id'] == 'c');
    expect((op['data'] as Map).containsKey('li'), isFalse);
    await tester.pumpWidget(const SizedBox());
  });
}
