import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/ai_edits.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/markdown/markdown_codec.dart';
import 'package:mica_flutter/ui/page_title_field.dart';

import 'support/markdown_codec.dart';
import 'support/title_workspace.dart';

void main() {
  setUpAll(useNativeMarkdownCodec);
  testWidgets('current-page AI sees live edits and title before insertion', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1400, 1000);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    String? sentPrompt;
    String? sentSystem;
    String? inserted;
    await tester.pumpWidget(
      titleWorkspace(
        titleBootstrap('Infisical 介绍'),
        (_, _) async => true,
        onAiStream: (prompt, {system}) {
          sentPrompt = prompt;
          sentSystem = system;
          return Stream.value('优化后的正文');
        },
        onAiCurrentPage: (markdown) async => inserted = markdown,
      ),
    );
    await tester.pumpAndSettle();
    await tester.enterText(
      find.descendant(
        of: find.byType(PageTitleField),
        matching: find.byType(TextField),
      ),
      'Infisical 实时标题',
    );
    // A local edit has not been echoed in a new bootstrap. Reading the host's
    // snapshot or exporting from the server would still return only 正文.
    await tester.tapAt(
      tester.getTopLeft(find.byType(MicaEditor)) + const Offset(40, 14),
    );
    await tester.sendKeyEvent(LogicalKeyboardKey.end);
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: '正文：尚未同步的 Ansible 配置',
        selection: TextSelection.collapsed(offset: 19),
      ),
    );
    await tester.pump();
    await tester.tap(find.text('问 AI'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('当前页面'));
    await tester.enterText(
      find.descendant(
        of: find.byType(AlertDialog),
        matching: find.byType(TextField),
      ),
      '优化下页面markdown语法格式排版问题',
    );
    await tester.tap(find.text('生成'));
    await tester.pumpAndSettle();
    expect(sentPrompt, contains('优化下页面markdown语法格式排版问题'));
    final request = jsonDecode(sentPrompt!) as Map<String, dynamic>;
    expect(request['current_page']['title'], 'Infisical 实时标题');
    expect(request['current_page']['markdown'], contains('尚未同步的 Ansible 配置'));
    expect(sentSystem, contains('current page'));
    expect(inserted, isNull, reason: 'generation only previews the result');
    await tester.tap(find.text('插入'));
    await tester.pumpAndSettle();
    expect(inserted, '优化后的正文');
  });

  for (final target in ['新建页面', '新建工作区']) {
    testWidgets('$target does not attach current-page content', (tester) async {
      tester.view.physicalSize = const Size(1400, 1000);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      String? sent;
      String? systemPrompt;
      await tester.pumpWidget(
        titleWorkspace(
          titleBootstrap('Private title'),
          (_, _) async => true,
          onAiStream: (prompt, {system}) {
            sent = prompt;
            systemPrompt = system;
            return Stream.value('新文档');
          },
          onAiCurrentPage: (_) async {},
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('问 AI'));
      await tester.pumpAndSettle();
      await tester.tap(find.text(target));
      await tester.enterText(
        find.descendant(
          of: find.byType(AlertDialog),
          matching: find.byType(TextField),
        ),
        '写一篇新文档',
      );
      await tester.tap(find.text('生成'));
      await tester.pumpAndSettle();
      expect(sent, '写一篇新文档');
      expect(systemPrompt, contains('level-1 heading'));
      await tester.tap(find.text('取消'));
      await tester.pumpAndSettle();
    });
  }

  // Before S3 the page body was pure Dart and could not fail; through the
  // engine it can (Web without its wasm). An empty string would make the AI
  // answer about — and propose edits to — a page it believes is empty.
  test(
    'documentMarkdown never stands in an empty body for a missing engine',
    () {
      final controller = EditorController(
        rootBlockId: 'root',
        onOps: (_) async {},
      )..load([EditorNode(id: 'p', kind: 'paragraph', text: '正文')]);
      addTearDown(controller.dispose);
      debugSetMarkdownCodec(const UnavailableMarkdownCodec());
      addTearDown(useNativeMarkdownCodec);
      expect(
        controller.documentMarkdown,
        throwsA(isA<MarkdownCodecException>()),
      );
    },
  );

  testWidgets(
    'without the engine the AI dialog offers no current-page target',
    (tester) async {
      tester.view.physicalSize = const Size(1400, 1000);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      final sent = <String>[];
      await tester.pumpWidget(
        titleWorkspace(
          titleBootstrap('Infisical 介绍'),
          (_, _) async => true,
          onAiStream: (prompt, {system}) {
            sent.add(prompt);
            return Stream.value('x');
          },
          onAiCurrentPage: (_) async {},
          onAiPreviewEdits: (_, _) async => throw const AiEditConflict(),
        ),
      );
      await tester.pumpAndSettle();
      debugSetMarkdownCodec(const UnavailableMarkdownCodec());
      addTearDown(useNativeMarkdownCodec);
      await tester.tap(find.text('问 AI'));
      await tester.pumpAndSettle();
      expect(find.text('当前页面'), findsNothing);
      expect(find.text('新建页面'), findsOneWidget, reason: 'other targets stay');
      expect(sent, isEmpty);
      await tester.tap(find.text('取消'));
      await tester.pumpAndSettle();
    },
  );
}
