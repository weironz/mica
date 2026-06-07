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
  for (final node in body.nodes) {
    _node(node, out);
  }
  return out
      .toString()
      .replaceAll(RegExp(r'\n{3,}'), '\n\n')
      .trim();
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
      for (final child in node.nodes) {
        _node(child, out);
      }
    default:
      if (node.querySelector(
            'table, ul, ol, pre, blockquote, h1, h2, h3, h4, h5, h6, p, img',
          ) !=
          null) {
        for (final child in node.nodes) {
          _node(child, out);
        }
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
    if (child is dom.Text) {
      sb.write(child.text);
    } else if (child is dom.Element) {
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
      } else {
        _gather(child, sb);
      }
    }
  }
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
