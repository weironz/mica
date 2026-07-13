import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/html_to_markdown.dart';

// Regression: pasting rich text used to drop bold / inline-code / italic /
// strike — htmlToMarkdown flattened every inline element except <a>/<img> to
// plain text, so parseInline downstream never saw the markers. These lock in
// that inline formatting now round-trips to Markdown markers, from both
// semantic tags AND styled spans (how Google Docs / Word encode it).
void main() {
  group('htmlToMarkdown — inline emphasis (semantic tags)', () {
    test('<strong> and <b> → **bold**', () {
      expect(htmlToMarkdown('<p>a <strong>b</strong> c</p>'), 'a **b** c');
      expect(htmlToMarkdown('<p>a <b>b</b> c</p>'), 'a **b** c');
    });

    test('<em> and <i> → *italic*', () {
      expect(htmlToMarkdown('<p>a <em>b</em> c</p>'), 'a *b* c');
      expect(htmlToMarkdown('<p>a <i>b</i> c</p>'), 'a *b* c');
    });

    test('inline <code> → `code`', () {
      expect(htmlToMarkdown('<p>run <code>ls -la</code> now</p>'), 'run `ls -la` now');
    });

    test('<del>/<s>/<strike> → ~~strike~~', () {
      expect(htmlToMarkdown('<p><del>x</del></p>'), '~~x~~');
      expect(htmlToMarkdown('<p><s>y</s></p>'), '~~y~~');
    });
  });

  group('htmlToMarkdown — inline emphasis (styled spans: Google Docs / Word)', () {
    test('font-weight:700 / bold → **bold**', () {
      expect(htmlToMarkdown('<p><span style="font-weight:700">g</span></p>'), '**g**');
      expect(htmlToMarkdown('<p><span style="font-weight:bold">g</span></p>'), '**g**');
    });

    test('font-style:italic → *italic*', () {
      expect(htmlToMarkdown('<p><span style="font-style:italic">it</span></p>'), '*it*');
    });

    test('text-decoration line-through → ~~strike~~', () {
      expect(
        htmlToMarkdown('<p><span style="text-decoration: line-through">z</span></p>'),
        '~~z~~',
      );
    });

    test('normal weight span is NOT bolded', () {
      expect(htmlToMarkdown('<p><span style="font-weight:400">n</span></p>'), 'n');
    });
  });

  group('htmlToMarkdown — edge cases', () {
    test('flanking whitespace stays OUTSIDE the markers (CommonMark)', () {
      // `**b **` would not parse as emphasis; the trailing space must move out.
      expect(htmlToMarkdown('<p>x<strong>b </strong>y</p>'), 'x**b** y');
    });

    test('nested bold+italic → ***x***', () {
      expect(htmlToMarkdown('<p><strong><em>x</em></strong></p>'), '***x***');
    });

    test('code containing a backtick gets a longer fence', () {
      expect(htmlToMarkdown('<p><code>a`b</code></p>'), '``a`b``');
    });

    test('whitespace-only emphasis emits no markers', () {
      expect(htmlToMarkdown('<p>a<strong> </strong>b</p>'), 'a b');
    });
  });

  group('htmlToMarkdown — bare inline (one Typora paragraph, no <p> wrapper)', () {
    test('coalesces top-level inline runs into ONE paragraph with marks', () {
      const html =
          '<strong>判断标准(BF3):</strong> <code>ip -br link</code> 显示 UP;'
          '<code>no-carrier</code> = 没插光纤';
      final md = htmlToMarkdown(html);
      expect(
        md.contains('\n'),
        isFalse,
        reason: 'one paragraph, not one block per inline element',
      );
      expect(md, contains('**判断标准(BF3):**'));
      expect(md, contains('`ip -br link`'));
      expect(md, contains('`no-carrier`'));
    });

    test('text + bold + text at top level stays one line', () {
      expect(htmlToMarkdown('a <b>b</b> c'), 'a **b** c');
    });

    test('still splits when a real block element separates inline runs', () {
      expect(htmlToMarkdown('<b>x</b><p>para</p><i>y</i>'), '**x**\n\npara\n\n*y*');
    });
  });

  group('htmlToMarkdown — no regression on links / blocks', () {
    test('links still become [text](href)', () {
      expect(
        htmlToMarkdown('<p>see <a href="https://x.dev">here</a></p>'),
        'see [here](https://x.dev)',
      );
    });

    test('bold inside a link keeps both', () {
      expect(
        htmlToMarkdown('<p><a href="https://x.dev"><strong>go</strong></a></p>'),
        '[**go**](https://x.dev)',
      );
    });

    test('heading + emphasis together', () {
      expect(htmlToMarkdown('<h2>Title with <code>x</code></h2>'), '## Title with `x`');
    });
  });
}
