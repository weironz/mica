/// Minimal Markdown → block-spec parser used to insert AI-generated content
/// into the live document. It maps each Markdown line to a backend block kind
/// (paragraph/heading/list/todo/quote/code_block). Inline marks (**bold** etc.)
/// are not yet applied — that arrives with the inline-marks milestone — so inline
/// syntax is currently kept verbatim in the text.
library;

import 'table.dart';

/// A block to create: its kind, plain text, and data map (heading level, etc.).
typedef BlockSpec = ({String kind, String text, Map<String, dynamic> data});

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
      result.add((
        kind: 'heading',
        text: h.group(2)!.trim(),
        data: {'level': h.group(1)!.length},
      ));
      i++;
      continue;
    }

    final t = todo.firstMatch(line);
    if (t != null) {
      result.add((
        kind: 'todo',
        text: t.group(2)!.trim(),
        data: {'checked': t.group(1)!.toLowerCase() == 'x'},
      ));
      i++;
      continue;
    }

    final b = bullet.firstMatch(line);
    if (b != null) {
      result.add((kind: 'bulleted_list', text: b.group(1)!.trim(), data: {}));
      i++;
      continue;
    }

    final n = numbered.firstMatch(line);
    if (n != null) {
      result.add((kind: 'numbered_list', text: n.group(1)!.trim(), data: {}));
      i++;
      continue;
    }

    final q = quote.firstMatch(line);
    if (q != null) {
      result.add((kind: 'quote', text: q.group(1)!.trim(), data: {}));
      i++;
      continue;
    }

    result.add((kind: 'paragraph', text: line, data: {}));
    i++;
  }

  if (result.isEmpty) {
    result.add((kind: 'paragraph', text: '', data: {}));
  }
  return result;
}
