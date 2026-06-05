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
  /// block's tinted backdrop. Most renderers need nothing here.
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
