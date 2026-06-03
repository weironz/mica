/// Minimal Markdown → block-spec parser used to insert AI-generated content
/// into the live document. It maps each Markdown line to a backend block kind
/// (paragraph/heading/list/todo/quote/code_block). Inline marks (**bold**,
/// *italic*, `code`, ~~strike~~, [link](url)) are parsed into clean text plus
/// `data.marks` via [parseInline]; code blocks and tables keep raw text.
library;

import 'marks.dart';
import 'table.dart';

/// A block to create: its kind, plain text, and data map (heading level, etc.).
typedef BlockSpec = ({String kind, String text, Map<String, dynamic> data});

/// Parse inline Markdown in [text] into clean text + marks, merged into [data].
BlockSpec _inline(String kind, String text, Map<String, dynamic> data) {
  final parsed = parseInline(text);
  final merged = {...data};
  if (parsed.marks.isNotEmpty) {
    merged['marks'] = marksToJson(parsed.marks);
  }
  return (kind: kind, text: parsed.text, data: merged);
}

List<BlockSpec> markdownToBlocks(String markdown) {
  final result = <BlockSpec>[];
  final lines = markdown.replaceAll('\r\n', '\n').split('\n');
  final fenceOpen = RegExp(r'^```(\w*)\s*$');
  final fenceClose = RegExp(r'^```\s*$');
  final heading = RegExp(r'^(#{1,6})\s+(.*)$');
  final todo = RegExp(r'^[-*]\s+\[([ xX])\]\s+(.*)$');
  final bullet = RegExp(r'^[-*]\s+(.*)$');
  final numbered = RegExp(r'^\d+\.\s+(.*)$');
  final quote = RegExp(r'^>\s?(.*)$');
  final divider = RegExp(r'^(-{3,}|\*{3,}|_{3,})$');

  var i = 0;
  while (i < lines.length) {
    final raw = lines[i].trimRight();

    final open = fenceOpen.firstMatch(raw);
    if (open != null) {
      final language = open.group(1) ?? '';
      final buffer = <String>[];
      i++;
      while (i < lines.length && !fenceClose.hasMatch(lines[i].trimRight())) {
        buffer.add(lines[i]);
        i++;
      }
      if (i < lines.length) i++; // consume closing fence
      result.add((
        kind: 'code_block',
        text: buffer.join('\n'),
        data: language.isEmpty ? {} : {'language': language},
      ));
      continue;
    }

    // GFM pipe table (a `|`-row followed by a `| --- |` separator).
    if (looksLikeGfmTable(lines, i)) {
      final parsed = parseGfmTable(lines, i);
      result.add((kind: 'table', text: '', data: parsed.table.toBlockData()));
      i = parsed.next;
      continue;
    }

    final line = raw.trim();
    if (line.isEmpty) {
      i++;
      continue;
    }

    // A horizontal rule has no block type yet; treat as a blank separator.
    if (divider.hasMatch(line)) {
      i++;
      continue;
    }

    final h = heading.firstMatch(line);
    if (h != null) {
      result.add(_inline('heading', h.group(2)!.trim(), {'level': h.group(1)!.length}));
      i++;
      continue;
    }

    final t = todo.firstMatch(line);
    if (t != null) {
      result.add(_inline('todo', t.group(2)!.trim(), {
        'checked': t.group(1)!.toLowerCase() == 'x',
      }));
      i++;
      continue;
    }

    final b = bullet.firstMatch(line);
    if (b != null) {
      result.add(_inline('bulleted_list', b.group(1)!.trim(), {}));
      i++;
      continue;
    }

    final n = numbered.firstMatch(line);
    if (n != null) {
      result.add(_inline('numbered_list', n.group(1)!.trim(), {}));
      i++;
      continue;
    }

    final q = quote.firstMatch(line);
    if (q != null) {
      result.add(_inline('quote', q.group(1)!.trim(), {}));
      i++;
      continue;
    }

    result.add(_inline('paragraph', line, {}));
    i++;
  }

  if (result.isEmpty) {
    result.add((kind: 'paragraph', text: '', data: {}));
  }
  return result;
}
