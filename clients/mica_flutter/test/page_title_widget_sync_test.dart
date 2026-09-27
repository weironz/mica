import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'support/title_workspace.dart';

void main() {
  final titleField = find.byWidgetPredicate(
    (w) => w is TextField && w.focusNode?.debugLabel == 'PageTitle',
  );
  TextEditingController controller(WidgetTester tester) =>
      tester.widget<TextField>(titleField).controller!;

  for (final name in ['Untitled', '未命名页面']) {
    testWidgets('placeholder name $name acknowledges a successful save', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(1440, 1000);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      Future<bool> save(_, String title) async => true;
      await tester.pumpWidget(titleWorkspace(titleBootstrap('旧标题'), save));
      await tester.enterText(titleField, name);
      await tester.pump(const Duration(milliseconds: 700));
      await tester.pumpWidget(titleWorkspace(titleBootstrap(name), save));
      expect(controller(tester).text, name);
      await tester.pumpWidget(titleWorkspace(titleBootstrap('远端改名'), save));
      expect(controller(tester).text, '远端改名');
      await tester.pumpWidget(const SizedBox());
    });
  }

  testWidgets('an older failed save cannot release a newer pending edit', (
    tester,
  ) async {
    final result = Completer<bool>();
    await tester.pumpWidget(
      titleWorkspace(titleBootstrap('旧标题'), (_, _) => result.future),
    );
    await tester.enterText(titleField, '第一段');
    await tester.pump(const Duration(milliseconds: 700));
    await tester.enterText(titleField, '第一段后续');
    result.complete(false);
    await tester.pump();
    await tester.pumpWidget(
      titleWorkspace(
        titleBootstrap('第一段'),
        (_, _) async => true,
        message: '无关请求失败',
      ),
    );
    expect(controller(tester).text, '第一段后续');
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('late save echo preserves newer input, caret and composition', (
    tester,
  ) async {
    final saves = <String>[];
    Future<bool> save(_, String title) async {
      saves.add(title);
      return true;
    }

    await tester.pumpWidget(titleWorkspace(titleBootstrap('旧标题'), save));
    await tester.enterText(titleField, '第一段');
    await tester.pump(const Duration(milliseconds: 700));
    expect(saves, ['第一段']);
    await tester.enterText(titleField, '第一段后续');
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: '第一段后续',
        selection: TextSelection.collapsed(offset: 5),
        composing: TextRange(start: 3, end: 5),
      ),
    );
    await tester.pumpWidget(titleWorkspace(titleBootstrap('第一段'), save));
    expect(controller(tester).text, '第一段后续');
    expect(controller(tester).selection.baseOffset, 5);
    expect(
      controller(tester).value.composing,
      const TextRange(start: 3, end: 5),
    );
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets(
    'trimmed acknowledgement releases pending for an external rename',
      (tester) async {
        tester.view.physicalSize = const Size(1440, 1000);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.resetPhysicalSize);
        addTearDown(tester.view.resetDevicePixelRatio);
        Future<bool> save(_, String title) async => true;
      await tester.pumpWidget(titleWorkspace(titleBootstrap('旧标题'), save));
      await tester.enterText(titleField, ' 新标题 ');
      await tester.pump(const Duration(milliseconds: 700));
      await tester.pumpWidget(titleWorkspace(titleBootstrap('新标题'), save));
      expect(controller(tester).text, ' 新标题 ');
      await tester.pumpWidget(titleWorkspace(titleBootstrap('远端改名'), save));
      expect(controller(tester).text, '远端改名');
      await tester.pumpWidget(const SizedBox());
    },
  );

  testWidgets('switching pages flushes the departing title once', (
    tester,
  ) async {
    final saves = <String>[];
    Future<bool> save(view, String title) async {
      saves.add('${view.id}:$title');
      return true;
    }

    await tester.pumpWidget(titleWorkspace(titleBootstrap('旧标题'), save));
    await tester.enterText(titleField, '未等到保存');
    await tester.pump(const Duration(milliseconds: 100));
    await tester.pumpWidget(
      titleWorkspace(titleBootstrap('下一页', id: 'page-b'), save),
    );
    expect(saves, ['page-a:未等到保存']);
    expect(controller(tester).text, '下一页');
    await tester.pump(const Duration(seconds: 1));
    expect(saves, ['page-a:未等到保存']);
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('a reported save failure permits subsequent external rename', (
    tester,
  ) async {
    Future<bool> save(_, String title) async => false;
    await tester.pumpWidget(titleWorkspace(titleBootstrap('旧标题'), save));
    await tester.enterText(titleField, '保存失败的标题');
    await tester.pump(const Duration(milliseconds: 700));
    await tester.pumpWidget(
      titleWorkspace(titleBootstrap('旧标题'), save, message: '保存失败'),
    );
    expect(controller(tester).text, '保存失败的标题');
    await tester.pumpWidget(
      titleWorkspace(titleBootstrap('远端改名'), save, message: '保存失败'),
    );
    expect(controller(tester).text, '远端改名');
    await tester.pumpWidget(const SizedBox());
  });
}
