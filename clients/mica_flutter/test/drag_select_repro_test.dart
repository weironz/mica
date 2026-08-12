// Reproduction attempt for two assertions the user hit while drag-selecting
// page content (2026-08-12, screenshots):
//
//   'package:flutter/src/rendering/object.dart': Failed assertion: line 4312
//   pos 14: 'debugNeedsLayout': is not true.
//
//   A FocusNode was used after being disposed.
//
// Both fired from a plain left-button drag across the page. A widget test can
// drive exactly that gesture, and unlike the running app it gives a stack trace.
//
// If these pass, the trigger needs something this harness does not reproduce
// (a real text-input connection, a real overlay route, the tab strip above the
// editor) — and that is worth knowing too, so they stay either way.

import 'package:flutter/gestures.dart' show PointerDeviceKind;
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/table.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

EditorNode _para(String id, String text) =>
    EditorNode(id: id, kind: 'paragraph', text: text, data: const {});

EditorNode _table() => EditorNode(
  id: 't',
  kind: 'table',
  text: '',
  data: TableData([
    ['h1', 'h2'],
    ['a1', 'a2'],
    ['b1', 'b2'],
  ]).toBlockData(),
);

Future<void> _pump(WidgetTester tester, List<EditorNode> nodes) async {
  await tester.pumpWidget(
    MaterialApp(
      localizationsDelegates: AppLocalizations.localizationsDelegates,
      supportedLocales: AppLocalizations.supportedLocales,
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
  await tester.pumpAndSettle();
}

void main() {
  testWidgets('drag-selecting text raises the format bar without asserting', (
    tester,
  ) async {
    // THE repro. The floating format bar only appears for a RANGED selection in
    // a FOCUSED editor, which is why the plain drags below never raised it — and
    // the bar is what carries the crash: it is positioned with
    // `CompositedTransformFollower`, and every IconButton in it has a Tooltip,
    // which on this Flutter is an OverlayPortal that asks its anchor for a paint
    // transform DURING layout. A RenderFollowerLayer has none until after.
    await _pump(tester, [
      _para('p1', 'The first paragraph of the page, long enough to select.'),
      _para('p2', 'A second paragraph, below it.'),
    ]);

    final box = tester.getRect(find.byType(MicaEditor));
    // Click to focus and place a caret, THEN drag to extend.
    await tester.tapAt(Offset(box.left + 40, box.top + 20));
    await tester.pump();

    final gesture = await tester.startGesture(
      Offset(box.left + 40, box.top + 20),
    );
    for (var i = 1; i <= 14; i++) {
      await gesture.moveTo(Offset(box.left + 40 + i * 14, box.top + 20));
      await tester.pump(const Duration(milliseconds: 16));
    }
    await gesture.up();
    await tester.pumpAndSettle();
    // Hovering a bar button is what mounts the tooltip's overlay child.
    final buttons = find.byType(IconButton);
    if (buttons.evaluate().isNotEmpty) {
      final pointer = TestPointer(1, PointerDeviceKind.mouse);
      await tester.sendEventToBinding(
        pointer.hover(tester.getCenter(buttons.first)),
      );
      await tester.pump(const Duration(seconds: 2));
      await tester.pumpAndSettle();
    }

    expect(tester.takeException(), isNull);
  });

  testWidgets('dragging a selection across plain paragraphs', (tester) async {
    await _pump(tester, [
      _para('p1', 'The first paragraph of the page.'),
      _para('p2', 'A second paragraph, below it.'),
      _para('p3', 'And a third one after that.'),
    ]);

    final box = tester.getRect(find.byType(MicaEditor));
    final gesture = await tester.startGesture(
      Offset(box.left + 40, box.top + 20),
    );
    // Many small moves, like a real drag: one big jump can skip the frame the
    // assertion fires on.
    for (var i = 1; i <= 12; i++) {
      await gesture.moveTo(
        Offset(box.left + 40 + i * 12, box.top + 20 + i * 6),
      );
      await tester.pump(const Duration(milliseconds: 16));
    }
    await gesture.up();
    await tester.pumpAndSettle();

    expect(tester.takeException(), isNull);
  });

  testWidgets('dragging a selection from above a table to below it', (
    tester,
  ) async {
    // The screenshot carrying the layout assertion had a table on screen, and a
    // drag that crosses one enters the cell-area selection path.
    await _pump(tester, [
      _para('p1', 'Text above the table.'),
      _table(),
      _para('p2', 'Text below the table.'),
    ]);

    final box = tester.getRect(find.byType(MicaEditor));
    final gesture = await tester.startGesture(
      Offset(box.left + 40, box.top + 12),
    );
    for (var i = 1; i <= 20; i++) {
      await gesture.moveTo(
        Offset(box.left + 40 + i * 8, box.top + 12 + i * 14),
      );
      await tester.pump(const Duration(milliseconds: 16));
    }
    await gesture.up();
    await tester.pumpAndSettle();

    expect(tester.takeException(), isNull);
  });

  testWidgets('drag-selecting INSIDE a table, cell to cell', (tester) async {
    // The cell-area drag: this is the path that opens and closes the per-cell
    // editor overlay, each of which owns a FocusNode disposed a frame after its
    // field is removed.
    await _pump(tester, [_para('p1', 'Above.'), _table()]);

    final box = tester.getRect(find.byType(MicaEditor));
    final start = Offset(box.left + 60, box.top + 90);
    final gesture = await tester.startGesture(start);
    for (var i = 1; i <= 16; i++) {
      await gesture.moveTo(start + Offset(i * 10, i * 4));
      await tester.pump(const Duration(milliseconds: 16));
    }
    await gesture.up();
    // Let the post-frame disposals run — that is when a FocusNode teardown
    // races anything still holding it.
    await tester.pumpAndSettle();
    await tester.pump(const Duration(milliseconds: 100));

    expect(tester.takeException(), isNull);
  });
}
