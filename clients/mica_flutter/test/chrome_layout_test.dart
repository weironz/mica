import 'dart:ui';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/chrome_layout.dart';

/// The bug this locks out: the bubble was placed at `anchor.top - height - gap`
/// with no clamping, so a code block scrolled against the top of the viewport put
/// its toolbar tooltips above the visible area — drawn, paid for, invisible.
/// Hovering Copy showed the button highlight and nothing else.
void main() {
  const bubble = Size(60, 20);

  // A viewport that does NOT start at y=0, because that is the normal case while
  // scrolled — and measuring against the document's own height is exactly the
  // mistake this function exists to avoid.
  const visible = Rect.fromLTRB(0, 500, 800, 1100);

  group('vertical placement', () {
    test('sits above the anchor when there is room', () {
      const anchor = Rect.fromLTWH(400, 700, 24, 24);

      final r = tooltipRect(anchor: anchor, bubble: bubble, visible: visible);

      expect(r.bottom, lessThanOrEqualTo(anchor.top));
      expect(r.top, 700 - 20 - 4);
    });

    test('flips below when above would fall outside the visible area', () {
      // The anchor is right at the top of what the user can see.
      const anchor = Rect.fromLTWH(400, 505, 24, 24);

      final r = tooltipRect(anchor: anchor, bubble: bubble, visible: visible);

      expect(r.top, greaterThanOrEqualTo(anchor.bottom));
      expect(
        r.top,
        anchor.bottom + 4,
        reason: 'flipped below, not clamped on top of the button',
      );
    });

    test('stays inside the visible area in both directions', () {
      for (final y in [500.0, 505.0, 700.0, 1075.0, 1099.0]) {
        final r = tooltipRect(
          anchor: Rect.fromLTWH(400, y, 24, 24),
          bubble: bubble,
          visible: visible,
        );

        expect(r.top, greaterThanOrEqualTo(visible.top), reason: 'y=$y');
        expect(r.bottom, lessThanOrEqualTo(visible.bottom), reason: 'y=$y');
      }
    });

    test(
      'a viewport shorter than the bubble still yields an on-screen rect',
      () {
        const tiny = Rect.fromLTRB(0, 0, 800, 10);

        final r = tooltipRect(
          anchor: const Rect.fromLTWH(400, 2, 24, 24),
          bubble: bubble,
          visible: tiny,
        );

        // Cannot fit, but must not throw and must not run off the top.
        expect(r.top, greaterThanOrEqualTo(tiny.top));
      },
    );
  });

  group('horizontal placement', () {
    test('centres on the anchor when there is room', () {
      const anchor = Rect.fromLTWH(400, 700, 24, 24);

      final r = tooltipRect(anchor: anchor, bubble: bubble, visible: visible);

      expect(r.center.dx, anchor.center.dx);
    });

    test('is pulled inside the left and right edges', () {
      final atLeft = tooltipRect(
        anchor: const Rect.fromLTWH(0, 700, 24, 24),
        bubble: bubble,
        visible: visible,
      );
      expect(atLeft.left, greaterThanOrEqualTo(visible.left));

      final atRight = tooltipRect(
        anchor: const Rect.fromLTWH(790, 700, 24, 24),
        bubble: bubble,
        visible: visible,
      );
      expect(atRight.right, lessThanOrEqualTo(visible.right));
    });

    test('a bubble wider than the viewport pins left instead of throwing', () {
      // An inverted clamp range is an exception, not a layout quirk.
      final r = tooltipRect(
        anchor: const Rect.fromLTWH(400, 700, 24, 24),
        bubble: const Size(2000, 20),
        visible: visible,
      );

      expect(r.left, visible.left + 4);
    });
  });

  test('keeps the bubble size it was given', () {
    final r = tooltipRect(
      anchor: const Rect.fromLTWH(400, 700, 24, 24),
      bubble: bubble,
      visible: visible,
    );

    expect(r.size, bubble);
  });
}
