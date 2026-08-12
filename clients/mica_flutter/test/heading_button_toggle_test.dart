// Pressing a heading button that is already lit puts the block back to text.
//
// Reported 2026-08-12: with the caret on an H2, pressing H2 re-applied H2 — so
// the highlighted button was the only control on either format bar that did
// nothing when pressed, and there was no way back to a paragraph from the
// floating bar at all (it carries no 「正文」 button).
//
// Pure and shared, because there are TWO of these button rows — the page
// toolbar in main.dart and the floating bar in editor.dart — and each had the
// behaviour written out separately. That is what let them differ.

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/model.dart' show headingButtonTarget;

void main() {
  group('headingButtonTarget', () {
    test('pressing the level you are ON goes back to a paragraph', () {
      final t = headingButtonTarget(kind: 'heading', level: 2, pressed: 2);
      expect(t.kind, 'paragraph');
      expect(t.data, isEmpty, reason: 'a paragraph carries no level');
    });

    test('pressing a DIFFERENT level restyles instead of clearing', () {
      // The half the user asked to leave alone: H2 → H1 must still be H1, not a
      // paragraph.
      final t = headingButtonTarget(kind: 'heading', level: 2, pressed: 1);
      expect(t.kind, 'heading');
      expect(t.data['level'], 1);
    });

    test('from a paragraph it applies the level', () {
      final t = headingButtonTarget(kind: 'paragraph', level: 0, pressed: 3);
      expect(t.kind, 'heading');
      expect(t.data['level'], 3);
    });

    test('from any other block kind it applies the level', () {
      for (final kind in ['quote', 'bulleted_list', 'code_block', null]) {
        final t = headingButtonTarget(kind: kind, level: 0, pressed: 1);
        expect(t.kind, 'heading', reason: 'from $kind');
        expect(t.data['level'], 1, reason: 'from $kind');
      }
    });

    test('a matching LEVEL on a non-heading block is not a toggle', () {
      // The trap this guards: level only means anything when the kind is a
      // heading. A quote that happens to carry level 2 must not let the H2
      // button clear it — the block was never a heading.
      final t = headingButtonTarget(kind: 'quote', level: 2, pressed: 2);
      expect(t.kind, 'heading');
      expect(t.data['level'], 2);
    });

    test('every level toggles itself and only itself', () {
      for (var on = 1; on <= 6; on++) {
        for (var pressed = 1; pressed <= 6; pressed++) {
          final t = headingButtonTarget(
            kind: 'heading',
            level: on,
            pressed: pressed,
          );
          if (on == pressed) {
            expect(t.kind, 'paragraph', reason: 'H$on pressing H$pressed');
          } else {
            expect(t.kind, 'heading', reason: 'H$on pressing H$pressed');
            expect(t.data['level'], pressed);
          }
        }
      }
    });
  });
}
