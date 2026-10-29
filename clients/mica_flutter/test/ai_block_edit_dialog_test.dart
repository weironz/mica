import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';

import 'support/markdown_codec.dart';
import 'support/title_workspace.dart';

void main() {
  setUpAll(useNativeMarkdownCodec);
  Future<void> open(WidgetTester tester) async {
    await tester.tap(find.text('问 AI'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('当前页面'));
    await tester.enterText(
      find.descendant(
        of: find.byType(AlertDialog),
        matching: find.byType(TextField),
      ),
      '只修改正文块',
    );
    await tester.tap(find.text('生成'));
    await tester.pumpAndSettle();
  }

  for (final fail in [false, true]) {
    testWidgets(
      fail
          ? 'invalid plan never enables applying'
          : 'current page previews block IDs then applies one undoable edit',
      (tester) async {
        tester.view.physicalSize = const Size(1400, 1000);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.resetPhysicalSize);
        addTearDown(tester.view.resetDevicePixelRatio);
        final writes = <List<Map<String, dynamic>>>[];
        EditorAiSnapshot? snapshot;
        String? sent;
        String? systemPrompt;
        var appended = false;
        await tester.pumpWidget(
          titleWorkspace(
            titleBootstrap('当前标题'),
            (_, _) async => true,
            onAiStream: (prompt, {system}) {
              sent = prompt;
              systemPrompt = system;
              return Stream.value(
                '{"edits":[{"block_ids":["body"],"markdown":"优化正文"}]}',
              );
            },
            onAiCurrentPage: (_) async {
              appended = true;
            },
            onAiPreviewEdits: (response, source) async {
              expect(jsonDecode(response)['edits'][0]['block_ids'], ['body']);
              snapshot = source;
              if (fail) throw StateError('Invalid block ID');
              return const AiEditPreview([
                AiBlockEdit(
                  blockIds: ['body'],
                  before: '正文',
                  after: '优化正文',
                  replacement: [
                    (
                      kind: 'paragraph',
                      text: '优化正文',
                      data: <String, dynamic>{},
                    ),
                  ],
                ),
              ]);
            },
            onApplyOperations: (ops) async {
              writes.add(ops);
            },
          ),
        );
        await tester.pumpAndSettle();
        await open(tester);
        expect(jsonDecode(sent!)['current_page']['blocks'][0]['id'], 'body');
        expect(snapshot!.nodes.single.text, '正文');
        expect(systemPrompt, contains('exact source IDs'));
        expect(
          writes,
          isEmpty,
          reason: 'generation/preview do not mutate the page',
        );
        expect(
          find.textContaining('"block_ids"'),
          findsNothing,
          reason: 'raw model JSON is not exposed in the preview',
        );
        if (fail) {
          expect(find.text('应用修改'), findsNothing);
          expect(find.textContaining('Invalid block ID'), findsOneWidget);
          await tester.tap(find.text('取消'));
        } else {
          expect(find.text('原文'), findsOneWidget);
          expect(find.text('修改后'), findsOneWidget);
          expect(find.text('优化正文'), findsOneWidget);
          await tester.tap(find.text('应用修改'));
          await tester.pumpAndSettle();
          expect(appended, isFalse);
          expect(writes.single.single['block_id'], 'body');
          expect(writes.single.single['text'], '优化正文');
          await tester.tapAt(
            tester.getTopLeft(find.byType(MicaEditor)) + const Offset(40, 14),
          );
          await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
          await tester.sendKeyEvent(LogicalKeyboardKey.keyZ);
          await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
          await tester.pumpAndSettle();
          expect(writes.last.single['text'], '正文');
        }
        await tester.pumpAndSettle();
      },
    );
  }

  testWidgets('save failure keeps the preview open and reports the failure', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1400, 1000);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    await tester.pumpWidget(
      titleWorkspace(
        titleBootstrap('标题'),
        (_, _) async => true,
        onAiStream: (_, {system}) => Stream.value('{}'),
        onAiCurrentPage: (_) async {},
        onAiPreviewEdits: (_, _) async => const AiEditPreview([
          AiBlockEdit(
            blockIds: ['body'],
            before: '正文',
            after: '替换内容',
            replacement: [
              (kind: 'paragraph', text: '替换内容', data: <String, dynamic>{}),
            ],
          ),
        ]),
        onApplyOperations: (_) async {
          throw StateError('storage unavailable');
        },
      ),
    );
    await tester.pumpAndSettle();
    await open(tester);
    await tester.tap(find.text('应用修改'));
    await tester.pumpAndSettle();
    expect(find.byType(AlertDialog), findsOneWidget);
    expect(find.textContaining('保存未完成'), findsOneWidget);
    await tester.tap(find.text('取消'));
    await tester.pumpAndSettle();
  });

  testWidgets('switching destination discards the generated block plan', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1400, 1000);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    await tester.pumpWidget(
      titleWorkspace(
        titleBootstrap('标题'),
        (_, _) async => true,
        onAiStream: (_, {system}) => Stream.value('{}'),
        onAiCurrentPage: (_) async {},
        onAiPreviewEdits: (_, _) async => const AiEditPreview([
          AiBlockEdit(
            blockIds: ['body'],
            before: '正文',
            after: '替换内容',
            replacement: [
              (kind: 'paragraph', text: '替换内容', data: <String, dynamic>{}),
            ],
          ),
        ]),
      ),
    );
    await tester.pumpAndSettle();
    await open(tester);
    expect(find.text('应用修改'), findsOneWidget);
    await tester.tap(find.text('新建页面'));
    await tester.pumpAndSettle();
    expect(find.text('替换内容'), findsNothing);
    expect(find.text('插入'), findsNothing);
    expect(find.text('生成'), findsOneWidget);
    await tester.tap(find.text('取消'));
    await tester.pumpAndSettle();
  });
}
