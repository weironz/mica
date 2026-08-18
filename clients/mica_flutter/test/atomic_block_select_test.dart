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

  // Both reported 2026-08-12 against a page with an image in it.
  group('an image is a thing you can stand next to, cut, and keep', () {
    late List<Map<String, dynamic>> ops;
    late EditorController c;

    void load(List<EditorNode> nodes) {
      ops = [];
      c = EditorController(
        rootBlockId: 'root',
        onOps: (b) async => ops.addAll(b),
      );
      c.load(nodes);
    }

    List<String> kinds() => c.nodes.map((n) => n.kind).toList();

    test('backspace on the blank line under an image removes the LINE', () {
      // It removed the IMAGE. The rule it fell into — "an atomic neighbour
      // cannot absorb text, so delete it and keep the current node" — is there
      // to salvage text, and an empty line has none to salvage.
      load([
        EditorNode(id: 'i', kind: 'image', text: '', data: {'file_id': 'f1'}),
        EditorNode(id: 'p', kind: 'paragraph', text: ''),
      ]);
      c.collapseTo(const DocPosition(1, 0));

      expect(c.mergeBackward(), isTrue);
      expect(
        kinds(),
        ['image'],
        reason: 'the blank line went, the image stayed',
      );
      expect(c.nodes.single.data['file_id'], 'f1');
      expect(
        c.selection!.focus,
        const DocPosition(0, 0),
        reason: 'the caret parks on the image, ready to delete it next',
      );
    });

    test('a SECOND backspace then deletes the image', () {
      // Two presses, not one — the pause between them is what makes an
      // accidental keystroke recoverable.
      load([
        EditorNode(id: 'i', kind: 'image', text: '', data: {'file_id': 'f1'}),
        EditorNode(id: 'p', kind: 'paragraph', text: ''),
      ]);
      c.collapseTo(const DocPosition(1, 0));
      c.mergeBackward();
      expect(c.mergeBackward(), isTrue);
      expect(kinds().where((k) => k == 'image'), isEmpty);
    });

    test('backspace at the start of a NON-empty line is unchanged', () {
      // The old rule still applies where it was meant to: there is real text
      // here, and it must survive.
      load([
        EditorNode(id: 'i', kind: 'image', text: '', data: {'file_id': 'f1'}),
        EditorNode(id: 'p', kind: 'paragraph', text: 'keep me'),
      ]);
      c.collapseTo(const DocPosition(1, 0));

      expect(c.mergeBackward(), isTrue);
      expect(kinds(), ['paragraph']);
      expect(c.nodes.single.text, 'keep me');
    });

    test('an image at the caret serializes for the clipboard', () {
      // Cut/copy both bail on an empty payload, and a collapsed selection used
      // to produce one — so Ctrl+X on an image did nothing at all.
      //
      // Asserts the ACTUAL payload, not just `isNotEmpty`: this test used to
      // call `selectionPlainText`, which production dropped in 8c20158, so it
      // stayed green for the whole time copy/cut on an image was broken again.
      load([
        EditorNode(id: 'p', kind: 'paragraph', text: 'above'),
        EditorNode(id: 'i', kind: 'image', text: '', data: {'file_id': 'f1'}),
      ]);
      c.collapseTo(const DocPosition(1, 0));

      final urls = {'f1': 'https://example.test/f1.png'};
      expect(c.selectionText(imageUrls: urls),
          contains('https://example.test/f1.png'));
      expect(c.selectionHtml(imageUrls: urls), contains('img'));
    });

    test('cutting an image removes it', () async {
      // The other half: copying it and leaving it on the page is not a cut.
      load([
        EditorNode(id: 'p', kind: 'paragraph', text: 'above'),
        EditorNode(id: 'i', kind: 'image', text: '', data: {'file_id': 'f1'}),
      ]);
      c.collapseTo(const DocPosition(1, 0));

      expect(c.deleteSelection(), isTrue);
      expect(kinds(), ['paragraph']);
      // Ops are dispatched through the send chain, so they land a microtask
      // later — asserting synchronously reads an empty list and says nothing.
      await Future<void>.delayed(Duration.zero);
      expect(
        ops.any((o) => o['type'] == 'delete_block' && o['block_id'] == 'i'),
        isTrue,
        reason: 'the deletion has to reach the server, not just the screen',
      );
    });

    test('a collapsed caret in ordinary TEXT still selects nothing', () {
      // The guard that keeps the above from turning every stray caret into a
      // selection: Ctrl+C with no selection must stay a no-op.
      load([EditorNode(id: 'p', kind: 'paragraph', text: 'hello')]);
      c.collapseTo(const DocPosition(0, 2));

      expect(c.selectionText(), isEmpty);
      expect(c.deleteSelection(), isFalse);
    });
  });
}
