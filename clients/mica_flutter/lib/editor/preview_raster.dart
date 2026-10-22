import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_math_fork/flutter_math.dart';

import 'mermaid_preview.dart';
import 'model.dart' show kMonoFont;
import '../ui/theme_tokens.dart';
import 'render.dart' show EditorTheme;

/// A "source → picture" preview producer for one previewer id ('math',
/// 'mermaid', …). Two forms (docs/architecture/render-architecture.md, Decision 2):
///
/// - **Offstage**: [buildOffstage] returns a widget; the pipeline renders it
///   far off-screen and captures it post-frame (flutter_math today).
/// - **Direct**: [produce] returns a future image (JS interop / server render
///   — e.g. mermaid on web). Return null from both to opt out.
///
/// Failure is silent by design: no image means the block keeps falling
/// through to its editable source form (the renderer registry's
/// null-decline) — a preview is an enhancement, never a gate.
abstract class RasterPreviewer {
  const RasterPreviewer();

  String get id;

  /// Offstage form: a widget to render off-screen and capture.
  ///
  /// [tokens] is the palette in effect. It has to be a parameter: what comes
  /// back is a bitmap, so whatever ink colour is used here is baked in — there
  /// is no later chance to recolour it the way a live widget would get.
  Widget? buildOffstage(String source, MicaTokens tokens) => null;

  /// Direct form: an async image producer. [targetWidth] is the layout width
  /// the preview will be shown at (logical px) — producers rasterize to fill
  /// it crisply instead of their natural size.
  ///
  /// [tokens] is the palette, for the same reason [buildOffstage] takes it: what
  /// comes back is a bitmap, so the colours are baked in at production time.
  Future<ui.Image?>? produce(
    String source,
    double targetWidth,
    MicaTokens tokens,
  ) => null;
}

/// Owns the `{previewer, source} → ui.Image` lifecycle: pending bookkeeping,
/// the off-screen paint host, post-frame capture with bounded retries, and
/// the per-previewer image maps the render object reads.
class RasterPreviewPipeline {
  RasterPreviewPipeline({
    required this.previewers,
    required this.requestRebuild,
    MicaTokens tokens = MicaTokens.light,
  }) : _tokens = tokens;

  final List<RasterPreviewer> previewers;

  MicaTokens _tokens;

  /// The palette the cached rasters were produced with.
  MicaTokens get tokens => _tokens;

  /// Switching palettes throws every raster away.
  ///
  /// The cache is keyed by SOURCE alone, and a raster is pixels: a formula
  /// captured with light ink stays in the map and gets painted onto a dark page,
  /// where it is invisible. Keying by `(source, palette)` instead would keep both
  /// copies of every formula alive for a switch nobody makes twice a minute — so
  /// invalidate, and let them re-capture. Textures are disposed POST-frame
  /// through the same path eviction uses, because the frame in flight may still
  /// be painting them.
  set tokens(MicaTokens value) {
    if (_tokens == value) return;
    _tokens = value;
    for (final map in _cache.values) {
      _evictedPendingDispose.addAll(map.values);
      map.clear();
    }
    _baselines.clear();
    // Pending captures were queued against the old palette; their offstage
    // widgets are rebuilt from `_tokens`, so leaving them pending would publish
    // a raster nobody asked for at this palette. Drop and let layout re-request.
    _pending.clear();
    _keys.clear();
    _tries.clear();
    // Not the negative cache: a formula that fails to render fails in both
    // palettes, and re-trying every broken block on a theme switch is the loop
    // `_failed` exists to prevent.
    _disposeEvictedPostFrame();
  }

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

  /// LRU cap per previewer. Without it, every distinct source string ever
  /// previewed kept its full-res texture for the life of the editor State —
  /// sources edited away included (freeze-audit CONFIRMED leak). Evicted
  /// images are disposed POST-frame, never mid-frame (see [_capNow]).
  ///
  /// It used to evict in INSERTION order, at insert time, while the renderer
  /// asked for a raster for every formula in the document on every layout.
  /// Past the cap that never settled (#62): each arrival evicted the oldest —
  /// often one on screen —, the arrival relayouted, layout asked for the
  /// evicted one again, and so on every frame, formulas flickering between
  /// source and typeset. Now:
  /// - the map's order is recency: the canvas calls [request] for every raster
  ///   it PAINTS, and a request for a cached source moves it to the fresh end;
  /// - eviction waits for the end of the frame that showed the arrivals
  ///   ([_scheduleCap]), so the canvas has recorded their SIZES — it keeps
  ///   those after eviction (render.dart `_previewDims`), so an evicted
  ///   off-screen formula neither reflows nor gets re-requested by layout; paint
  ///   asks for it once it is back on screen;
  /// - a raster drawn by that latest painting frame is never evicted: the cap
  ///   goes soft rather than take pixels from under the viewport (that is
  ///   exactly the loop).
  static const int _maxPerPreviewer = 48;

  /// Same order of magnitude as the canvas's remembered sizes; a double each.
  static const int _maxBaselines = 4096;
  final List<ui.Image> _evictedPendingDispose = [];

