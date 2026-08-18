import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/markdown.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/model.dart';

/// The text/plain flavor of a copy and the paste that reads it back have to
/// agree on ONE language. They did not: the flavor was written Markdown-FREE
/// (a code block lost its fence, a list item got `•`) while
/// `_pasteFromClipboard` parses a plain-text paste AS Markdown — which is what
/// makes pasting an LLM answer work and is not up for removal.
///
/// The bill came due whenever the HTML flavor went missing (on Windows the two
/// flavors are separate SetClipboardData calls, so HTML can fail alone): a page
/// whose bash block held `# 1. check state` — a shell COMMENT — came back with
/// that line as an h1, its real `##` headings flattened to paragraphs, and 14
/// blocks became 48. Reproduced from the live workspace before it was fixed.
EditorController _selectAll(List<EditorNode> nodes) {
  final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
  c.load(nodes);
  c.selection = DocSelection(
    anchor: const DocPosition(0, 0),
    focus: DocPosition(nodes.length - 1, nodes.last.text.length),
  );
  return c;
}

void main() {
  test('a copied code block survives the text/plain flavor', () {
    const code = '# 1. 查看当前用户与节点\n'
        'docker exec headscale users list\n\n'
        '# 6. 验证\n'
        'docker exec headscale nodes list';
    final c = _selectAll([
      EditorNode(
        id: 'a',
        kind: 'heading',
        text: '一、本次改名的完整操作命令',
        data: {'level': 2},
      ),
      EditorNode(id: 'b', kind: 'paragraph', text: '全部在宿主机上执行。'),
      EditorNode(id: 'c', kind: 'code_block', text: code, data: {'language': 'bash'}),
    ]);

    final blocks = markdownToBlocks(c.selectionText());
    expect(blocks.map((b) => b.kind), ['heading', 'paragraph', 'code_block']);
    // The heading keeps its level — the stripped flavor dropped `##` entirely,
    // so every real heading came back as a paragraph.
    expect(blocks.first.data['level'], 2);
    // And the shell comments stay comments instead of becoming h1s.
    expect(blocks.last.text, code);
    expect(
      blocks.where((b) => b.kind == 'heading' && b.data['level'] == 1),
      isEmpty,
      reason: 'a `#` inside a fence is a comment, not a heading',
    );
  });

  test('a copied list survives the text/plain flavor', () {
    final c = _selectAll([
      EditorNode(id: 'a', kind: 'bulleted_list', text: '项目一'),
      EditorNode(id: 'b', kind: 'bulleted_list', text: '项目二'),
      EditorNode(id: 'c', kind: 'todo', text: '做这个', data: {'checked': true}),
    ]);
    final blocks = markdownToBlocks(c.selectionText());
    expect(
      blocks.map((b) => b.kind),
      ['bulleted_list', 'bulleted_list', 'todo'],
      reason: '`• item` is not list syntax — it pasted back as a paragraph',
    );
    expect(blocks.last.data['checked'], true);
  });

  test('inline marks survive the text/plain flavor', () {
    final c = _selectAll([
      EditorNode(
        id: 'a',
        kind: 'paragraph',
        text: '正文 粗体 收尾',
        data: {'marks': marksToJson([Mark(3, 5, 'bold')])},
      ),
    ]);
    final blocks = markdownToBlocks(c.selectionText());
    expect(blocks.single.kind, 'paragraph');
    expect(blocks.single.text, '正文 粗体 收尾');
    expect(blocks.single.data['marks'], isNotEmpty);
  });
}
