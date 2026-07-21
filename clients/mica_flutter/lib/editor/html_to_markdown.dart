/// Convert pasted clipboard HTML to Markdown, preserving block structure
/// (headings, lists, code, tables, links, images). Pure Dart via package:html,
/// so it runs EVERYWHERE — the desktop clipboard pulls (rich_paste_stub.dart)
/// and the web paste listener (rich_paste_web.dart) share this one converter;
/// there is deliberately no per-platform mirror to drift out of sync.
///
/// Fidelity contract: what the source page SHOWED is what lands in the
/// document. Text-node content is backslash-escaped so literal `*`/`#`/`>`/…
/// cannot re-parse as structure; markers this converter emits itself (`- `,
/// `# `, `> `, `**`) are added after escaping and stay live.
library;

import 'package:html/dom.dart' as dom;
import 'package:html/parser.dart' as html_parser;

import 'marks.dart' show mathRunSpans;

String htmlToMarkdown(String source) {
  final doc = html_parser.parse(source);
  final body = doc.body ?? doc.documentElement;
  if (body == null) return '';
  final out = StringBuffer();
  _noItalic = false;
  _inQuote = false;
  _msoIndents = null;
  _emitChildren(body.nodes, out);
  return out.toString().replaceAll(RegExp(r'\n{3,}'), '\n\n').trim();
}

/// While true, `_inlineMarks` drops 'italic' — set inside `<blockquote>`
/// subtrees. Sources habitually STYLE quote text italic (Word's quote styles,
/// WeChat articles' inline css, many site themes); that is presentation, not
/// emphasis, and mica renders quotes upright (bar + muted ink). Converting the
/// decoration to a real `*italic*` mark made every pasted quote come out
/// slanted. (File-private mutable state is fine: conversion is synchronous.)
bool _noItalic = false;

/// While true, we are inside a `<blockquote>` subtree: nested-list indentation
/// is capped below 4 columns, because the quote-side parser has no list stack
/// and 4+ leading columns inside a quote become an indented CODE block.
bool _inQuote = false;

/// Backslash-escape characters that re-parse as INLINE Markdown (emphasis,
/// code spans, strikethrough, link brackets, raw HTML `<`, inline math `$`).
/// The parser unescapes any backslash-escaped ASCII punctuation, so the page's
/// literal text survives the round trip unchanged.
///
/// EXCEPT valid math runs, which pass through verbatim. LLM answers copied
/// from a browser arrive as HTML with literal `$\eta = 2$` in the text;
/// escaping their `$` (and doubling the `\` of every LaTeX command) meant a
/// pasted formula could never become a math mark — while the same text pasted
/// as PLAIN text parsed fine. [mathRunSpans] applies the same Pandoc rules the
/// downstream parser does, so exactly what would parse is what survives; a
/// lone `$` with no valid closer (prices, "$5 and $10") still escapes.
String _escapeMdInline(String s) {
  final spans = mathRunSpans(s);
  if (spans.isEmpty) return _escapeAllMdInline(s);
  final out = StringBuffer();
  var cursor = 0;
  for (final r in spans) {
    out.write(_escapeAllMdInline(s.substring(cursor, r.start)));
    out.write(s.substring(r.start, r.end));
    cursor = r.end;
  }
  out.write(_escapeAllMdInline(s.substring(cursor)));
  return out.toString();
}

String _escapeAllMdInline(String s) =>
    s.replaceAllMapped(RegExp(r'[\\`*_~\[\]<$]'), (m) => '\\${m[0]}');

/// Escape a paragraph-level LINE that would re-parse as a block construct.
/// `*`/`` ` ``/`~` starts are already dead via [_escapeMdInline]; this covers
/// the rest: ATX headings, quotes, `-`/`+` bullets, ordered markers, breaks.
String _escapeBlockStart(String line) {
  final nm = RegExp(r'^(\d{1,9})([.)])(\s|$)').firstMatch(line);
  if (nm != null) {
    final digits = nm.group(1)!;
    return '$digits\\${line.substring(digits.length)}';
  }
  if (RegExp(r'^(?:#{1,6}|[-+])(?:\s|$)').hasMatch(line) ||
      line.startsWith('>') ||
      RegExp(r'^-{3,}\s*$').hasMatch(line)) {
    return '\\$line';
  }
  return line;
}

