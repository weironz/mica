// How long a big rich paste blocks the UI isolate on desktop. The whole
// conversion is synchronous there: tokenize (package:html) → element tree →
// JSON → FFI → engine → Markdown, and the paste then parses that Markdown
// again as a fragment. Office HTML is the realistic worst case: every run of
// text is a <span style=…>, so the tree is several times the visible text.
//
// Measured 2026-11-01 (Linux, JIT test VM, release engine): 100 KB → 19 ms
// total (the old all-Dart converter took 33 ms); 1 MB → ~235 ms (old: 145 ms
// convert-only; the extra is the tree → JSON → FFI hand-off). Big pastes stay
// synchronous: a 1 MB clipboard is rare, and an async paste would need the
// caret and selection to survive an await. The gates are ~2x the measured
// numbers. They catch a regression in the hand-off or the engine, not
// scheduler noise on a slow runner.
// VM-only: it drives the real FFI codec.
@TestOn('vm')
library;

import 'package:flutter_test/flutter_test.dart';
import 'package:html/parser.dart' as html_parser;
import 'package:mica_flutter/editor/html_to_markdown.dart';
import 'package:mica_flutter/markdown/markdown_codec.dart';

import '../support/markdown_codec.dart';

/// Word-shaped clipboard HTML of roughly [kb] kilobytes: styled paragraphs,
/// a nested list and a data table per section.
String officeHtml(int kb) {
  const span = '<span style="font-family:Calibri;font-size:11pt;color:#1F1F1F">';
  final sb = StringBuffer('<html><body><!--StartFragment-->');
  var n = 0;
  while (sb.length < kb * 1024) {
    n++;
    sb.write('<h2>${span}Section $n</span></h2>');
    for (var p = 0; p < 6; p++) {
      sb.write('<p class="MsoNormal">${span}Paragraph $p of section $n has ');
      sb.write('</span><b>${span}bold</span></b>$span, </span>');
      sb.write('<i>${span}italic *starred* text</span></i>$span and a ');
      sb.write('<a href="https://example.com/$n/$p">${span}link</span></a>.</span></p>');
    }
    sb.write('<ul><li>${span}one</span></li><li>${span}two</span>'
        '<ul><li>${span}two-a</span></li></ul></li></ul>');
    sb.write('<table><tr><th>Name</th><th>Qty</th><th>Note</th></tr>');
    for (var r = 0; r < 8; r++) {
      sb.write('<tr><td>${span}item $r</span></td><td>$r</td>'
          '<td><b>note</b> | pipe</td></tr>');
    }
    sb.write('</table>');
  }
  sb.write('<!--EndFragment--></body></html>');
  return sb.toString();
}

int _ms(Stopwatch sw) => sw.elapsedMicroseconds ~/ 1000;

/// Median of [runs] timings of [f], in ms.
int median(int runs, void Function() f) {
  final xs = <int>[];
  for (var i = 0; i < runs; i++) {
    final sw = Stopwatch()..start();
    f();
    xs.add(_ms(sw));
  }
  xs.sort();
  return xs[xs.length ~/ 2];
}

void main() {
  setUpAll(useNativeMarkdownCodec);

  for (final (kb, budgetMs) in [(100, 50), (1024, 500)]) {
    test('$kb KB office paste: convert + fragment parse within $budgetMs ms', () {
      final html = officeHtml(kb);
      // Warm up the JIT and the engine once.
      markdownCodec.parseBlocks(htmlToMarkdown(html), fragment: true);

      final tokenize = median(3, () => html_parser.parse(html));
      late String md;
      final convert = median(3, () => md = htmlToMarkdown(html));
      final parse = median(
        3,
        () => markdownCodec.parseBlocks(md, fragment: true),
      );
      final total = convert + parse;
      // ignore: avoid_print
      print('html-paste ${kb}KB: html ${html.length} B → md ${md.length} B; '
          'tokenize $tokenize ms, convert (incl. tokenize) $convert ms, '
          'fragment parse $parse ms, total $total ms');
      expect(md, contains('Section 1'));
      expect(total, lessThan(budgetMs));
    });
  }
}
