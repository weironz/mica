import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

// Reported 2026-08-28, with a screenshot: caret on the blank line right under
// the page title, Backspace pressed, "空行依然在原地" — the line stayed put and
// the body never rose. It was not a dead key: `mergeBackward` bailed (nothing
// above the FIRST block to merge into) and the handler quietly moved focus to
// the page title, leaving the blank line behind. Nothing about that is visible
// from the body, so the whole gesture read as "delete does not work here".
//
// The widget tests are the point: a controller-only test cannot see the branch
// that was wrong, because the branch lives in the key handler.
void main() {
  Future<(List<Map<String, dynamic>>, bool Function())> pump(
    WidgetTester tester,
    List<EditorNode> nodes, {
    bool withTitle = true,
  }) async {
    final ops = <Map<String, dynamic>>[];
    var exitedTop = false;
    final commands = EditorCommandHook();
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
            onApplyOperations: (o) async => ops.addAll(o),
            commandHook: commands,
            // Null when the host hides the page title — there is then no line
            // above to step into.
            onExitTop: withTitle ? () => exitedTop = true : null,
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    // Exactly what ArrowDown from the title does: focus the body, caret at the
    // start of the first line.
    commands.focusFirstLine();
    await tester.pump();
    return (ops, () => exitedTop);
  }

  testWidgets('Backspace on the blank first line removes it and rises to the '
      'title', (tester) async {
    final (ops, exitedTop) = await pump(tester, [
      EditorNode(id: 'blank', kind: 'paragraph', text: ''),
      EditorNode(id: 'link', kind: 'paragraph', text: 'https://example.com'),
    ]);

    await tester.sendKeyEvent(LogicalKeyboardKey.backspace);
    await tester.pumpAndSettle();

    expect(
      ops.where((o) => o['type'] == 'delete_block' && o['block_id'] == 'blank'),
      hasLength(1),
      reason: 'the blank line is gone from the document, not just from view',
    );
    expect(exitedTop(), isTrue, reason: 'the caret continues into the title');
  });

  testWidgets('a blank first line that is the ONLY block survives',
      (tester) async {
    // A document always keeps one block; deleting this one would leave nothing
    // to type into. The caret still steps up into the title.
    final (ops, exitedTop) = await pump(tester, [
      EditorNode(id: 'only', kind: 'paragraph', text: ''),
    ]);

    await tester.sendKeyEvent(LogicalKeyboardKey.backspace);
    await tester.pumpAndSettle();

    expect(ops.where((o) => o['type'] == 'delete_block'), isEmpty);
    expect(exitedTop(), isTrue);
  });

  testWidgets('a first line WITH text is left alone', (tester) async {
    // Backspace at the start of a non-empty first line must not eat the line —
    // it only moves the caret up into the title.
    final (ops, exitedTop) = await pump(tester, [
      EditorNode(id: 'p', kind: 'paragraph', text: 'keep me'),
      EditorNode(id: 'q', kind: 'paragraph', text: 'below'),
    ]);

    await tester.sendKeyEvent(LogicalKeyboardKey.backspace);
    await tester.pumpAndSettle();

    expect(ops.where((o) => o['type'] == 'delete_block'), isEmpty);
    expect(exitedTop(), isTrue);
  });

  testWidgets('with the page title hidden the blank line still goes',
      (tester) async {
    // No title to step into: the caret stays at the top of the body, on the
    // line that rose into the gap.
    final (ops, _) = await pump(tester, [
      EditorNode(id: 'blank', kind: 'paragraph', text: ''),
      EditorNode(id: 'link', kind: 'paragraph', text: 'https://example.com'),
    ], withTitle: false);

    await tester.sendKeyEvent(LogicalKeyboardKey.backspace);
    await tester.pumpAndSettle();

    expect(
      ops.where((o) => o['type'] == 'delete_block' && o['block_id'] == 'blank'),
      hasLength(1),
    );
  });

  group('dropEmptyFirstLine', () {
    EditorController fresh(List<EditorNode> nodes) {
      final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
      c.load(nodes);
      return c;
    }

    test('leaves the caret at the top of the line that rose', () {
      final c = fresh([
        EditorNode(id: 'blank', kind: 'paragraph', text: ''),
        EditorNode(id: 'p', kind: 'paragraph', text: 'below'),
      ]);
      c.collapseTo(const DocPosition(0, 0));
      expect(c.dropEmptyFirstLine(), isTrue);
      expect(c.nodes.single.id, 'p');
      expect(c.selection!.focus, const DocPosition(0, 0));
    });

    test('never touches an atomic first block', () {
      // An image carries a file_id and no text — "empty" it is not.
      final c = fresh([
        EditorNode(id: 'i', kind: 'image', text: '', data: {'file_id': 'f1'}),
        EditorNode(id: 'p', kind: 'paragraph', text: 'below'),
      ]);
      c.collapseTo(const DocPosition(0, 0));
      expect(c.dropEmptyFirstLine(), isFalse);
      expect(c.nodes.first.data['file_id'], 'f1');
    });
  });
}
