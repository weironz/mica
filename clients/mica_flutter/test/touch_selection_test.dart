import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/touch_selection.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

import 'support/markdown_codec.dart';

/// Finger selection on phones: a double-tap or long-press selects one or two
/// CJK characters (never a run of unpredictable length), a triple-tap the
/// paragraph, and a finger selection shows two drag handles plus a
/// 剪切/复制/粘贴/全选 menu instead of the formatting toolbar.
void main() {
  setUpAll(useNativeMarkdownCodec); // copy/cut build HTML through the engine

  group('touchWordBounds', () {
    const s = '今天天气很好，适合散步';
    test('the touched character and the next one', () {
      expect(touchWordBounds(s, 0), (0, 2));
      expect(touchWordBounds(s, 2), (2, 4));
    });
    test('the one before it at the end of a run', () {
      expect(touchWordBounds(s, 5), (4, 6)); // 好 before ，
      expect(touchWordBounds(s, 10), (9, 11)); // the last character
    });
    test('punctuation alone, never joined to a neighbour', () {
      expect(touchWordBounds(s, 6), (6, 7)); // ，
      expect(touchWordBounds('“引用”', 0), (0, 1));
    });
    test('a lone CJK character between Latin words is just itself', () {
      expect(touchWordBounds('a 中 b', 2), (2, 3));
    });
    test('Latin words and numbers keep the whole word', () {
      expect(touchWordBounds('hello brave world', 8), (6, 11));
      expect(touchWordBounds('v0.14.4 ok', 1), (0, 2));
      expect(touchWordBounds('中文English混排', 4), (2, 9));
    });
    test('never splits a surrogate pair', () {
      const ext = '𠀀𠀁好'; // two CJK Ext-B characters (2 code units each)
      expect(touchWordBounds(ext, 1), (0, 4));
      expect(touchWordBounds(ext, 4), (2, 5));
      expect(touchWordBounds('a😀b', 2), isNot((2, 3)));
    });
    test('the same answer every time for the same spot', () {
      for (var i = 0; i < s.length; i++) {
        final (a, b) = touchWordBounds(s, i);
        expect(b - a, inInclusiveRange(1, 2), reason: 'offset $i');
        expect(a <= i && i < b, isTrue, reason: 'offset $i is inside');
      }
    });
  });

  group('MultiTapCounter', () {
    test('a finger gets more room than a mouse', () {
      final c = MultiTapCounter();
      c.bump(Offset.zero, Duration.zero, touch: true);
      expect(
        c.bump(
          const Offset(20, 0),
          const Duration(milliseconds: 100),
          touch: true,
        ),
        2,
      );
      final m = MultiTapCounter();
      m.bump(Offset.zero, Duration.zero, touch: false);
      expect(
        m.bump(
          const Offset(20, 0),
          const Duration(milliseconds: 100),
          touch: false,
        ),
        1,
      );
    });
  });

  // -- Through the real gesture pipeline ------------------------------------

  const line = '今天天气很好适合散步';

  Future<TextInputClient> pumpEditor(WidgetTester tester) async {
    tester.view.physicalSize = const Size(800, 600);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    // Clipboard writes from the menu go to a mocked platform channel.
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      (call) async => null,
    );
    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        locale: const Locale('zh'),
        home: Scaffold(
          body: MicaEditor(
            rootBlockId: 'root',
            // Pad lines keep the target well below y=0, so the menu that sits
            // ABOVE the selection is on screen and tappable.
            nodes: [
              EditorNode(id: 'p0', kind: 'paragraph', text: 'pad line one'),
              EditorNode(id: 'p1', kind: 'paragraph', text: 'pad line two'),
              EditorNode(id: 'p2', kind: 'paragraph', text: 'pad line three'),
              EditorNode(id: 'p3', kind: 'paragraph', text: 'pad line four'),
              EditorNode(id: 'a', kind: 'paragraph', text: line),
            ],
            version: 0,
            canEdit: true,
            onApplyOperations: (_) async {},
          ),
        ),
      ),
    );
    await tester.pump();
    return tester.state(find.byType(MicaEditor)) as TextInputClient;
  }

  // Inside the 5th line (calibrated like multitap_select_test.dart).
  Offset target(WidgetTester tester) =>
      tester.getTopLeft(find.byType(MicaEditor)) + const Offset(40, 170);

  final menu = find.byKey(const ValueKey('touch-selection-menu'));
  final startHandle = find.byKey(const ValueKey('touch-handle-start'));
  final endHandle = find.byKey(const ValueKey('touch-handle-end'));
  final formatBar = find.byIcon(Icons.format_bold);

  TextSelection selectionOf(TextInputClient s) =>
      s.currentTextEditingValue!.selection;

  testWidgets('long-press selects two characters, with handles and a menu', (
    tester,
  ) async {
    final state = await pumpEditor(tester);
    await tester.longPressAt(target(tester));
    await tester.pump();

    final sel = selectionOf(state);
    expect(sel.isCollapsed, isFalse, reason: 'a long-press selects');
    expect(
      sel.end - sel.start,
      inInclusiveRange(1, 2),
      reason: 'one or two characters, never the whole run',
    );
    expect(startHandle, findsOneWidget);
    expect(endHandle, findsOneWidget);
    expect(menu, findsOneWidget);
    for (final label in ['剪切', '复制', '粘贴', '全选', '格式']) {
      expect(
        find.descendant(of: menu, matching: find.text(label)),
        findsOneWidget,
      );
    }
    expect(
      formatBar,
      findsNothing,
      reason: 'a finger selection gets the text menu, not the format bar',
    );
  });

  testWidgets('double-tap selects the same characters each time', (
    tester,
  ) async {
    final state = await pumpEditor(tester);
    final p = target(tester);
    Future<TextSelection> doubleTap(int id) async {
      for (var i = 0; i < 2; i++) {
        final g = await tester.startGesture(p, pointer: id + i);
        await g.up();
        await tester.pump(const Duration(milliseconds: 30));
      }
      return selectionOf(state);
    }

    final first = await doubleTap(10);
    expect(first.end - first.start, inInclusiveRange(1, 2));
    expect(menu, findsOneWidget);
    // Collapse (a tap on the first pad line, clear of the menu), wait out
    // the multi-tap window, and do it again.
    await tester.pump(const Duration(milliseconds: 600));
    final away =
        tester.getTopLeft(find.byType(MicaEditor)) + const Offset(300, 20);
    final g = await tester.startGesture(away, pointer: 20);
    await g.up();
    await tester.pump(const Duration(milliseconds: 600));
    final second = await doubleTap(30);
    expect(second, first, reason: 'the same spot selects the same characters');
  });

  testWidgets('triple-tap selects the paragraph', (tester) async {
    final state = await pumpEditor(tester);
    final p = target(tester);
    for (var i = 0; i < 3; i++) {
      final g = await tester.startGesture(p, pointer: 40 + i);
      await g.up();
      await tester.pump(const Duration(milliseconds: 30));
    }
    final sel = selectionOf(state);
    expect((sel.start, sel.end), (0, line.length));
    expect(menu, findsOneWidget);
    expect(endHandle, findsOneWidget);
  });

  testWidgets('dragging the end handle grows the selection', (tester) async {
    final state = await pumpEditor(tester);
    await tester.longPressAt(target(tester));
    await tester.pump();
    final before = selectionOf(state);

    final g = await tester.startGesture(
      tester.getCenter(endHandle),
      pointer: 50,
    );
    await tester.pump();
    expect(menu, findsNothing, reason: 'the menu steps aside while dragging');
    for (var i = 0; i < 6; i++) {
      await g.moveBy(const Offset(20, 0));
      await tester.pump();
    }
    await g.up();
    await tester.pump();

    final after = selectionOf(state);
    expect(after.start, before.start, reason: 'the other end stays put');
    expect(after.end, greaterThan(before.end));
    expect(menu, findsOneWidget, reason: 'the menu returns after the drag');
  });

  testWidgets('全选 selects everything; 复制 keeps the selection, drops the menu', (
    tester,
  ) async {
    await pumpEditor(tester);
    await tester.longPressAt(target(tester));
    await tester.pump();

    final startY = tester.getCenter(startHandle).dy;
    await tester.tap(find.descendant(of: menu, matching: find.text('全选')));
    await tester.pump();
    expect(startHandle, findsOneWidget);
    expect(endHandle, findsOneWidget);
    expect(
      tester.getCenter(startHandle).dy,
      lessThan(startY - 50),
      reason: 'select-all moved the start up to the first line',
    );

    await tester.tap(find.descendant(of: menu, matching: find.text('复制')));
    await tester.pump();
    expect(menu, findsNothing);
    expect(endHandle, findsOneWidget, reason: 'the selection is kept');
  });

  testWidgets('格式 swaps the menu for the format bar, keeping the handles',
      (tester) async {
    await pumpEditor(tester);
    await tester.longPressAt(target(tester));
    await tester.pump();
    expect(formatBar, findsNothing);

    await tester.tap(find.descendant(of: menu, matching: find.text('格式')));
    await tester.pump();
    expect(menu, findsNothing, reason: 'the format bar takes its place');
    expect(formatBar, findsOneWidget);
    expect(startHandle, findsOneWidget);
    expect(endHandle, findsOneWidget);

    // The next touch on the page goes back to the plain finger flow.
    final away =
        tester.getTopLeft(find.byType(MicaEditor)) + const Offset(300, 20);
    await tester.tapAt(away);
    await tester.pump(const Duration(milliseconds: 500));
    await tester.longPressAt(target(tester));
    await tester.pump();
    expect(menu, findsOneWidget);
    expect(formatBar, findsNothing);
  });

  testWidgets('a mouse double-click still gets the format bar', (tester) async {
    final state = await pumpEditor(tester);
    final p = target(tester);
    for (var i = 0; i < 2; i++) {
      final g = await tester.startGesture(
        p,
        pointer: 60 + i,
        kind: PointerDeviceKind.mouse,
      );
      await g.up();
      await tester.pump(const Duration(milliseconds: 30));
    }
    final sel = selectionOf(state);
    expect(
      (sel.start, sel.end),
      (0, line.length),
      reason: 'the mouse keeps the whole-run word',
    );
    expect(formatBar, findsOneWidget);
    expect(menu, findsNothing);
    expect(endHandle, findsNothing);
  });
}
