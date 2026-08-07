import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

// Regression: the find bar lives INSIDE the editor's Focus, so its key events
// bubble up to the editor's key handler — and every branch there edits the
// document. Before the fix, Backspace in the find field deleted document text
// and Ctrl+V pasted over the current match (the match IS the document
// selection), while the user believed they were editing the query.
//
// Both assertions matter and neither alone is enough:
//   - "no document ops" proves the document was not silently mutated;
//   - "the query changed" proves the key actually reached the field, rather
//     than being swallowed by a blanket `return handled`.
void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  // The editor's own Ctrl+V branch is desktop-only (`!kIsWeb`) and the field's
  // paste comes from DefaultTextEditingShortcuts, which is platform-keyed — so
  // pin the platform. Via the variant, not debugDefaultTargetPlatformOverride
  // in setUp/tearDown: the binding asserts that foundation debug vars are unset
  // when the test BODY ends, which is before any tearDown runs.
  final windows = TargetPlatformVariant.only(TargetPlatform.windows);

  /// Pumps an editor over one paragraph and returns (find hook, captured ops).
  /// Ops are debounced by the controller — pump past 400ms before asserting.
  Future<(EditorFindHook, List<Object?>)> pumpEditor(
    WidgetTester tester,
    String text,
  ) async {
    final findHook = EditorFindHook();
    final ops = <Object?>[];
    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        locale: const Locale('zh'),
        home: Scaffold(
          body: MicaEditor(
            rootBlockId: 'root',
            nodes: [EditorNode(id: 'a', kind: 'paragraph', text: text)],
            version: 0,
            canEdit: true,
            onApplyOperations: (o) async => ops.addAll(o),
            findHook: findHook,
          ),
        ),
      ),
    );
    await tester.pump();
    return (findHook, ops);
  }

  /// Opens the bar and seeds a query, returning the find field. Two pumps: the
  /// focus request lands in a post-frame callback.
  Future<TextField> openFindWith(
    WidgetTester tester,
    EditorFindHook hook,
    String query,
  ) async {
    hook.open();
    await tester.pump();
    await tester.pump();
    await tester.enterText(find.byType(TextField), query);
    await tester.pump();
    return tester.widget<TextField>(find.byType(TextField));
  }

  testWidgets('Backspace edits the query, not the document', (tester) async {
    final (hook, ops) = await pumpEditor(tester, 'hello world');
    final field = await openFindWith(tester, hook, 'world');
    expect(find.text('1/1'), findsOneWidget, reason: 'the match is selected');

    await tester.sendKeyEvent(LogicalKeyboardKey.backspace);
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 600)); // past the op debounce

    expect(field.controller!.text, 'worl', reason: 'Backspace hit the field');
    expect(ops, isEmpty, reason: 'the document must not be touched');
  }, variant: windows);

  testWidgets('Ctrl+V pastes into the query, not the document', (tester) async {
    final (hook, ops) = await pumpEditor(tester, 'hello world');
    // The field's paste asks the platform for the clipboard; answer it.
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      (call) async => call.method == 'Clipboard.getData'
          ? <String, dynamic>{'text': 'XY'}
          : null,
    );
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform,
        null,
      ),
    );
    final field = await openFindWith(tester, hook, 'world');

    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyV);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 600));

    expect(field.controller!.text, contains('XY'), reason: 'pasted into field');
    expect(ops, isEmpty, reason: 'the document must not be touched');
  }, variant: windows);

  testWidgets('with the bar closed the editor still owns Backspace', (
    tester,
  ) async {
    // The guard is scoped to `_findOpen` — prove it did not disarm the editor.
    final (_, ops) = await pumpEditor(tester, 'hello world');
    await tester.tapAt(tester.getCenter(find.byType(MicaEditor)));
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.backspace);
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 600));

    expect(ops, isNotEmpty, reason: 'Backspace still edits the document');
  }, variant: windows);
}
