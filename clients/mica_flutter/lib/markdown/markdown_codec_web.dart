import 'dart:convert';
import 'dart:js_interop';

import 'mark.dart';
import 'markdown_codec.dart';
import 'markdown_wire.dart';

@JS('micaMd')
external _MicaMd? get _micaMd;
@JS('micaMdReady')
external JSPromise<JSAny?>? get _micaMdReady;
@JS('micaMdError')
external JSString? get _micaMdError;

extension type _MicaMd._(JSObject _) implements JSObject {
  external String parseBody(String markdown, bool fragment);
  external String parseInlineBatch(String sourcesJson);
  external String decodeCells(String cellsJson);
  external String encodeCells(String inlinesJson);
  external String serializeBody(String blocksJson, String assetsJson);
  external String serializeInline(String inlineJson);
  external String renderHtmlBody(
    String blocksJson,
    String imageSrcsJson,
    String profile,
  );
  external String mathRunSpans(String source);
  external String parseInlineMath(String source);
  external String engineInfo();
}

Future<MarkdownCodec> createMarkdownCodec() async {
  final ready = _micaMdReady;
  if (ready == null) {
    throw const MarkdownCodecException(
      'mica_md.js was not loaded (index.html)',
    );
  }
  await ready.toDart.timeout(const Duration(seconds: 10));
  final md = _micaMd;
  if (md == null) {
    throw MarkdownCodecException(_micaMdError?.toDart ?? 'wasm init failed');
  }
  return _WasmMarkdownCodec(md);
}

class _WasmMarkdownCodec implements MarkdownCodec {
  _WasmMarkdownCodec(this.md);
  final _MicaMd md;
  T _call<T>(T Function() call) {
    try {
      return call();
    } on Object catch (e) {
      throw MarkdownCodecException('$e');
    }
  }

  @override
  List<BlockSpec> parseBlocks(String markdown, {bool fragment = false}) =>
      _call(() => decodeBlocks(md.parseBody(markdown, fragment)));
  @override
  InlineText parseInline(String source) => parseInlineBatch([source]).single;
  @override
  List<InlineText> parseInlineBatch(List<String> sources) =>
      _call(() => decodeInlines(md.parseInlineBatch(jsonEncode(sources))));
  @override
  List<InlineText> decodeCells(List<Object?> cells) =>
      _call(() => decodeInlines(md.decodeCells(jsonEncode(cells))));
  @override
  List<Object> encodeCells(List<InlineText> cells) => _call(
    () => (jsonDecode(md.encodeCells(encodeInlines(cells))) as List)
        .cast<Object>(),
  );
  @override
  String serializeBlocks(
    List<BlockSpec> blocks, {
    Map<String, String> assets = const {},
  }) => _call(() => md.serializeBody(encodeBlocks(blocks), jsonEncode(assets)));
  @override
  String serializeInline(String text, List<Mark> marks) => _call(
    () => md.serializeInline(
      jsonEncode(encodeInline((text: text, marks: marks))),
    ),
  );
  @override
  String renderClipboardHtml(
    List<BlockSpec> blocks, {
    Map<String, String> imageSrcs = const {},
  }) => _call(
    () => md.renderHtmlBody(
      encodeBlocks(blocks),
      jsonEncode(imageSrcs),
      'clipboard',
    ),
  );
  @override
  List<({int start, int end})> mathRunSpans(String source) =>
      _call(() => decodeSpans(md.mathRunSpans(source)));
  @override
  InlineText parseInlineMath(String source) => _call(
    () => decodeInline(
      jsonDecode(md.parseInlineMath(source)) as Map<String, dynamic>,
    ),
  );
  @override
  ({String crateVersion, int schemaVersion, String sourceFingerprint})
  get engineInfo => _call(() {
    final info = jsonDecode(md.engineInfo()) as Map<String, dynamic>;
    return (
      crateVersion: info['crate_version'] as String,
      schemaVersion: info['schema_version'] as int,
      sourceFingerprint: info['source_fingerprint'] as String,
    );
  });
}
