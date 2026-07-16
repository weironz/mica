import 'dart:ui' as ui;

import 'package:flutter/painting.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/render.dart';

// FoldPlan is the load-bearing piece of inline math: every offset that crosses
// between the document ("let \eta=2 be" — N chars) and the folded TextPainter
// ("let ￼ be" — 1 char per formula) goes through it. An off-by-one here
// is a caret in the wrong place, a selection covering half a formula, or a
// click editing the wrong character — so the mapping is pinned in both
// directions, on every edge.

InlineAtom atom(int start, int end, {int painterIndex = -1}) => InlineAtom(
  docStart: start,
  docEnd: end,
  source: 'x' * (end - start),
  painterIndex: painterIndex,
  size: const Size(40, 20),
  baseline: 15,
  renderer: const MathInlineAtomRenderer(),
);

void main() {
  group('FoldPlan single atom', () {
    // doc:      0123456789…   text "ab" + run [2,7) + "cd"  (len 9)
    // painter:  ab⬚cd          run is 1 unit at painter 2
    final plan = FoldPlan([atom(2, 7)]);

    test('doc→painter around the run', () {
      expect(plan.docToPainter(0), 0);
      expect(plan.docToPainter(2), 2); // run start = placeholder lead
      expect(plan.docToPainter(7), 3); // run end = placeholder trail
      expect(plan.docToPainter(9), 5); // node end
    });

    test('doc→painter inside the run collapses to an edge', () {
      expect(plan.docToPainter(4), 2, reason: 'floor = leading edge');
      expect(
        plan.docToPainter(4, ceilInsideAtom: true),
        3,
        reason: 'ceil = trailing edge, so ranges cover the placeholder',
      );
    });

    test('painter→doc lands on run edges, never inside', () {
      expect(plan.painterToDoc(0), 0);
      expect(plan.painterToDoc(2), 2); // leading edge → run start
      expect(plan.painterToDoc(3), 7); // trailing edge → run end
      expect(plan.painterToDoc(5), 9);
    });

    test('round-trips for every offset outside the run', () {
      for (final d in [0, 1, 2, 7, 8, 9]) {
        expect(plan.painterToDoc(plan.docToPainter(d)), d, reason: 'doc $d');
      }
    });
  });

  group('FoldPlan two atoms, adjacent text', () {
    // doc: "a" [1,4) "b" [5,9) "c"   len 10
    // painter: a⬚b⬚c              len 5
    final plan = FoldPlan([atom(1, 4), atom(5, 9)]);

    test('shifts accumulate across atoms', () {
      expect(plan.docToPainter(0), 0);
      expect(plan.docToPainter(1), 1);
      expect(plan.docToPainter(4), 2); // after first run
      expect(plan.docToPainter(5), 3); // second run start
      expect(plan.docToPainter(9), 4); // after second run
      expect(plan.docToPainter(10), 5);
    });

    test('painter→doc across both', () {
      expect(plan.painterToDoc(1), 1);
      expect(plan.painterToDoc(2), 4);
      expect(plan.painterToDoc(3), 5);
      expect(plan.painterToDoc(4), 9);
      expect(plan.painterToDoc(5), 10);
    });
  });

  group('FoldPlan whole-node run', () {
    // The entire text is one formula: doc len 6 → painter len 1.
    final plan = FoldPlan([atom(0, 6)]);

    test('edges', () {
      expect(plan.docToPainter(0), 0);
      expect(plan.docToPainter(6), 1);
      expect(plan.docToPainter(3), 0);
      expect(plan.painterToDoc(0), 0);
      expect(plan.painterToDoc(1), 6);
    });
  });

  group('buildFoldedSpan', () {
    const base = TextStyle(fontSize: 15);

    test('display text is the source with runs replaced by U+FFFC', () {
      final r = buildFoldedSpan(
        'ab34567cd',
        [Mark(2, 7, 'math')],
        base,
        [atom(2, 7)],
      );
      final painter = TextPainter(
        text: r.span,
        textDirection: TextDirection.ltr,
      )..setPlaceholderDimensions(r.dims);
      painter.layout();
      expect(painter.plainText.codeUnits, [
        97, 98, 0xFFFC, 99, 100, //
      ]);
      expect(r.dims, hasLength(1));
      expect(r.dims.single.size, const Size(40, 20));
      expect(
        r.dims.single.alignment,
        ui.PlaceholderAlignment.baseline,
        reason: 'a known baseline must be used, not middle',
      );
      painter.dispose();
    });

    test('marks around the run keep their styling, remapped', () {
      // "XXab" where XX is bold and [2,4) is the math run… use:
      // text "bbMMtt", bold [0,2), math [2,4), plain tail.
      final r = buildFoldedSpan(
        'bbMMtt',
        [Mark(0, 2, 'bold'), Mark(2, 4, 'math')],
        base,
        [atom(2, 4)],
      );
      // Walk leaves: first text leaf must be bold.
      final leaves = <TextSpan>[];
      void walk(InlineSpan s) {
        if (s is TextSpan) {
          if ((s.children == null || s.children!.isEmpty) && s.text != null) {
            leaves.add(s);
          }
          s.children?.forEach(walk);
        }
      }

      walk(r.span);
      final bold = leaves.firstWhere((l) => l.text == 'bb');
      expect(bold.style?.fontWeight, FontWeight.w600);
      final tail = leaves.firstWhere((l) => l.text == 'tt');
      expect(tail.style?.fontWeight, isNot(FontWeight.w600));
    });

    test('no baseline degrades to middle alignment', () {
      final a = InlineAtom(
        docStart: 0,
        docEnd: 3,
        source: 'xyz',
        painterIndex: 0,
        size: const Size(40, 20),
        baseline: null,
        renderer: const MathInlineAtomRenderer(),
      );
      final r = buildFoldedSpan('xyz', [Mark(0, 3, 'math')], base, [a]);
      expect(r.dims.single.alignment, ui.PlaceholderAlignment.middle);
    });
  });
}
