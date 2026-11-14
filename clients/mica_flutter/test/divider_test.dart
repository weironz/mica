import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/markdown/markdown_codec.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'support/markdown_codec.dart';

EditorController _fresh(List<EditorNode> nodes) {
  final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
  c.load(nodes);
  return c;
}

EditorNode _p(String id, String text) =>
    EditorNode(id: id, kind: 'paragraph', text: text);


void main() {
  setUpAll(useNativeMarkdownCodec);
  test('typing ``` or ~~~ converts to a code block', () {
    for (final fence in ['```', '~~~']) {
      final c = _fresh([_p('a', '')]);
      c.selection = const DocSelection.collapsed(DocPosition(0, 0));
      c.nodes[0].text = fence;
      c.selection = const DocSelection.collapsed(DocPosition(0, 3));
      expect(c.applyInputRules(), isTrue, reason: fence);
      expect(c.nodes[0].kind, 'code_block', reason: fence);
      expect(c.nodes[0].text, '', reason: fence);
    }
  });

  test('typing + space converts to a bulleted list', () {
    final c = _fresh([_p('a', '')]);
    c.selection = const DocSelection.collapsed(DocPosition(0, 0));
    c.nodes[0].text = '+ ';
    c.selection = const DocSelection.collapsed(DocPosition(0, 2));
    expect(c.applyInputRules(), isTrue);
    expect(c.nodes[0].kind, 'bulleted_list');
  });

  test('typing a closed dollar-dollar line converts to a math block', () {
    final c = _fresh([_p('a', '')]);
    c.selection = const DocSelection.collapsed(DocPosition(0, 0));
    c.nodes[0].text = r'$$x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}$$';
    c.selection = DocSelection.collapsed(DocPosition(0, c.nodes[0].text.length));
    expect(c.applyInputRules(), isTrue);
    expect(c.nodes[0].kind, 'math_block');
    expect(c.nodes[0].text, r'x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}');
    expect(c.nodes[1].kind, 'paragraph', reason: 'caret parks after the atom');
  });

  test('typing --- converts to a divider with a trailing paragraph', () {
    final c = _fresh([_p('a', '')]);
    c.selection = const DocSelection.collapsed(DocPosition(0, 0));
    c.nodes[0].text = '---';
    c.selection = const DocSelection.collapsed(DocPosition(0, 3));
    expect(c.applyInputRules(), isTrue);

    expect(c.nodes[0].kind, 'divider');
    expect(c.nodes.length, 2);
    expect(c.nodes[1].kind, 'paragraph');
    // Caret lands in the trailing paragraph, not on the atomic divider.
    expect(c.selection!.focus.node, 1);
  });

  test('insertDivider after a non-empty paragraph inserts below it', () {
    final c = _fresh([_p('a', 'hello')]);
    c.selection = const DocSelection.collapsed(DocPosition(0, 5));
    c.insertDivider();

    expect(c.nodes.map((n) => n.kind).toList(),
        ['paragraph', 'divider', 'paragraph']);
    expect(c.nodes[0].text, 'hello');
    expect(c.selection!.focus.node, 2);
  });

  test('backspace at start of paragraph after a divider deletes the divider',
      () {
    final c = _fresh([
      _p('a', 'top'),
      EditorNode(id: 'd', kind: 'divider', text: ''),
      _p('b', 'bottom'),
    ]);
    c.selection = const DocSelection.collapsed(DocPosition(2, 0));
    expect(c.mergeBackward(), isTrue);

    expect(c.nodes.map((n) => n.id).toList(), ['a', 'b']);
    expect(c.nodes.every((n) => n.kind != 'divider'), isTrue);
  });

  test('a divider sits closer to its neighbours than a paragraph break', () {
    final paragraph = EditorTheme.gapAbove('paragraph', 'paragraph');
    expect(EditorTheme.gapAbove('divider', 'paragraph'), lessThan(paragraph));
    expect(EditorTheme.gapAbove('paragraph', 'divider'), lessThan(paragraph));
    // Headings keep their section spacing even right after a rule.
    expect(EditorTheme.gapAbove('heading', 'divider'), 28);
  });

  testWidgets('divider layout leaves a compact gap on both sides',
      (tester) async {
    await tester.pumpWidget(MaterialApp(
      localizationsDelegates: AppLocalizations.localizationsDelegates,
      supportedLocales: AppLocalizations.supportedLocales,
      home: Scaffold(
        body: MicaEditor(
          rootBlockId: 'root',
          nodes: [
            _p('a', 'above'),
            EditorNode(id: 'd', kind: 'divider', text: ''),
            _p('b', 'below'),
          ],
          version: 0,
          canEdit: true,
          onApplyOperations: (_) async {},
        ),
      ),
    ));
    await tester.pumpAndSettle();
    final render =
        tester.renderObject<RenderDocument>(find.byType(DocumentSurface));
    final (aboveTop, aboveHeight) = render.debugBoxAt(0);
    final (dividerTop, dividerHeight) = render.debugBoxAt(1);
    final (belowTop, _) = render.debugBoxAt(2);
    expect(dividerHeight, 16);
    final lineY = dividerTop + dividerHeight / 2;
    expect(lineY - (aboveTop + aboveHeight), 12);
    expect(belowTop - lineY, 12);
  });

  test('the codec emits a divider block', () {
    final blocks = markdownCodec.parseBlocks('a\n\n---\n\nb');
    expect(blocks.map((b) => b.kind).toList(),
        ['paragraph', 'divider', 'paragraph']);
  });

  test('undo removes an inserted divider', () async {
    final c = _fresh([_p('a', 'hello')]);
    c.selection = const DocSelection.collapsed(DocPosition(0, 5));
    c.insertDivider();
    expect(c.nodes.any((n) => n.kind == 'divider'), isTrue);

    c.undo();
    await pumpEventQueue();
    expect(c.nodes.any((n) => n.kind == 'divider'), isFalse);
    expect(c.nodes.first.text, 'hello');
  });
}
