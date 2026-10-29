import 'package:flutter/foundation.dart';

import '../diagnostics.dart';
import 'mark.dart';
import 'markdown_codec_io.dart'
    if (dart.library.js_interop) 'markdown_codec_web.dart'
    as platform;

typedef BlockSpec = ({String kind, String text, Map<String, dynamic> data});
typedef InlineText = ({String text, List<Mark> marks});

class MarkdownCodecException implements Exception {
  const MarkdownCodecException(this.message);
  final String message;
  @override
  String toString() => 'MarkdownCodecException: $message';
}

abstract interface class MarkdownCodec {
  List<BlockSpec> parseBlocks(String markdown, {bool fragment = false});
  InlineText parseInline(String source);
  List<InlineText> parseInlineBatch(List<String> sources);
  List<InlineText> decodeCells(List<Object?> cells);
  List<Object> encodeCells(List<InlineText> cells);
  String serializeBlocks(
    List<BlockSpec> blocks, {
    Map<String, String> assets = const {},
  });
  String serializeInline(String text, List<Mark> marks);
  String renderClipboardHtml(
    List<BlockSpec> blocks, {
    Map<String, String> imageSrcs = const {},
  });
  List<({int start, int end})> mathRunSpans(String source);
  InlineText parseInlineMath(String source);
  ({String crateVersion, int schemaVersion, String sourceFingerprint})
  get engineInfo;
}

MarkdownCodec? _codec;
MarkdownCodec get markdownCodec =>
    _codec ??
    (throw StateError('initMarkdownCodec() was not awaited before first use'));

Future<void> initMarkdownCodec() async {
  final started = DateTime.now();
  try {
    _codec = await platform.createMarkdownCodec();
  } catch (error, stack) {
    logCrash('markdown codec unavailable: $error\n$stack');
    _codec = const UnavailableMarkdownCodec();
  }
  // The elapsed window covers only this await: probes budget the codec, not
  // the surrounding startup chain that merely contains it.
  final elapsed = DateTime.now().difference(started).inMilliseconds;
  debugPrint('markdown codec ready in $elapsed ms');
}

@visibleForTesting
void debugSetMarkdownCodec(MarkdownCodec? codec) => _codec = codec;

/// [read]'s result, or null when the engine is unavailable — for callers that
/// must tell "no engine" apart from an empty result (never substitute '').
T? withEngineOrNull<T>(T Function() read) {
  try {
    return read();
  } on MarkdownCodecException catch (error) {
    debugPrint('markdown engine unavailable: $error');
    return null;
  }
}

class UnavailableMarkdownCodec implements MarkdownCodec {
  const UnavailableMarkdownCodec();
  Never _unavailable() =>
      throw const MarkdownCodecException('Markdown engine unavailable');
  @override
  List<BlockSpec> parseBlocks(String markdown, {bool fragment = false}) =>
      _unavailable();
  @override
  InlineText parseInline(String source) => _unavailable();
  @override
  List<InlineText> parseInlineBatch(List<String> sources) => _unavailable();
  @override
  List<InlineText> decodeCells(List<Object?> cells) => _unavailable();
  @override
  List<Object> encodeCells(List<InlineText> cells) => _unavailable();
  @override
  String serializeBlocks(
    List<BlockSpec> blocks, {
    Map<String, String> assets = const {},
  }) => _unavailable();
  @override
  String serializeInline(String text, List<Mark> marks) => _unavailable();
  @override
  String renderClipboardHtml(
    List<BlockSpec> blocks, {
    Map<String, String> imageSrcs = const {},
  }) => _unavailable();
  @override
  List<({int start, int end})> mathRunSpans(String source) => _unavailable();
  @override
  InlineText parseInlineMath(String source) => _unavailable();
  @override
  ({String crateVersion, int schemaVersion, String sourceFingerprint})
  get engineInfo => _unavailable();
}
