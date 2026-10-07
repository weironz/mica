import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/ui/page_title_field.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

void main() {
  testWidgets(
    'a long page title wraps instead of hiding the rest horizontally',
    (tester) async {
      final controller = TextEditingController(
        text:
            'A long document title that needs several lines on a narrow screen',
      );
      final focus = FocusNode();
      addTearDown(controller.dispose);
      addTearDown(focus.dispose);
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: SizedBox(
              width: 280,
              child: PageTitleField(
                controller: controller,
                focusNode: focus,
                appearance: const EditorAppearance(),
                hintText: 'Untitled',
                onChanged: (_) {},
                onEnterBody: () {},
                onArrowDown: () {},
              ),
            ),
          ),
        ),
      );
      final titleHeight = tester.getSize(find.byType(TextField)).height;
      expect(titleHeight, greaterThan(100));
      expect(tester.takeException(), isNull);
      controller.text = 'Short title';
      await tester.pump();
      expect(
        tester.getSize(find.byType(TextField)).height,
        lessThan(titleHeight),
      );
      await tester.pumpWidget(const SizedBox());
    },
  );

  testWidgets(
    'Enter enters the body; Down does not split the title; IME confirms',
    (tester) async {
      final controller = TextEditingController(text: 'Title');
      final focus = FocusNode();
      var enters = 0;
      var downs = 0;
      addTearDown(controller.dispose);
      addTearDown(focus.dispose);
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: PageTitleField(
              controller: controller,
              focusNode: focus,
              appearance: const EditorAppearance(tokens: MicaTokens.dark_),
              hintText: 'Untitled',
              onChanged: (_) {},
              onEnterBody: () => enters++,
              onArrowDown: () => downs++,
            ),
          ),
        ),
      );
      focus.requestFocus();
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      expect(enters, 1);
      expect(
        controller.text,
        'Title',
        reason: 'Enter must not insert a title newline',
      );
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
      expect(downs, 1);
      expect(enters, 1);

      tester.testTextInput.updateEditingValue(
        const TextEditingValue(
          text: '标题',
          selection: TextSelection.collapsed(offset: 2),
          composing: TextRange(start: 0, end: 2),
        ),
      );
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      expect(
        enters,
        1,
        reason: 'IME candidate confirmation must retain title focus',
      );
      expect(focus.hasFocus, isTrue);
      await tester.testTextInput.receiveAction(TextInputAction.next);
      expect(
        enters,
        1,
        reason: 'software IME action must not split a live preedit',
      );
      expect(focus.hasFocus, isTrue);
      tester.testTextInput.updateEditingValue(
        const TextEditingValue(
          text: '标题',
          selection: TextSelection.collapsed(offset: 2),
        ),
      );
      await tester.pump();
      await tester.testTextInput.receiveAction(TextInputAction.next);
      expect(enters, 2, reason: 'software keyboard next enters the body');
      await tester.pumpWidget(const SizedBox());
    },
  );

  testWidgets('read-only page titles do not run edit commands', (tester) async {
    final controller = TextEditingController(text: 'Read only');
    final focus = FocusNode();
    var commands = 0;
    addTearDown(controller.dispose);
    addTearDown(focus.dispose);
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: PageTitleField(
            controller: controller,
            focusNode: focus,
            appearance: const EditorAppearance(),
            hintText: 'Untitled',
            readOnly: true,
            onChanged: (_) {},
            onEnterBody: () => commands++,
            onArrowDown: () => commands++,
          ),
        ),
      ),
    );
    focus.requestFocus();
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
    expect(commands, 0);
    expect(controller.text, 'Read only');
    await tester.pumpWidget(const SizedBox());
  });
}
