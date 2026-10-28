// Differential equivalence test for the issue-#61 render rewrite.
//
// inlineToMarkdown used to pick each span by a min-over-all-marks scan (O(M)
// per span, O(M²) per export). The rewrite streams the start-sorted marks with
// an active heap + start-group ties instead. This file pins OUTPUT equivalence
// by fuzzing the live serializer against the old algorithm, whose mark-
// selection scan is reproduced as `oldInlineToMarkdown` below. The old
// selection scan remains the reference. Serialization rules fixed by public
// round-trip regressions in markdown_engine_regressions_test.dart are shared
// with production: dollar-bearing math, whitespace code padding and decoded
// destination/title escaping. The old $$$ form loses a literal dollar.
//
// The HTML half of this comparison died with the Dart HTML writer in S3 T3.6:
// the clipboard's text/html flavor is produced by the Rust engine now.
import 'dart:math';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/marks.dart';

// ---------------------------------------------------------------------------
// OLD implementation (verbatim from the pre-#61 _renderSpan), renamed so it
// can sit next to the new one.
// ---------------------------------------------------------------------------

bool _oldRendersTerminal(String kind) =>
    kind == 'code' || kind == 'math' || kind == 'html' || kind == 'footnote';

String oldInlineToMarkdown(String text, List<Mark> marks) {
  if (marks.isEmpty) return text;
  return _oldRenderSpan(text, 0, text.length, marks);
}

String _oldRenderSpan(String text, int lo, int hi, List<Mark> marks) {
  final out = StringBuffer();
  var pos = lo;
  while (pos < hi) {
    Mark? pick;
    var ps = 0, pe = 0;
    var pickTerm = false;
    for (final m in marks) {
      final s = m.start < pos ? pos : m.start;
      final e = m.end > hi ? hi : m.end;
      if (e <= s) continue;
      final term = _oldRendersTerminal(m.type);
      if (pick == null ||
          s < ps ||
          (s == ps && e > pe) ||
          (s == ps && e == pe && (pickTerm == term || (pickTerm && !term)))) {
        pick = m;
        ps = s;
        pe = e;
        pickTerm = term;
      }
    }
    if (pick == null) {
      out.write(escapeInline(text.substring(pos, hi)));
      break;
    }
    var lead = escapeInline(text.substring(pos, ps));
    if (pick.type == 'link' && lead.endsWith('!')) {
      lead = '${lead.substring(0, lead.length - 1)}\\!';
    }
    out.write(lead);
    if (pick.type == 'html') {
      out.write(text.substring(ps, pe));
      pos = pe;
      continue;
    }
    if (pick.type == 'math') {
      final source = text.substring(ps, pe);
      out.write(source.contains(kDollar) ? '\\($source\\)' : '$kDollar$source$kDollar');
      pos = pe;
      continue;
    }
    if (pick.type == 'footnote') {
      final label = pick.href ?? text.substring(ps, pe);
      out.write('[^$label]');
      pos = pe;
      continue;
    }
    if (pick.type == 'code') {
      final raw = text.substring(ps, pe);
      var longest = 0, cur = 0;
      for (var k = 0; k < raw.length; k++) {
        if (raw[k] == '`') {
          cur++;
          if (cur > longest) longest = cur;
        } else {
          cur = 0;
        }
      }
      final fence = '`' * (longest + 1);
      final pad =
          raw.startsWith('`') ||
          raw.endsWith('`') ||
          (raw.startsWith(' ') && raw.endsWith(' ') && raw.codeUnits.any((c) => c != 0x20));
      out.write(pad ? '$fence $raw $fence' : '$fence$raw$fence');
      pos = pe;
      continue;
    }
    final inner = [
      for (final m in marks)
        if (!identical(m, pick) && m.end > ps && m.start < pe) m,
    ];
    final body = _oldRenderSpan(text, ps, pe, inner);
    switch (pick.type) {
      case 'bold':
        out.write('**$body**');
      case 'italic':
        out.write('*$body*');
      case 'strike':
        out.write('~~$body~~');
      case 'link':
        final href = pick.href ?? '';
        final plain = text.substring(ps, pe);
        if (pick.title == null &&
            inner.isEmpty &&
            plain.startsWith('www.') &&
            href == 'http://$plain') {
          out.write(plain);
        } else if (pick.title == null &&
            inner.isEmpty &&
            (href == plain || href == 'mailto:$plain')) {
          out.write('<$plain>');
        } else {
          final dest = escapeMarkdownDestination(href);
          final t = pick.title;
          out.write(
            t == null
                ? '[$body]($dest)'
                : '[$body]($dest "${escapeMarkdownTitle(t)}")',
          );
        }
      case 'image':
        final href = pick.href ?? '';
        final dest = escapeMarkdownDestination(href);
        final t = pick.title;
        out.write(
          t == null
              ? '![$body]($dest)'
              : '![$body]($dest "${escapeMarkdownTitle(t)}")',
        );
      default:
        out.write(body);
    }
    pos = pe;
  }
  return out.toString();
}

