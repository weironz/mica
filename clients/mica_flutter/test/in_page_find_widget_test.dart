import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

// End-to-end wiring for the Ctrl+F in-page find bar: the host opens it via
// EditorFindHook, the bar renders inside the editor, shows a live match count,
// next/prev navigate (wrapping), and close dismisses it.
void main() {
  Future<EditorFindHook> pumpEditor(
    WidgetTester tester,
    List<EditorNode> nodes,
  ) async {
    final findHook = EditorFindHook();
    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        locale: const Locale('zh'),
        home: Scaffold(
          body: MicaEditor(
            rootBlockId: 'root',
            nodes: nodes,
            version: 0,
            canEdit: true,
            onApplyOperations: (_) async {},
            findHook: findHook,
          ),
        ),
      ),
    );
    await tester.pump();
    return findHook;
  }

  testWidgets('opens, counts, navigates (wraps), and closes', (tester) async {
    final hook = await pumpEditor(tester, [
      EditorNode(id: 'a', kind: 'paragraph', text: 'hello world'),
      EditorNode(id: 'b', kind: 'paragraph', text: 'world of mica'),
    ]);

    expect(find.byType(TextField), findsNothing);

    hook.open();
    await tester.pump();
    await tester.pump();
    expect(find.byType(TextField), findsOneWidget);

    await tester.enterText(find.byType(TextField), 'world');
    await tester.pump();
    expect(find.text('1/2'), findsOneWidget);

    // Enter → next match (the primary keyboard path via onSubmitted).
    await tester.testTextInput.receiveAction(TextInputAction.search);
    await tester.pump();
    expect(find.text('2/2'), findsOneWidget);

    // The next button advances and wraps back to the first.
    await tester.tap(find.byTooltip('下一个 (Enter)'));
    await tester.pump();
    await tester.pump();
    expect(find.text('1/2'), findsOneWidget, reason: 'next wraps to the first');

    await tester.tap(find.byTooltip('关闭 (Esc)'));
    await tester.pump();
    expect(find.byType(TextField), findsNothing);
  });

  testWidgets('shows 无结果 when the query matches nothing', (tester) async {
    final hook = await pumpEditor(tester, [
      EditorNode(id: 'a', kind: 'paragraph', text: 'hello world'),
    ]);
    hook.open();
    await tester.pump();
    await tester.pump();
    await tester.enterText(find.byType(TextField), 'zzz');
    await tester.pump();
    expect(find.text('无结果'), findsOneWidget);
  });
}
