import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

// A divider is a line with no text: RenderDocument.positionAt refuses to park
// the caret on any atomic node, so a click on the line landed in the paragraph
// above and the line could never be selected — or deleted. And once the caret
// DID land on an atomic block (an image click does exactly that), Backspace ran
// into the "an empty styled line falls back to a paragraph" rule and quietly
// turned the picture into a blank line, file_id and all.

void main() {
  group('backspace / delete on a whole-block caret stop', () {
    late List<Map<String, dynamic>> ops;
    late EditorController c;

    void load(List<EditorNode> nodes) {
      ops = [];
      c = EditorController(rootBlockId: 'root', onOps: (b) async => ops.addAll(b));
      c.load(nodes);
    }

    List<String> kinds() => c.nodes.map((n) => n.kind).toList();

    test('backspace deletes the divider instead of blanking it', () {
      load([
        EditorNode(id: 'p', kind: 'paragraph', text: 'hello'),
        EditorNode(id: 'd', kind: 'divider', text: '', data: {}),
        EditorNode(id: 'q', kind: 'paragraph', text: 'after'),
      ]);
      c.collapseTo(const DocPosition(1, 0));
      expect(c.mergeBackward(), isTrue);
      expect(kinds(), ['paragraph', 'paragraph'],
          reason: 'the divider is gone — not turned into a stray blank line');
      expect(c.selection!.focus, const DocPosition(0, 5),
          reason: 'the caret lands at the end of the block above');
    });

    test('backspace on an image deletes it, keeping its file_id intact', () {
      // The bug this pins: the image became an empty paragraph with data {} —
      // clicking a picture (which parks the caret on it) and hitting Backspace
      // destroyed it through a path that never emitted a delete.
      load([
        EditorNode(id: 'p', kind: 'paragraph', text: 'hi'),
        EditorNode(id: 'i', kind: 'image', text: '', data: {'file_id': 'f1'}),
      ]);
      c.collapseTo(const DocPosition(1, 0));
      expect(c.mergeBackward(), isTrue);
      expect(kinds(), ['paragraph']);
      expect(c.nodes.any((n) => n.kind == 'image'), isFalse);
    });

    test('delete removes the block under the caret, not its neighbour',
        () async {
      load([
        EditorNode(id: 'd', kind: 'divider', text: '', data: {}),
        EditorNode(id: 'p', kind: 'paragraph', text: 'keep me'),
      ]);
      c.collapseTo(const DocPosition(0, 0));
      expect(c.mergeForward(), isTrue);
      expect(kinds(), ['paragraph']);
      expect(c.nodes.single.text, 'keep me',
          reason: 'the paragraph must survive with its text — the old merge '
              'pulled it INTO the divider node');
    });

    test('delete still removes an atomic block that FOLLOWS the caret', () {
      load([
        EditorNode(id: 'p', kind: 'paragraph', text: 'x'),
        EditorNode(id: 'd', kind: 'divider', text: '', data: {}),
      ]);
      c.collapseTo(const DocPosition(0, 1));
      expect(c.mergeForward(), isTrue);
      expect(kinds(), ['paragraph'], reason: 'the pre-existing path still works');
    });

    test('deleting the only block leaves a usable document', () {
      load([EditorNode(id: 'd', kind: 'divider', text: '', data: {})]);
      c.collapseTo(const DocPosition(0, 0));
      expect(c.mergeBackward(), isTrue);
      expect(c.nodes, isNotEmpty, reason: 'never leave an empty document');
      expect(c.nodes.first.isAtomic, isFalse);
    });

    test('a divider emits a real delete_block op', () async {
      load([
        EditorNode(id: 'p', kind: 'paragraph', text: 'hi'),
        EditorNode(id: 'd', kind: 'divider', text: '', data: {}),
      ]);
      c.collapseTo(const DocPosition(1, 0));
      c.mergeBackward();
      await Future<void>.delayed(Duration.zero);
      expect(
        ops.where((o) => o['type'] == 'delete_block' && o['block_id'] == 'd'),
        hasLength(1),
        reason: 'the block must be deleted, not rewritten into a paragraph',
      );
    });
  });

  group('clicking a divider selects it', () {
    // onSelectionChanged reports the focused BLOCK ID, which is exactly the
    // question here: did the click land on the divider, or snap away to the
    // paragraph as it used to?
    Future<List<String?>> pump(WidgetTester tester) async {
      final selected = <String?>[];
      await tester.pumpWidget(
        MaterialApp(
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          locale: const Locale('zh'),
          home: Scaffold(
            body: MicaEditor(
              rootBlockId: 'root',
              nodes: [
                EditorNode(id: 'p', kind: 'paragraph', text: 'above'),
                EditorNode(id: 'd', kind: 'divider', text: '', data: {}),
                EditorNode(id: 'q', kind: 'paragraph', text: 'below'),
              ],
              version: 0,
              canEdit: true,
              onApplyOperations: (_) async {},
              onSelectionChanged: (blockId, _) => selected.add(blockId),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      return selected;
    }

    testWidgets('a click on the line parks the caret on the divider itself',
        (tester) async {
      final selected = await pump(tester);
      final origin = tester.getTopLeft(find.byType(MicaEditor));
      // Sweep down through the divider's band. The exact y depends on the
      // paragraph's line height, which is not this test's business — what
      // matters is that SOME click on the line selects it.
      for (var dy = 20.0; dy < 140; dy += 2) {
        selected.clear();
        await tester.tapAt(origin + Offset(200, dy));
        await tester.pump();
        if (selected.contains('d')) return; // selected the divider
      }
      fail('no click anywhere down the page ever selected the divider');
    });

    testWidgets('a click on a paragraph still selects the paragraph',
        (tester) async {
      // The opt-in must not swallow ordinary clicks.
      final selected = await pump(tester);
      selected.clear();
      await tester.tapAt(tester.getTopLeft(find.byType(MicaEditor)) +
          const Offset(20, 8));
      await tester.pump();
      expect(selected, isNot(contains('d')));
    });
  });
}
