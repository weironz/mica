import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_math_fork/flutter_math.dart';

import 'mermaid_preview.dart';
import 'model.dart' show kMonoFont;
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

  /// Direct form: an async image producer. [targetWidth] is the layout width
  /// the preview will be shown at (logical px) — producers rasterize to fill
  /// it crisply instead of their natural size.
  Future<ui.Image?>? produce(String source, double targetWidth) => null;
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

  /// Top-to-alphabetic-baseline distance of each captured preview, logical px
  /// at the offstage widget's own size (pre-pixelRatio), keyed like [_cache].
  /// Inline atoms align formulas to the text baseline with this; block
  /// renderers ignore it. Nullable per source: an error-fallback capture may
  /// not report one, and consumers must degrade (to middle alignment), not
  /// crash.
  final Map<String, Map<String, double>> _baselines = {};

  /// Sources whose preview failed (bad mermaid syntax, capture gave up).
  /// Negative cache: without it the renderer re-requests the same failing
  /// source every layout pass, looping the producer forever. An edit changes
  /// the source string, which naturally retries.
  final Map<String, Set<String>> _failed = {};

  /// Insertion-order cap per previewer. Without it, every distinct source
  /// string ever previewed kept its full-res texture for the life of the
  /// editor State — sources edited away included (freeze-audit CONFIRMED
  /// leak). Evicted images are disposed POST-frame, never mid-frame:
  /// `request()` runs during layout, and the current frame's paint must not
  /// draw a disposed image — dropping the map entry keeps it out of the NEXT
  /// paint, the deferred dispose keeps the current rasterizer safe.
  static const int _maxPerPreviewer = 48;
  final List<ui.Image> _evictedPendingDispose = [];

  void _capAfterInsert(String id) {
    final map = imagesOf(id);
    while (map.length > _maxPerPreviewer) {
      final oldest = map.keys.first;
      final img = map.remove(oldest);
      _baselines[id]?.remove(oldest);
      if (img != null) _evictedPendingDispose.add(img);
    }
    // The negative cache holds only strings, but a broken block being typed
    // in mints one per keystroke — trim it too.
    final failed = _failed[id];
    while (failed != null && failed.length > 256) {
      failed.remove(failed.first);
    }
    if (_evictedPendingDispose.isNotEmpty) {
      final batch = List<ui.Image>.of(_evictedPendingDispose);
      _evictedPendingDispose.clear();
      WidgetsBinding.instance.addPostFrameCallback((_) {
        for (final img in batch) {
          img.dispose();
        }
      });
    }
  }

  /// Dispose every cached preview texture. The owning editor State disposed
  /// its own image cache but never this pipeline — preview textures were
  /// reclaimed only by GC finalizers, non-deterministically (freeze audit).
  /// Same timing contract as the editor's `_imageCache` disposal: called from
  /// `State.dispose`, after the last frame that painted them.
  void dispose() {
    for (final map in _cache.values) {
      for (final img in map.values) {
        img.dispose();
      }
    }
    _cache.clear();
    _baselines.clear();
    _pending.clear();
    _keys.clear();
    _tries.clear();
    _failed.clear();
    _evictedPendingDispose.clear();
  }

  RasterPreviewer? _byId(String id) =>
      previewers.where((p) => p.id == id).firstOrNull;

  /// Captured images for [id], keyed by source — handed to the render object.
  Map<String, ui.Image> imagesOf(String id) => _cache[id] ??= {};

  /// All captured images, per previewer id — the render object's view.
  Map<String, Map<String, ui.Image>> get images => _cache;

  /// Baselines for [id], keyed by source (see [_baselines]).
  Map<String, double> baselinesOf(String id) => _baselines[id] ??= {};

  /// All captured baselines, per previewer id — the render object's view.
  Map<String, Map<String, double>> get baselines => _baselines;

  /// Ask for a preview of [source]. Safe to call from layout: work is
  /// deferred post-frame. No-op when cached, already pending, or unknown id.
  /// [targetWidth] reaches direct-form producers (see [RasterPreviewer.produce]).
  void request(String id, String source, [double targetWidth = 800]) {
    final previewer = _byId(id);
    if (previewer == null) return;
    if (imagesOf(id).containsKey(source)) return;
    if (_failed[id]?.contains(source) ?? false) return;
    final pending = _pending[id] ??= {};
    if (pending.contains(source)) return;

    final direct = previewer.produce(source, targetWidth);
    if (direct != null) {
      pending.add(source);
      direct.then((img) {
        requestRebuild(() {
          pending.remove(source);
          if (img != null) {
            imagesOf(id)[source] = img;
          } else {
            (_failed[id] ??= {}).add(source);
          }
          _capAfterInsert(id);
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
  ///
  /// The [_BaselineTap] sits INSIDE the boundary: it is a transparent proxy,
  /// so toImage captures the same pixels, while its performLayout — the one
  /// place getDistanceToBaseline is legal — files the widget's baseline away
  /// for the capture in [_capture] to publish. Probed through this exact tree
  /// (Column + RepaintBoundary): \frac{a}{b} reports 19.936, within 0.004px
  /// of a bare layout.
  Widget offstageHost() => Column(
    mainAxisSize: MainAxisSize.min,
    crossAxisAlignment: CrossAxisAlignment.start,
    children: [
      for (final p in previewers)
        for (final source in (_pending[p.id] ?? const <String>{}))
          if (_keys[p.id]?[source] != null)
            RepaintBoundary(
              key: _keys[p.id]![source],
              child: _BaselineTap(
                onBaseline: (v) {
                  if (v != null) baselinesOf(p.id)[source] = v;
                },
                child: p.buildOffstage(source) ?? const SizedBox.shrink(),
              ),
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
        final boundary = _keys[p.id]?[source]?.currentContext
            ?.findRenderObject();
        if (boundary is! RenderRepaintBoundary) continue; // not built yet
        try {
          final img = await boundary.toImage(
            pixelRatio: EditorTheme.mathPixelRatio,
          );
          imagesOf(p.id)[source] = img;
          captured.add(source);
        } catch (_) {
          // Not painted yet — retry a few frames, then give up (the block
          // stays on its source form; failed-set stops re-requests).
          final tries = ((_tries[p.id] ??= {})[source] ?? 0) + 1;
          _tries[p.id]![source] = tries;
          if (tries > 20) {
            captured.add(source);
            (_failed[p.id] ??= {}).add(source);
            // Layout may have filed a baseline before the capture gave up; a
            // baseline without its image would lie to consumers.
            _baselines[p.id]?.remove(source);
          }
        }
      }
      if (captured.isNotEmpty) {
        any = true;
        for (final s in captured) {
          pending.remove(s);
          _keys[p.id]?.remove(s);
          _tries[p.id]?.remove(s);
        }
        _capAfterInsert(p.id);
      }
      if (pending.isNotEmpty) left = true;
    }
    if (any) requestRebuild(() {});
    if (left) {
      WidgetsBinding.instance.addPostFrameCallback((_) => _capture());
    }
  }
}

/// Mermaid diagrams — the direct form (JS interop on web; elsewhere
/// [mermaidAvailable] is false and the previewer simply isn't registered,
/// so ```mermaid blocks stay highlighted source).
class MermaidPreviewer extends RasterPreviewer {
  const MermaidPreviewer();

  @override
  String get id => 'mermaid';

  @override
  Future<ui.Image?>? produce(String source, double targetWidth) =>
      renderMermaid(source, targetWidth);
}

/// LaTeX formulas through flutter_math_fork — the offstage form.
class MathPreviewer extends RasterPreviewer {
  const MathPreviewer();

  @override
  String get id => 'math';

  @override
  Widget buildOffstage(String source) => Math.tex(
    source,
    textStyle: const TextStyle(
      fontSize: EditorTheme.mathRasterFontSize,
      color: EditorTheme.text,
    ),
    onErrorFallback: (e) => Text(
      source,
      style: const TextStyle(
        fontFamily: kMonoFont,
        fontSize: 14,
        color: Color(0xFFB91C1C),
      ),
    ),
  );
}

/// Transparent proxy that reports its child's alphabetic baseline from inside
/// its own performLayout — the one phase where [RenderBox.getDistanceToBaseline]
/// is legal to call. Painting and hit-testing pass straight through, so a
/// RepaintBoundary above it captures identical pixels.
class _BaselineTap extends SingleChildRenderObjectWidget {
  const _BaselineTap({required this.onBaseline, super.child});

  final ValueChanged<double?> onBaseline;

  @override
  RenderObject createRenderObject(BuildContext context) =>
      _RenderBaselineTap(onBaseline);

  @override
  void updateRenderObject(
    BuildContext context,
    covariant _RenderBaselineTap renderObject,
  ) {
    renderObject.onBaseline = onBaseline;
  }
}

class _RenderBaselineTap extends RenderProxyBox {
  _RenderBaselineTap(this.onBaseline);

  ValueChanged<double?> onBaseline;

  @override
  void performLayout() {
    super.performLayout();
    onBaseline(child?.getDistanceToBaseline(TextBaseline.alphabetic));
  }
}
