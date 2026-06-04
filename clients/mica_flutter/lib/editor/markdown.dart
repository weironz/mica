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
  _OpenItem(this.index, this.kind, this.contentCol, this.raw, this.base,
      {this.qdepth = 0, this.endsHard = false});
  final int index;
  final String kind;
  final int contentCol;
  String raw;
  final Map<String, dynamic> base;
  bool hadBlank = false;
  final int qdepth;
  bool endsHard;
}

/// A fence opener: 3+ backticks or tildes; a backtick fence's info string
/// may not contain backticks.
({String ch, int len, String info})? _fenceOpen(String content) {
  if (content.isEmpty) return null;
  final c = content[0];
  if (c != '`' && c != '~') return null;
  var n = 0;
  while (n < content.length && content[n] == c) {
    n++;
  }
  if (n < 3) return null;
  final info = content.substring(n);
  if (c == '`' && info.contains('`')) return null;
  return (ch: c, len: n, info: info);
}

/// A fence closer: a run of the opening char at least as long, nothing else.
bool _fenceClose(String content, String ch, int len) {
  final t = content.trimRight();
  if (t.length < len) return false;
  for (var k = 0; k < t.length; k++) {
    if (t[k] != ch) return false;
  }
  return true;
}

/// Strip leading `>` quote markers (one optional space each; ≤3 spaces may
/// sit between nested markers) → (depth, rest). Mirrors the Rust engine.
(int, String) stripQuoteMarkers(String content) {
  var i = 0;
  var depth = 0;
  while (true) {
    var j = i;
    var spaces = 0;
    while (j < content.length && content[j] == ' ') {
      j++;
      spaces++;
    }
    if (spaces > 3 || j >= content.length || content[j] != '>') break;
    j++;
    if (j < content.length && content[j] == ' ') j++;
    depth++;
    i = j;
  }
  return (depth, content.substring(i));
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

  // Pass 1: link reference definitions (possibly spanning lines) — they
  // resolve case-insensitively and vanish. Definitions inside fences don't
  // count, and a definition can't interrupt a paragraph.
  final defs = <String, ({String dest, String? title})>{};
  final defLines = <int>{};
  _collectRefDefinitions(lines, defs, defLines);
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
  // Block quotes (flat model: `data.quote` = depth): is a group open, did a
  // blank just close one (next quote gets `qbreak`), pending empty group.
  var quoteActive = false;
  var quoteBoundary = false;
  int? pendingEmptyQuote;
  int quoteDepthOf(BlockSpec b) {
    final d = (b.data['quote'] as int?) ?? 0;
    return b.kind == 'quote' ? (d < 1 ? 1 : d) : d;
  }
  void resetListState() {
    listStack.clear();
    open = null;
    lastList = null;
    pendingLoose = false;
  }

  final heading = RegExp(r'^(#{1,6})([ \t]+(.*))?$');
  // Strip an ATX closing sequence: trailing `#`s preceded by a space (or
  // making up the whole text).
  String stripAtxClosing(String text) {
    final t = text.trimRight();
    var k = t.length;
    while (k > 0 && t[k - 1] == '#') {
      k--;
    }
    if (k == t.length) return t;
    if (k == 0) return '';
    if (t[k - 1] == ' ') return t.substring(0, k - 1).trimRight();
    return t;
  }

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
      final body = (h.group(3) ?? '').trim();
      return ('heading', stripAtxClosing(body), {'level': h.group(1)!.length});
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
    // Two or more trailing spaces on the source line = a hard line break if
    // a continuation joins (canonicalized to a backslash break).
    final endsHard = lines[i].endsWith('  ');

    if (line.isEmpty) {
      // Blank lines between items keep the list context, end the open
      // paragraph (items merely note the blank), and make the list loose
      // if it continues.
      if (open != null &&
          (open!.kind == 'bulleted_list' ||
              open!.kind == 'numbered_list' ||
              open!.kind == 'todo')) {
        open!.hadBlank = true;
      } else {
        open = null;
      }
      pendingLoose = pendingLoose || listStack.isNotEmpty;
      // A content-less `>` group becomes an empty quote block; a blank
      // after a quote separates blockquotes (the next gets `qbreak`).
      final pending = pendingEmptyQuote;
      if (pending != null) {
        result.add((kind: 'quote', text: '', data: {
          if (pending > 1) 'quote': pending,
          if (quoteBoundary) 'qbreak': true,
        }));
        pendingEmptyQuote = null;
      }
      if (result.isNotEmpty && quoteDepthOf(result.last) > 0) {
        quoteBoundary = true;
      }
      quoteActive = false;
      i++;
      continue;
    }
    final col = leadCol(raw, line);

    // A 4+-column line cannot start a new block while a top-level
    // paragraph-like block is open — it is lazy continuation text.
    if (col >= 4 && listStack.isEmpty && open != null && !open!.hadBlank) {
      open!.raw = '${open!.raw}${open!.endsHard ? '\\\n' : '\n'}$line';
      open!.endsHard = endsHard;
      reapply(open!);
      i++;
      continue;
    }

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

    final fence = col < 4 ? _fenceOpen(line) : null;
    if (fence != null) {
      resetListState();
      // Only the first word of the info string is the language.
      final infoWords = fence.info.trim().split(RegExp(r'\s+'));
      final language = unescapeMd(infoWords.isEmpty ? '' : infoWords.first);
      final buffer = <String>[];
      i++;
      while (i < lines.length) {
        final l = lines[i];
        final lt = l.trimLeft();
        if (l.length - lt.length < 4 && _fenceClose(lt, fence.ch, fence.len)) {
          break;
        }
        // Content lines shed up to the opening fence's indentation.
        buffer.add(col > 0 ? deindentColumns(l, col) : l);
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
      final blanks = <String>[];
      while (i < lines.length) {
        final l = lines[i];
        if (l.trim().isEmpty) {
          // Blank-ish lines keep indentation past the 4-column margin.
          blanks.add(deindentColumns(l, 4));
          i++;
          continue;
        }
        if (leadCol(l, l.trimLeft()) < 4) break;
        code.addAll(blanks);
        blanks.clear();
        // Trailing spaces are code content — keep the line untrimmed.
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

    // Block quote: strip `>` markers → depth + rest (mirrors Rust).
    final (qdepth, qrest) = stripQuoteMarkers(line);
    if (qdepth > 0) {
      listStack.clear();
      lastList = null;
      pendingLoose = false;
      final qrestTrim = qrest.trimLeft();

      // `>` with nothing after: a paragraph break inside the quote — or,
      // if the group never gets content, an empty blockquote.
      if (qrestTrim.isEmpty) {
        if (!quoteActive && pendingEmptyQuote == null) {
          pendingEmptyQuote = qdepth;
        }
        open = null;
        quoteActive = true;
        i++;
        continue;
      }

      // Lazy/marked continuation of the open quoted paragraph.
      final qc = classifyLine(qrestTrim);
      if (qc.$1 == 'paragraph' &&
          !qrestTrim.startsWith('```') &&
          open != null &&
          open!.qdepth >= qdepth &&
          !open!.hadBlank) {
        open!.raw = '${open!.raw}${open!.endsHard ? '\\\n' : '\n'}$qrestTrim';
        open!.endsHard = endsHard;
        reapply(open!);
        quoteActive = true;
        i++;
        continue;
      }

      pendingEmptyQuote = null;
      final qbreak = quoteBoundary;
      quoteBoundary = false;

      // Fenced code inside the quote: runs while the markers do.
      if (qrestTrim.startsWith('```')) {
        final language = qrestTrim.substring(3).trim();
        final buffer = <String>[];
        i++;
        while (i < lines.length) {
          final lcontent = lines[i].trimRight().trimLeft();
          final (d2, r2) = stripQuoteMarkers(lcontent);
          if (d2 < qdepth) break; // the quote ended — so does the fence
          if (r2.trimLeft().startsWith('```')) {
            i++;
            break;
          }
          buffer.add(r2);
          i++;
        }
        result.add((kind: 'code_block', text: buffer.join('\n'), data: {
          if (language.isNotEmpty) 'language': language,
          'quote': qdepth,
          if (qbreak) 'qbreak': true,
        }));
        open = null;
        quoteActive = true;
        continue;
      }

      // Indented code inside the quote (per marked line).
      if (qrest.length - qrestTrim.length >= 4) {
        result.add((kind: 'code_block', text: deindentColumns(qrest, 4), data: {
          'quote': qdepth,
          if (qbreak) 'qbreak': true,
        }));
        open = null;
        quoteActive = true;
        i++;
        continue;
      }

      if (isThematicBreak(qrestTrim)) {
        result.add((kind: 'divider', text: '', data: {
          'quote': qdepth,
          if (qbreak) 'qbreak': true,
        }));
        open = null;
        quoteActive = true;
        i++;
        continue;
      }

      // Quoted content block: plain text becomes the `quote` kind (depth in
      // `data.quote` past 1); any other kind carries `data.quote`.
      final qkind = qc.$1 == 'paragraph' ? 'quote' : qc.$1;
      final qtext = qc.$2;
      final qdata = Map<String, dynamic>.of(qc.$3);
      if (qkind != 'quote' || qdepth > 1) qdata['quote'] = qdepth;
      if (qbreak) qdata['qbreak'] = true;
      if (qkind == 'image') {
        result.add((
          kind: 'image',
          text: parseInline(qtext, defs: defs).text,
          data: qdata,
        ));
        open = null;
      } else {
        final qbase = Map<String, dynamic>.of(qdata);
        result.add(_inline(qkind, qtext, qdata, defs: defs));
        open = (qkind == 'quote' ||
                qkind == 'bulleted_list' ||
                qkind == 'numbered_list' ||
                qkind == 'todo')
            ? _OpenItem(result.length - 1, qkind, 0, qtext, qbase,
                qdepth: qdepth, endsHard: endsHard)
            : null;
      }
      quoteActive = true;
      i++;
      continue;
    }

    final c = classifyLine(line);
    var kind = c.$1;
    var text = c.$2;
    var data = Map<String, dynamic>.of(c.$3);
    // An indented (4+ columns) marker cannot start a list at top level —
    // the line is paragraph continuation or code, never a new item.
    if ((kind == 'bulleted_list' || kind == 'numbered_list' || kind == 'todo') &&
        col >= 4 &&
        listStack.isEmpty) {
      kind = 'paragraph';
      text = line;
      data = {};
    }

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
          open!.raw = open!.raw.isEmpty
              ? line
              : '${open!.raw}${open!.endsHard ? '\\\n' : '\n'}$line';
          open!.endsHard = endsHard;
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
      open = _OpenItem(result.length - 1, kind, contentCol, text, base,
          endsHard: endsHard);
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
      open = _OpenItem(result.length - 1, 'paragraph', 0, line, {},
          endsHard: endsHard);
    }
    i++;
  }

  final pendingTail = pendingEmptyQuote;
  if (pendingTail != null) {
    result.add((kind: 'quote', text: '', data: {
      if (pendingTail > 1) 'quote': pendingTail,
      if (quoteBoundary) 'qbreak': true,
    }));
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

/// Scan all link reference definitions, fence-aware and multi-line —
/// mirrors the Rust engine's collect_ref_definitions.
void _collectRefDefinitions(List<String> lines,
    Map<String, ({String dest, String? title})> defs, Set<int> defLines) {
  ({String ch, int len})? inFence;
  var prevPara = false;
  var i = 0;
  while (i < lines.length) {
    final line = lines[i].trimRight();
    final content = line.trimLeft();
    final col = line.length - content.length;
    if (inFence != null) {
      if (_fenceClose(content, inFence.ch, inFence.len)) inFence = null;
      prevPara = false;
      i++;
      continue;
    }
    final fence = col < 4 ? _fenceOpen(content) : null;
    if (fence != null) {
      inFence = (ch: fence.ch, len: fence.len);
      prevPara = false;
      i++;
      continue;
    }
    if (content.isEmpty) {
      prevPara = false;
      i++;
      continue;
    }
    if (!prevPara && col < 4 && content.startsWith('[')) {
      final def = _parseRefDefinitionMulti(lines, i);
      if (def != null) {
        defs.putIfAbsent(
            normalizeLabel(def.label), () => (dest: def.dest, title: def.title));
        for (var k = i; k < i + def.used; k++) {
          defLines.add(k);
        }
        i += def.used;
        continue; // a definition doesn't open a paragraph
      }
    }
    prevPara = true;
    i++;
  }
}

({String label, String dest, String? title, int used})? _parseRefDefinitionMulti(
    List<String> lines, int i) {
  final first = lines[i].trim();
  final close = matchingBracket(first, 0);
  if (close < 0 || close + 1 >= first.length || first[close + 1] != ':') {
    return null;
  }
  final label = first.substring(1, close);
  if (label.trim().isEmpty) return null;
  for (var k = 1; k < close; k++) {
    if ((first[k] == '[' || first[k] == ']') && first[k - 1] != r'\') return null;
  }
  final afterColon = first.substring(close + 2).trim();
  var used = 1;
  String destLine;
  if (afterColon.isEmpty) {
    if (i + 1 >= lines.length) return null;
    final l2 = lines[i + 1].trim();
    if (l2.isEmpty) return null;
    used = 2;
    destLine = l2;
  } else {
    destLine = afterColon;
  }
  final d = _parseDefDest(destLine);
  if (d == null) return null;
  if (d.rest.isNotEmpty) {
    if (!d.hadWs) return null; // same-line title needs whitespace before it
    final t = _parseDefTitle(lines, i + used - 1, d.rest);
    if (t == null) return null;
    return (label: label, dest: d.dest, title: t.$1, used: used + t.$2);
  }
  if (i + used < lines.length) {
    final nt = lines[i + used].trim();
    if (nt.isNotEmpty && (nt[0] == '"' || nt[0] == "'" || nt[0] == '(')) {
      final t = _parseDefTitle(lines, i + used, nt);
      if (t != null) {
        return (label: label, dest: d.dest, title: t.$1, used: used + 1 + t.$2);
      }
    }
  }
  return (label: label, dest: d.dest, title: null, used: used);
}

({String dest, String rest, bool hadWs})? _parseDefDest(String s) {
  if (s.startsWith('<')) {
    final gt = s.indexOf('>', 1);
    if (gt < 0) return null;
    final rest = s.substring(gt + 1);
    final hadWs = rest.isEmpty || rest.startsWith(' ') || rest.startsWith('\t');
    return (dest: unescapeMd(s.substring(1, gt)), rest: rest.trim(), hadWs: hadWs);
  }
  final m = RegExp(r'[ \t]').firstMatch(s);
  final end = m?.start ?? s.length;
  if (end == 0) return null;
  return (
    dest: unescapeMd(s.substring(0, end)),
    rest: s.substring(end).trim(),
    hadWs: end < s.length,
  );
}

(String, int)? _parseDefTitle(List<String> lines, int lineIdx, String start) {
  final open = start[0];
  final close = switch (open) { '"' => '"', "'" => "'", '(' => ')', _ => null };
  if (close == null) return null;
  final body = start.substring(1);
  final pos = _findUnescapedChar(body, close);
  if (pos >= 0) {
    if (body.substring(pos + 1).trim().isNotEmpty) return null;
    return (unescapeMd(body.substring(0, pos)), 0);
  }
  var acc = body;
  var extra = 0;
  while (true) {
    extra++;
    if (lineIdx + extra >= lines.length) return null;
    final lt = lines[lineIdx + extra].trimRight();
    final p2 = _findUnescapedChar(lt, close);
    if (p2 >= 0) {
      if (lt.substring(p2 + 1).trim().isNotEmpty) return null;
      acc = '$acc\n${lt.substring(0, p2)}';
      return (unescapeMd(acc), extra);
    }
    acc = '$acc\n$lt';
  }
}

int _findUnescapedChar(String s, String target) {
  var prevBackslash = false;
  for (var idx = 0; idx < s.length; idx++) {
    final c = s[idx];
    if (c == target && !prevBackslash) return idx;
    prevBackslash = c == r'\' && !prevBackslash;
  }
  return -1;
}
