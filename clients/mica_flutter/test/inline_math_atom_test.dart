import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/marks.dart';

// The caret-atomicity rules for typeset inline formulas: a formula renders as
// one atom, so the caret rests on either edge but never inside its source, and
// Backspace/Delete at an edge removes the whole thing. These are the pure
// functions render/controller lean on — tested directly rather than restated.

void main() {
  // "ab" + math run [2,7) + "cd"
  final marks = [Mark(2, 7, 'math')];

  group('snapOutOfMathRun', () {
    test('offsets outside a run pass through', () {
      for (final o in [0, 1, 2, 7, 8, 9]) {
        expect(snapOutOfMathRun(marks, o), o, reason: 'offset $o');
      }
    });

    test('a caret inside snaps to the nearer edge', () {
      expect(snapOutOfMathRun(marks, 3), 2); // 1 from start, 4 from end
      expect(snapOutOfMathRun(marks, 6), 7); // 4 from start, 1 from end
    });

    test('prefEnd forces the far edge (range endpoints)', () {
      expect(snapOutOfMathRun(marks, 3, prefEnd: true), 7);
      expect(snapOutOfMathRun(marks, 6, prefEnd: false), 2);
    });

    test('no marks, no snap', () {
      expect(snapOutOfMathRun(const [], 3), 3);
    });
  });

  group('delete-target lookups', () {
    test('mathRunEndingAt: Backspace deletes when caret is at run end', () {
      expect(mathRunEndingAt(marks, 7)?.start, 2);
      expect(mathRunEndingAt(marks, 6), isNull);
      expect(mathRunEndingAt(marks, 2), isNull, reason: 'that is the start');
    });

    test('mathRunStartingAt: Delete deletes when caret is at run start', () {
      expect(mathRunStartingAt(marks, 2)?.end, 7);
      expect(mathRunStartingAt(marks, 3), isNull);
      expect(mathRunStartingAt(marks, 7), isNull, reason: 'that is the end');
    });

    test('an empty run (no source) is not a delete target', () {
      final empty = [Mark(3, 3, 'math')];
      expect(mathRunEndingAt(empty, 3), isNull);
      expect(mathRunStartingAt(empty, 3), isNull);
    });
  });
}
