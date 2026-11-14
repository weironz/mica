import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/block_marquee.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

final _marqueeRect = find.byKey(const ValueKey('block-marquee-rect'));

class _Page {
  final frameKey = GlobalKey();
  final bodyKey = GlobalKey();
  final hook = EditorMarqueeHook();
  final applied = <DocOp>[];

  Widget build({Key? editorKey}) => MaterialApp(
    localizationsDelegates: AppLocalizations.localizationsDelegates,
    supportedLocales: AppLocalizations.supportedLocales,
    home: Scaffold(
      // The real pane fills its slot (Positioned.fill); a loose Scaffold body
      // would let the scroll view shrink-wrap and leave no blank space below.
      body: SizedBox.expand(
        child: EditorMarqueeRegion(
          enabled: true,
          editorSurfaceKey: bodyKey,
          pageFrameKey: frameKey,
          hook: hook,
          onOutsidePointerDown: () {},
          child: SingleChildScrollView(
            child: Center(
              child: SizedBox(
                key: frameKey,
                width: 520,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    // Stands in for the page title rows above the body.
                    const SizedBox(height: 48),
                    Padding(
                      key: bodyKey,
                      padding: const EdgeInsets.only(top: 4),
                      child: MicaEditor(
                        key: editorKey,
                        rootBlockId: 'root',
                        nodes: [
                          EditorNode(
                            id: 'first',
                            kind: 'paragraph',
                            text: 'first',
                          ),
                          EditorNode(
                            id: 'second',
                            kind: 'paragraph',
                            text: 'second',
                          ),
                          EditorNode(
                            id: 'third',
                            kind: 'paragraph',
                            text: 'third',
                          ),
                        ],
                        version: 0,
                        canEdit: true,
                        marqueeHook: hook,
                        onApplyOperations: (ops) async => applied.addAll(ops),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    ),
  );
}

Future<_Page> _pump(WidgetTester tester, {Key? editorKey}) async {
  tester.view.physicalSize = const Size(1200, 700);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  final page = _Page();
  await tester.pumpWidget(page.build(editorKey: editorKey));
  await tester.pump();
  return page;
}

RenderDocument _doc(WidgetTester tester) =>
    tester.renderObject<RenderDocument>(find.byType(DocumentSurface));

/// Global centre-line y of block [i].
double _blockY(WidgetTester tester, int i) {
  final doc = _doc(tester);
  return doc.localToGlobal(doc.blockBox(i)!.center).dy;
}

({int lo, int hi})? _range(WidgetTester tester) => tester
    .widget<BlockMarqueeHighlight>(find.byType(BlockMarqueeHighlight))
    .selection
    .range;

DocSelection? _surfaceSelection(WidgetTester tester) =>
    tester.widget<DocumentSurface>(find.byType(DocumentSurface)).selection;

Future<void> _drag(
  WidgetTester tester,
  Offset from,
  Offset to, {
  void Function()? whileHeld,
}) async {
  final gesture = await tester.startGesture(
    from,
    kind: PointerDeviceKind.mouse,
  );
  await gesture.moveTo(Offset.lerp(from, to, 0.5)!);
  await tester.pump();
  await gesture.moveTo(to);
  await tester.pump();
  whileHeld?.call();
  await gesture.up();
  await tester.pump();
}

void main() {
  testWidgets('a margin drag draws a marquee and selects whole blocks', (
    tester,
  ) async {
    final page = await _pump(tester);
    final frame = tester.getRect(find.byKey(page.frameKey));
    final from = Offset(frame.left - 80, _blockY(tester, 0) - 6);
    final to = Offset(frame.right + 80, _blockY(tester, 1));

    var rectShown = false;
    await _drag(
      tester,
      from,
      to,
      whileHeld: () => rectShown = _marqueeRect.evaluate().isNotEmpty,
    );

    expect(rectShown, isTrue, reason: 'the marquee rectangle is visible');
    expect(_marqueeRect, findsNothing, reason: 'it disappears on release');
    expect(_range(tester), (lo: 0, hi: 1));
    expect(
      _surfaceSelection(tester),
      isNull,
      reason: 'block mode paints block tints, not a text-run highlight',
    );
    expect(
      find.byIcon(Icons.format_bold),
      findsNothing,
      reason: 'the inline format bar is for text ranges, not whole blocks',
    );
  });

  // Most of the blank below the body is the editor's own trailing space, so
  // the press lands inside the editor and must not also start a text drag.
  for (final where in ['inside', 'outside']) {
    testWidgets('dragging up from the blank below the last block ($where '
        'the editor)', (tester) async {
      final page = await _pump(tester);
      final body = tester.getRect(find.byKey(page.bodyKey));
      final frame = tester.getRect(find.byKey(page.frameKey));
      final lastBottom = page.hook.contentBottom()!;
      final startY = where == 'inside' ? lastBottom + 40 : body.bottom + 60;
      expect(startY < body.bottom, where == 'inside');
      var rectShown = false;
      await _drag(
        tester,
        Offset(frame.center.dx, startY),
        Offset(frame.center.dx, _blockY(tester, 1)),
        whileHeld: () => rectShown = _marqueeRect.evaluate().isNotEmpty,
      );
      expect(rectShown, isTrue);
      expect(_range(tester), (lo: 1, hi: 2));
      expect(_surfaceSelection(tester), isNull);
    });
  }

  // Left of the text is the editor's handle gutter; it reads as margin, so it
  // must start a marquee too — but a press on a block's drag handle still
  // belongs to the block drag.
  testWidgets('a drag from the left handle gutter starts a marquee', (
    tester,
  ) async {
    await _pump(tester);
    final doc = _doc(tester);
    final gutterX = doc.localToGlobal(Offset.zero).dx + 6;
    final textY = _blockY(tester, 0);
    var rectShown = false;
    await _drag(
      tester,
      Offset(gutterX, textY - 6),
      Offset(gutterX + 200, _blockY(tester, 1)),
      whileHeld: () => rectShown = _marqueeRect.evaluate().isNotEmpty,
    );
    expect(rectShown, isTrue);
    expect(_range(tester), (lo: 0, hi: 1));
    expect(_surfaceSelection(tester), isNull);
  });

  // The natural marquee starts at the page's top-left corner, beside the
  // title rows, above where the body begins.
  testWidgets('a drag from beside the title starts a marquee', (tester) async {
    final page = await _pump(tester);
    final frame = tester.getRect(find.byKey(page.frameKey));
    final body = tester.getRect(find.byKey(page.bodyKey));
    final from = Offset(frame.left - 60, frame.top + 10);
    expect(from.dy < body.top, isTrue);
    var rectShown = false;
    await _drag(
      tester,
      from,
      Offset(frame.left + 200, _blockY(tester, 1)),
      whileHeld: () => rectShown = _marqueeRect.evaluate().isNotEmpty,
    );
    expect(rectShown, isTrue);
    expect(_range(tester), (lo: 0, hi: 1));
  });

  testWidgets('a press on the scrollbar strip does not start a marquee', (
    tester,
  ) async {
    await _pump(tester);
    final pane = tester.getRect(find.byType(EditorMarqueeRegion));
    var rectShown = false;
    await _drag(
      tester,
      Offset(pane.right - 4, _blockY(tester, 0)),
      Offset(pane.right - 300, _blockY(tester, 2)),
      whileHeld: () => rectShown = _marqueeRect.evaluate().isNotEmpty,
    );
    expect(rectShown, isFalse);
    expect(_range(tester), isNull);
  });

  testWidgets('a press on a drag handle does not start a marquee', (
    tester,
  ) async {
    await _pump(tester);
    final doc = _doc(tester);
    final box = doc.blockBox(0)!;
    final handle = doc.localToGlobal(Offset(box.left - 12, box.center.dy));
    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await mouse.addPointer(location: doc.localToGlobal(box.center));
    await mouse.moveTo(handle);
    await tester.pump();
    expect(doc.dragHandleAt(doc.globalToLocal(handle)), 0);
    var rectShown = false;
    await _drag(
      tester,
      handle,
      handle + const Offset(0, 40),
      whileHeld: () => rectShown = _marqueeRect.evaluate().isNotEmpty,
    );
    await mouse.removePointer();
    expect(rectShown, isFalse);
    expect(_range(tester), isNull);
  });

  testWidgets('a marquee that touches no block selects nothing', (
    tester,
  ) async {
    final page = await _pump(tester);
    final frame = tester.getRect(find.byKey(page.frameKey));
    await _drag(
      tester,
      Offset(frame.left - 100, _blockY(tester, 0)),
      Offset(frame.left - 40, _blockY(tester, 2)),
    );
    expect(_range(tester), isNull);
  });

  testWidgets('Delete removes the selected blocks whole', (tester) async {
    final page = await _pump(tester);
    final frame = tester.getRect(find.byKey(page.frameKey));
    await _drag(
      tester,
      Offset(frame.left - 80, _blockY(tester, 0) - 6),
      Offset(frame.right + 80, _blockY(tester, 1)),
    );
    await tester.sendKeyEvent(LogicalKeyboardKey.delete);
    await tester.pump();

    final nodes = tester
        .widget<DocumentSurface>(find.byType(DocumentSurface))
        .nodes;
    expect([for (final n in nodes) n.id], ['third']);
    expect(
      [
        for (final op in page.applied)
          if (op['type'] == 'delete_block') op['block_id'],
      ],
      ['first', 'second'],
    );
    expect(_range(tester), isNull);
  });

  testWidgets('Escape or a plain margin click leaves block mode', (
    tester,
  ) async {
    final page = await _pump(tester);
    final frame = tester.getRect(find.byKey(page.frameKey));
    Future<void> select() => _drag(
      tester,
      Offset(frame.left - 80, _blockY(tester, 0) - 6),
      Offset(frame.right + 80, _blockY(tester, 2)),
    );

    await select();
    expect(_range(tester), (lo: 0, hi: 2));
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pump();
    expect(_range(tester), isNull);
    expect(
      _surfaceSelection(tester),
      const DocSelection.collapsed(DocPosition(0, 0)),
    );

    await select();
    expect(_range(tester), (lo: 0, hi: 2));
    await tester.tapAt(
      Offset(frame.left - 80, _blockY(tester, 1)),
      kind: PointerDeviceKind.mouse,
    );
    await tester.pump();
    expect(_range(tester), isNull);
  });

  // Opening another page swaps the keyed editor: the new State binds the hook
  // in initState BEFORE the old State's dispose runs, so the old one must not
  // clear a binding it no longer owns.
  testWidgets('the marquee still works after the editor is replaced', (
    tester,
  ) async {
    final page = await _pump(tester, editorKey: const ValueKey('doc-a'));
    await tester.pumpWidget(page.build(editorKey: const ValueKey('doc-b')));
    await tester.pump();
    final frame = tester.getRect(find.byKey(page.frameKey));
    await _drag(
      tester,
      Offset(frame.left - 80, _blockY(tester, 0) - 6),
      Offset(frame.right + 80, _blockY(tester, 2)),
    );
    expect(_range(tester), (lo: 0, hi: 2));
  });
}
