import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/cell_edit_controller.dart';

// The WYSIWYG table-cell editor stores CLEAN text + marks (so bold shows bold
// while editing, no `**` source) and serializes back to Markdown for storage —
// the cell schema and GFM round-trip are unchanged.
void main() {
  test('parses raw cell Markdown into clean text (no delimiters) + marks', () {
    final c = CellEditController('a **bold** and `code`');
    expect(c.text, 'a bold and code');
    expect(c.text, isNot(contains('*')));
    expect(c.text, isNot(contains('`')));
    expect(c.marks.map((m) => m.type), containsAll(['bold', 'code']));
  });

  test('serialize round-trips text + marks back to Markdown', () {
    for (final md in ['a **b** c', '`x` **y** *z*', 'plain', '[t](http://x.dev)']) {
      expect(CellEditController(md).serialize(), md, reason: md);
    }
  });

  test('deleting text before a mark shifts it left (marks track edits)', () {
    final c = CellEditController('xx **bold**');
    expect(c.text, 'xx bold');
    c.value = const TextEditingValue(
      text: 'bold',
      selection: TextSelection.collapsed(offset: 0),
    );
    expect(c.serialize(), '**bold**');
  });

  test('appending after a mark leaves it intact', () {
    final c = CellEditController('**bold** x');
    c.value = const TextEditingValue(
      text: 'bold xY',
      selection: TextSelection.collapsed(offset: 7),
    );
    expect(c.serialize(), '**bold** xY');
  });

  test('inserting inside a mark extends it', () {
    final c = CellEditController('**bold**');
    c.value = const TextEditingValue(
      text: 'boXld',
      selection: TextSelection.collapsed(offset: 3),
    );
    expect(c.serialize(), '**boXld**');
  });

  test('toggleMark applies bold to the selection', () {
    final c = CellEditController('hello world');
    c.selection = const TextSelection(baseOffset: 0, extentOffset: 5);
    expect(c.toggleMark('bold'), isTrue);
    expect(c.serialize(), '**hello** world');
  });

  test('toggleMark on already-marked text removes it', () {
    final c = CellEditController('**hello** world');
    c.selection = const TextSelection(baseOffset: 0, extentOffset: 5);
    expect(c.toggleMark('bold'), isTrue);
    expect(c.serialize(), 'hello world');
  });

  test('toggleMark is a no-op on a collapsed selection', () {
    final c = CellEditController('hi');
    c.selection = const TextSelection.collapsed(offset: 1);
    expect(c.toggleMark('bold'), isFalse);
  });

  test('a cell built from multi-line raw keeps the newline (display autogrow)', () {
    // The cell DISPLAY path (parseInline) must preserve \n or a committed
    // multi-line cell would render collapsed to one line.
    final c = CellEditController('first\nsecond');
    expect(c.text, 'first\nsecond');
  });

  test('multi-line cell text is preserved through serialize', () {
    final c = CellEditController('line one');
    c.value = const TextEditingValue(
      text: 'line one\nline two',
      selection: TextSelection.collapsed(offset: 17),
    );
    expect(c.text, contains('\n'));
    expect(c.serialize(), contains('\n'));
  });
}
