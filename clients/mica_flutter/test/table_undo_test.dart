// Does undo reach TABLE state at all?
//
// Reported 2026-08-12: "Ctrl+Z does nothing for tables — column width, bold in
// a cell". Everything a table is lives in `node.data`, not `node.text`, and the
// history layer snapshots nodes with `EditorNode.copy()`, whose `data` copy is
// SHALLOW (`Map<String, dynamic>.from`). A shallow copy shares the `rows` and
// `widths` lists with the live node, so an in-place mutation of either rewrites
// the undo snapshot too — and undo then restores what is already on screen.
//
// These tests sit at the controller level on purpose: they separate "the
// history layer cannot represent this" from "the keystroke never arrives".

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
        ['A', 'B'],
        ['a1', 'b1'],
      ], header: true, widths: [1.0, 1.0]).toBlockData(),
    ),
  ]);
  return c;
}

void main() {
  test('undo restores column widths', () {
    final c = _doc();
    c.setTableColumnWidths(0, [2.0, 0.5]);
    expect(TableData.fromBlock(c.nodes.single.data).widths, [2.0, 0.5]);

    c.undo();
    expect(
      TableData.fromBlock(c.nodes.single.data).widths,
      [1.0, 1.0],
      reason: 'undo must put the widths back',
    );
  });

  test('undo restores the overall table width', () {
    final c = _doc();
    c.setTableWidth(0, 0.5);
    expect(TableData.fromBlock(c.nodes.single.data).tableWidth, 0.5);

    c.undo();
    expect(TableData.fromBlock(c.nodes.single.data).tableWidth, 1.0);
  });

  test('undo restores a cell edit', () {
    final c = _doc();
    c.setTableCell(0, 1, 0, 'changed');
    expect(TableData.fromBlock(c.nodes.single.data).rows[1][0], 'changed');

    c.undo();
    expect(
      TableData.fromBlock(c.nodes.single.data).rows[1][0],
      'a1',
      reason: 'undo must put the cell text back',
    );
  });

  test('a drag preview does not become its own undo step', () {
    // Dragging emits many previews and one commit. A preview must NOT be
    // separately undoable, or one Ctrl+Z would step back a single mouse-move.
    final c = _doc();
    c.previewTableColumnWidths(0, [1.5, 0.5]);
    c.previewTableColumnWidths(0, [1.8, 0.2]);
    c.setTableColumnWidths(0, [2.0, 0.5]);

    c.undo();
    expect(
      TableData.fromBlock(c.nodes.single.data).widths,
      [1.0, 1.0],
      reason: 'one undo goes back past the whole drag, not one frame of it',
    );
  });
}
