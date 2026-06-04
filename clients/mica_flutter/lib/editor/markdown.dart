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

/// Spec thematic break: 3+ of the same -/*/_ with optional spaces between.
bool isThematicBreak(String line) {
  var marker = '';
  var count = 0;
  for (var i = 0; i < line.length; i++) {
    final c = line[i];
    if (c == ' ' || c == '\t') continue;
    if (c == '-' || c == '*' || c == '_') {
      if (marker.isEmpty) {
        marker = c;
      } else if (c != marker) {
        return false;
      }
      count++;
    } else {
      return false;
    }
  }
  return count >= 3;
}

/// `===`/`---` underline (≤3 leading spaces) → setext heading level, or 0.
int setextLevel(String raw) {
  final lead = raw.length - raw.trimLeft().length;
  if (lead > 3) return 0;
  final t = raw.trim();
  if (t.isEmpty) return 0;
  if (t.split('').every((c) => c == '=')) return 1;
  if (t.split('').every((c) => c == '-')) return 2;
  return 0;
}

/// Strip [columns] of leading indentation (tabs = 4-column stops).
String deindentColumns(String line, int columns) {
  var col = 0;
  for (var i = 0; i < line.length; i++) {
    final c = line[i];
    if (c == ' ') {
      col += 1;
    } else if (c == '\t') {
      col = (col ~/ 4 + 1) * 4;
    } else {
      return line.substring(i);
    }
    if (col >= columns) {
      return ' ' * (col - columns) + line.substring(i + 1);
    }
  }
  return '';
}

List<BlockSpec> markdownToBlocks(String markdown) {
  final result = <BlockSpec>[];
  final lines = markdown.replaceAll('\r\n', '\n').split('\n');
  // Leading-width stack mapping source indentation columns to nesting levels
  // (tolerates 2/3/4-space styles; tabs count as 4) — mirrors the Rust
  // engine. Only list/todo items nest; other blocks reset the stack.
  final listStack = <int>[];
  int leadCol(String raw, String content) {
    var col = 0;
    for (var k = 0; k < raw.length - content.length; k++) {
      col = raw.codeUnitAt(k) == 0x09 ? (col ~/ 4 + 1) * 4 : col + 1;
    }
    return col;
  }

  int listLevel(String raw, String content) {
    final col = leadCol(raw, content);
    while (listStack.isNotEmpty && col < listStack.last) {
      listStack.removeLast();
    }
    if (listStack.isEmpty || col > listStack.last) {
      listStack.add(col);
    }
    return listStack.length - 1;
  }
  final fenceOpen = RegExp(r'^```(\w*)\s*$');
  final fenceClose = RegExp(r'^```\s*$');
  final heading = RegExp(r'^(#{1,6})\s+(.*)$');
  final todo = RegExp(r'^[-*]\s+\[([ xX])\]\s+(.*)$');
  final bullet = RegExp(r'^[-*]\s+(.*)$');
  final numbered = RegExp(r'^\d+\.\s+(.*)$');
  final quote = RegExp(r'^>\s?(.*)$');
  final image = RegExp(r'^!\[([^\]]*)\]\(([^)\s]+)\)$');

  var i = 0;
  while (i < lines.length) {
    final raw = lines[i].trimRight();

    final open = fenceOpen.firstMatch(raw);
    if (open != null) {
      listStack.clear();
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
      listStack.clear();
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

    // Indented code block: 4+ columns at top level (inside a list that
    // means nesting instead).
    if (listStack.isEmpty && leadCol(raw, raw.trimLeft()) >= 4) {
      final code = <String>[];
      var blanks = 0;
      while (i < lines.length) {
        final l = lines[i].trimRight();
        if (l.trim().isEmpty) {
          blanks++;
          i++;
          continue;
        }
        if (leadCol(l, l.trimLeft()) < 4) break;
        code.addAll(List.filled(blanks, ''));
        blanks = 0;
        code.add(deindentColumns(l, 4));
        i++;
      }
      result.add((kind: 'code_block', text: code.join('\n'), data: {}));
      continue;
    }

    // A horizontal rule (`---`, `***`, `___`, `- - -`) becomes a divider.
    if (isThematicBreak(line)) {
      listStack.clear();
      result.add((kind: 'divider', text: '', data: {}));
      i++;
      continue;
    }

    // A standalone image: ![alt](url). The url is kept as an external source;
    // the editor renders it and (when wired) can re-host it to avoid dead links.
    final img = image.firstMatch(line);
    if (img != null) {
      result.add((
        kind: 'image',
        text: img.group(1)!.trim(),
        data: {'url': img.group(2)!.trim()},
      ));
      i++;
      continue;
    }

    final h = heading.firstMatch(line);
    if (h != null) {
      listStack.clear();
      result.add(_inline('heading', h.group(2)!.trim(), {'level': h.group(1)!.length}));
      i++;
      continue;
    }

    final t = todo.firstMatch(line);
    if (t != null) {
      final level = listLevel(raw, line);
      result.add(_inline('todo', t.group(2)!.trim(), {
        'checked': t.group(1)!.toLowerCase() == 'x',
        if (level > 0) 'indent': level,
      }));
      i++;
      continue;
    }

    final b = bullet.firstMatch(line);
    if (b != null) {
      final level = listLevel(raw, line);
      result.add(_inline(
        'bulleted_list',
        b.group(1)!.trim(),
        {if (level > 0) 'indent': level},
      ));
      i++;
      continue;
    }

    final n = numbered.firstMatch(line);
    if (n != null) {
      final level = listLevel(raw, line);
      result.add(_inline(
        'numbered_list',
        n.group(1)!.trim(),
        {if (level > 0) 'indent': level},
      ));
      i++;
      continue;
    }

    final q = quote.firstMatch(line);
    if (q != null) {
      listStack.clear();
      result.add(_inline('quote', q.group(1)!.trim(), {}));
      i++;
      continue;
    }

    listStack.clear();
    // Setext heading: this paragraph line underlined by `===`/`---`.
    if (i + 1 < lines.length) {
      final level = setextLevel(lines[i + 1]);
      if (level > 0) {
        result.add(_inline('heading', line, {'level': level}));
        i += 2;
        continue;
      }
    }
    result.add(_inline('paragraph', line, {}));
    i++;
  }

  if (result.isEmpty) {
    result.add((kind: 'paragraph', text: '', data: {}));
  }
  return result;
}