  /// Sources the canvas painted during the most recent frame that painted any
  /// (see [_markPainted]); protected from eviction.
  Map<String, Set<String>> _painted = {};
  bool _paintFrameOpen = false;
  bool _capScheduled = false;
  bool _disposed = false;

  /// A request for a source we hold comes from paint (layout only asks for
  /// sources it has never seen a size for): bump it to most-recent and record
  /// it as on screen. The set is replaced on the first paint of each frame, so
  /// it always describes the LATEST frame that drew rasters.
  void _markPainted(String id, String source) {
    final map = imagesOf(id);
    final img = map.remove(source);
    if (img != null) map[source] = img;
    if (!_paintFrameOpen) {
      _paintFrameOpen = true;
      _painted = {};
      WidgetsBinding.instance.addPostFrameCallback(
        (_) => _paintFrameOpen = false,
      );
    }
    (_painted[id] ??= {}).add(source);
  }

  void _capAfterInsert(String id) {
    // The negative cache holds only strings, but a broken block being typed
    // in mints one per keystroke — trim it too.
    final failed = _failed[id];
    while (failed != null && failed.length > 256) {
      failed.remove(failed.first);
    }
    _scheduleCap();
  }

  /// Run [_capNow] at the end of the next frame — the one the arrival's
  /// rebuild lays out and paints.
  void _scheduleCap() {
    if (_capScheduled) return;
    _capScheduled = true;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      _capScheduled = false;
      if (!_disposed) _capNow();
    });
  }

  void _capNow() {
    var evicted = false;
    for (final MapEntry(key: id, value: map) in _cache.entries) {
      if (map.length > _maxPerPreviewer) {
        final keep = _painted[id] ?? const <String>{};
        // Oldest-first snapshot; mutate the map, not the list being walked.
        for (final source in map.keys.toList(growable: false)) {
          if (map.length <= _maxPerPreviewer) break;
          if (keep.contains(source)) continue;
          final img = map.remove(source);
          // The baseline stays: like the size, it describes the formula, not
          // the pixels, and folded layout keeps using it while they are away.
          if (img != null) {
            _evictedPendingDispose.add(img);
            evicted = true;
          }
        }
      }
      // Baselines of evicted rasters are kept (above), so bound them too; one
      // is only dropped once its raster is gone as well.
      final baselines = _baselines[id];
      if (baselines != null && baselines.length > _maxBaselines) {
        for (final source in baselines.keys.toList(growable: false)) {
          if (baselines.length <= _maxBaselines) break;
          if (!map.containsKey(source)) baselines.remove(source);
        }
      }
    }
    if (!evicted) return;
    // The canvas still holds the evicted rasters in its snapshot: rebuild so
    // the next frame drops them, and dispose only after THAT frame.
    requestRebuild(() {});
    _disposeEvictedPostFrame();
  }

  /// Hand the evicted textures to the next frame boundary to dispose. Shared by
  /// the size cap and by a palette switch — both drop images that the frame
  /// currently being painted may still be drawing.
  void _disposeEvictedPostFrame() {
    if (_evictedPendingDispose.isEmpty) return;
    final batch = List<ui.Image>.of(_evictedPendingDispose);
    _evictedPendingDispose.clear();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      for (final img in batch) {
        img.dispose();
      }
    });
  }

  /// Dispose every cached preview texture. The owning editor State disposed
  /// its own image cache but never this pipeline — preview textures were
  /// reclaimed only by GC finalizers, non-deterministically (freeze audit).
  /// Same timing contract as the editor's `_imageCache` disposal: called from
  /// `State.dispose`, after the last frame that painted them.
  void dispose() {
    _disposed = true;
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

  /// Ask for a preview of [source]. Safe to call from layout and paint: work
  /// is deferred post-frame. No-op when already pending, failed, or unknown
  /// id; for a cached source it is the "painted now" signal ([_markPainted]).
  /// [targetWidth] reaches direct-form producers (see [RasterPreviewer.produce]).
  void request(String id, String source, [double targetWidth = 800]) {
    final previewer = _byId(id);
    if (previewer == null) return;
    if (imagesOf(id).containsKey(source)) {
      _markPainted(id, source);
      return;
    }
    if (_failed[id]?.contains(source) ?? false) return;
    final pending = _pending[id] ??= {};
    if (pending.contains(source)) return;

    final direct = previewer.produce(source, targetWidth, _tokens);
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

    if (previewer.buildOffstage(source, _tokens) == null) return;
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
                child:
                    p.buildOffstage(source, _tokens) ?? const SizedBox.shrink(),
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
  Future<ui.Image?>? produce(
    String source,
    double targetWidth,
    MicaTokens tokens,
  ) => renderMermaid(source, targetWidth, tokens);
}

/// LaTeX formulas through flutter_math_fork — the offstage form.
class MathPreviewer extends RasterPreviewer {
  const MathPreviewer();

  @override
  String get id => 'math';

  @override
  Widget buildOffstage(String source, MicaTokens tokens) => Math.tex(
    source,
    textStyle: TextStyle(
      fontSize: EditorTheme.mathRasterFontSize,
      color: tokens.text.primary,
    ),
    onErrorFallback: (e) => Text(
      source,
      style: TextStyle(
        fontFamily: kMonoFont,
        fontSize: 14,
        color: tokens.status.danger,
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
