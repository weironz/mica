// P1-6:块数据里的深度(`data.li` / `data.quote` / `data.indent`)读出来一律
// clamp 到 `kMaxNestingDepth`(= Rust `MAX_NESTING_DEPTH`)。REST/MCP 能写进
// `li: 1e9`;复制成 Markdown 时 `'    ' * (li + 1)` 曾经不过 clamp。
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';

EditorController _doc(List<EditorNode> nodes) {
  final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
  c.load(nodes);
  c.selection = DocSelection(
    anchor: const DocPosition(0, 0),
    focus: DocPosition(nodes.length - 1, nodes.last.text.length),
  );
  return c;
}

int _leading(String md, String needle) {
  final line = md.split('\n').firstWhere((l) => l.contains(needle));
  return line.length - line.replaceFirst(RegExp(r'^[ >]+'), '').length;
}

void main() {
  test('the cap is the one Rust uses', () {
    expect(kMaxNestingDepth, 8);
  });

  test('huge stored depths copy as Markdown at the capped depth', () {
    final c = _doc([
      EditorNode(id: 'a', kind: 'bulleted_list', text: 'item'),
      EditorNode(id: 'b', kind: 'paragraph', text: 'li-child', data: {'li': 1000}),
      EditorNode(id: 'c', kind: 'paragraph', text: 'sep'),
      EditorNode(id: 'd', kind: 'quote', text: 'q-deep', data: {'quote': 1000}),
      EditorNode(id: 'e', kind: 'paragraph', text: 'sep2'),
      EditorNode(
          id: 'f',
          kind: 'bulleted_list',
          text: 'deep-item',
          data: {'indent': 1000}),
    ]);
    for (final md in [c.selectionText(), c.documentMarkdown()]) {
      // 8 层 li = (8 + 1) * 4 = 36 列;8 层 quote = `> ` * 8 = 16;indent 8 = 32。
      expect(_leading(md, 'li-child'), lessThanOrEqualTo(36), reason: md);
      expect(_leading(md, 'q-deep'), lessThanOrEqualTo(16), reason: md);
      expect(_leading(md, 'deep-item'), lessThanOrEqualTo(32), reason: md);
    }
  });
}