void main() {
  const alphabet = [
    'a',
    'b',
    ' ',
    '*',
    '_',
    '`',
    '~',
    '\$',
    '\\',
    '(',
    ')',
    '[',
    ']',
    '<',
    '>',
    '&',
    ';',
    ':',
    'w',
    '.',
    '/',
    '@',
    '"',
    "'",
    '\n',
  ];
  const types = [
    'bold',
    'italic',
    'code',
    'strike',
    'link',
    'image',
    'math',
    'footnote',
    'html',
  ];

  List<Mark> randomMarks(Random rng, int len) {
    final out = <Mark>[];
    final n = rng.nextInt(9);
    for (var i = 0; i < n; i++) {
      final a = rng.nextInt(len + 1);
      var b = a + 1 + rng.nextInt(4);
      if (b > len + 2) b = len + 2; // ends may run past the text — legal
      if (a >= b) continue;
      final type = types[rng.nextInt(types.length)];
      out.add(
        Mark(
          a,
          b,
          type,
          href: (type == 'link' || type == 'image' || type == 'footnote')
              ? [
                  'https://x.co/${rng.nextInt(9)}',
                  'u${rng.nextInt(9)}',
                  'mailto:a@b.c',
                  'lab${rng.nextInt(9)}',
                ][rng.nextInt(4)]
              : null,
          title: (type == 'link' || type == 'image') && rng.nextBool()
              ? 't "q" ${rng.nextInt(9)}'
              : null,
        ),
      );
    }
    return out;
  }

  String randomText(Random rng) {
    final n = rng.nextInt(40);
    return [
      for (var i = 0; i < n; i++) alphabet[rng.nextInt(alphabet.length)],
    ].join();
  }

  test('inlineToMarkdown matches the pre-rewrite scan (fuzz)', () {
    final rng = Random(6100);
    final cases = <(String, List<Mark>)>[];
    for (var i = 0; i < 4000; i++) {
      final text = randomText(rng);
      cases.add((text, randomMarks(rng, text.length)));
    }
    // Targeted corners: equal ranges with terminal/nestable picks, degenerate
    // ends, plain-text edge cases.
    cases.add(('abcd', [Mark(0, 4, 'code'), Mark(0, 4, 'link', href: '/x')]));
    cases.add(('abcd', [Mark(0, 4, 'link', href: '/x'), Mark(0, 4, 'code')]));
    cases.add(('a', [Mark(0, 1, 'bold'), Mark(0, 1, 'italic')]));
    cases.add(('abc', [Mark(0, 3, 'bold'), Mark(1, 2, 'italic')]));
    cases.add(('www.x', [Mark(0, 5, 'link', href: 'http://www.x')]));
    cases.add(('a@b.c', [Mark(0, 5, 'link', href: 'mailto:a@b.c')]));
    for (final (i, c) in cases.indexed) {
      final (text, marks) = c;
      expect(
        inlineToMarkdown(text, marks),
        oldInlineToMarkdown(text, marks),
        reason: 'markdown case $i text=$text marks=$marks',
      );
    }
  });

}
