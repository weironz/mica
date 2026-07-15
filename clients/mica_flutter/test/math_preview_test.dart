import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/marks.dart';

// Pins the one decision behind the floating math preview (`_paintMathPreview`
// in render.dart): which math run, if any, does the caret sit inside?
//
// This imports the REAL function rather than restating the rule. A test that
// re-implements the thing it tests passes while the bug ships — that is exactly
// how a footer bug got past a green suite in this repo before.
//
// The rest of _paintMathPreview is a canvas draw: no offsets are remapped, no
// text layout changes, so there is nothing else here that can be wrong in a way
// a unit test would catch. The card's placement needs eyes.

void main() {
  // `let $x^2$ be` — the math run covers "x^2" at [4, 7).
  final marks = [Mark(4, 7, 'math'), Mark(9, 12, 'code')];

  test('a caret inside the run finds it', () {
    expect(mathRunAt(marks, 5)?.start, 4);
    expect(mathRunAt(marks, 6)?.start, 4);
  });

  test('a caret on either edge does not', () {
    // Typing your way past a formula must not strobe the card open and shut.
    expect(mathRunAt(marks, 4), isNull, reason: 'left edge');
    expect(mathRunAt(marks, 7), isNull, reason: 'right edge');
  });

  test('a caret elsewhere does not', () {
    expect(mathRunAt(marks, 0), isNull);
    expect(mathRunAt(marks, 12), isNull);
  });

  test('other mark types are not math', () {
    // The code run at [9, 12) must never trigger a formula card.
    expect(mathRunAt(marks, 10), isNull);
  });

  test('an empty run can never match', () {
    // `$$` has no formula in it to preview, so no caret position is "inside".
    final empty = [Mark(4, 4, 'math')];
    for (var c = 0; c <= 8; c++) {
      expect(mathRunAt(empty, c), isNull, reason: 'caret $c');
    }
  });

  test('no marks, no card', () {
    expect(mathRunAt(const [], 3), isNull);
  });

  test('the first containing run wins when two overlap', () {
    final overlapping = [Mark(0, 10, 'math'), Mark(2, 5, 'math')];
    expect(mathRunAt(overlapping, 3)?.end, 10);
  });
}
