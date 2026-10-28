import 'package:flutter/widgets.dart' hide TableCell;
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/cell_edit_controller.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/table.dart' show TableCell;
import 'package:mica_flutter/markdown/markdown_codec.dart';
import 'support/markdown_codec.dart';

// The WYSIWYG table-cell editor works on the cell's text + marks directly (the
// same model a paragraph uses — bold shows bold while typing, no `**` source).
// Storage goes through the engine, so these tests decode their source cells
// through the real codec, exactly like an edit session opening a stored cell.
TableCell _cell(String source) {
  final decoded = markdownCodec.decodeCells([source]).single;
  return TableCell(decoded.text, decoded.marks);
}

void main() {
  setUpAll(useNativeMarkdownCodec);

  test('a stored cell decodes to clean text (no delimiters) + marks', () {
    final c = CellEditController(_cell('a **bold** and `code`'));
    expect(c.text, 'a bold and code');
    expect(c.text, isNot(contains('*')));
    expect(c.text, isNot(contains('`')));
    expect(c.marks.map((m) => m.type), containsAll(['bold', 'code']));
  });

  test('the cell model round-trips text + marks back to storage', () {
    for (final md in ['a **b** c', '`x` **y** *z*', 'plain', '[t](http://x.dev)']) {
      final opening = _cell(md);
      final c = CellEditController(opening);
      addTearDown(c.dispose);
      expect(c.cell, opening, reason: md);
    }
  });

  test('deleting text before a mark shifts it left (marks track edits)', () {
    final c = CellEditController(_cell('xx **bold**'));
    addTearDown(c.dispose);
    expect(c.text, 'xx bold');
    c.value = const TextEditingValue(
      text: 'bold',
      selection: TextSelection.collapsed(offset: 0),
    );
    expect(c.text, 'bold');
    expect(c.marks.single.type, 'bold');
    expect(c.marks.single.start, 0);
    expect(c.marks.single.end, 4);
  });

  test('appending after a mark leaves it intact', () {
    final c = CellEditController(_cell('**bold** x'));
    addTearDown(c.dispose);
    c.value = const TextEditingValue(
      text: 'bold xY',
      selection: TextSelection.collapsed(offset: 7),
    );
    expect(c.marks.single.end, 4);
  });

  test('inserting inside a mark extends it', () {
    final c = CellEditController(_cell('**bold**'));
    addTearDown(c.dispose);
    c.value = const TextEditingValue(
      text: 'boXld',
      selection: TextSelection.collapsed(offset: 3),
    );
    expect(c.marks.single.start, 0);
    expect(c.marks.single.end, 5);
  });

  test('toggleMark applies bold to the selection', () {
    final c = CellEditController(_cell('hello world'));
    addTearDown(c.dispose);
    c.selection = const TextSelection(baseOffset: 0, extentOffset: 5);
    expect(c.toggleMark('bold'), isTrue);
    expect(c.marks.single.type, 'bold');
    expect(c.marks.single.start, 0);
    expect(c.marks.single.end, 5);
  });

  test('toggleMark on already-marked text removes it', () {
    final c = CellEditController(_cell('**hello** world'));
    addTearDown(c.dispose);
    c.selection = const TextSelection(baseOffset: 0, extentOffset: 5);
    expect(c.toggleMark('bold'), isTrue);
    expect(c.marks, isEmpty);
  });

  test('toggleMark is a no-op on a collapsed selection', () {
    final c = CellEditController(_cell('hi'));
    addTearDown(c.dispose);
    c.selection = const TextSelection.collapsed(offset: 1);
    expect(c.toggleMark('bold'), isFalse);
  });

  test('a cell built from multi-line raw keeps the newline (display autogrow)', () {
    // The cell DISPLAY path must preserve \n or a committed multi-line cell
    // would render collapsed to one line.
    final c = CellEditController(_cell('first\nsecond'));
    addTearDown(c.dispose);
    expect(c.text, 'first\nsecond');
  });

  test('multi-line cell text survives editing through the model', () {
    final c = CellEditController(_cell('line one'));
    addTearDown(c.dispose);
    c.value = const TextEditingValue(
      text: 'line one\nline two',
      selection: TextSelection.collapsed(offset: 17),
    );
    expect(c.text, contains('\n'));
    // Storage of a mark-free newline cell keeps the newline readable back.
    final stored = markdownCodec
        .encodeCells([(text: c.text, marks: c.marks)])
        .single;
    expect(stored is Map || stored is String, isTrue);
    final decoded = markdownCodec.decodeCells([stored]).single;
    expect(decoded.text, contains('\n'));
  });

  test('marksEqual is order-insensitive over the structured fields', () {
    final a = [Mark(0, 2, 'bold'), Mark(3, 5, 'link', href: 'u')];
    final b = [Mark(3, 5, 'link', href: 'u'), Mark(0, 2, 'bold')];
    expect(marksEqual(a, b), isTrue);
    expect(marksEqual(a, [Mark(0, 2, 'bold')]), isFalse);
    expect(
      marksEqual([Mark(0, 2, 'link', href: 'u')], [Mark(0, 2, 'link', href: 'v')]),
      isFalse,
    );
  });
}