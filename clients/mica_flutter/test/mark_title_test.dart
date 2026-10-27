import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/cell_edit_controller.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/model.dart';
import 'support/markdown_codec.dart';

// Regression for issue #150: every Mark reconstruction (shift/split/normalize)
// rebuilt the mark from (start, end, type, href) and DROPPED the link title, so
// any ordinary edit persisted `[x](/u)` and the `"tip"` was gone for good.
EditorController _doc(List<EditorNode> nodes) {
  final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
  c.load(nodes);
  return c;
}

EditorNode _linkParagraph(String text, List<Mark> marks) => EditorNode(
  id: 'a',
  kind: 'paragraph',
  text: text,
  data: {'marks': marksToJson(marks)},
);

void main() {
  setUpAll(useNativeMarkdownCodec);
  test('typing after a titled link keeps the title', () {
    final c = _doc([
      _linkParagraph('pre x', [
        Mark(4, 5, 'link', href: '/u', title: 'tip'),
      ]),
    ]);
    c.selection = const DocSelection.collapsed(DocPosition(0, 5));
    c.setFocusedText('pre x!', 6, 6);

    final mark = marksFromData(c.nodes.single.data).single;
    expect(mark.href, '/u');
    expect(mark.title, 'tip', reason: 'shiftMarks rebuilt the mark bare');
    expect(c.documentMarkdown(), contains('[x](/u "tip")!'));
  });

  test('Enter split across a titled link keeps the title on both halves', () {
    final c = _doc([
      _linkParagraph('abcd', [Mark(1, 4, 'link', href: '/u', title: 'tip')]),
    ]);
    c.selection = const DocSelection.collapsed(DocPosition(0, 2));
    c.applyNewlineSplit('ab', 'cd');

    expect(c.nodes.length, 2);
    final first = marksFromData(c.nodes[0].data).single;
    final second = marksFromData(c.nodes[1].data).single;
    expect((first.start, first.end), (1, 2));
    expect((second.start, second.end), (0, 2));
    expect(first.title, 'tip');
    expect(second.title, 'tip');
  });

  test('adjacent same-href links with different titles are not merged', () {
    final c = _doc([
      _linkParagraph('abcd', [
        Mark(0, 2, 'link', href: '/u', title: 'one'),
        Mark(2, 4, 'link', href: '/u', title: 'two'),
      ]),
    ]);
    // Any mark toggle runs `_normalize`, which used to fuse the two links into
    // one href-only mark and erase both titles.
    c.selection = const DocSelection(
      anchor: DocPosition(0, 0),
      focus: DocPosition(0, 4),
    );
    c.toggleMark('bold');

    final links = marksFromData(c.nodes.single.data)
        .where((m) => m.type == 'link')
        .toList();
    expect(links.length, 2);
    expect(links.map((m) => m.title), ['one', 'two']);
    expect(c.documentMarkdown(), contains('"one"'));
    expect(c.documentMarkdown(), contains('"two"'));
  });

  test('editing a table cell keeps a link title through serialize', () {
    final ctrl = CellEditController('[x](/u "tip")');
    ctrl.value = const TextEditingValue(
      text: 'x!',
      selection: TextSelection.collapsed(offset: 2),
    );
    expect(ctrl.serialize(), contains('"tip"'));
  });
}