// Copy a link out of Mica, paste it back, and it used to arrive as literal
// Markdown source.
//
// The clipboard's HTML flavor round-trips through `[url](url)` — a link whose
// LABEL is itself a URL — and that shape hit two bugs at once in the inline
// parser:
//
//   1. The nested-link guard (CommonMark forbids a link inside a link) asked
//      "does this label parse to anything with a link mark". A GFM extended
//      autolink produces exactly that mark, so a bare-URL label counted as a
//      nested link and the whole `[...](...)` was refused — leaving the literal
//      source with only the parenthesised half autolinked. That is what the
//      user saw on screen.
//   2. Even once the link was accepted, the label was re-parsed WITH autolinks
//      on, so it got a second link mark of its own: two overlapping marks over
//      one range pointing at different hrefs.
//
// Both are really the same rule — text inside a link never autolinks — and
// both halves are now off in the label path.
//
// **This file is deliberately the same table of cases as the Rust side**
// (`crates/markdown/tests/gfm_extensions.rs`, `a_url_shaped_label_is_still_a_link`).
// Rust is the authority and Dart is the mirror; a case added there belongs here
// too, or the two engines start disagreeing about what one document means.
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/markdown.dart';

/// The single block a one-paragraph source parses to: its text and its marks.
({String text, List<Map<String, dynamic>> marks}) one(String md) {
  final specs = markdownToBlocks(md);
  expect(specs.length, 1, reason: 'one block for: $md');
  final spec = specs.single;
  final marks = (spec.data['marks'] as List?) ?? const [];
  return (text: spec.text ?? '', marks: marks.cast<Map<String, dynamic>>());
}

void main() {
  test('a URL-shaped label is still a link', () {
    final r = one('[https://github.com/a/b](https://github.com/a/b)');
    expect(
      r.text,
      'https://github.com/a/b',
      reason: 'the label becomes the link TEXT, not literal markdown source',
    );
    expect(r.marks.length, 1, reason: 'one mark: ${r.marks}');
    expect(r.marks.single['start'], 0);
    expect(r.marks.single['end'], 22);
    expect(r.marks.single['type'], 'link');
    expect(r.marks.single['href'], 'https://github.com/a/b');
  });

  /// The second bug: with autolinks on inside the label this produced two
  /// overlapping marks, and which one won depended on mark order.
  test('a differing label and href yields exactly one mark', () {
    final r = one('[https://a.example](https://b.example)');
    expect(r.text, 'https://a.example');
    expect(r.marks.length, 1, reason: 'exactly one mark: ${r.marks}');
    expect(r.marks.single['href'], 'https://b.example', reason: 'the href wins');
  });

  /// The rule this must NOT break: a genuinely nested EXPLICIT link is still
  /// refused, per CommonMark.
  test('an explicit link inside a label still refuses the outer link', () {
    expect(one('[a [b](/c) d](/e)').text, contains('['));
  });

  /// And a bare URL in ordinary text still autolinks — the flag is off only
  /// inside the label check, not globally.
  test('a bare URL in running text still autolinks', () {
    final r = one('see https://x.example here');
    expect(r.text, 'see https://x.example here');
    expect(r.marks.length, 1);
    expect(r.marks.single['href'], 'https://x.example');
  });
}
