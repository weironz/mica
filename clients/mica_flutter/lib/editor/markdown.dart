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

/// Ordered-list marker: 1–9 digits + `.` or `)` + space (or end-of-line for
/// an empty item) — mirrors the Rust engine's numbered_list_marker.
({int start, String rest})? numberedListMarker(String content) {
  var d = 0;
  while (d < content.length &&
      content.codeUnitAt(d) >= 0x30 &&
      content.codeUnitAt(d) <= 0x39) {
    d++;
  }
  if (d == 0 || d > 9) return null;
  final start = int.parse(content.substring(0, d));
  final rest = content.substring(d);
  if (rest == '.' || rest == ')') return (start: start, rest: '');
  if (rest.length >= 2 && (rest[0] == '.' || rest[0] == ')') && rest[1] == ' ') {
    return (start: start, rest: rest.substring(2));
  }
  return null;
}

/// A paragraph-like block kept open for multi-line continuation.
class _OpenItem {
  _OpenItem(this.index, this.kind, this.contentCol, this.raw, this.base);
  final int index;
  final String kind;
  final int contentCol;
  String raw;
  final Map<String, dynamic> base;
  bool hadBlank = false;
}

/// Parse inline Markdown in [text] into clean text + marks, merged into [data].
BlockSpec _inline(
  String kind,
  String text,
  Map<String, dynamic> data, {
  Map<String, ({String dest, String? title})> defs = const {},
}) {
  final parsed = parseInline(text, defs: defs);
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

  // Pass 1: link reference definitions — collected, then skipped.
  final defs = <String, ({String dest, String? title})>{};
  final defLines = <int>{};
  for (var k = 0; k < lines.length; k++) {
    final d = parseRefDefinition(lines[k]);
    if (d != null) {
      defs.putIfAbsent(normalizeLabel(d.label), () => (dest: d.dest, title: d.title));
      defLines.add(k);
    }
  }
  // Stack of open list items' CONTENT columns (tabs count as 4): a new item
  // is a child only when its marker reaches the parent's content column —
  // mirrors the Rust engine. Other blocks reset the stack.
  final listStack = <int>[];
  int leadCol(String raw, String content) {
    var col = 0;
    for (var k = 0; k < raw.length - content.length; k++) {
      col = raw.codeUnitAt(k) == 0x09 ? (col ~/ 4 + 1) * 4 : col + 1;
    }
    return col;
  }

  // The most recently pushed paragraph-like block (paragraph or list/todo
  // item), kept open for multi-line continuation — mirrors the Rust engine:
  // lazy lines join with a soft break; for items, a blank + indented line
  // starts a second paragraph (\n\n join, list turns loose).
  _OpenItem? open;
  (int, int, String)? lastList; // (block index, level, marker char)
  var pendingLoose = false;
  void resetListState() {
    listStack.clear();
    open = null;
    lastList = null;
    pendingLoose = false;
  }

  final fenceClose = RegExp(r'^```\s*$');
  final heading = RegExp(r'^(#{1,6})\s+(.*)$');

  // A line that is exactly `![alt](url "title")` — spec destination rules
  // (angle brackets, balanced parens, nested brackets in the alt); other
  // image forms stay inline marks inside a paragraph.
  (String, String, String?)? parseMarkdownImage(String content) {
    if (content.length < 2 || content[0] != '!' || content[1] != '[') {
      return null;
    }
    final close = matchingBracket(content, 1);
    if (close < 0 || close + 1 >= content.length || content[close + 1] != '(') {
      return null;
    }
    final suffix = parseLinkSuffix(content, close + 2);
    if (suffix == null || suffix.next != content.length) return null;
    return (content.substring(2, close), suffix.dest, suffix.title);
  }

  // Map one non-blank line to (kind, text, data) — mirrors the Rust
  // classify_markdown_line (heading, todo, image, bullet, numbered, quote).
  (String, String, Map<String, dynamic>) classifyLine(String content) {
    final h = heading.firstMatch(content);
    if (h != null) {
      return ('heading', h.group(2)!.trim(), {'level': h.group(1)!.length});
    }
    if (content.startsWith('- [ ] ')) {
      return ('todo', content.substring(6), {'checked': false});
    }
    if (content.startsWith('- [x] ') || content.startsWith('- [X] ')) {
      return ('todo', content.substring(6), {'checked': true});
    }
    final img = parseMarkdownImage(content);
    if (img != null) {
      return ('image', img.$1, {
        'url': img.$2,
        if (img.$3 != null) 'title': img.$3,
      });
    }
    if (content.startsWith('- ') || content.startsWith('* ') || content.startsWith('+ ')) {
      return ('bulleted_list', content.substring(2), {});
    }
    if (content == '-' || content == '*' || content == '+') {
      // A bare marker is an empty list item.
      return ('bulleted_list', '', {});
    }
    final nm = numberedListMarker(content);
    if (nm != null) {
      return ('numbered_list', nm.rest, {if (nm.start != 1) 'start': nm.start});
    }
    if (content.startsWith('> ')) {
      return ('quote', content.substring(2), {});
    }
    return ('paragraph', content, {});
  }

  void reapply(_OpenItem o) {
    result[o.index] = _inline(o.kind, o.raw, Map<String, dynamic>.of(o.base), defs: defs);
  }

  var i = 0;
  while (i < lines.length) {
    if (defLines.contains(i)) {
      i++;
      continue;
    }
    final raw = lines[i].trimRight();
    final line = raw.trimLeft();

    if (line.isEmpty) {
      // Blank lines between items keep the list context, end the open
      // paragraph (items merely note the blank), and make the list loose
      // if it continues.
      if (open != null && open!.kind != 'paragraph') {
        open!.hadBlank = true;
      } else {
        open = null;
      }
      pendingLoose = pendingLoose || listStack.isNotEmpty;
      i++;
      continue;
    }
    final col = leadCol(raw, line);

    // Setext underline of the open (possibly multi-line) paragraph: the
    // whole continued paragraph becomes the heading.
    if (open != null && open!.kind == 'paragraph') {
      final level = setextLevel(raw);
      if (level > 0) {
        result[open!.index] = _inline('heading', open!.raw, {'level': level}, defs: defs);
        open = null;
        i++;
        continue;
      }
    }

    if (line.startsWith('```')) {
      resetListState();
      final language = line.substring(3).trim();
      final buffer = <String>[];
      i++;
      while (i < lines.length && !fenceClose.hasMatch(lines[i].trim())) {
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
      resetListState();
      final parsed = parseGfmTable(lines, i);
      result.add((kind: 'table', text: '', data: parsed.table.toBlockData()));
      i = parsed.next;
      continue;
    }

    // Indented code block: 4+ columns outside any open construct (after a
    // blank, a line below the item's content column ends the list).
    if (col >= 4 &&
        ((listStack.isEmpty && open == null) ||
            (open != null && open!.hadBlank && col < open!.contentCol))) {
      resetListState();
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
      resetListState();
      result.add((kind: 'divider', text: '', data: {}));
      i++;
      continue;
    }

    final c = classifyLine(line);
    final kind = c.$1;
    var text = c.$2;
    final data = Map<String, dynamic>.of(c.$3);

    // Continuation (CommonMark): a paragraph line joins the open block with
    // a soft break (lazy lines included); after a blank, a line indented to
    // the item's content column starts a second paragraph inside the item
    // (the list turns loose). An empty list item or an ordered marker other
    // than `1.` cannot interrupt a paragraph — those lines stay text.
    if (open != null) {
      final weakItem = (kind == 'bulleted_list' || kind == 'numbered_list' || kind == 'todo') &&
          open!.kind == 'paragraph' &&
          !open!.hadBlank &&
          (text.isEmpty || data.containsKey('start'));
      if (kind == 'paragraph' || weakItem) {
        if (open!.hadBlank) {
          if (col >= open!.contentCol && open!.raw.isNotEmpty) {
            open!.hadBlank = false;
            open!.raw = '${open!.raw}\n\n$line';
            open!.base['loose'] = true;
            pendingLoose = false;
            reapply(open!);
            i++;
            continue;
          }
          // The blank closed the item; whatever follows is a new block.
        } else {
          open!.raw = open!.raw.isEmpty ? line : '${open!.raw}\n$line';
          reapply(open!);
          i++;
          continue;
        }
      }
    }

    // List/todo items: content column, nesting level, loose / <ol start> /
    // marker-change bookkeeping; the item stays open for continuations.
    if (kind == 'bulleted_list' || kind == 'numbered_list' || kind == 'todo') {
      // Content column: marker width plus up to 3 extra spaces consumed
      // (more means the item starts with indented code — the spaces stay
      // in the text, the column sits right after the marker).
      final markerWidth = line.length - text.length;
      var extra = 0;
      while (extra < text.length && text.codeUnitAt(extra) == 0x20) {
        extra++;
      }
      int contentCol;
      if (text.isEmpty) {
        contentCol = col + markerWidth + 1;
      } else if (extra <= 3) {
        text = text.substring(extra);
        contentCol = col + markerWidth + extra;
      } else {
        contentCol = col + markerWidth;
      }
      while (listStack.isNotEmpty && col < listStack.last) {
        listStack.removeLast();
      }
      final level = listStack.length;
      listStack.add(contentCol);
      if (level > 0) data['indent'] = level;
      // The marker character: `-`/`*`/`+` for bullets and todos, the
      // delimiter (`.`/`)`) for ordered items.
      String markerChar;
      if (kind == 'numbered_list') {
        var d = 0;
        while (line.codeUnitAt(d) >= 0x30 && line.codeUnitAt(d) <= 0x39) {
          d++;
        }
        markerChar = line[d];
      } else {
        markerChar = line[0];
      }
      // Does this item continue the previous run (same level, same kind,
      // same marker)? A marker change starts a new list.
      final continuesRun = lastList != null &&
          lastList!.$2 == level &&
          result[lastList!.$1].kind == kind &&
          lastList!.$3 == markerChar;
      final sameLevelBreak = !continuesRun &&
          lastList != null &&
          lastList!.$2 == level &&
          result[lastList!.$1].kind == kind;
      if (sameLevelBreak) {
        data['marker'] = markerChar;
      }
      // The blank line belongs to whichever list the boundary sits in:
      // same-or-shallower level → this item is loose; deeper level → the
      // blank separated a parent's text from its sublist, so the parent is.
      if (pendingLoose) {
        if (lastList != null && level > lastList!.$2) {
          final prev = result[lastList!.$1];
          result[lastList!.$1] =
              (kind: prev.kind, text: prev.text, data: {...prev.data, 'loose': true});
        } else {
          data['loose'] = true;
        }
        pendingLoose = false;
      }
      // Only the number on the item that BEGINS an ordered run sets the
      // list's start; later numbers are ignored by the spec.
      if (kind == 'numbered_list' && data.containsKey('start') && continuesRun) {
        data.remove('start');
      }
      final base = Map<String, dynamic>.of(data);
      result.add(_inline(kind, text, data, defs: defs));
      lastList = (result.length - 1, level, markerChar);
      open = _OpenItem(result.length - 1, kind, contentCol, text, base);
      i++;
      continue;
    }

    resetListState();
    if (kind == 'image') {
      // The alt is plain text — inline markup flattens (spec alt rule).
      final alt = parseInline(text, defs: defs).text;
      result.add((kind: 'image', text: alt, data: data));
      i++;
      continue;
    }
    result.add(_inline(kind, text, data, defs: defs));
    // Paragraphs stay open for lazy continuation and setext underlines.
    if (kind == 'paragraph') {
      open = _OpenItem(result.length - 1, 'paragraph', 0, line, {});
    }
    i++;
  }

  // Promote a paragraph that is exactly one image (e.g. a reference-form
  // `![alt][label]` on its own line) to an image block; markup inside the
  // alt flattens, the same as the direct `![alt](url)` fast path.
  for (var k = 0; k < result.length; k++) {
    final b = result[k];
    if (b.kind != 'paragraph' || b.text.isEmpty) continue;
    final marks = (b.data['marks'] as List?) ?? const [];
    for (final m in marks) {
      if (m['type'] == 'image' && m['start'] == 0 && m['end'] == b.text.length) {
        result[k] = (kind: 'image', text: b.text, data: {
          'url': m['href'] ?? '',
          if (m['title'] != null) 'title': m['title'],
        });
        break;
      }
    }
  }

  if (result.isEmpty) {
    result.add((kind: 'paragraph', text: '', data: {}));
  }
  return result;
}
