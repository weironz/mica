import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/table.dart';

EditorController _doc() {
  final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
  c.load([
    EditorNode(
      id: 't',
      kind: 'table',
      text: '',
      data: TableData([
        ['A', 'B', 'C'],
        ['a1', 'b1', 'c1'],
        ['a2', 'b2', 'c2'],
      ], header: true, widths: [1.0, 2.0, 3.0]).toBlockData(),
    ),
  ]);
  return c;
}

void main() {
  test('moveTableColumn moves the rightmost column left (cells + width)', () {
    final c = _doc();
    c.moveTableColumn(0, 2, -1);
    final t = TableData.fromBlock(c.nodes.single.data);
    expect(t.rows[0], ['A', 'C', 'B']);
    expect(t.rows[1], ['a1', 'c1', 'b1']);
    expect(t.widths, [1.0, 3.0, 2.0]);
  });

  test('moveTableColumn clamps at the edges', () {
    final c = _doc();
    c.moveTableColumn(0, 0, -1); // no-op
    c.moveTableColumn(0, 2, 1); // no-op
    final t = TableData.fromBlock(c.nodes.single.data);
    expect(t.rows[0], ['A', 'B', 'C']);
  });

  test('setTableWidth persists and round-trips; default omits the key', () {
    final c = _doc();
    // Default-width tables must NOT serialize 'width' (parser conformance
    // with the Rust engine, which doesn't model table width).
    expect(c.nodes.single.data.containsKey('width'), isFalse);
    c.setTableWidth(0, 0.6);
    expect(c.nodes.single.data['width'], 0.6);
    final t = TableData.fromBlock(c.nodes.single.data);
    expect(t.tableWidth, 0.6);
    expect(t.rows[0], ['A', 'B', 'C']); // grid untouched
    // Clamped at the extremes.
    c.setTableWidth(0, 5.0);
    expect(TableData.fromBlock(c.nodes.single.data).tableWidth, 1.0);
  });

  test('moveTableRow reorders body rows but never the header', () {
    final c = _doc();
    c.moveTableRow(0, 2, -1);
    var t = TableData.fromBlock(c.nodes.single.data);
    expect(t.rows[1], ['a2', 'b2', 'c2']);
    expect(t.rows[2], ['a1', 'b1', 'c1']);
    // Header row can't move; body row can't move above the header.
    c.moveTableRow(0, 0, 1);
    c.moveTableRow(0, 1, -1);
    t = TableData.fromBlock(c.nodes.single.data);
    expect(t.rows[0], ['A', 'B', 'C']);
    expect(t.rows[1], ['a2', 'b2', 'c2']);
  });
}
