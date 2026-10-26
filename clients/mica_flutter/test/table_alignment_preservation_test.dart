import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/table.dart';

void main() {
  test('table geometry and column operations preserve alignment metadata', () {
    final table = parseGfmTable(['| A | B |', '| :---: | ---: |', '| a | b |'], 0).table;
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
