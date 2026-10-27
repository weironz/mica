// Mica-internal copy → paste round-trip over the Clipboard dialect.
//
// This is the ONLY guard on the cross-language contract "Rust writes the
// clipboard HTML ⇒ Dart's htmlToMarkdown reads it back". Both halves are
// independent implementations, so a change on either side that the fixture
// corpus does not exercise shows up here and nowhere else.
//
//   conformance .md → engine blocks → Clipboard HTML → Dart htmlToMarkdown
//                   → engine blocks → compare
//
// Normalization: marks are compared as a set (the writer and the parser may
// emit equal-range ties in different order — the serializer's tie-break reads
// order, the comparison must not), and only schema keys of `data` are compared
// (bookkeeping keys like `li`/`raw` are transport, not content).
//
// Known exceptions are listed EXPLICITLY below with the reason; a new failure
// is a bug, not a fixture to add here.
@TestOn('vm')
library;

import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/html_to_markdown.dart';
import 'package:mica_flutter/markdown/markdown_codec.dart';

import 'support/markdown_codec.dart';

/// Fixtures the Clipboard dialect cannot round-trip, with the reason.
///
/// Empty is the goal; each entry is a user-visible loss on copy → paste
/// inside Mica and must state what is lost and why it is accepted (S3 triage).
const _knownLossy = <String, String>{
  // A link's TITLE does not survive the HTML flavor: the dialect writes
  // <a href> without a title attribute (the old Dart writer did not either),
  // so a titled link comes back as a plain link. Same loss as before S3.
  '13-links.md': 'link titles are not carried by the clipboard HTML flavor',
  '19-entities-defs.md': 'link titles are not carried by the clipboard HTML flavor',
  // An image's TITLE (alt text survives, title does not): <img src alt> has
  // no title attribute. Same loss as the old Dart writer.
  '16-images.md': 'image titles are not carried by the clipboard HTML flavor',
  // A soft break inside a quote/list item text flattens to a space through
  // the HTML path — htmlToMarkdown joins <p> lines with a space. The old Dart
  // clipboard HTML had the same behavior (its writer emitted the same
  // single <p>), so this is not a regression; re-pasting keeps one block,
  // only the intra-block newline is lost.
  '05-quote.md': 'soft break inside a quote flattens to a space',
  '15-list-items.md': 'soft break inside a list item flattens to a space',
  '17-quotes.md': 'soft break inside a quote flattens to a space',
  // A hard break (`\` + newline) survives as an HTML <br> whose reader side
  // renders back as two spaces — the stored form differs (backslash vs
  // spaces), a pre-existing difference of the HTML read path, unchanged by S3.
  '18-spans-breaks.md': 'hard-break marker flavor differs over HTML',
  // `data.marker` (`*` vs `-`) is transport detail the HTML flavor does not
  // express: every bullet comes back as a `-` item. Content-equal.
  '03-lists.md': 'the `*` bullet marker flavor does not survive HTML',
  // A lazy continuation of a paragraph holding autolinks: the HTML flavor
  // writes it as one <p>, whose lines join with spaces on read (soft-break
  // flattening, same as the quote cases).
  '22-gfm.md': 'soft break in an autolink paragraph flattens to a space',
  // `loose` (a blank line between item paragraphs) does not survive: the
  // HTML list flavor has no per-item looseness, so it re-imports tight.
  // Content-equal; the loose flag is a rendering hint.
  '20-item-containers.md': 'list looseness does not survive the HTML flavor',
  // A raw HTML <b class="x"> inside a paragraph is escaped as literal text on
  // write and comes back without marks — inline raw HTML is not a mark in
  // this dialect (same before S3).
  '21-html.md': 'raw inline HTML does not come back as marks',
  // GFM callouts (`> [!NOTE]`) become plain blockquotes over HTML — the
  // alert type rides in no attribute. Same loss as the old Dart writer.
  '24-callouts.md': 'callout alert type does not survive the HTML flavor',
  // Link titles (same class as 13-links).
  '34-link-title-escape.md': 'link titles are not carried by the clipboard HTML flavor',
  // A code span's leading/trailing whitespace is trimmed by the HTML reader's
  // text extraction (`a code   span b` → `a code span b`); tab-collapse in
  // code spans likewise. The written HTML is correct; the reader normalizes.
  '38-hardbreak-spans.md': 'code-span edge whitespace is normalized by the reader',
  '40-code-span-strip.md': 'code-span edge whitespace is normalized by the reader',
  // An EMPTY code block with a `a~~` language: the HTML flavor writes an
  // empty <pre><code class="language-a~~"> whose reader yields no block.
  '33-tilde-fallback.md': 'an empty code block does not survive the HTML flavor',
  // A lazy continuation inside a list item (soft break, list case).
  '42-lazy-item-continuation.md': 'soft break inside a list item flattens to a space',
  // Beyond the 32-level inline nesting budget the HTML reader re-nests
  // differently than the Markdown parser did — the deep-bracket soup loses
  // a level over HTML. Nothing real nests that deep.
  '43-inline-nesting-budget.md': 'inline nesting beyond the 32-level budget re-nests differently',
};

