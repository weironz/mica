// The table cell editor must stay in its cell when the page scrolls.
//
// Reported 2026-08-04: put the caret in a cell, scroll the wheel, and the
// cell's text "escapes" — it detaches from the table and slides relative to it,
// leaving the cell itself looking empty (the renderer deliberately skips
// painting the cell being edited, so once the overlay is elsewhere there is
// nothing left in the cell to see).
//
// The cause was that the editor is an `OverlayEntry` positioned at a
// `localToGlobal` screen coordinate recomputed on each BUILD. Typing rebuilds
// it, so relayout was covered; scrolling does not, and the editor does not own
// the scrollable (the host does), so there was no ScrollController here to hang
// a listener on either. The fix follows the canvas' leader layer instead of
// computing a screen position, which the compositor keeps in step no matter who
// scrolls.
//
// The assertion is deliberately RELATIVE — the editor's offset from the canvas
// origin — because that is the actual invariant ("it stays in its cell"), and
// it holds whatever the scroll offset happens to be.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/editor/table.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

EditorNode _table() => EditorNode(
  id: 't',
  kind: 'table',
  text: '',
  data: TableData([
    ['h1', 'h2'],
    ['a1', 'a2'],
  ]).toBlockData(),
);

void main() {
  testWidgets('the cell editor stays in its cell across a scroll', (
    tester,
  ) async {
    final scroll = ScrollController();
    addTearDown(scroll.dispose);

    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        locale: const Locale('zh'),
        home: Scaffold(
          body: SingleChildScrollView(
            controller: scroll,
            child: Column(
              children: [
                // Headroom above and below so there is somewhere to scroll to.
                const SizedBox(height: 400),
                MicaEditor(
                  rootBlockId: 'root',
                  nodes: [_table()],
                  version: 0,
                  canEdit: true,
                  onApplyOperations: (_) async {},
                ),
                const SizedBox(height: 800),
              ],
            ),
          ),
        ),
      ),
    );
    await tester.pump();

    // Open the editor on a body cell by tapping it, the way a user does.
    final r = tester.renderObject<RenderDocument>(find.byType(DocumentSurface));
    final cell = r.tableCellRect(0, 1, 0);
    expect(cell, isNotNull, reason: 'the table must have laid out');
    await tester.tapAt(
      tester.getTopLeft(find.byType(DocumentSurface)) + cell!.center,
    );
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));

    final field = find.byType(TextField);
    expect(field, findsOneWidget, reason: 'the tap opens the cell editor');

    Offset relativeToCanvas() =>
        tester.getTopLeft(field) -
        tester.getTopLeft(find.byType(DocumentSurface));

    final before = relativeToCanvas();

    scroll.jumpTo(scroll.offset + 220);
    await tester.pump();
    await tester.pump();

    expect(
      scroll.offset,
      greaterThan(0),
      reason: 'the harness must actually have scrolled',
    );
    final after = relativeToCanvas();
    expect(
      (after - before).distance,
      lessThan(1.0),
      reason:
          'the editor must move WITH the canvas, not stay pinned to the screen '
          '(before=$before after=$after)',
    );
  });
}
