import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/model.dart';

// Controller-level behaviour of the inline-math atom, against a REAL
// EditorController. These pin the bugs the adversarial review of IM6 confirmed:
// typing at a formula's leading edge, deleting it whole, editing its source,
// and the concurrent index/id hazard.

EditorController seed(String text, List<Map<String, dynamic>> marks) {
  final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
  c.load([
    EditorNode(id: 'p', kind: 'paragraph', text: text, data: {'marks': marks}),
  ]);
  return c;
}

List<String> mathSources(EditorController c, int node) {
  final n = c.nodes[node];
  final chars = n.text;
  return [
    for (final m in marksFromData(n.data))
      if (m.type == 'math') chars.substring(m.start, m.end),
  ];
}

void main() {
  // "aXYb" with math over "XY" at [1,3).

  test('typing at a formula leading edge lands OUTSIDE it', () {
    final c = seed('aXYb', [
      {'start': 1, 'end': 3, 'type': 'math'},
    ]);
    c.collapseTo(const DocPosition(0, 1)); // caret at run.start
    // What the IME insert does: text becomes 'azXYb', caret moves to 2.
    c.setFocusedText('azXYb', 2, 2);
    expect(mathSources(c, 0), [
      'XY',
    ], reason: 'the z must not be swallowed into the LaTeX');
    expect(
      mathRunAt(marksFromData(c.nodes[0].data), c.selection!.focus.offset),
      isNull,
      reason: 'the caret must not be stranded inside the run',
    );
  });

  test('typing at a formula trailing edge also stays outside', () {
    final c = seed('aXYb', [
      {'start': 1, 'end': 3, 'type': 'math'},
    ]);
    c.collapseTo(const DocPosition(0, 3)); // caret at run.end
    c.setFocusedText('aXYzb', 4, 4);
    expect(mathSources(c, 0), ['XY']);
  });

  test('deleting a formula at its trailing edge removes it whole', () {
    final c = seed('aXYb', [
      {'start': 1, 'end': 3, 'type': 'math'},
    ]);
    c.collapseTo(const DocPosition(0, 3));
    expect(c.deleteMathAtomBackward(), isTrue);
    expect(c.nodes[0].text, 'ab', reason: 'the source XY is gone');
    expect(mathSources(c, 0), isEmpty);
    expect(c.selection!.focus, const DocPosition(0, 1));
  });

  test('deleting at the leading edge (Delete key) removes it whole', () {
    final c = seed('aXYb', [
      {'start': 1, 'end': 3, 'type': 'math'},
    ]);
    c.collapseTo(const DocPosition(0, 1));
    expect(c.deleteMathAtomForward(), isTrue);
    expect(c.nodes[0].text, 'ab');
    expect(mathSources(c, 0), isEmpty);
  });

  test('editing the source rewrites just the run, mark tracks new length', () {
    final c = seed('aXYb', [
      {'start': 1, 'end': 3, 'type': 'math'},
    ]);
    expect(c.setInlineMathSource(0, 1, 3, 'LONGER'), isTrue);
    expect(c.nodes[0].text, 'aLONGERb');
    expect(mathSources(c, 0), ['LONGER']);
  });

  test('setInlineMathSource refuses when the run no longer matches', () {
    // The concurrent hazard: dialog captured [1,3), but the run moved.
    final c = seed('aXYb', [
      {'start': 2, 'end': 4, 'type': 'math'},
    ]);
    expect(
      c.setInlineMathSource(0, 1, 3, 'Z'),
      isFalse,
      reason: 'no math run covers [1,3) — refuse rather than corrupt',
    );
    expect(c.nodes[0].text, 'aXYb', reason: 'text untouched');
  });

  test('a bold mark before the formula survives an edit', () {
    // "BBxyCC": bold [0,2), math [2,4).
    final c = seed('BBxyCC', [
      {'start': 0, 'end': 2, 'type': 'bold'},
      {'start': 2, 'end': 4, 'type': 'math'},
    ]);
    expect(c.setInlineMathSource(0, 2, 4, 'PQR'), isTrue);
    final marks = marksFromData(c.nodes[0].data);
    final bold = marks.firstWhere((m) => m.type == 'bold');
    expect((bold.start, bold.end), (0, 2), reason: 'bold unmoved');
    final m = marks.firstWhere((mk) => mk.type == 'math');
    expect(c.nodes[0].text.substring(m.start, m.end), 'PQR');
  });
}
