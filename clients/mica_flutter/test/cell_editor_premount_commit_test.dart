// A cell editor committed IN THE SAME FRAME it was opened must tear itself
// down completely.
//
// Reported 2026-08-28 as a debug red screen ("A FocusNode was used after being
// disposed", 37 repeats in one session) that popped while merely READING a
// page. The chain: `OverlayEntry.mounted` stays false until the overlay
// rebuilds a frame after `insert()`, and commit's old guard
// `if (entry.mounted) entry.remove()` therefore SKIPPED the remove for a
// commit landing inside that window (tap a cell, then pan/click before the
// next frame — easy when a heavy page makes frames long). `_cellEntry` was
// nulled anyway, so the entry became a ghost nothing could remove; its
// FocusNode was disposed a frame later, and every Overlay rebuild afterwards
// rebuilt the ghost's TextField over the dead node — red screen forever until
// restart. Release builds skip the assert, so the same ghost just sat there
// invisibly eating a cell's edits.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
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
  testWidgets('commit before the entry ever mounts leaves no ghost overlay', (
    tester,
  ) async {
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
              _table(),
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
    final cell = r.tableCellRect(1, 1, 0);
    expect(cell, isNotNull);

    // Open the cell editor and, WITHOUT pumping a frame, start a drag
    // elsewhere — `_onPanStart` commits the editor while `entry.mounted` is
    // still false. This is the same-frame window the old guard fell into.
    await tester.tapAt(origin + cell!.center);
    final gesture = await tester.startGesture(origin + const Offset(30, 8));
    await gesture.moveBy(const Offset(40, 0));
    await gesture.up();

    // Now let frames run: the deferred dispose fires, and any ghost entry
    // would rebuild its TextField over the disposed FocusNode.
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));
    expect(tester.takeException(), isNull);
    expect(
      find.byType(TextField),
      findsNothing,
      reason: 'the committed editor must be GONE, not lingering unmounted',
    );

    // The killer in the report: a later, unrelated overlay rebuild. Open the
    // editor again (that inserts a fresh entry and rebuilds the overlay) — a
    // ghost from the first round would rebuild here and throw.
    await tester.tapAt(origin + cell.center);
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));
    expect(tester.takeException(), isNull);
    expect(
      find.byType(TextField),
      findsOneWidget,
      reason: 'exactly the new editor — no ghost twin',
    );
  });
}