/// Inline elements (whitelist). Anything else — `p`, `div`, headings, lists,
/// `table`, `img`, unknown custom elements — is treated as block-level.
const _inlineTags = {
  'a',
  'strong',
  'b',
  'em',
  'i',
  'code',
  's',
  'del',
  'strike',
  'u',
  'mark',
  'sub',
  'sup',
  'small',
  'span',
  'font',
  'br',
  'wbr',
  'abbr',
  'cite',
  'q',
  'kbd',
  'samp',
  'var',
  'time',
  'label',
  'ins',
  'big',
  'tt',
};

bool _isInlineNode(dom.Node n) =>
    n is dom.Text || (n is dom.Element && _inlineTags.contains(_tag(n)));

/// Emit a node list, coalescing runs of consecutive inline siblings (text +
/// `<strong>`/`<code>`/… ) into ONE Markdown paragraph. Without this, a
/// clipboard fragment that is bare inline content (a single paragraph copied
/// from Typora arrives with no `<p>` wrapper) would put every `<strong>`/`<code>`
/// on its own line and drop its emphasis — the inline markers are only added
/// when the element is *gathered*, not when `_node` treats it as a block.
void _emitChildren(List<dom.Node> nodes, StringBuffer out) {
  final run = <dom.Node>[];
  void flush() {
    if (run.isEmpty) return;
    final sb = StringBuffer();
    for (final n in run) {
      _gatherOne(n, sb);
    }
    run.clear();
    final text = sb.toString().replaceAll(RegExp(r'\s+'), ' ').trim();
    if (text.isNotEmpty) {
      out.writeln(_escapeBlockStart(text));
      out.writeln();
    }
  }

  for (final n in nodes) {
    if (_isInlineNode(n)) {
      run.add(n);
    } else {
      flush();
      _node(n, out);
    }
  }
  flush();
}

String _tag(dom.Element e) => (e.localName ?? '').toLowerCase();

