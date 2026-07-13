// Web implementation of rich clipboard paste. Captures the native `paste`
// event (capture phase, before Flutter's hidden input) and, when the clipboard
// carries `text/html`, converts that HTML to Markdown so structure (headings,
// lists, code, tables) survives — matching how Typora pastes web content.
//
// dart:html is legacy but dependency-free and sufficient for reading the
// clipboard's HTML flavor and parsing it via the browser DOM.
// ignore_for_file: deprecated_member_use, avoid_web_libraries_in_flutter
import 'dart:html' as html;
import 'dart:typed_data';

typedef RichPasteHandler = bool Function(String markdown, String plain, bool rich);
typedef ImagePasteHandler = void Function(
  Uint8List bytes,
  String mime,
  String name,
);

RichPasteHandler? _handler;
ImagePasteHandler? _imageHandler;
bool _installed = false;

// Web reads the clipboard through the DOM `paste` event (setRichPasteHandler),
// not by explicit pull, so these facade pulls are unused on web.
Future<Uint8List?> readClipboardImage() async => null;
Future<String?> readClipboardHtmlAsMarkdown() async => null;

void setRichImagePasteHandler(ImagePasteHandler? handler) {
  _imageHandler = handler;
}

/// Extract a pasted image file from `files` (preferred) or `items`.
html.File? _clipboardImage(html.DataTransfer data) {
  final files = data.files;
  if (files != null) {
    for (final f in files) {
      if (f.type.startsWith('image/')) return f;
    }
  }
  final items = data.items;
  if (items != null) {
    final n = items.length ?? 0;
    for (var i = 0; i < n; i++) {
      final item = items[i];
      if (item.kind == 'file' && (item.type?.startsWith('image/') ?? false)) {
        final file = item.getAsFile();
        if (file != null) return file;
      }
    }
  }
  return null;
}

/// dart2js types FileReader's readAsArrayBuffer result inconsistently; accept
/// the common byte representations.
Uint8List? _bytesOf(Object? result) {
  if (result is ByteBuffer) return result.asUint8List();
  if (result is Uint8List) return result;
  if (result is List<int>) return Uint8List.fromList(result);
  return null;
}

void setRichPasteHandler(RichPasteHandler? handler) {
  _handler = handler;
  if (_installed) return;
  _installed = true;
  html.document.addEventListener('paste', (event) {
    final data = (event as html.ClipboardEvent).clipboardData;
    if (data == null) return;

    // A pasted bitmap (screenshot, copied image) arrives as a file — in the
    // clipboard's `files` and/or `items`. Upload it rather than dropping as text.
    final imageHandler = _imageHandler;
    if (imageHandler != null) {
      final image = _clipboardImage(data);
      if (image != null) {
        event.preventDefault();
        event.stopPropagation();
        final reader = html.FileReader()..readAsArrayBuffer(image);
        reader.onLoadEnd.first.then((_) {
          final bytes = _bytesOf(reader.result);
          if (bytes != null) {
            imageHandler(
              bytes,
              image.type.isEmpty ? 'image/png' : image.type,
              image.name.isEmpty ? 'pasted-image.png' : image.name,
            );
          }
        });
        return;
      }
    }

    final handler = _handler;
    if (handler == null) return;
    final htmlData = data.getData('text/html');
    final plain = data.getData('text/plain');
    final hasHtml = htmlData.trim().isNotEmpty;
    final content = hasHtml ? htmlToMarkdown(htmlData) : plain;

    if (handler(content, plain, hasHtml)) {
      event.preventDefault();
      event.stopPropagation();
    }
  }, true);
}

