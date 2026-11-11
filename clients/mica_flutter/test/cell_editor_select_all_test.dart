// Ctrl+A in a table cell editor selects the whole cell inside Flutter, so a
// Ctrl+B pressed right after formats all of it. On the web the browser's own
// select-all goes through the hidden text input and, straight after typing,
// could reach the framework only after Ctrl+B had already read a collapsed
// selection (clipboard_e2e "committed cell lost its marks", 2026-11-11).
import 'package:flutter/material.dart' hide TableCell;
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/cell_edit_controller.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/editor/table.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

void main() {
  testWidgets('Ctrl+A then Ctrl+B bolds the whole cell', (tester) async {
    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        locale: const Locale('zh'),
        home: Scaffold(
          body: MicaEditor(
            rootBlockId: 'root',
            nodes: [
              EditorNode(id: 'p', kind: 'paragraph', text: 'a paragraph'),
              EditorNode(
                id: 't',
                kind: 'table',
                text: '',
                data: TableData([
                  [const TableCell('h1'), const TableCell('h2')],
                  [const TableCell('a1'), const TableCell('a2')],
                ]).toBlockData(),
              ),
            ],
            version: 0,
            canEdit: true,
            onApplyOperations: (_) async {},
          ),
        ),
      ),
    );
    await tester.pump();

    final r = tester.renderObject<RenderDocument>(find.byType(DocumentSurface));
    final origin = tester.getTopLeft(find.byType(DocumentSurface));
    await tester.tapAt(origin + r.tableCellRect(1, 1, 0)!.center);
    await tester.pump();
    await tester.pump();

    final field = find.byType(TextField);
    expect(field, findsOneWidget);
    final controller =
        tester.widget<TextField>(field).controller! as CellEditController;
    controller.value = const TextEditingValue(
      text: 'a1 edited',
      selection: TextSelection.collapsed(offset: 9),
    );
    await tester.pump();

    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyA);
    expect(
      controller.selection,
      const TextSelection(baseOffset: 0, extentOffset: 9),
    );
    await tester.sendKeyEvent(LogicalKeyboardKey.keyB);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await tester.pump();

    expect(
      rangeHasMark(controller.marks, 0, 9, 'bold'),
      isTrue,
      reason: 'the whole cell is bold, not just a collapsed caret',
    );
  });
}
