import 'package:flutter/painting.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/render.dart';

// Regression: a quote block baked FontStyle.italic into its BASE style, so an
// italic *mark* had nothing to toggle — quoted text could never be un-italicised
// (toggling the mark left the base italic behind). Emphasis is now left entirely
// to marks; the left bar + muted ink still distinguish a quote.
void main() {
  TextStyle styleOf(String kind) =>
      EditorTheme.styleFor(EditorNode(id: 'n', kind: kind, text: 'x'));

  test('quote base style is upright, not italic', () {
    expect(styleOf('quote').fontStyle, isNot(FontStyle.italic));
  });

  test('a paragraph is upright too (baseline)', () {
    expect(styleOf('paragraph').fontStyle, isNot(FontStyle.italic));
  });
}