/// Convert pasted HTML to Markdown, preserving block structure AND inline
/// formatting: bold/italic/strike/inline-code (from `<b>/<strong>/<i>/<em>/
/// <s>/<del>/<code>` and styled `<span>`s), links, and images. Tables become
/// GitHub-flavored pipe tables.
String htmlToMarkdown(String source) {
  final parsed = html.DomParser().parseFromString(source, 'text/html');
  final body = parsed.querySelector('body') ?? parsed.documentElement;
  if (body == null) return '';
  final out = StringBuffer();
  _emitChildren(body.nodes, out);
  // Collapse 3+ blank lines.
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

bool _isInlineNode(html.Node n) =>
    n is html.Text ||
    (n is html.Element && _inlineTags.contains(n.tagName.toLowerCase()));

/// Emit a node list, coalescing runs of consecutive inline siblings (text +
/// `<strong>`/`<code>`/…) into ONE Markdown paragraph. Without this, a clipboard
/// fragment that is bare inline content (a single paragraph copied from Typora
/// arrives with no `<p>` wrapper) would put every `<strong>`/`<code>` on its own
/// line and drop its emphasis — the inline markers are only added when the
/// element is *gathered*, not when `_node` treats it as a block.
void _emitChildren(List<html.Node> nodes, StringBuffer out) {
  final run = <html.Node>[];
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

void _node(html.Node node, StringBuffer out) {
  if (node is html.Text) {
    final text = (node.text ?? '').replaceAll(RegExp(r'\s+'), ' ').trim();
    if (text.isNotEmpty) {
      out.writeln(text);
      out.writeln();
    }
    return;
  }
  if (node is! html.Element) return;

  final tag = node.tagName.toLowerCase();
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
      // A block-level image → its own ![alt](src) line.
      final src = _cleanSrc(node.getAttribute('src'));
      if (src != null) {
        out.writeln('![${node.getAttribute('alt') ?? ''}]($src)');
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
      // Containers: coalesce inline runs, recurse over block children.
      _emitChildren(node.nodes, out);
    default:
      // Unknown elements that contain block-level structure (GitHub wraps
      // README tables in <markdown-accessiblity-table>; sites nest content
      // in all sorts of custom elements) recurse as containers — flattening
      // would destroy the tables/lists inside. Only leaf-ish unknowns
      // flatten to inline text.
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

void _list(html.Element list, StringBuffer out, {required bool ordered, int depth = 0}) {
  var index = 1;
  for (final item in list.children) {
    if (item.tagName.toLowerCase() != 'li') continue;
    final marker = ordered ? '$index.' : '-';
    final text = _directInline(item);
    out.writeln('${'  ' * depth}$marker $text');
    for (final child in item.children) {
      final t = child.tagName.toLowerCase();
      if (t == 'ul' || t == 'ol') {
        _list(child, out, ordered: t == 'ol', depth: depth + 1);
      }
    }
    index++;
  }
}

void _table(html.Element table, StringBuffer out) {
  final rows = table.querySelectorAll('tr');
  if (rows.isEmpty) return;
  var headerWritten = false;
  for (final row in rows) {
    final cells = row.children
        .where((c) => c.tagName.toLowerCase() == 'td' || c.tagName.toLowerCase() == 'th')
        .toList();
    if (cells.isEmpty) continue;
    // Pipes inside a cell would split it on re-parse — escape them; cell
    // line breaks collapse to spaces (GFM cells are single-line).
    String cell(html.Element c) =>
        _inline(c).replaceAll('|', r'\|').replaceAll('\n', ' ');
    out.writeln('| ${cells.map(cell).join(' | ')} |');
    if (!headerWritten) {
      out.writeln('| ${cells.map((_) => '---').join(' | ')} |');
      headerWritten = true;
    }
  }
  out.writeln();
}

/// Flattened inline text of an element (whitespace-collapsed).
String _inline(html.Node node) {
  final sb = StringBuffer();
  _gather(node, sb);
  return sb.toString().replaceAll(RegExp(r'[ \t]+'), ' ').trim();
}

/// Inline text of a list item, excluding nested lists.
String _directInline(html.Element item) {
  final sb = StringBuffer();
  for (final child in item.nodes) {
    if (child is html.Text) {
      sb.write(child.text);
    } else if (child is html.Element) {
      final t = child.tagName.toLowerCase();
      if (t != 'ul' && t != 'ol') _gather(child, sb);
    }
  }
  return sb.toString().replaceAll(RegExp(r'[ \t]+'), ' ').trim();
}

/// Text of a `<pre>` preserving line breaks: real newlines are kept, and block
/// children / `<br>` (used by many sites to lay out code lines) become newlines.
String _codeText(html.Node node) {
  final sb = StringBuffer();
  _codeGather(node, sb);
  return sb.toString();
}

void _codeGather(html.Node node, StringBuffer sb) {
  for (final child in node.nodes) {
    if (child is html.Text) {
      sb.write(child.text);
    } else if (child is html.Element) {
      final tag = child.tagName.toLowerCase();
      if (tag == 'br') {
        _appendNewline(sb);
      } else {
        _codeGather(child, sb);
        // Block-level line containers imply a line break — but only if the
        // gathered text doesn't already end with one (many highlighters wrap
        // each line in a <div>/<span> AND keep a real trailing newline, which
        // would otherwise produce a blank line between every row).
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

void _gather(html.Node node, StringBuffer sb) {
  for (final child in node.nodes) {
    _gatherOne(child, sb);
  }
}

/// Append one inline node's Markdown (text or an inline element) to [sb].
void _gatherOne(html.Node child, StringBuffer sb) {
  if (child is html.Text) {
    sb.write(child.text);
    return;
  }
  if (child is! html.Element) return;
  final tag = child.tagName.toLowerCase();
  if (tag == 'br') {
    sb.write(' ');
  } else if (tag == 'a') {
    // Preserve hyperlinks as inline Markdown links.
    final href = _cleanHref(child.getAttribute('href'));
    final inner = StringBuffer();
    _gather(child, inner);
    final text = inner.toString().trim();
    if (href != null && text.isNotEmpty) {
      sb.write('[$text]($href)');
    } else {
      sb.write(text);
    }
  } else if (tag == 'img') {
    final src = _cleanSrc(child.getAttribute('src'));
    if (src != null) {
      sb.write('![${child.getAttribute('alt') ?? ''}]($src)');
    }
  } else if (tag == 'code') {
    sb.write(_inlineCode(child.text ?? ''));
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
Set<String> _inlineMarks(html.Element e) {
  final marks = <String>{};
  switch (e.tagName.toLowerCase()) {
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
  final style = (e.getAttribute('style') ?? '').toLowerCase();
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

/// A usable link target, or null for empty / in-page / javascript: hrefs.
String? _cleanHref(String? href) {
  final h = href?.trim() ?? '';
  if (h.isEmpty || h.startsWith('#') || h.startsWith('javascript:')) return null;
  return h;
}

/// A usable image source, or null for empty / inline data: URIs.
String? _cleanSrc(String? src) {
  final s = src?.trim() ?? '';
  if (s.isEmpty || s.startsWith('data:')) return null;
  return s;
}
