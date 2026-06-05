import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/markdown.dart';
import 'package:mica_flutter/editor/marks.dart';

void main() {
  group('inline footnote reference', () {
    test('strips to bare label under a footnote mark', () {
      final parsed = parseInline('Text[^1] more.');
      // Bracket+caret syntax gone; only the label remains in the text.
      expect(parsed.text, 'Text1 more.');
      final m = parsed.marks.singleWhere((m) => m.type == 'footnote');
      expect(m.href, '1');
    });

    test('round-trips through inline export', () {
      final parsed = parseInline('See[^a] and[^a] again[^gfm-spec].');
      final back = inlineToMarkdown(parsed.text, parsed.marks);
      expect(back, 'See[^a] and[^a] again[^gfm-spec].');
    });

    test('undefined reference still round-trips (degrades to literal label)', () {
      final parsed = parseInline('A dangling [^missing] note.');
      expect(parsed.text, 'A dangling missing note.'); // bare label kept inline
      final back = inlineToMarkdown(parsed.text, parsed.marks);
      expect(back, 'A dangling [^missing] note.');
    });

    test('a label with whitespace or brackets is NOT a footnote', () {
      final parsed = parseInline('not [^a b] one.');
      expect(parsed.marks.any((m) => m.type == 'footnote'), isFalse);
    });
  });

  group('footnote definition block', () {
    test('parses to a footnote_def carrying label and inline content', () {
      final blocks = markdownToBlocks('[^n]: see **here**.');
      final def = blocks.singleWhere((b) => b.kind == 'footnote_def');
      expect(def.data['label'], 'n');
      expect(def.text, 'see here.');
      final marks = (def.data['marks'] as List).cast<Map>();
      expect(marks.any((m) => m['type'] == 'bold'), isTrue);
    });

    test('a multi-line definition joins 4-column continuation lines', () {
      final blocks =
          markdownToBlocks('[^long]: first line\n    second line');
      final def = blocks.singleWhere((b) => b.kind == 'footnote_def');
      expect(def.text, 'first line\nsecond line');
    });

    test('definition content with a footnote ref round-trips inline', () {
      final blocks = markdownToBlocks('[^x]: links to[^y] another.');
      final def = blocks.singleWhere((b) => b.kind == 'footnote_def');
      final back = inlineToMarkdown(
          def.text, marksFromData(def.data));
      expect(back, 'links to[^y] another.');
    });
  });
}
