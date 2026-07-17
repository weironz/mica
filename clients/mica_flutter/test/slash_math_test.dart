import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

/// Reproduces the slash-menu "Math formula" flow end to end:
/// type `/math`, apply the option, fill the LaTeX dialog, press OK —
/// the block must end up as a `math_block` carrying the source.
void main() {
  testWidgets('slash Math formula writes the LaTeX source', (tester) async {
    final ops = <List<Map<String, dynamic>>>[];
    final nodes = [EditorNode(id: 'a', kind: 'paragraph', text: '')];

    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        locale: const Locale('en'),
        home: Scaffold(
          body: MicaEditor(
            rootBlockId: 'root',
            nodes: nodes,
            version: 0,
            canEdit: true,
            onApplyOperations: (batch) async {
              ops.add(List<Map<String, dynamic>>.from(batch));
            },
          ),
        ),
      ),
    );
    await tester.pump();

    // Focus the editor surface and open the slash menu by typing "/math".
    await tester.tap(find.byType(MicaEditor));
    await tester.pump();

    final state = tester.state(find.byType(MicaEditor)) as TextInputClient;
    state.updateEditingValue(
      const TextEditingValue(
        text: '/math',
        selection: TextSelection.collapsed(offset: 5),
      ),
    );
    await tester.pump();

    // The slash overlay should list "Math formula"; apply it with Enter.
    expect(find.text('Math formula'), findsOneWidget,
        reason: 'slash menu should be open and filtered to the math entry');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();

    // The LaTeX source dialog opens; type a formula and confirm. Users
    // habitually include the $$ delimiters — they must be stripped on save.
    expect(find.byType(AlertDialog), findsOneWidget,
        reason: 'math source dialog should open');
    await tester.enterText(find.byType(TextField).last, r'$$E = mc^2$$');
    await tester.tap(find.text('OK'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));

    // Let any pending IME echo / post-frame callbacks run.
    await tester.pump(const Duration(milliseconds: 400));

    // The op stream must carry the math_block conversion + the text.
    final flat = ops.expand((b) => b).toList();
    final trace = flat.map((o) => '${o['type']}:${o['kind']}/"${o['text']}"');
    // ignore: avoid_print
    print('OPS: ${trace.join('  |  ')}');

    expect(
      flat.any((o) => o['kind'] == 'math_block'),
      isTrue,
      reason: 'the slash apply must convert the block to math_block',
    );
    final last = flat.lastWhere((o) => o['type'] == 'update_block');
    expect(last['text'], r'E = mc^2',
        reason: 'the dialog OK must persist the BARE LaTeX source — '
            '\$\$ delimiters stripped (the typesetter cannot parse them)');
    expect(last['kind'] ?? 'math_block', 'math_block',
        reason: 'nothing may revert the block back to a paragraph');
    expect(
      flat.any((o) =>
          o['type'] == 'insert_block' &&
          (o['block'] as Map?)?['type'] == 'paragraph'),
      isTrue,
      reason: 'the caret must park on a paragraph after the atomic block '
          'so IME echoes cannot clobber it',
    );
  });

  testWidgets('clicking the Math formula slash item also works', (tester) async {
    final ops = <List<Map<String, dynamic>>>[];
    final nodes = [EditorNode(id: 'a', kind: 'paragraph', text: '')];

    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        locale: const Locale('en'),
        home: Scaffold(
          body: MicaEditor(
            rootBlockId: 'root',
            nodes: nodes,
            version: 0,
            canEdit: true,
            onApplyOperations: (batch) async {
              ops.add(List<Map<String, dynamic>>.from(batch));
            },
          ),
        ),
      ),
    );
    await tester.pump();

    await tester.tap(find.byType(MicaEditor));
    await tester.pump();

    final state = tester.state(find.byType(MicaEditor)) as TextInputClient;
    state.updateEditingValue(
      const TextEditingValue(
        text: '/math',
        selection: TextSelection.collapsed(offset: 5),
      ),
    );
    await tester.pump();

    // Click the menu entry with the mouse instead of pressing Enter.
    expect(find.text('Math formula'), findsOneWidget);
    await tester.tap(find.text('Math formula'));
    await tester.pump();

    expect(find.byType(AlertDialog), findsOneWidget,
        reason: 'math source dialog should open from the click path');
    await tester.enterText(find.byType(TextField).last, r'a^2+b^2=c^2');
    await tester.tap(find.text('OK'));
    await tester.pump(const Duration(milliseconds: 400));

    final flat = ops.expand((b) => b).toList();
    final trace = flat.map((o) => '${o['type']}:${o['kind']}/"${o['text']}"');
    // ignore: avoid_print
    print('OPS(click): ${trace.join('  |  ')}');

    expect(flat.any((o) => o['kind'] == 'math_block'), isTrue,
        reason: 'click apply must convert the block to math_block');
    final last = flat.lastWhere((o) => o['type'] == 'update_block');
    expect(last['text'], r'a^2+b^2=c^2');
  });

  testWidgets('typing the full "/math formula" label keeps the menu open',
      (tester) async {
    final nodes = [EditorNode(id: 'a', kind: 'paragraph', text: '')];

    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        locale: const Locale('en'),
        home: Scaffold(
          body: MicaEditor(
            rootBlockId: 'root',
            nodes: nodes,
            version: 0,
            canEdit: true,
            onApplyOperations: (_) async {},
          ),
        ),
      ),
    );
    await tester.pump();
    await tester.tap(find.byType(MicaEditor));
    await tester.pump();

    final state = tester.state(find.byType(MicaEditor)) as TextInputClient;
    // The menu item is labeled "Math formula" — typing its full name crosses
    // a space; the menu must survive as long as the query still matches.
    state.updateEditingValue(
      const TextEditingValue(
        text: '/math formula',
        selection: TextSelection.collapsed(offset: 13),
      ),
    );
    await tester.pump();

    expect(find.text('Math formula'), findsOneWidget,
        reason: 'menu must stay open while the spaced query still matches');
  });

  test('a stale IME echo cannot clobber an atomic block', () {
    final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
    c.load([
      EditorNode(id: 'm', kind: 'math_block', text: r'E = mc^2'),
      EditorNode(id: 'p', kind: 'paragraph', text: ''),
    ]);
    // Selection stuck inside the atomic block (e.g. a focus/echo race) —
    // the echoed text must be dropped, not written into the formula.
    c.selection = DocSelection.collapsed(const DocPosition(0, 0));
    c.setFocusedText('/math', 5, 5);
    expect(c.nodes[0].text, r'E = mc^2');
    expect(c.nodes[0].kind, 'math_block');
  });

  test('applySlashCommand to an atomic kind parks the caret after it', () {
    final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
    c.load([EditorNode(id: 'a', kind: 'paragraph', text: '/math')]);
    c.selection = DocSelection.collapsed(const DocPosition(0, 5));
    c.applySlashCommand(0, 5, 'math_block', {});
    expect(c.nodes[0].kind, 'math_block');
    expect(c.nodes.length, 2, reason: 'a trailing paragraph is created');
    expect(c.nodes[1].kind, 'paragraph');
    expect(c.selection!.focus.node, 1,
        reason: 'the caret must not live inside the atomic block');
  });
}
