import 'dart:async';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/editor/table.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

void main() {
  EditorNode paragraph(String id, String text) =>
      EditorNode(id: id, kind: 'paragraph', text: text);

  test(
    'selection notifications leave content revision and persistence alone',
    () async {
      final ops = <DocOp>[];
      final controller = EditorController(
        rootBlockId: 'root',
        onOps: (batch) async => ops.addAll(batch),
      );
      addTearDown(controller.dispose);
      controller.load([paragraph('a', 'abc')]);
      final revision = controller.contentRevision;
      var notifications = 0;
      controller.addListener(() => notifications++);
      controller.collapseTo(const DocPosition(0, 1));
      controller.collapseTo(const DocPosition(0, 1));
      expect(notifications, 1);
      expect(controller.contentRevision, revision);
      controller.setFocusedText('abc', 2, 2);
      controller.setFocusedText('abc', 2, 2);
      expect(notifications, 2);
      expect(controller.contentRevision, revision);
      await controller.flushPending();
      expect(ops, isEmpty, reason: 'an IME selection echo must not save text');
    },
  );

  test(
    'structural edits ending at the same caret still invalidate content',
    () async {
      final controller = EditorController(
        rootBlockId: 'root',
        onOps: (_) async {},
      );
      addTearDown(controller.dispose);
      controller.load([paragraph('a', 'abc'), paragraph('b', 'next')]);
      controller.collapseTo(const DocPosition(0, 0));
      var previous = controller.contentRevision;
      controller.insertParagraphAtTop('title remainder');
      expect(controller.contentRevision, greaterThan(previous));
      expect(controller.nodes.first.text, 'title remainder');
      previous = controller.contentRevision;
      controller.deleteNode(0);
      expect(controller.contentRevision, greaterThan(previous));
      expect(controller.nodes.first.text, 'abc');
      previous = controller.contentRevision;
      controller.setFocusedText('changed', 7, 7);
      expect(controller.contentRevision, greaterThan(previous));
      await controller.flushPending();
      await pumpEventQueue();
      previous = controller.contentRevision;
      expect(controller.canUndo, isTrue);
      controller.undo();
      expect(controller.contentRevision, greaterThan(previous));
    },
  );

  Future<void> pumpEditor(
    WidgetTester tester,
    List<EditorNode> nodes, {
    ScrollController? scroll,
    Future<Map<String, String>> Function(List<String>)? resolveImages,
    Offset inset = Offset.zero,
    ApplyOps? onOps,
  }) async {
    final editor = MicaEditor(
      rootBlockId: 'root',
      nodes: nodes,
      version: 0,
      canEdit: true,
      onApplyOperations: onOps ?? (_) async {},
      onResolveImageUrls: resolveImages,
    );
    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: Scaffold(
          body: Align(
            alignment: Alignment.topLeft,
            child: Padding(
              padding: EdgeInsets.only(left: inset.dx, top: inset.dy),
              child: SizedBox(
                width: 500,
                height: 260,
                child: scroll == null
                    ? editor
                    : SingleChildScrollView(controller: scroll, child: editor),
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pump();
  }

  testWidgets('selection does not restart unresolved image URL requests', (
    tester,
  ) async {
    var requests = 0;
    final pending = Completer<Map<String, String>>();
    await pumpEditor(
      tester,
      [
        paragraph('a', 'an ordinary paragraph'),
        EditorNode(
          id: 'img',
          kind: 'image',
          text: '',
          data: {'file_id': 'file'},
        ),
      ],
      resolveImages: (_) {
        requests++;
        return pending.future;
      },
    );
    await tester.tapAt(const Offset(50, 14));
    await tester.pump();
    final before = requests;
    expect(before, greaterThan(0));
    for (var i = 0; i < 8; i++) {
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowRight);
      await tester.pump();
    }
    expect(requests, before);
    await tester.pumpWidget(const SizedBox());
    pending.complete({});
  });

  testWidgets(
    'format bar waits until mouse drag finishes; keyboard remains immediate',
    (tester) async {
      await pumpEditor(tester, [
        paragraph('a', 'Select this complete sentence with the mouse.'),
      ]);
      const start = Offset(50, 14);
      await tester.tapAt(start);
      await tester.pump();
      final drag = await tester.startGesture(
        start,
        kind: PointerDeviceKind.mouse,
      );
      await drag.moveTo(start + const Offset(30, 0));
      await tester.pump();
      await drag.moveTo(start + const Offset(100, 0));
      await tester.pump();
      final surface = tester.widget<DocumentSurface>(
        find.byType(DocumentSurface),
      );
      expect(surface.selection!.isCollapsed, isFalse);
      expect(find.byIcon(Icons.format_bold), findsNothing);
      await drag.up();
      await tester.pump();
      expect(find.byIcon(Icons.format_bold), findsOneWidget);
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowLeft);
      await tester.pump();
      expect(find.byIcon(Icons.format_bold), findsNothing);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowRight);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      await tester.pump();
      expect(find.byIcon(Icons.format_bold), findsOneWidget);
    },
  );

  testWidgets('keyboard document selection clears a previous table cell area', (
    tester,
  ) async {
    await pumpEditor(tester, [
      paragraph('p', 'ordinary paragraph'),
      EditorNode(
        id: 't',
        kind: 'table',
        text: '',
        data: TableData([
          ['h1', 'h2'],
          ['v1', 'v2'],
        ]).toBlockData(),
      ),
    ]);
    await tester.tapAt(const Offset(50, 14));
    await tester.pump();
    final render = tester.renderObject<RenderDocument>(
      find.byType(DocumentSurface),
    );
    render.tableBlockSelection = (node: 1, r0: 1, c0: 0, r1: 1, c1: 1);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyA);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await tester.pump();
    expect(
      render.tableBlockSelection,
      isNull,
      reason: 'copy/delete must follow the new document range',
    );
    expect(
      tester
          .widget<DocumentSurface>(find.byType(DocumentSurface))
          .selection!
          .isMultiNode,
      isTrue,
    );
  });

  testWidgets('cancelling an edge drag stops automatic scrolling', (
    tester,
  ) async {
    final scroll = ScrollController();
    addTearDown(scroll.dispose);
    await pumpEditor(tester, [
      for (var i = 0; i < 60; i++) paragraph('p$i', 'line $i'),
    ], scroll: scroll);
    const start = Offset(50, 14);
    await tester.tapAt(start);
    await tester.pump();
    final drag = await tester.startGesture(
      start,
      kind: PointerDeviceKind.mouse,
    );
    await drag.moveTo(start + const Offset(30, 0));
    await tester.pump();
    await drag.moveTo(const Offset(150, 250));
    for (var i = 0; i < 10; i++) {
      await tester.pump(const Duration(milliseconds: 16));
    }
    expect(
      scroll.offset,
      greaterThan(0),
      reason: 'the held drag must really scroll',
    );
    await drag.cancel();
    await tester.pump();
    final stopped = scroll.offset;
    for (var i = 0; i < 12; i++) {
      await tester.pump(const Duration(milliseconds: 16));
    }
    expect(
      scroll.offset,
      stopped,
      reason: 'cancel must stop frames and selection updates',
    );
  });

  testWidgets('a cancelled block drag never commits a move', (tester) async {
    final ops = <DocOp>[];
    await pumpEditor(tester, [
      paragraph('a', 'first'),
      paragraph('b', 'second'),
      paragraph('c', 'third'),
    ], onOps: (batch) async => ops.addAll(batch));
    final render = tester.renderObject<RenderDocument>(
      find.byType(DocumentSurface),
    );
    final caret = render.caretRectFor(const DocPosition(0, 0))!;
    final handle = Offset(caret.left - 12, caret.center.dy);
    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await mouse.addPointer(location: handle);
    await mouse.moveTo(handle);
    await tester.pump();
    expect(
      render.dragHandleAt(handle),
      0,
      reason: 'the gesture must start on the real handle',
    );
    await mouse.down(handle);
    await mouse.moveTo(handle + const Offset(3, 0));
    await tester.pump();
    await mouse.moveTo(const Offset(100, 140));
    await tester.pump();
    expect(
      render.dropIndex,
      isNotNull,
      reason: 'a real block move must have begun',
    );
    await mouse.cancel();
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 100));
    expect(ops.where((o) => o['type'] == 'move_block'), isEmpty);
    expect(
      tester
          .widget<DocumentSurface>(find.byType(DocumentSurface))
          .nodes
          .map((n) => n.id),
      ['a', 'b', 'c'],
    );
    expect(render.dropIndex, isNull);
  });

  testWidgets('edge-scroll distance follows elapsed time at 60 and 120 Hz', (
    tester,
  ) async {
    Future<double> dragAt(int milliseconds) async {
      final scroll = ScrollController();
      await pumpEditor(tester, [
        for (var i = 0; i < 60; i++) paragraph('p$i', 'line $i'),
      ], scroll: scroll);
      await tester.tapAt(const Offset(50, 14));
      await tester.pump();
      final mouse = await tester.startGesture(
        const Offset(50, 14),
        kind: PointerDeviceKind.mouse,
      );
      await mouse.moveTo(const Offset(80, 14));
      await tester.pump();
      await mouse.moveTo(const Offset(150, 250));
      await tester.pump(); // seed the clock at the same instant
      for (var elapsed = 0; elapsed < 320; elapsed += milliseconds) {
        await tester.pump(Duration(milliseconds: milliseconds));
      }
      final distance = scroll.offset;
      await mouse.cancel();
      await tester.pumpWidget(const SizedBox());
      scroll.dispose();
      return distance;
    }

    final at60 = await dragAt(16);
    final at120 = await dragAt(8);
    expect(at60, greaterThan(200));
    expect(at120, closeTo(at60, 1));
  });

  testWidgets(
    'ordinary typing and composition report the newly laid out caret to the OS',
    (tester) async {
      await pumpEditor(tester, [
        paragraph('a', ''),
      ], inset: const Offset(20, 40));
      await tester.tapAt(const Offset(70, 54));
      await tester.pump();
      final client = tester.state(find.byType(MicaEditor)) as TextInputClient;
      final geometry =
          tester.testTextInput.log
                  .lastWhere(
                    (c) => c.method == 'TextInput.setEditableSizeAndTransform',
                  )
                  .arguments
              as Map;
      final transform = geometry['transform'] as List;
      expect(transform[12], 20);
      expect(transform[13], 40);
      Future<void> check(TextEditingValue value) async {
        tester.testTextInput.log.clear();
        tester.testTextInput.updateEditingValue(value);
        await tester.pump();
        final calls = tester.testTextInput.log.where(
          (c) => c.method == 'TextInput.setCaretRect',
        );
        expect(
          calls,
          isNotEmpty,
          reason: 'typing must update the candidate anchor',
        );
        final render = tester.renderObject<RenderDocument>(
          find.byType(DocumentSurface),
        );
        final selection = tester
            .widget<DocumentSurface>(find.byType(DocumentSurface))
            .selection!;
        final rect = render.caretRectFor(selection.focus)!;
        expect(
          rect.top,
          greaterThan(30),
          reason: 'the fixture must force a line wrap',
        );
        final args = calls.last.arguments as Map;
        expect(args['y'], closeTo(rect.top, 0.01));
        expect(args['x'], closeTo(rect.left, 0.01));
        final composing =
            tester.testTextInput.log
                    .lastWhere((c) => c.method == 'TextInput.setMarkedTextRect')
                    .arguments
                as Map;
        expect(composing['y'], closeTo(rect.top, 0.01));
        expect(composing['x'], closeTo(rect.left, 0.01));
        expect(client.currentTextEditingValue!.composing, value.composing);
      }

      final text = List.filled(18, '中文输入').join();
      await check(
        TextEditingValue(
          text: text,
          selection: TextSelection.collapsed(offset: text.length),
        ),
      );
      final composingText = '$text pinyin';
      await check(
        TextEditingValue(
          text: composingText,
          selection: TextSelection.collapsed(offset: composingText.length),
          composing: TextRange(
            start: text.length + 1,
            end: composingText.length,
          ),
        ),
      );
      tester.testTextInput.updateEditingValue(
        TextEditingValue(
          text: '$text 拼音',
          selection: TextSelection.collapsed(offset: text.length + 3),
        ),
      );
      await tester.pump();
      expect(client.currentTextEditingValue!.text, '$text 拼音');
      expect(client.currentTextEditingValue!.composing, TextRange.empty);
      await tester.pumpWidget(const SizedBox());
    },
  );
}
