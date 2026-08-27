// The px↔scale arithmetic behind the settings font-size detents. The pref
// stores a SCALE (px / 16) for backward compatibility with the old free
// percent slider, so the conversion is the part that can silently be wrong —
// an asymmetric pair would make the slider land on one px and store another.
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/main.dart';

void main() {
  test('every detent round-trips px → scale → px exactly', () {
    for (var px = kFontPxMin; px <= kFontPxMax; px++) {
      expect(fontPxFromScale(fontScaleFromPx(px)), px);
    }
  });

  test('16px is exactly scale 1.0 — the default nobody has touched', () {
    expect(fontScaleFromPx(16), 1.0);
    expect(fontPxFromScale(1.0), 16);
  });

  // What an EXISTING install sees on first open after the update: the old
  // slider stored free values like 1.13; they must display as a sane whole px,
  // not crash or clamp to an extreme.
  test('legacy free-slider values round to their nearest detent', () {
    expect(fontPxFromScale(1.13), 18); // 18.08 → 18
    expect(fontPxFromScale(0.85), 14); // old minimum, 13.6 → 14
    expect(fontPxFromScale(1.4), 22); // old maximum, 22.4 → 22
  });

  test('out-of-range values clamp instead of escaping the detents', () {
    expect(fontPxFromScale(0.1), kFontPxMin);
    expect(fontPxFromScale(9.0), kFontPxMax);
    expect(fontScaleFromPx(1), kFontPxMin / kFontBasePx);
    expect(fontScaleFromPx(99), kFontPxMax / kFontBasePx);
  });
}
