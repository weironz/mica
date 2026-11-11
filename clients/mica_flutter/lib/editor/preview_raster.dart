import 'dart:async';
import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/widgets.dart';

import '../ui/theme_tokens.dart';
import 'mermaid_preview.dart';
import 'mermaid_theme.dart';
import 'preview_engine.dart';
import 'render.dart' show EditorTheme;
import 'resource_demand.dart';
import 'svg_raster.dart';

part 'preview_pipeline.dart';

typedef RasterPreview = ({ui.Image image, double? baseline});

/// A null preview leaves the editable source visible.
abstract class RasterPreviewer {
  const RasterPreviewer();

  String get id;

  Future<RasterPreview?> produce(
    String source,
    double targetWidth,
    MicaTokens tokens,
  );
}

class MermaidPreviewer extends RasterPreviewer {
  const MermaidPreviewer();

  @override
  String get id => 'mermaid';

  @override
  Future<RasterPreview?> produce(
    String source,
    double targetWidth,
    MicaTokens tokens,
  ) async {
    final image = await renderMermaid(source, targetWidth, tokens);
    return image == null ? null : (image: image, baseline: null);
  }
}

/// RaTeX uses a different layout style for inline and displayed formulas.
class MathPreviewer extends RasterPreviewer {
  const MathPreviewer({required this.display});

  final bool display;

  @override
  String get id => display ? 'math' : 'math-inline';

  @override
  Future<RasterPreview?> produce(
    String source,
    double targetWidth,
    MicaTokens tokens,
  ) async {
    final math = await mathSvg(source, display, cssHex(tokens.text.primary));
    if (math == null ||
        !math.width.isFinite ||
        !math.height.isFinite ||
        !math.baseline.isFinite ||
        math.width <= 0 ||
        math.height <= 0) {
      return null;
    }
    const ratio = EditorTheme.mathPixelRatio;
    final image = await rasterizeSvg(
      math.svg,
      math.width * EditorTheme.mathRasterFontSize * ratio,
    );
    if (image == null) return null;
    final baseline = (math.baseline / math.height).clamp(0.0, 1.0);
    return (image: image, baseline: image.height * baseline / ratio);
  }
}
