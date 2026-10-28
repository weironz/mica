import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/table.dart';
import 'package:mica_flutter/markdown/markdown_codec.dart';
import 'support/markdown_codec.dart';

/// A table parsed by the engine, keeping the separator's per-column alignment
/// (the `:---:`/`---:` colons) — the metadata `parseGfmTable` used to extract
/// on the Dart side before the model went single-source.
TableData parseGfmTable(String markdown) {
  final blocks = markdownCodec.parseBlocks(markdown);
  return TableData.fromBlock(
    blocks.singleWhere((b) => b.kind == 'table').data,
  );
}

void main() {
  setUpAll(useNativeMarkdownCodec);
  test('table geometry and column operations preserve alignment metadata', () {
    final table = parseGfmTable('| A | B |\n| :---: | ---: |\n| a | b |');
    final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
    addTearDown(c.dispose);
    c.load([EditorNode(id: 't', kind: 'table', text: '', data: {...table.toBlockData(), 'extension': 'keep'})]);
    c.previewTableWidth(0, 0.6);
    expect(c.nodes.single.data['aligns'], ['center', 'right']);
    expect(c.nodes.single.data['extension'], 'keep');
    c.setTableWidth(0, 0.7);
    expect(c.nodes.single.data['aligns'], ['center', 'right']);
    expect(c.nodes.single.data['extension'], 'keep');
    c.moveTableColumn(0, 0, 1);
    expect(c.nodes.single.data['aligns'], ['right', 'center']);
    c.insertTableColumn(0, 1);
    expect(c.nodes.single.data['aligns'], ['right', '', 'center']);
    c.deleteTableColumn(0, 0);
    expect(c.nodes.single.data['aligns'], ['', 'center']);
    expect(c.documentMarkdown(), contains(':---:'));
    c.setTableAlign(0, 'left');
    expect(c.nodes.single.data['aligns'], isNull);
    expect(c.nodes.single.data['extension'], 'keep');
  });
}
