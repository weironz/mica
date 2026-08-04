// A single `~` between word characters is a RANGE, not a strikethrough.
//
// Reported 2026-08-04: "1个汉字 ≈ 1~2个 Token，英文…1个单词 ≈ 1~2个 Token" came
// back as "…≈ 1<del>2个 Token，…≈ 1</del>2个 Token" — the two tildes of two
// separate ranges paired up with each other, and (because this is a WYSIWYG
// editor) the tildes themselves were consumed into the mark, so the writer
// could not get them back by editing the text.
//
// GitHub renders the same thing, so this is a DELIBERATE divergence rather than
// a conformance fix. What GitHub has and Mica does not is a visible source: on
// GitHub you see the tildes and can escape them.
//
// **This file is deliberately the same table of cases as the Rust side**
// (`crates/markdown/tests/gfm_extensions.rs`,
// `a_single_tilde_between_word_characters_is_a_range`). Rust is the authority
// and Dart is the mirror; a case added there belongs here too, or the two
// engines start disagreeing about what one document means.
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/markdown.dart';

/// The single block a one-paragraph source parses to: its text and its marks.
({String text, String marks}) one(String md) {
  final specs = markdownToBlocks(md);
  expect(specs.length, 1, reason: 'one block for: $md');
  final spec = specs.single;
  return (
    text: spec.text ?? '',
    marks: ((spec.data['marks'] as List?) ?? const []).toString(),
  );
}

void main() {
  test('a single tilde between word characters is a range', () {
    final r = one('中文 1个汉字 ≈ 1~2个 Token，英文 1个单词 ≈ 1~2个 Token');
    expect(r.text, contains('1~2个 Token，'), reason: 'tildes survive');
    expect(r.marks, isNot(contains('strike')), reason: 'no strike: ${r.marks}');
  });

  /// The same range written with CJK numerals. The CJK flanking amendment
  /// treats a CJK char as a word BOUNDARY; for `~` it must count as a word
  /// character instead, or this slips through.
  test('a CJK-numeral range is not a strikethrough either', () {
    final r = one('大约 一~二 个，再来 一~二 个');
    expect(r.marks, isNot(contains('strike')), reason: r.marks);
  });

  /// What must keep working: the GFM spec's own single-tilde form, with word
  /// boundaries around it.
  test('a spaced single tilde still strikes', () {
    expect(one('Hello, ~there~ world!').marks, contains('strike'));
  });

  /// The double tilde is untouched in every position, intraword included.
  test('the double tilde is untouched', () {
    expect(one('~~Hi~~ and a1~~b~~c2').marks, contains('strike'));
  });
}
