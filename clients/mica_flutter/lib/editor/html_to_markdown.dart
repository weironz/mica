/// Pasted clipboard HTML → Markdown, for the paste path to parse as a fragment.
///
/// The conversion itself lives in the Rust engine (`crates/md-core/src/html/
/// paste.rs`), beside the parser whose escaping rules it has to agree with.
/// Until 2026-11-01 it was ~780 lines of Dart here — the last Markdown writer
/// outside the engine. What stays on this side is only the HTML tokenizing,
/// with the `html` package the app already ships (both desktop and web paste
/// call this, so there is one converter, not a per-platform pair): the parsed
/// tree goes to the engine as plain JSON, `{"t": text}` for a text node,
/// `{"e": tag, "a": attrs, "c": children}` for an element, `{"m": 0}` for
/// anything else. Do not add Markdown syntax here.
library;

import 'package:html/dom.dart' as dom;
import 'package:html/parser.dart' as html_parser;

import '../markdown/markdown_codec.dart' show markdownCodec, withEngineOrNull;

/// Markdown for the HTML in [source], or '' when there is none — or when the
/// engine is unavailable, so the caller falls back to the plain-text flavor.
String htmlToMarkdown(String source) {
  final doc = html_parser.parse(source);
  final body = doc.body ?? doc.documentElement;
  if (body == null) return '';
  return withEngineOrNull(() => markdownCodec.htmlToMarkdown(_tree(body))) ??
      '';
}

Map<String, Object?> _tree(dom.Node node) {
  if (node is dom.Text) return {'t': node.text};
  if (node is dom.Element) {
    return {
      'e': (node.localName ?? '').toLowerCase(),
      'a': {
        for (final entry in node.attributes.entries)
          '${entry.key}'.toLowerCase(): entry.value,
      },
      'c': [for (final child in node.nodes) _tree(child)],
    };
  }
  return {'m': 0};
}
