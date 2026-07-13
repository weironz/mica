/// Convert pasted clipboard HTML to Markdown, preserving block structure
/// (headings, lists, code, tables, links, images). Pure Dart via package:html,
/// so it runs off the web — the desktop counterpart of rich_paste_web.dart's
/// dart:html htmlToMarkdown (kept in sync with it).
library;

import 'package:html/dom.dart' as dom;
import 'package:html/parser.dart' as html_parser;

String htmlToMarkdown(String source) {
  final doc = html_parser.parse(source);
  final body = doc.body ?? doc.documentElement;
  if (body == null) return '';
  final out = StringBuffer();
  _emitChildren(body.nodes, out);
  return out
      .toString()
      .replaceAll(RegExp(r'\n{3,}'), '\n\n')
      .trim();
}

/// Inline elements (whitelist). Anything else — `p`, `div`, headings, lists,
/// `table`, `img`, unknown custom elements — is treated as block-level.
const _inlineTags = {
  'a', 'strong', 'b', 'em', 'i', 'code', 's', 'del', 'strike', 'u', 'mark',
  'sub', 'sup', 'small', 'span', 'font', 'br', 'wbr', 'abbr', 'cite', 'q',
  'kbd', 'samp', 'var', 'time', 'label', 'ins', 'big', 'tt',
};

bool _isInlineNode(dom.Node n) =>
    n is dom.Text ||
    (n is dom.Element && _inlineTags.contains(_tag(n)));

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
    final text = sb.toString().replaceAll(RegExp(r'[ \t]+'), ' ').trim();
    if (text.isNotEmpty) {
      out.writeln(text);
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
    final text = node.text.replaceAll(RegExp(r'\s+'), ' ').trim();
    if (text.isNotEmpty) {
      out.writeln(text);
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
      final text = _inline(node);
      if (text.isNotEmpty) {
        out.writeln(text);
        out.writeln();
      }
    case 'pre':
      out.writeln('```');
      out.writeln(_codeText(node).trimRight());
      out.writeln('```');
      out.writeln();
    case 'blockquote':
      final text = _inline(node);
      if (text.isNotEmpty) {
        for (final line in text.split('\n')) {
          out.writeln('> $line');
        }
        out.writeln();
      }
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
        out.writeln('![${node.attributes['alt'] ?? ''}]($src)');
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
        out.writeln(text);
        out.writeln();
      }
  }
}

void _list(dom.Element list, StringBuffer out,
    {required bool ordered, int depth = 0}) {
  var index = 1;
  for (final item in list.children) {
    if (_tag(item) != 'li') continue;
    final marker = ordered ? '$index.' : '-';
    final text = _directInline(item);
    out.writeln('${'  ' * depth}$marker $text');
    for (final child in item.children) {
      final t = _tag(child);
      if (t == 'ul' || t == 'ol') {
        _list(child, out, ordered: t == 'ol', depth: depth + 1);
      }
    }
    index++;
  }
}

void _table(dom.Element table, StringBuffer out) {
  final rows = table.querySelectorAll('tr');
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
  return sb.toString().replaceAll(RegExp(r'[ \t]+'), ' ').trim();
}

String _directInline(dom.Element item) {
  final sb = StringBuffer();
  for (final child in item.nodes) {
    if (child is dom.Text) {
      sb.write(child.text);
    } else if (child is dom.Element) {
      final t = _tag(child);
      if (t != 'ul' && t != 'ol') _gather(child, sb);
    }
  }
  return sb.toString().replaceAll(RegExp(r'[ \t]+'), ' ').trim();
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
    sb.write(child.text);
    return;
  }
  if (child is! dom.Element) return;
  final tag = _tag(child);
  if (tag == 'br') {
    sb.write(' ');
  } else if (tag == 'a') {
    final href = _cleanHref(child.attributes['href']);
    final inner = StringBuffer();
    _gather(child, inner);
    final text = inner.toString().trim();
    if (href != null && text.isNotEmpty) {
      sb.write('[$text]($href)');
    } else {
      sb.write(text);
    }
  } else if (tag == 'img') {
    final src = _cleanSrc(child.attributes['src']);
    if (src != null) {
      sb.write('![${child.attributes['alt'] ?? ''}]($src)');
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
    final w = RegExp(r'font-weight\s*:\s*([a-z0-9]+)').firstMatch(style)?.group(1);
    if (w == 'bold' || w == 'bolder' || (int.tryParse(w ?? '') ?? 0) >= 600) {
      marks.add('bold');
    }
  }
  if (RegExp(r'font-style\s*:\s*italic').hasMatch(style)) marks.add('italic');
  if (RegExp(r'text-decoration[^;]*line-through').hasMatch(style)) {
    marks.add('strike');
  }
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
  if (h.isEmpty || h.startsWith('#') || h.startsWith('javascript:')) return null;
  return h;
}

String? _cleanSrc(String? src) {
  final s = src?.trim() ?? '';
  if (s.isEmpty || s.startsWith('data:')) return null;
  return s;
}
