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

/// Convert pasted HTML to Markdown, preserving block structure. Inline emphasis
/// is flattened to text (the editor has no inline marks yet); links keep their
/// text. Tables become GitHub-flavored pipe tables.
String htmlToMarkdown(String source) {
  final parsed = html.DomParser().parseFromString(source, 'text/html');
  final body = parsed.querySelector('body') ?? parsed.documentElement;
  if (body == null) return '';
  final out = StringBuffer();
  for (final node in body.nodes) {
    _node(node, out);
  }
  // Collapse 3+ blank lines.
  return out
      .toString()
      .replaceAll(RegExp(r'\n{3,}'), '\n\n')
      .trim();
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
      // Containers: recurse over children as blocks.
      for (final child in node.nodes) {
        _node(child, out);
      }
    default:
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
    out.writeln('| ${cells.map(_inline).join(' | ')} |');
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
    if (child is html.Text) {
      sb.write(child.text);
    } else if (child is html.Element) {
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
      } else {
        _gather(child, sb);
      }
    }
  }
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
