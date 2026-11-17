// #184: with the interface text size above 100% (Settings, or OS text scaling)
// clicking a table cell enlarged its text, and a line that exactly filled the
// cell wrapped and spilled over the row divider. The canvas paints cells with
// the editor's own font settings and ignores the app's MediaQuery text scale;
// the overlay TextField inherited it. Both must draw the same size.
import 'package:flutter/material.dart' hide TableCell;
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/editor/table.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

void main() {
  testWidgets('the cell editor ignores the UI text scale, like the canvas', (
    tester,
  ) async {
    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        locale: const Locale('zh'),
        // The interface-size preference works exactly like this
        // (ui/interface_text_scale.dart): a MediaQuery text scaler.
        builder: (context, child) => MediaQuery(
          data: MediaQuery.of(
            context,
          ).copyWith(textScaler: const TextScaler.linear(1.25)),
          child: child!,
        ),
        home: Scaffold(
          body: MicaEditor(
            rootBlockId: 'root',
            nodes: [
              EditorNode(
                id: 't',
                kind: 'table',
                text: '',
                data: TableData([
                  [const TableCell('h1'), const TableCell('h2')],
                  [const TableCell('body'), const TableCell('a2')],
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
    final cell = r.tableCellRect(0, 1, 0)!;
    final rowHeight = cell.height;
    await tester.tapAt(
      tester.getTopLeft(find.byType(DocumentSurface)) + cell.center,
    );
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));

    // EditableText reads its scaler from the nearest MediaQuery.
    expect(
      MediaQuery.textScalerOf(tester.element(find.byType(EditableText))),
      TextScaler.noScaling,
      reason: 'the field must not enlarge text the canvas painted unscaled',
    );
    // One line of text still fits the row: the field did not grow past it.
    expect(
      tester.getSize(find.byType(TextField)).height,
      lessThanOrEqualTo(rowHeight),
    );
  });
}
