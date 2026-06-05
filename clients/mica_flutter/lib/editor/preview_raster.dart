import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_math_fork/flutter_math.dart';

import 'render.dart' show EditorTheme;

/// A "source → picture" preview producer for one previewer id ('math',
/// 'mermaid', …). Two forms (docs/render-architecture.md, Decision 2):
///
/// - **Offstage**: [buildOffstage] returns a widget; the pipeline renders it
///   far off-screen and captures it post-frame (flutter_math today).
/// - **Direct**: [produce] returns a future image (JS interop, server render
///   — mermaid on web later). Return null from both to opt out.
///
/// Failure is silent by design: no image means the block keeps falling
/// through to its editable source form (the renderer registry's
/// null-decline) — a preview is an enhancement, never a gate.
abstract class RasterPreviewer {
  const RasterPreviewer();

  String get id;

  /// Offstage form: a widget to render off-screen and capture.
  Widget? buildOffstage(String source) => null;

  /// Direct form: an async image producer.
  Future<ui.Image?>? produce(String source) => null;
}

/// Owns the `{previewer, source} → ui.Image` lifecycle: pending bookkeeping,
/// the off-screen paint host, post-frame capture with bounded retries, and
/// the per-previewer image maps the render object reads.
class RasterPreviewPipeline {
  RasterPreviewPipeline({
    required this.previewers,
    required this.requestRebuild,
  });

  final List<RasterPreviewer> previewers;

  /// Host hook: apply [fn] inside setState when still mounted. The pipeline
  /// itself is plain state — the owning widget controls rebuild safety.
  final void Function(VoidCallback fn) requestRebuild;

  final Map<String, Map<String, ui.Image>> _cache = {};
  final Map<String, Set<String>> _pending = {};
  final Map<String, Map<String, GlobalKey>> _keys = {};
  final Map<String, Map<String, int>> _tries = {};

  RasterPreviewer? _byId(String id) =>
      previewers.where((p) => p.id == id).firstOrNull;

  /// Captured images for [id], keyed by source — handed to the render object.
  Map<String, ui.Image> imagesOf(String id) => _cache[id] ??= {};

  /// Ask for a preview of [source]. Safe to call from layout: work is
  /// deferred post-frame. No-op when cached, already pending, or unknown id.
  void request(String id, String source) {
    final previewer = _byId(id);
    if (previewer == null) return;
    if (imagesOf(id).containsKey(source)) return;
    final pending = _pending[id] ??= {};
    if (pending.contains(source)) return;

    final direct = previewer.produce(source);
    if (direct != null) {
      pending.add(source);
      direct.then((img) {
        requestRebuild(() {
          pending.remove(source);
          if (img != null) imagesOf(id)[source] = img;
        });
      });
      return;
    }

    if (previewer.buildOffstage(source) == null) return;
    pending.add(source);
    (_keys[id] ??= {})[source] = GlobalKey();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      requestRebuild(() {});
      WidgetsBinding.instance.addPostFrameCallback((_) => _capture());
    });
  }

  /// The off-screen paint host. Must be POSITIONED far off-screen, not
  /// Offstage — Offstage skips painting and toImage would capture nothing.
  Widget offstageHost() => Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          for (final p in previewers)
            for (final source in (_pending[p.id] ?? const <String>{}))
              if (_keys[p.id]?[source] != null)
                RepaintBoundary(
                  key: _keys[p.id]![source],
                  child: p.buildOffstage(source) ?? const SizedBox.shrink(),
                ),
        ],
      );

  Future<void> _capture() async {
    var any = false;
    var left = false;
    for (final p in previewers) {
      final pending = _pending[p.id];
      if (pending == null || pending.isEmpty) continue;
      final captured = <String>[];
      for (final source in pending.toList()) {
        final boundary =
            _keys[p.id]?[source]?.currentContext?.findRenderObject();
        if (boundary is! RenderRepaintBoundary) continue; // not built yet
        try {
          final img =
              await boundary.toImage(pixelRatio: EditorTheme.mathPixelRatio);
          imagesOf(p.id)[source] = img;
          captured.add(source);
        } catch (_) {
          // Not painted yet — retry a few frames, then give up (the block
          // stays on its source form).
          final tries = ((_tries[p.id] ??= {})[source] ?? 0) + 1;
          _tries[p.id]![source] = tries;
          if (tries > 20) captured.add(source);
        }
      }
      if (captured.isNotEmpty) {
        any = true;
        for (final s in captured) {
          pending.remove(s);
          _keys[p.id]?.remove(s);
          _tries[p.id]?.remove(s);
        }
      }
      if (pending.isNotEmpty) left = true;
    }
    if (any) requestRebuild(() {});
    if (left) {
      WidgetsBinding.instance.addPostFrameCallback((_) => _capture());
    }
  }
}

/// LaTeX formulas through flutter_math_fork — the offstage form.
class MathPreviewer extends RasterPreviewer {
  const MathPreviewer();

  @override
  String get id => 'math';

  @override
  Widget buildOffstage(String source) => Math.tex(
        source,
        textStyle: const TextStyle(fontSize: 18, color: EditorTheme.text),
        onErrorFallback: (e) => Text(
          source,
          style: const TextStyle(
            fontFamily: 'monospace',
            fontSize: 14,
            color: Color(0xFFB91C1C),
          ),
        ),
      );
}
