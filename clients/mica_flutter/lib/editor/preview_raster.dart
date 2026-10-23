import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_math_fork/flutter_math.dart';

import 'mermaid_preview.dart';
import 'model.dart' show kMonoFont;
import 'resource_demand.dart';
import '../ui/theme_tokens.dart';
import 'render.dart' show EditorTheme;

part 'preview_pipeline.dart';

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
