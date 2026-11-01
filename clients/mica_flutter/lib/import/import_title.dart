import '../markdown/markdown_codec.dart'
    show BlockSpec, MarkdownCodec, markdownCodec, withEngineOrNull;

/// The title a Markdown import should use: the first heading's text, else the
/// first line of the first block with text (trimmed to a sane length), else
/// [fallback].
///
/// Shared by the cloud import path and the local one so a file imported into
/// either world gets the same name (issue #108). The blocks come from the
/// engine, so front matter is skipped and `# **Bold**` names the page "Bold",
/// not "**Bold**". If the engine is not loaded the fallback is used: guessing
/// from raw lines would be a second Markdown reader.
String titleFromMarkdown(
  String markdown,
  String fallback, {
  MarkdownCodec? codec,
}) {
  final c = codec ?? markdownCodec;
  final blocks = withEngineOrNull(() => c.parseBlocks(markdown));
  return blocks == null ? fallback : titleFromBlocks(blocks, fallback);
}

/// [titleFromMarkdown] over already-parsed blocks.
String titleFromBlocks(List<BlockSpec> blocks, String fallback) {
  for (final b in blocks) {
    final line = b.text.split('\n').first.trim();
    if (line.isEmpty) continue;
    if (b.kind == 'heading') return line;
    return line.length > 60 ? line.substring(0, 60) : line;
  }
  return fallback;
}
