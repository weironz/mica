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

/// What the Clipboard HTML flavor is known to lose, per fixture. Only the
/// listed aspect is erased before comparing, so everything else in that
/// fixture is still guarded (skipping the whole file hid two reader bugs).
/// Every loss predates S3: the Rust writer is byte-identical to the old Dart
/// one on the 30 port samples, and the reader is unchanged.
enum _Loss {
  /// `<a href>` / `<img src alt>` carry no title attribute.
  title,

  /// Lines of one `<p>` join with a space on read (quote / list item / lazy
  /// continuation soft breaks, and an item's blank-line paragraph break).
  /// Each newline run becomes one space; mark offsets are remapped.
  softBreak,

  /// `*` vs `-` bullet flavor is not expressed in HTML.
  marker,

  /// HTML lists have no per-item looseness; they re-import tight.
  loose,

  /// Adjacent ordered lists are written as ONE `<ol>`, so a list that
  /// restarts (`3)` after `1.`) or starts at 7 is renumbered on read.
  listStart,

  /// GFM callout type (`> [!NOTE]`) rides in no attribute.
  alert,

  /// Raw inline HTML is written as escaped text, so it comes back without
  /// its `html` mark (the text itself survives).
  rawHtml,

  /// Adjacent quote groups (`data.qbreak`) merge: consecutive
  /// `<blockquote>`s carry no group boundary the reader keeps.
  quoteGroups,

  /// A bare `>` line (empty quote paragraph) yields no block on read.
  emptyQuote,

  /// Italic inside a quote is dropped ON PURPOSE by the reader (c1185887):
  /// Word / WeChat style whole quotes italic, which is decoration, not
  /// emphasis. A paste heuristic, not a dialect gap.
  quoteItalic,
}

const _losses = <String, Set<_Loss>>{
  '03-lists.md': {_Loss.marker},
  '05-quote.md': {_Loss.softBreak},
  '13-links.md': {_Loss.title},
  '15-list-items.md': {
    _Loss.softBreak,
    _Loss.loose,
    _Loss.marker,
    _Loss.listStart,
  },
  '16-images.md': {_Loss.title},
  '17-quotes.md': {_Loss.softBreak, _Loss.quoteGroups, _Loss.emptyQuote},
  '19-entities-defs.md': {_Loss.title},
  '21-html.md': {_Loss.rawHtml},
  '22-gfm.md': {_Loss.softBreak},
  '24-callouts.md': {_Loss.alert, _Loss.quoteItalic},
  '34-link-title-escape.md': {_Loss.title},
  '42-lazy-item-continuation.md': {_Loss.softBreak},
};

/// Fixtures whose loss changes text LENGTH (so every later mark offset moves)
/// or block structure, which no per-aspect erasure can line up. Keep this
/// list short; a new entry needs the same kind of reason.
const _notComparable = <String, String>{
  // `\` + newline comes back as two spaces + newline (one char longer).
  '18-spans-breaks.md': 'hard-break marker flavor differs over HTML',
  // Raw HTML inside an item: the reader collapses its whitespace runs.
  '20-item-containers.md': 'whitespace runs around raw HTML collapse on read',
  // The reader trims / collapses code-span edge whitespace (shorter text).
  '38-hardbreak-spans.md': 'code-span edge whitespace is normalized by the reader',
  '40-code-span-strip.md': 'code-span edge whitespace is normalized by the reader',
  // An empty `<pre><code>` yields no block at all on read.
  '33-tilde-fallback.md': 'an empty code block does not survive the HTML flavor',
  // Past the 32-level inline nesting budget the HTML reader re-nests.
  '43-inline-nesting-budget.md':
      'inline nesting beyond the 32-level budget re-nests differently',
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
      final reason = _notComparable[name];
      if (reason != null) {
        markTestSkipped('$name: $reason');
        return;
      }
      final losses = _losses[name] ?? const <_Loss>{};
      final source = md.readAsStringSync();
      final blocks = markdownCodec.parseBlocks(source);
      final html = markdownCodec.renderClipboardHtml(blocks);
      final back = markdownCodec.parseBlocks(htmlToMarkdown(html));
      expect(
        _normalize(back, losses),
        _normalize(blocks, losses),
        reason: 'copy → paste inside Mica changed the content in $name',
      );
    });
  }
}

/// Content-only view of a block list: kind, text, schema data keys, and marks
/// as a sorted set.
List<Map<String, dynamic>> _normalize(
  List<BlockSpec> blocks,
  Set<_Loss> losses,
) {
  return [
    for (final b in blocks)
      if (!(losses.contains(_Loss.emptyQuote) &&
          b.kind == 'quote' &&
          b.text.isEmpty))
        _block(b, losses),
  ];
}

Map<String, dynamic> _block(BlockSpec b, Set<_Loss> losses) {
  var text = b.text;
  var data = b.data;
  if (losses.contains(_Loss.softBreak) && text.contains('\n')) {
    // at[i] = where old offset i lands once each newline run is one space.
    final at = <int>[];
    final out = StringBuffer();
    for (var i = 0; i < text.length; i++) {
      at.add(out.length);
      if (text[i] != '\n') {
        out.write(text[i]);
      } else if (i == 0 || text[i - 1] != '\n') {
        out.write(' ');
      }
    }
    at.add(out.length);
    text = out.toString();
    final marks = data['marks'];
    if (marks is List) {
      data = {
        ...data,
        'marks': [
          for (final m in marks)
            if (m is Map)
              {...m, 'start': at[m['start'] as int], 'end': at[m['end'] as int]},
        ],
      };
    }
  }
  return {
    'kind': b.kind,
    'text': text,
    'data': _schemaData(
      data,
      losses,
      quoted: b.kind == 'quote' || b.data['quote'] != null,
    ),
  };
}

Map<String, dynamic> _schemaData(
  Map<String, dynamic> data,
  Set<_Loss> losses, {
  required bool quoted,
}) {
  final out = <String, dynamic>{};
  final erased = {
    if (losses.contains(_Loss.title)) 'title',
    if (losses.contains(_Loss.marker)) 'marker',
    if (losses.contains(_Loss.loose)) 'loose',
    if (losses.contains(_Loss.listStart)) 'start',
    if (losses.contains(_Loss.alert)) 'alert',
    if (losses.contains(_Loss.quoteGroups)) 'qbreak',
  };
  for (final key in _schemaKeys) {
    if (!data.containsKey(key) || erased.contains(key)) continue;
    final value = data[key];
    if (key == 'marks' && value is List) {
      final marks = [
        for (final m in value)
          if (m is Map &&
              !(losses.contains(_Loss.rawHtml) && m['type'] == 'html') &&
              !(quoted &&
                  losses.contains(_Loss.quoteItalic) &&
                  m['type'] == 'italic'))
            {
              'start': m['start'],
              'end': m['end'],
              'type': m['type'],
              if (m['href'] != null) 'href': m['href'],
              if (m['title'] != null && !losses.contains(_Loss.title))
                'title': m['title'],
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