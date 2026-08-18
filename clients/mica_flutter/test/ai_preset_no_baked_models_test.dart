import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/main.dart';

/// No provider preset may seed a model NAME.
///
/// It used to seed `deepseek-chat`. On 2026-08-19 a live `/v1/models` against
/// api.deepseek.com returned only `deepseek-v4-flash` and `deepseek-v4-pro` —
/// the seeded name was gone, and the production instance was still configured
/// with it. A retired model id does not announce itself: it looks like a
/// working default until a completion fails for reasons that point nowhere near
/// the preset table. Base URLs are stable enough to ship; model catalogues are
/// not, which is why there is a fetch button instead.
void main() {
  test('no preset ships a model name', () {
    for (final preset in aiPresetsForTest) {
      expect(
        preset.seedModel,
        isEmpty,
        reason:
            '${preset.label} seeds "${preset.seedModel}" — vendors retire model '
            'ids between releases, so this build cannot make that claim. Leave '
            'it blank and let the fetch button answer.',
      );
    }
  });

  test('every preset still ships a base URL', () {
    // The counterpart: endpoints DO belong in the build — they change rarely,
    // and without one there is nothing to fetch a model list from.
    for (final preset in aiPresetsForTest) {
      expect(preset.baseUrlValue, startsWith('http'), reason: preset.label);
    }
  });
}
