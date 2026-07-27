import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/emoji_picker.dart';

// The picker's contract is a THREE-way result, and that is what most of these
// pin: an emoji sets the icon, `''` clears it, null changes nothing. Collapsing
// `''` into null (the obvious "no value" reading) would silently turn "remove
// this page's icon" into a no-op, so the two are asserted apart rather than
// through a single `isNull`/`isNotNull` check.

const _strings = EmojiPickerStrings(
  title: '选择图标',
  searchHint: '搜索表情或关键词',
  removeIcon: '移除图标',
  noResultsTitle: '没有匹配的表情',
  noResultsBody: '换一个关键词试试，或者清空搜索框从分类里挑。',
  categorySmileys: '表情',
  categoryPeople: '人物',
  categoryNature: '自然',
  categoryFood: '食物',
  categoryObjects: '物品',
  categorySymbols: '符号',
  categoryFlags: '旗帜',
);

/// Captures how the future resolved. `completed` is separate from `value`
/// because "resolved with null" and "still open" are both `value == null`.
class _Outcome {
  bool completed = false;
  String? value;
}

Finder _cell(String emoji) => find.byKey(ValueKey('emoji-cell-$emoji'));

BoxDecoration _decoOf(WidgetTester tester, String emoji) =>
    tester.widget<Container>(_cell(emoji)).decoration! as BoxDecoration;

void main() {
  Future<_Outcome> openPicker(WidgetTester tester, {String? current}) async {
    final outcome = _Outcome();
    await tester.pumpWidget(
      MaterialApp(
        home: Builder(
          builder: (context) => Center(
            child: TextButton(
              onPressed: () async {
                outcome.value = await showEmojiPicker(
                  context,
                  strings: _strings,
                  current: current,
                );
                outcome.completed = true;
              },
              child: const Text('open'),
            ),
          ),
        ),
      ),
    );
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
    return outcome;
  }

  testWidgets('tapping an emoji resolves with that emoji', (tester) async {
    final outcome = await openPicker(tester);
    // First glyph of the first section, so it is on screen without scrolling.
    await tester.tap(_cell('😀'));
    await tester.pumpAndSettle();

    expect(outcome.completed, isTrue);
    expect(outcome.value, '😀');
  });

  testWidgets('remove icon resolves with the empty string, not null', (
    tester,
  ) async {
    final outcome = await openPicker(tester, current: '😀');
    expect(find.text('移除图标'), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('emoji-picker-remove')));
    await tester.pumpAndSettle();

    expect(outcome.completed, isTrue);
    // The whole point: '' means CLEAR, and must survive as a value.
    expect(outcome.value, isNotNull);
    expect(outcome.value, '');
  });

  testWidgets('Esc resolves with null — nothing chosen, icon untouched', (
    tester,
  ) async {
    final outcome = await openPicker(tester, current: '😀');
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();

    expect(outcome.completed, isTrue);
    expect(outcome.value, isNull);
  });

  testWidgets('typing filters the grid down to matching emoji', (tester) async {
    await openPicker(tester);
    expect(_cell('😀'), findsOneWidget);

    // Chinese keyword: the UI's primary language, so it must hit.
    await tester.enterText(find.byType(TextField), '火箭');
    await tester.pumpAndSettle();

    expect(_cell('🚀'), findsOneWidget);
    expect(_cell('😀'), findsNothing);
    // Sections with no hits disappear along with their header.
    expect(find.text('物品'), findsOneWidget);
    expect(find.text('表情'), findsNothing);

    // English keyword on the same glyph.
    await tester.enterText(find.byType(TextField), 'rocket');
    await tester.pumpAndSettle();
    expect(_cell('🚀'), findsOneWidget);
  });

  testWidgets('Enter picks the first filtered result', (tester) async {
    final outcome = await openPicker(tester);
    await tester.enterText(find.byType(TextField), '火箭');
    await tester.pumpAndSettle();
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();

    expect(outcome.value, '🚀');
  });

  testWidgets('a search that matches nothing shows the empty state', (
    tester,
  ) async {
    await openPicker(tester);
    await tester.enterText(find.byType(TextField), 'zzzzz');
    await tester.pumpAndSettle();

    expect(find.text('没有匹配的表情'), findsOneWidget);
    expect(find.text('换一个关键词试试，或者清空搜索框从分类里挑。'), findsOneWidget);
    expect(_cell('😀'), findsNothing);
    // An empty grid must not throw — it is a normal state, not a failure.
    expect(tester.takeException(), isNull);

    // Enter with nothing to pick is a no-op rather than a crash or a wrong pick.
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(find.text('没有匹配的表情'), findsOneWidget);
  });

  testWidgets('the current emoji is visibly marked', (tester) async {
    await openPicker(tester, current: '😀');

    final selected = _decoOf(tester, '😀');
    expect(selected.border, isNotNull);
    expect(selected.color, isNotNull);

    final other = _decoOf(tester, '😃');
    expect(other.border, isNull);
    expect(other.color, isNull);
  });

  testWidgets('with no current icon nothing is marked', (tester) async {
    await openPicker(tester);
    expect(_decoOf(tester, '😀').border, isNull);
  });
}
