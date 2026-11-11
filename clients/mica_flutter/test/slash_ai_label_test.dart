import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

/// The AI entry of the slash menu shows the localized label (问 AI in the
/// Chinese UI), never the built-in English fallback, and stays findable by
/// both its Chinese and English names.
void main() {
  Future<TextInputClient> pump(WidgetTester tester) async {
    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        locale: const Locale('zh'),
        home: Scaffold(
          body: MicaEditor(
            rootBlockId: 'root',
            nodes: [EditorNode(id: 'a', kind: 'paragraph', text: '')],
            version: 0,
            canEdit: true,
            onApplyOperations: (_) async {},
            onAiStream: (prompt, {system}) => const Stream.empty(),
          ),
        ),
      ),
    );
    await tester.pump();
    await tester.tap(find.byType(MicaEditor));
    await tester.pump();
    return tester.state(find.byType(MicaEditor)) as TextInputClient;
  }

  void type(TextInputClient s, String text) => s.updateEditingValue(
    TextEditingValue(
      text: text,
      selection: TextSelection.collapsed(offset: text.length),
    ),
  );

  testWidgets('zh slash menu labels the AI entry 问 AI', (tester) async {
    final state = await pump(tester);
    type(state, '/');
    await tester.pump();
    expect(find.text('问 AI'), findsOneWidget);
    expect(find.text('Ask AI'), findsNothing);
  });

  testWidgets('the AI entry matches its English name in the zh UI', (
    tester,
  ) async {
    final state = await pump(tester);
    type(state, '/ask');
    await tester.pump();
    expect(find.text('问 AI'), findsOneWidget);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pump();
  });
}
