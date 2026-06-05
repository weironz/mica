part of 'render.dart';

/// Renderer for one atomic block kind (divider, image, math, table, …).
///
/// `RenderDocument`'s three passes (layout, background, body/overlay paint)
/// dispatch through [RenderDocument.atomicRenderers] instead of per-kind
/// branches; the text pipeline stays the engine for text-bearing kinds (see
/// docs/render-architecture.md). Adding a block type means adding a renderer
/// here and registering it — the main loops don't change.
abstract class AtomicBlockRenderer {
  const AtomicBlockRenderer();

  /// The `EditorNode.kind` this renderer claims.
  String get kind;

  /// Build the node's layout slice, or return null to NOT claim the node this
  /// pass — it then falls through to the text pipeline. That null is the
  /// graceful-degradation hook: math (and later mermaid) returns null while
  /// its raster isn't ready / its source is empty, so the source stays
  /// visible and editable as plain text.
  _NodeLayout? layout(
    RenderDocument host,
    EditorNode node,
    double y,
    double maxWidth,
  );

  /// Paint the block body. Selection, caret, and quote bars are the host's.
  void paint(
    RenderDocument host,
    Canvas canvas,
    Offset offset,
    _NodeLayout l,
    int index,
  );

  /// Paint behind the selection highlight (first pass) — e.g. the math
  /// block's tinted backdrop. Dispatched by *kind* (unlike [paint], which
  /// dispatches on the layout's producer): the backdrop marks the block's
  /// identity and shows on the fallen-through source form too. Most
  /// renderers need nothing here.
  void paintBackground(
    RenderDocument host,
    Canvas canvas,
    Offset offset,
    _NodeLayout l,
    int index,
  ) {}

  /// Paint above everything but the caret (last pass) — e.g. the image
  /// hover toolbar + resize handle. Most renderers need nothing here.
  void paintOverlay(
    RenderDocument host,
    Canvas canvas,
    Offset offset,
    _NodeLayout l,
    int index,
  ) {}
}

// -----------------------------------------------------------------------------
// Math block
// -----------------------------------------------------------------------------

class MathBlockRenderer extends AtomicBlockRenderer {
  const MathBlockRenderer();

  @override
  String get kind => 'math_block';

  @override
  _NodeLayout? layout(
    RenderDocument host,
    EditorNode node,
    double y,
    double maxWidth,
  ) {
    // Empty source, or raster not captured yet: request it and decline — the
    // text pipeline shows the editable LaTeX source meanwhile.
    if (node.text.trim().isEmpty) return null;
    final img = host._mathImages[node.text];
    if (img == null) {
      host.onRequestMath?.call(node.text);
      return null;
    }
    // Rendered formula: centered, sized from the capture (image is at device
    // pixel ratio; draw at logical size, downscale to fit).
    const dpr = EditorTheme.mathPixelRatio;
    var w = img.width / dpr;
    var h = img.height / dpr;
    final avail =
        (maxWidth - EditorTheme.gutter - 24).clamp(40.0, double.infinity);
    if (w > avail) {
      h *= avail / w;
      w = avail;
    }
    return _NodeLayout(TextPainter(textDirection: TextDirection.ltr))
      ..kind = 'math_block'
      ..nodeId = node.id
      ..boxLeft = EditorTheme.gutter
      ..contentLeft = EditorTheme.gutter +
          ((maxWidth - EditorTheme.gutter - w) / 2).clamp(0.0, double.infinity)
      ..mathImage = img
      ..mathSize = Size(w, h)
      ..boxTop = y
      ..textTop = y + 10
      ..textHeight = h
      ..boxHeight = h + 20;
  }

  @override
  void paintBackground(
    RenderDocument host,
    Canvas canvas,
    Offset offset,
    _NodeLayout l,
    int index,
  ) {
    canvas.drawRRect(
      RRect.fromRectAndRadius(
        Rect.fromLTWH(offset.dx + l.boxLeft, offset.dy + l.boxTop - 6,
            host.size.width - l.boxLeft, l.boxHeight + 12),
        const Radius.circular(6),
      ),
      Paint()..color = const Color(0xFFF5F3FF),
    );
  }

  @override
  void paint(
    RenderDocument host,
    Canvas canvas,
    Offset offset,
    _NodeLayout l,
    int index,
  ) {
    final img = l.mathImage;
    if (img == null) return;
    final dst = Rect.fromLTWH(offset.dx + l.contentLeft, offset.dy + l.textTop,
        l.mathSize.width, l.mathSize.height);
    canvas.drawImageRect(
      img,
      Rect.fromLTWH(0, 0, img.width.toDouble(), img.height.toDouble()),
      dst,
      Paint()..filterQuality = FilterQuality.medium,
    );
  }
}

// -----------------------------------------------------------------------------
// Divider
// -----------------------------------------------------------------------------

class DividerRenderer extends AtomicBlockRenderer {
  const DividerRenderer();

  @override
  String get kind => 'divider';

  @override
  _NodeLayout? layout(
    RenderDocument host,
    EditorNode node,
    double y,
    double maxWidth,
  ) {
    final liInset = node.liLevel != null ? 26.0 + 24.0 * node.liLevel! : 0.0;
    return _NodeLayout(TextPainter(textDirection: TextDirection.ltr))
      ..kind = 'divider'
      ..nodeId = node.id
      ..quoteDepth = node.quoteDepth
      ..boxLeft = EditorTheme.gutter + liInset
      ..contentLeft = EditorTheme.gutter + liInset + 16.0 * node.quoteDepth
      ..boxTop = y
      ..textTop = y
      ..textHeight = RenderDocument._dividerHeight
      ..boxHeight = RenderDocument._dividerHeight;
  }

  @override
  void paint(
    RenderDocument host,
    Canvas canvas,
    Offset offset,
    _NodeLayout l,
    int index,
  ) {
    final cy = offset.dy + l.boxTop + l.boxHeight / 2;
    canvas.drawLine(
      Offset(offset.dx + l.contentLeft, cy),
      Offset(offset.dx + host.size.width, cy),
      Paint()
        ..color = EditorTheme.quoteBar
        ..strokeWidth = 1.5,
    );
  }
}
