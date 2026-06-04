import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/model.dart';

/// Set the focused paragraph's [text] with the caret at its end, then run the
/// input rules — mirrors what happens after a keystroke is committed.
void _type(EditorController c, String text) {
  c.nodes[0].text = text;
  c.selection = DocSelection.collapsed(DocPosition(0, text.length));
  c.applyInputRules();
}

EditorController _fresh() {
  final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
  c.load([EditorNode(id: 'a', kind: 'paragraph', text: '')]);
  return c;
}

void main() {
  test('bold rule strips ** and marks the run', () {
    final c = _fresh();
    // The rule fires the instant the closing ** is typed (caret right after it).
    _type(c, 'a **b**');
    final node = c.nodes[0];
    expect(node.text, 'a b');
    final marks = marksFromData(node.data);
    expect(marks.length, 1);
    expect(marks.first.type, 'bold');
    expect(node.text.substring(marks.first.start, marks.first.end), 'b');
    // Caret sits right after the converted run.
    expect(c.selection!.focus.offset, marks.first.end);
  });

  test('italic and code and strike', () {
    for (final probe in const [
      ('x *i*', 'x i', 'italic', 'i'),
      ('x `c`', 'x c', 'code', 'c'),
      ('x ~~s~~', 'x s', 'strike', 's'),
      ('x ~t~', 'x t', 'strike', 't'),
      ('x _u_', 'x u', 'italic', 'u'),
    ]) {
      final c = _fresh();
      _type(c, probe.$1);
      final node = c.nodes[0];
      expect(node.text, probe.$2, reason: probe.$1);
      final marks = marksFromData(node.data);
      expect(marks.length, 1, reason: probe.$1);
      expect(marks.first.type, probe.$3, reason: probe.$1);
      expect(
        node.text.substring(marks.first.start, marks.first.end),
        probe.$4,
        reason: probe.$1,
      );
    }
  });

  test('link rule captures label and href', () {
    final c = _fresh();
    _type(c, 'see [docs](https://x.io)');
    final node = c.nodes[0];
    expect(node.text, 'see docs');
    final marks = marksFromData(node.data);
    expect(marks.length, 1);
    expect(marks.first.type, 'link');
    expect(marks.first.href, 'https://x.io');
    expect(node.text.substring(marks.first.start, marks.first.end), 'docs');
  });

  test('typing the 7th char of **bold** does not misfire as italic', () {
    final c = _fresh();
    // Intermediate state while typing the closing ** of "**bold**".
    _type(c, '**bold*');
    expect(c.nodes[0].text, '**bold*'); // unchanged, no conversion yet
    expect(marksFromData(c.nodes[0].data), isEmpty);
  });

  test('bold around already-italic run keeps the inner mark', () {
    final c = _fresh();
    // Simulate "**hi**" where 'hi' already carries an italic mark at [2,4)
    // (i.e. mark offsets track the actual text, as setFocusedText maintains).
    c.nodes[0]
      ..text = '**hi**'
      ..data = {
        'marks': marksToJson([Mark(2, 4, 'italic')]),
      };
    c.selection = DocSelection.collapsed(const DocPosition(0, 6));
    c.applyInputRules();
    final marks = marksFromData(c.nodes[0].data);
    expect(c.nodes[0].text, 'hi');
    expect(marks.map((m) => m.type).toSet(), {'italic', 'bold'});
    for (final m in marks) {
      expect(c.nodes[0].text.substring(m.start, m.end), 'hi');
    }
  });
}
