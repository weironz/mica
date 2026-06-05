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
// Image
// -----------------------------------------------------------------------------

class ImageRenderer extends AtomicBlockRenderer {
  const ImageRenderer();

  @override
  String get kind => 'image';

  /// The node's load key: our storage `file_id`, or the external `url`.
  static String? _key(RenderDocument host, _NodeLayout l) {
    final node = host._nodes.firstWhere(
      (n) => n.id == l.nodeId,
      orElse: () => EditorNode(id: '', kind: 'paragraph', text: ''),
    );
    return (node.data['file_id'] ?? node.data['url']) as String?;
  }

  @override
  _NodeLayout? layout(
    RenderDocument host,
    EditorNode node,
    double y,
    double maxWidth,
  ) {
    // Images load by file_id (our storage) or external url. The host decides
    // whether to actually fetch a url (it skips fetching when auto-re-hosting,
    // since the url will soon become a file_id).
    final key = (node.data['file_id'] ?? node.data['url']) as String?;
    final img = key == null ? null : host._images[key];
    if (img == null && key != null && !host._imageErrors.contains(key)) {
      host.onRequestImage?.call(key);
    }

    final maxW = (maxWidth - EditorTheme.gutter).clamp(1.0, double.infinity);
    final align = switch (node.data['align']) {
      'center' => 1,
      'right' => 2,
      _ => 0,
    };
    final requested = (node.data['width'] as num?)?.toDouble();

    double w, h;
    if (img != null) {
      final natW = img.width.toDouble();
      final natH = img.height.toDouble();
      w = (requested ?? natW).clamp(40.0, maxW);
      h = w * (natH / (natW == 0 ? 1 : natW));
    } else {
      w = (requested ?? 320).clamp(40.0, maxW);
      h = RenderDocument._imagePlaceholderH;
    }

    final left = EditorTheme.gutter +
        switch (align) {
      1 => (maxW - w) / 2,
      2 => maxW - w,
      _ => 0.0,
    };

    // Hover toolbar (top-right inside the image) + right-edge resize handle.
    const btn = 26.0, pad = 4.0;
    final barW = _imageActions.length * btn + pad * 2;
    final barH = btn + pad * 2;
    final barLeft = (left + w - barW - 6).clamp(left + 2, left + w);
    final barTop = y + 6;
    final buttons = [
      for (var k = 0; k < _imageActions.length; k++)
        Rect.fromLTWH(barLeft + pad + k * btn, barTop + pad, btn, btn),
    ];

    return _NodeLayout(TextPainter(textDirection: TextDirection.ltr))
      ..kind = 'image'
      ..nodeId = node.id
      ..contentLeft = left
      ..boxTop = y
      ..textTop = y
      ..textHeight = h
      ..boxHeight = h + RenderDocument._imageGap
      ..imageDst = Rect.fromLTWH(left, y, w, h)
      ..imageBar = Rect.fromLTWH(barLeft, barTop, barW, barH)
      ..imageButtons = buttons
      ..imageResize = Rect.fromLTWH(left + w - 5, y + h / 2 - 18, 10, 36);
  }

  @override
  void paint(
    RenderDocument host,
    Canvas canvas,
    Offset offset,
    _NodeLayout l,
    int index,
  ) {
    final dst = l.imageDst;
    if (dst == null) return;
    final r = dst.shift(offset);
    final rr = RRect.fromRectAndRadius(r, const Radius.circular(6));
    final key = _key(host, l);
    final decoded = key == null ? null : host._images[key];
    if (decoded != null) {
      canvas.save();
      canvas.clipRRect(rr);
      paintImage(
        canvas: canvas,
        rect: r,
        image: decoded,
        fit: BoxFit.cover,
        filterQuality: FilterQuality.medium,
      );
      canvas.restore();
      return;
    }
    // Placeholder: rounded fill + centered icon (broken on error, else image).
    canvas.drawRRect(rr, Paint()..color = EditorTheme.codeBg);
    final isError = key != null && host._imageErrors.contains(key);
    final icon = isError ? Icons.broken_image_outlined : Icons.image_outlined;
    final glyph = TextPainter(
      text: TextSpan(
        text: String.fromCharCode(icon.codePoint),
        style: TextStyle(
          fontFamily: icon.fontFamily,
          package: icon.fontPackage,
          fontSize: 28,
          color: EditorTheme.faint,
        ),
      ),
      textDirection: TextDirection.ltr,
    )..layout();
    glyph.paint(
      canvas,
      r.center - Offset(glyph.width / 2, glyph.height / 2),
    );
    glyph.dispose();
  }

  @override
  void paintOverlay(
    RenderDocument host,
    Canvas canvas,
    Offset offset,
    _NodeLayout l,
    int index,
  ) {
    if (host._hoverImage != index) return;
    final bar = l.imageBar;
    final resize = l.imageResize;
    if (resize != null) {
      final rr = resize.shift(offset).deflate(2);
      canvas.drawRRect(
        RRect.fromRectAndRadius(rr, const Radius.circular(3)),
        Paint()..color = const Color(0xCC1E293B),
      );
    }
    if (bar == null) return;
    canvas.drawRRect(
      RRect.fromRectAndRadius(bar.shift(offset), const Radius.circular(7)),
      Paint()..color = const Color(0xF21E293B),
    );
    const icons = {
      'expand': Icons.open_in_full,
      'left': Icons.format_align_left,
      'center': Icons.format_align_center,
      'right': Icons.format_align_right,
      'delete': Icons.delete_outline,
    };
    for (var k = 0; k < l.imageButtons.length; k++) {
      final action = _imageActions[k];
      final rect = l.imageButtons[k].shift(offset);
      final color = action == 'delete'
          ? const Color(0xFFFCA5A5)
          : const Color(0xFFE2E8F0);
      final glyph = TextPainter(
        text: TextSpan(
          text: String.fromCharCode(icons[action]!.codePoint),
          style: TextStyle(
            fontFamily: icons[action]!.fontFamily,
            package: icons[action]!.fontPackage,
            fontSize: 16,
            color: color,
          ),
        ),
        textDirection: TextDirection.ltr,
      )..layout();
      glyph.paint(canvas, rect.center - Offset(glyph.width / 2, glyph.height / 2));
      glyph.dispose();
    }
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
