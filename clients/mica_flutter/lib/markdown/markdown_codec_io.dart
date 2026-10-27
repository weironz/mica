import 'dart:convert';

import '../local/native_rust.dart';
import '../src/rust/api/markdown.dart' as ffi;
import '../src/rust/frb_generated.dart';
import 'mark.dart';
import 'markdown_codec.dart';
import 'markdown_wire.dart';

Future<MarkdownCodec> createMarkdownCodec() async {
  if (!RustLib.instance.initialized) {
    try {
      await initNativeRust();
    } catch (_) {
      // Another native entry point may have completed initialization meanwhile.
      // Propagate loading failures rather than treating every exception as ready.
      if (!RustLib.instance.initialized) rethrow;
    }
  }
  return _FfiMarkdownCodec();
}

class _FfiMarkdownCodec implements MarkdownCodec {
  T _call<T>(T Function() call) {
    try {
      return call();
    } on Object catch (error) {
      throw MarkdownCodecException('$error');
    }
  }

  @override
  List<BlockSpec> parseBlocks(String markdown, {bool fragment = false}) =>
      _call(
        () => decodeBlocks(
          ffi.mdParseBody(markdown: markdown, fragment: fragment),
        ),
      );
  @override
  InlineText parseInline(String source) => parseInlineBatch([source]).single;
  @override
  List<InlineText> parseInlineBatch(List<String> sources) => _call(
    () =>
        decodeInlines(ffi.mdParseInlineBatch(sourcesJson: jsonEncode(sources))),
  );
  @override
  List<InlineText> decodeCells(List<Object?> cells) => _call(
    () => decodeInlines(ffi.mdDecodeCells(cellsJson: jsonEncode(cells))),
  );
  @override
  List<Object> encodeCells(List<InlineText> cells) => _call(
    () =>
        (jsonDecode(ffi.mdEncodeCells(inlinesJson: encodeInlines(cells)))
                as List)
            .cast<Object>(),
  );
  @override
  String serializeBlocks(
    List<BlockSpec> blocks, {
    Map<String, String> assets = const {},
  }) => _call(
    () => ffi.mdSerializeBody(
      blocksJson: encodeBlocks(blocks),
      assetsJson: jsonEncode(assets),
    ),
  );
  @override
  String serializeInline(String text, List<Mark> marks) => _call(
    () => ffi.mdSerializeInline(
      inlineJson: jsonEncode(encodeInline((text: text, marks: marks))),
    ),
  );
  @override
  String renderClipboardHtml(
    List<BlockSpec> blocks, {
    Map<String, String> imageSrcs = const {},
  }) => _call(
    () => ffi.mdRenderHtmlBody(
      blocksJson: encodeBlocks(blocks),
      imageSrcsJson: jsonEncode(imageSrcs),
      profile: 'clipboard',
    ),
  );
  @override
  List<({int start, int end})> mathRunSpans(String source) =>
      _call(() => decodeSpans(ffi.mdMathRunSpans(source: source)));
  @override
  InlineText parseInlineMath(String source) => _call(
    () => decodeInline(
      jsonDecode(ffi.mdParseInlineMath(source: source)) as Map<String, dynamic>,
    ),
  );
  @override
  ({String crateVersion, int schemaVersion, String sourceFingerprint})
  get engineInfo => _call(() {
    final info = jsonDecode(ffi.mdEngineInfo()) as Map<String, dynamic>;
    return (
      crateVersion: info['crate_version'] as String,
      schemaVersion: info['schema_version'] as int,
      sourceFingerprint: info['source_fingerprint'] as String,
    );
  });
}