/// `data` keys that describe the block itself (compared).
const _schemaKeys = {
  'marks',
  'level',
  'checked',
  'indent',
  'start',
  'marker',
  'loose',
  'quote',
  'qbreak',
  'alert',
  'label',
  'language',
  'align',
  'header',
  'rows',
  'file_id',
  'url',
  'name',
  'title',
};

void main() {
  setUpAll(useNativeMarkdownCodec);
  final dir = Directory('../../crates/markdown/tests/fixtures/conformance');

  test('fixture directory is reachable', () {
    expect(dir.existsSync(), isTrue, reason: 'expected ${dir.absolute.path}');
  });

  final mdFiles = dir
      .listSync()
      .whereType<File>()
      .where((f) => f.path.endsWith('.md'))
      .toList()
    ..sort((a, b) => a.path.compareTo(b.path));

  for (final md in mdFiles) {
    final name = md.uri.pathSegments.last;
    test('clipboard round-trip: $name', () {
      final reason = _knownLossy[name];
      if (reason != null) {
        markTestSkipped('$name: $reason');
        return;
      }
      final source = md.readAsStringSync();
      final blocks = markdownCodec.parseBlocks(source);
      final html = markdownCodec.renderClipboardHtml(blocks);
      final back = markdownCodec.parseBlocks(htmlToMarkdown(html));
      expect(
        _normalize(back),
        _normalize(blocks),
        reason: 'copy → paste inside Mica changed the content in $name',
      );
    });
  }
}

/// Content-only view of a block list: kind, text, schema data keys, and marks
/// as a sorted set.
List<Map<String, dynamic>> _normalize(List<BlockSpec> blocks) {
  return [
    for (final b in blocks)
      {
        'kind': b.kind,
        'text': b.text,
        'data': _schemaData(b.data),
      },
  ];
}

Map<String, dynamic> _schemaData(Map<String, dynamic> data) {
  final out = <String, dynamic>{};
  for (final key in _schemaKeys) {
    if (!data.containsKey(key)) continue;
    final value = data[key];
    if (key == 'marks' && value is List) {
      final marks = [
        for (final m in value)
          if (m is Map)
            {
              'start': m['start'],
              'end': m['end'],
              'type': m['type'],
              if (m['href'] != null) 'href': m['href'],
              if (m['title'] != null) 'title': m['title'],
            },
      ]..sort((a, b) => '${a['start']}|${a['end']}|${a['type']}'
          .compareTo('${b['start']}|${b['end']}|${b['type']}'));
      if (marks.isNotEmpty) out[key] = marks;
      continue;
    }
    out[key] = value;
  }
  return out;
}