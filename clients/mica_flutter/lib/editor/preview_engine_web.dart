import 'dart:async';
import 'dart:convert';
import 'dart:js_interop';

import 'package:flutter/foundation.dart';
import 'package:web/web.dart' as web;

import '../src/rust/api/render.dart' show MermaidTheme;
import 'preview_engine.dart' show MathSvg, PreviewUnavailable;

@visibleForTesting
String previewWorkerUrl = 'mica_preview_worker.js';

web.Worker? _worker;
int _nextId = 0;
final _pending = <int, Completer<String?>>{};

/// Drop [worker] and answer everything still waiting on it. Only [crashed],
/// the request that brought the renderer down, gets a final "no preview";
/// the rest were merely in flight and are told to retry.
void _retireWorker(web.Worker worker, {int? crashed}) {
  if (!identical(_worker, worker)) return;
  worker.terminate();
  _worker = null;
  for (final MapEntry(key: id, value: job) in _pending.entries) {
    if (id == crashed) {
      job.complete(null);
    } else {
      job.completeError(const PreviewUnavailable());
    }
  }
  _pending.clear();
}

web.Worker _startWorker() {
  final worker = web.Worker(
    previewWorkerUrl.toJS,
    web.WorkerOptions(type: 'module'),
  );
  worker.onmessage = ((web.MessageEvent event) {
    if (!identical(_worker, worker)) return;
    final data = (event.data as JSArray<JSAny?>).toDart;
    final id = (data[0] as JSNumber?)?.toDartInt;
    final status = data.length > 2 ? (data[2] as JSString?)?.toDart : null;
    if (id == null || status == 'crashed') {
      _retireWorker(worker, crashed: id);
    } else if (status == 'unavailable') {
      // The worker closed itself: a failed import stays failed in it.
      _retireWorker(worker);
    } else {
      _pending.remove(id)?.complete((data[1] as JSString?)?.toDart);
    }
  }).toJS;
  worker.onerror = ((web.Event event) {
    event.preventDefault();
    _retireWorker(worker);
  }).toJS;
  return worker;
}

Future<String?> _request(
  String kind,
  String source,
  String theme, [
  bool? display,
]) async {
  final worker = _worker ??= _startWorker();
  final id = _nextId++;
  final job = _pending[id] = Completer<String?>();
  try {
    worker.postMessage(
      <JSAny?>[id.toJS, kind.toJS, source.toJS, theme.toJS, display?.toJS].toJS,
    );
    return await job.future;
  } finally {
    _pending.remove(id);
  }
}

Future<String?> mermaidSvg(String source, MermaidTheme theme) => _request(
  'mermaid',
  source,
  jsonEncode({
    'dark': theme.dark,
    'canvas': theme.canvas,
    'surface': theme.surface,
    'surface_alt': theme.surfaceAlt,
    'text': theme.text,
    'subtle_text': theme.subtleText,
    'border': theme.border,
    'line': theme.line,
    'error': theme.error,
    'warning': theme.warning,
    'success': theme.success,
  }),
);

Future<MathSvg?> mathSvg(String source, bool display, String color) async {
  final raw = await _request('math', source, color, display);
  if (raw == null) return null;
  final math = jsonDecode(raw) as Map<String, dynamic>;
  return (
    svg: math['svg'] as String,
    width: (math['width'] as num).toDouble(),
    height: (math['height'] as num).toDouble(),
    baseline: (math['baseline'] as num).toDouble(),
  );
}