void _node(dom.Node node, StringBuffer out) {
  if (node is dom.Text) {
    final text = _escapeMdInline(
      node.text,
    ).replaceAll(RegExp(r'\s+'), ' ').trim();
    if (text.isNotEmpty) {
      out.writeln(_escapeBlockStart(text));
      out.writeln();
    }
    return;
  }
  if (node is! dom.Element) return;

  final tag = _tag(node);
  switch (tag) {
    case 'h1':
    case 'h2':
    case 'h3':
    case 'h4':
    case 'h5':
    case 'h6':
      final level = int.parse(tag.substring(1));
      final text = _inline(node);
      if (text.isNotEmpty) {
        out.writeln('${'#' * level} $text');
        out.writeln();
      }
    case 'p':
      // mica's own copy flavor: a footnote definition travels as
      // `<p data-mica-fndef="label">…</p>` — reconstruct the GFM leader
      // (escaping would have killed a literal `[^label]:`).
      final fndef = node.attributes['data-mica-fndef'];
      if (fndef != null) {
        final text = _inline(node);
        out.writeln('[^$fndef]: $text');
        out.writeln();
        return;
      }
      if (_msoListItem(node, out)) return;
      final text = _inline(node);
      if (text.isNotEmpty) {
        out.writeln(_escapeBlockStart(text));
        out.writeln();
      }
    case 'pre':
      _fencedCode(node, out, '');
      out.writeln();
    case 'blockquote':
      _emitQuote(node, out, '');
    case 'ul':
      _list(node, out, ordered: false);
      out.writeln();
    case 'ol':
      _list(node, out, ordered: true);
      out.writeln();
    case 'table':
      _table(node, out);
    case 'hr':
      out.writeln('---');
      out.writeln();
    case 'br':
      out.writeln();
    case 'img':
      final src = _cleanSrc(node.attributes['src']);
      if (src != null) {
        out.writeln(
          '![${_escapeMdInline(node.attributes['alt'] ?? '')}]'
          '(${_wrapDest(src)})',
        );
        out.writeln();
      }
    case 'dl':
      // Markdown has no definition list: each term becomes a bold line, each
      // definition its own paragraph — instead of one glued-together blob.
      for (final child in node.children) {
        final ct = _tag(child);
        if (ct != 'dt' && ct != 'dd') continue;
        final text = _inline(child);
        if (text.isEmpty) continue;
        out.writeln(ct == 'dt' ? '**$text**' : _escapeBlockStart(text));
        out.writeln();
      }
    case 'div':
    case 'section':
    case 'article':
    case 'main':
    case 'header':
    case 'footer':
    case 'aside':
    case 'nav':
    case 'figure':
    case 'picture':
    case 'span':
      _emitChildren(node.nodes, out);
    default:
      if (node.querySelector(
            'table, ul, ol, pre, blockquote, h1, h2, h3, h4, h5, h6, p, img',
          ) !=
          null) {
        _emitChildren(node.nodes, out);
        return;
      }
      final text = _inline(node);
      if (text.isNotEmpty) {
        out.writeln(_escapeBlockStart(text));
        out.writeln();
      }
  }
}

/// Convert a `<blockquote>`'s children RECURSIVELY (paragraphs, lists, nested
/// quotes keep their block structure), then prefix every produced line with
/// `> ` (a bare `>` on blanks keeps it one quote group). Italics are
/// suppressed inside: see [_noItalic]. [indent] prefixes each line when the
/// quote is itself a list-item child.
void _emitQuote(dom.Element node, StringBuffer out, String indent) {
  final inner = StringBuffer();
  final savedItalic = _noItalic;
  final savedQuote = _inQuote;
  _noItalic = true;
  _inQuote = true;
  _emitChildren(node.nodes, inner);
  _noItalic = savedItalic;
  _inQuote = savedQuote;
  final body = inner
      .toString()
      .replaceAll(RegExp(r'\n{3,}'), '\n\n')
      .trimRight();
  if (body.isEmpty) return;
  for (final line in body.split('\n')) {
    out.writeln(line.isEmpty ? '$indent>' : '$indent> $line');
  }
  out.writeln();
}

/// `<pre>` → a fenced code block at [indent] (list items use their content
/// column so the parser attaches the fence as an item child). The language
/// comes from the `language-*`/`lang-*` class on `<pre>` or its `<code>`.
void _fencedCode(dom.Element pre, StringBuffer out, String indent) {
  final cls =
      '${pre.attributes['class'] ?? ''} '
      '${pre.querySelector('code')?.attributes['class'] ?? ''}';
  final lang =
      RegExp(r'(?:language-|lang-)([\w+#-]+)').firstMatch(cls)?.group(1) ?? '';
  out.writeln('$indent```$lang');
  for (final l in _codeText(pre).trimRight().split('\n')) {
    out.writeln('$indent$l');
  }
  out.writeln('$indent```');
}

/// The GFM task-list checkbox of a list item, or null: the first
/// `<input type=checkbox>` whose nearest enclosing `<li>` is [item] itself.
dom.Element? _taskCheckbox(dom.Element item) {
  for (final input in item.querySelectorAll('input')) {
    if ((input.attributes['type'] ?? '').toLowerCase() != 'checkbox') continue;
    dom.Node? p = input.parent;
    while (p != null && !identical(p, item)) {
      if (p is dom.Element && _tag(p) == 'li') break;
      p = p.parent;
    }
    if (p != null && identical(p, item)) return input;
  }
  return null;
}

void _list(
  dom.Element list,
  StringBuffer out, {
  required bool ordered,
  String indent = '',
}) {
  // `<ol start="5">` — seed the numbering so the list round-trips its start.
  var index = int.tryParse(list.attributes['start'] ?? '') ?? 1;
  // Indent for a list nested as a DIRECT child of this list ("sibling"
  // nesting — `<ul><li>a</li><ul>…</ul></ul>` is invalid HTML but Word and
  // older editors emit it, and html5 parsing keeps it): it belongs under the
  // last item we wrote.
  var lastChildIndent = _capQuoteIndent('$indent  ');
  for (final item in _listItems(list)) {
    final t = _tag(item);
    if (t == 'ul' || t == 'ol') {
      _list(item, out, ordered: t == 'ol', indent: lastChildIndent);
      continue;
    }
    if (t != 'li') continue;
    // GFM task list: `<li><input type=checkbox checked>` → a todo marker.
    final box = _taskCheckbox(item);
    final marker = box != null
        ? (box.attributes.containsKey('checked') ? '- [x]' : '- [ ]')
        : (ordered ? '$index.' : '-');
    final text = _directInline(item);
    out.writeln('$indent$marker $text');
    // A child item must be indented to THIS item's content column — `- ` is
    // 2 wide but `12. ` is 4, so a fixed 2-space indent left ordered children
    // shy of the column and the parser read them as top-level. For todos the
    // parser counts only the `- ` as marker (the `[x]` is content).
    final width = box != null ? 2 : marker.length + 1;
    final childIndent = _capQuoteIndent(indent + ' ' * width);
    lastChildIndent = childIndent;
    for (final child in item.children) {
      final ct = _tag(child);
      if (ct == 'ul' || ct == 'ol') {
        _list(child, out, ordered: ct == 'ol', indent: childIndent);
      } else if (ct == 'pre') {
        // The parser supports fenced-code CHILDREN of items (data.li); the
        // old flattening dumped raw newlines into the item line instead.
        _fencedCode(child, out, childIndent);
      } else if (ct == 'blockquote') {
        _emitQuote(child, out, childIndent);
      } else if (ct == 'table') {
        final sb = StringBuffer();
        _table(child, sb);
        for (final l in sb.toString().trimRight().split('\n')) {
          out.writeln('$childIndent$l');
        }
      }
    }
    index++;
  }
}

/// Inside a blockquote, keep list indentation under 4 columns — the parser's
/// quote branch reads deeper indents as code (it has no in-quote list stack).
String _capQuoteIndent(String indent) =>
    _inQuote && indent.length >= 4 ? '   ' : indent;

/// Word/WPS "flat" list paragraphs: no `<ul>` at all — each item is a
/// `<p style='…;mso-list:l0 level2 lfo1'>` whose bullet/number glyph lives in
/// a `mso-list:Ignore` span. Levels in the style are ABSOLUTE (a fragment can
/// start at level2), so indentation is emitted RELATIVE to the run's first
/// item, each level indented to its parent's content column. Returns false
/// when the paragraph is not a Word list item.
///
/// Run state (reset when the previous sibling is not an mso list paragraph):
List<String>? _msoIndents; // relative level → line indent
List<bool>? _msoOrdered; //  relative level → ordered marker (for its width)
int _msoBase = 1;

bool _msoIsListP(dom.Element? e) =>
    e != null &&
    _tag(e) == 'p' &&
    RegExp(
      r'mso-list:[^;"]*\blevel\d+',
      caseSensitive: false,
    ).hasMatch(e.attributes['style'] ?? '');

bool _msoListItem(dom.Element p, StringBuffer out) {
  final style = p.attributes['style'] ?? '';
  final m = RegExp(
    r'mso-list:[^;"]*\blevel(\d+)',
    caseSensitive: false,
  ).firstMatch(style);
  if (m == null) return false;
  final level = (int.tryParse(m.group(1)!) ?? 1).clamp(1, 9);
  var markerText = '';
  for (final span in p.querySelectorAll('span')) {
    final st = (span.attributes['style'] ?? '').toLowerCase();
    if (st.contains('mso-list') && st.contains('ignore')) {
      markerText = span.text.trim();
      span.remove();
      break;
    }
  }
  // `1.` `(1)` `a)` `一、` → ordered; Wingdings glyphs (`·` `l` `§` `Ø`) → bullet.
  final ordered = RegExp(
    r'^\(?([0-9a-zA-Z]{1,3}|[一二三四五六七八九十]{1,3})[.)、]',
  ).hasMatch(markerText);

  if (_msoIndents == null || !_msoIsListP(p.previousElementSibling)) {
    _msoIndents = null; // new run
  }
  var indents = _msoIndents;
  var kinds = _msoOrdered;
  if (indents == null || kinds == null || level < _msoBase) {
    _msoBase = level;
    indents = _msoIndents = [''];
    kinds = _msoOrdered = [ordered];
  }
  var rel = level - _msoBase;
  if (rel >= indents.length) {
    // Deeper: indent to the deepest known item's content column (level jumps
    // of 2+ clamp to one step — the parser could not nest a hole anyway).
    final parentWidth = kinds.last ? 3 : 2; // '1. ' vs '- '
    indents.add(_capQuoteIndent(indents.last + ' ' * parentWidth));
    kinds.add(ordered);
    rel = indents.length - 1;
  } else {
    kinds[rel] = ordered;
    indents.removeRange(rel + 1, indents.length);
    kinds.removeRange(rel + 1, kinds.length);
  }

  final text = _inline(p);
  if (text.isNotEmpty) {
    out.writeln('${indents[rel]}${ordered ? '1.' : '-'} $text');
    // Blank ONLY after the run's last item: a blank between consecutive items
    // used to push the next nested item over the indented-code threshold, and
    // without a trailing blank a following paragraph would lazy-continue INTO
    // the last item.
    if (!_msoIsListP(p.nextElementSibling)) out.writeln();
  }
  return true;
}

void _table(dom.Element table, StringBuffer out) {
  // Only THIS table's rows: querySelectorAll('tr') also returned rows of
  // NESTED tables (Word/Outlook layout nesting), duplicating their content
  // as extra malformed rows.
  final rows = <dom.Element>[];
  void collect(dom.Element el) {
    for (final c in el.children) {
      final t = _tag(c);
      if (t == 'tr') {
        rows.add(c);
      } else if (t == 'thead' || t == 'tbody' || t == 'tfoot') {
        collect(c);
      }
    }
  }

  collect(table);
  if (rows.isEmpty) return;
  var headerWritten = false;
  for (final row in rows) {
    final cells = row.children
        .where((c) => _tag(c) == 'td' || _tag(c) == 'th')
        .toList();
    if (cells.isEmpty) continue;
    String cell(dom.Element c) =>
        _inline(c).replaceAll('|', r'\|').replaceAll('\n', ' ');
    out.writeln('| ${cells.map(cell).join(' | ')} |');
    if (!headerWritten) {
      out.writeln('| ${cells.map((_) => '---').join(' | ')} |');
      headerWritten = true;
    }
  }
  out.writeln();
}

String _inline(dom.Node node) {
  final sb = StringBuffer();
  _gather(node, sb);
  // \s+ (not just spaces/tabs): source-formatting newlines inside a heading
  // or emphasis are inter-word whitespace — kept raw they split the block.
  return sb.toString().replaceAll(RegExp(r'\s+'), ' ').trim();
}

/// A list's items and nested lists, seen THROUGH wrapper elements.
///
/// Per the spec an `<ol>`/`<ul>` may only contain `<li>`, but real copy
/// sources put a wrapper in between — Google's AI overview emits
/// `<ol><div data-bfc><li>…</li></div>…</ol>` — and the HTML parser preserves
/// it. `_list` iterated DIRECT children and skipped anything that was not an
/// `li`, so every item sat inside a skipped `<div>` and the ENTIRE list
/// disappeared from the paste: the user saw the surrounding paragraphs arrive
/// and the numbered list simply not be there.
///
/// Nested `<ul>`/`<ol>` are yielded whole rather than descended into, so a
/// sub-list keeps its own structure (and this cannot hoist grandchildren up a
/// level). Same spirit as the sibling-nesting case `_list` already handles:
/// browsers keep invalid list markup, so the converter has to read it.
Iterable<dom.Element> _listItems(dom.Element list) sync* {
  for (final child in list.children) {
    final t = _tag(child);
    if (t == 'li' || t == 'ul' || t == 'ol') {
      yield child;
    } else {
      yield* _listItems(child);
    }
  }
}

String _directInline(dom.Element item) {
  final sb = StringBuffer();
  for (final child in item.nodes) {
    if (child is dom.Element) {
      final t = _tag(child);
      // Block children are emitted by _list as the item's CHILD blocks;
      // gathering them here leaked raw newlines into the item line (a <pre>'s
      // second line landed at column 0 and was lazily joined or re-parsed).
      if (t == 'ul' ||
          t == 'ol' ||
          t == 'pre' ||
          t == 'blockquote' ||
          t == 'table' ||
          t == 'input') {
        continue;
      }
    }
    // _gatherOne, NOT _gather: _gather descends into the element's children and
    // so drops the wrapper — a `<li>` whose content is `<a href>text</a>` would
    // lose the link (and `<strong>`/`<code>` their emphasis). _gatherOne treats
    // the element itself, emitting `[text](href)` / `**text**` / `` `text` ``.
    _gatherOne(child, sb);
  }
  return sb.toString().replaceAll(RegExp(r'\s+'), ' ').trim();
}

String _codeText(dom.Node node) {
  final sb = StringBuffer();
  _codeGather(node, sb);
  return sb.toString();
}

void _codeGather(dom.Node node, StringBuffer sb) {
  for (final child in node.nodes) {
    if (child is dom.Text) {
      sb.write(child.text);
    } else if (child is dom.Element) {
      final tag = _tag(child);
      if (tag == 'br') {
        _appendNewline(sb);
      } else {
        _codeGather(child, sb);
        if (tag == 'div' || tag == 'p' || tag == 'tr' || tag == 'li') {
          _appendNewline(sb);
        }
      }
    }
  }
}

void _appendNewline(StringBuffer sb) {
  final s = sb.toString();
  if (s.isNotEmpty && !s.endsWith('\n')) sb.write('\n');
}

void _gather(dom.Node node, StringBuffer sb) {
  for (final child in node.nodes) {
    _gatherOne(child, sb);
  }
}

/// Append one inline node's Markdown (text or an inline element) to [sb].
void _gatherOne(dom.Node child, StringBuffer sb) {
  if (child is dom.Text) {
    sb.write(_escapeMdInline(child.text));
    return;
  }
  if (child is! dom.Element) return;
  // mica's own copy flavor marks formulas and footnote refs with data-mica-*
  // wrappers (see inlineToHtml in marks.dart) — their content is emitted
  // verbatim so `$…$` / `[^label]` survive the escaping pass and re-parse.
  if (child.attributes.containsKey('data-mica-math')) {
    sb.write(child.text);
    return;
  }
  final fnLabel = child.attributes['data-mica-footnote'];
  if (fnLabel != null) {
    sb.write('[^$fnLabel]');
    return;
  }
  final tag = _tag(child);
  if (tag == 'br') {
    sb.write(' ');
  } else if (tag == 'a') {
    final href = _cleanHref(child.attributes['href']);
    final inner = StringBuffer();
    _gather(child, inner);
    final text = inner.toString().trim();
    if (href != null && text.isNotEmpty) {
      sb.write('[$text](${_wrapDest(href)})');
    } else {
      sb.write(text);
    }
  } else if (tag == 'img') {
    final src = _cleanSrc(child.attributes['src']);
    if (src != null) {
      sb.write(
        '![${_escapeMdInline(child.attributes['alt'] ?? '')}]'
        '(${_wrapDest(src)})',
      );
    }
  } else if (tag == 'code') {
    sb.write(_inlineCode(child.text));
  } else {
    final marks = _inlineMarks(child);
    if (marks.isEmpty) {
      _gather(child, sb);
    } else {
      final inner = StringBuffer();
      _gather(child, inner);
      sb.write(_wrapMarks(inner.toString(), marks));
    }
  }
}

/// A link/image destination: `<>`-wrap when it contains whitespace or parens
/// (an unbalanced `)` would otherwise cut the destination short on re-parse).
String _wrapDest(String dest) =>
    RegExp(r'[\s()]').hasMatch(dest) ? '<$dest>' : dest;

/// Inline formatting marks implied by an element's tag AND its inline `style`
/// — Google Docs / Word emit styled `<span>`s (`font-weight:700`), not
/// `<b>`/`<em>`. Returns any of 'bold' | 'italic' | 'strike'; inline `<code>`
/// is handled separately (literal content).
Set<String> _inlineMarks(dom.Element e) {
  final marks = <String>{};
  switch (_tag(e)) {
    case 'b':
    case 'strong':
      marks.add('bold');
    case 'i':
    case 'em':
      marks.add('italic');
    case 's':
    case 'del':
    case 'strike':
      marks.add('strike');
  }
  final style = (e.attributes['style'] ?? '').toLowerCase();
  if (style.contains('font-weight')) {
    final w = RegExp(
      r'font-weight\s*:\s*([a-z0-9]+)',
    ).firstMatch(style)?.group(1);
    if (w == 'bold' || w == 'bolder' || (int.tryParse(w ?? '') ?? 0) >= 600) {
      marks.add('bold');
    }
  }
  if (RegExp(r'font-style\s*:\s*italic').hasMatch(style)) marks.add('italic');
  if (RegExp(r'text-decoration[^;]*line-through').hasMatch(style)) {
    marks.add('strike');
  }
  if (_noItalic) marks.remove('italic'); // quotes render upright — see flag doc
  return marks;
}

/// Wrap inline content in Markdown emphasis markers, keeping any leading/
/// trailing whitespace OUTSIDE the markers — CommonMark rejects `** x **` as
/// emphasis (a delimiter run can't touch whitespace on its inner side).
String _wrapMarks(String inner, Set<String> marks) {
  final core = inner.trim();
  if (core.isEmpty) return inner;
  final lead = inner.substring(0, inner.length - inner.trimLeft().length);
  final trail = inner.substring(inner.trimRight().length);
  var open = '';
  var close = '';
  if (marks.contains('bold')) {
    open = '$open**';
    close = '**$close';
  }
  if (marks.contains('italic')) {
    open = '$open*';
    close = '*$close';
  }
  if (marks.contains('strike')) {
    open = '$open~~';
    close = '~~$close';
  }
  return '$lead$open$core$close$trail';
}

/// Inline `<code>` → a backtick span. Content is literal; fence with one more
/// backtick than the longest run inside, padding a space when it touches a
/// backtick (CommonMark code-span rules).
String _inlineCode(String raw) {
  final core = raw.trim();
  if (core.isEmpty) return raw;
  final lead = raw.substring(0, raw.length - raw.trimLeft().length);
  final trail = raw.substring(raw.trimRight().length);
  var maxRun = 0, cur = 0;
  for (final u in core.codeUnits) {
    if (u == 0x60) {
      cur++;
      if (cur > maxRun) maxRun = cur;
    } else {
      cur = 0;
    }
  }
  final fence = '`' * (maxRun + 1);
  final pad = (core.startsWith('`') || core.endsWith('`')) ? ' ' : '';
  return '$lead$fence$pad$core$pad$fence$trail';
}

String? _cleanHref(String? href) {
  final h = href?.trim() ?? '';
  if (h.isEmpty || h.startsWith('#') || h.startsWith('javascript:'))
    return null;
  return h;
}

String? _cleanSrc(String? src) {
  final s = src?.trim() ?? '';
  if (s.isEmpty || s.startsWith('data:')) return null;
  return s;
}
