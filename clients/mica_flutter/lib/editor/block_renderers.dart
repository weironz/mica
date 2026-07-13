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
  /// graceful-degradation hook: math/mermaid return null while their preview
  /// isn't ready (or failed), and mermaid declines while the caret sits in
  /// the block, so the source stays visible and editable as plain text.
  /// [index] is the node's position — for consulting the selection.
  _NodeLayout? layout(
    RenderDocument host,
    EditorNode node,
    int index,
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
    int index,
    double y,
    double maxWidth,
  ) {
    // Empty source, or raster not captured yet: request it and decline — the
    // text pipeline shows the editable LaTeX source meanwhile.
    if (node.text.trim().isEmpty) return null;
    final img = host._previewImages['math']?[node.text];
    if (img == null) {
      host.onRequestPreview?.call('math', node.text, maxWidth);
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
    int index,
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
      // Matte the image onto white first, so a transparent (alpha) PNG's
      // see-through areas read as the page background — not black.
      canvas.drawRect(r, Paint()..color = const Color(0xFFFFFFFF));
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
// Table
// -----------------------------------------------------------------------------

class TableRenderer extends AtomicBlockRenderer {
  const TableRenderer();

  @override
  String get kind => 'table';

  @override
  _NodeLayout? layout(
    RenderDocument host,
    EditorNode node,
    int index,
    double top,
    double maxWidth,
  ) {
    final table = TableData.fromBlock(node.data);
    final cols = table.columns.clamp(1, 64);
    // Per-column GFM alignment (separator colons) overrides the whole-table
    // setting where present.
    TextAlign alignAt(int c) => switch (table.alignFor(c)) {
      'center' => TextAlign.center,
      'right' => TextAlign.right,
      _ => TextAlign.left,
    };
    final gridTop = top + RenderDocument._tTopGutter;
    const x0 = EditorTheme.gutter;
    final fullW = (maxWidth - EditorTheme.gutter).clamp(60.0, double.infinity);
    // The table spans a fraction of the content width (dragging its right
    // edge adjusts it); columns share that span by weight.
    final availW = (fullW * table.tableWidth.clamp(0.15, 1.0))
        .clamp(60.0, fullW);

    // Column widths from normalized weights.
    final weights = [
      for (var c = 0; c < cols; c++)
        c < table.widths.length ? table.widths[c] : 1.0,
    ];
    final weightSum = weights.fold<double>(0, (a, b) => a + b);
    final colW = [for (final w in weights) availW * w / weightSum];
    final colEdges = <double>[x0];
    for (final w in colW) {
      colEdges.add(colEdges.last + w);
    }

    const padH = 10.0;
    const padV = 9.0;
    const minRowH = 42.0;
    final cells = <_TableCell>[];
    final rowHandles = <Rect>[];
    var yy = gridTop;
    for (var r = 0; r < table.rows.length; r++) {
      final isHeader = table.header && r == 0;
      final painters = <TextPainter>[];
      var rowH = minRowH;
      for (var c = 0; c < cols; c++) {
        final raw = c < table.rows[r].length ? table.rows[r][c] : '';
        final tp = TextPainter(
          text: RenderDocument.cellDisplaySpan(
            raw,
            host._appearance.applyTo(
              TextStyle(
                color: EditorTheme.text,
                fontSize: 15,
                height: 1.4,
                fontWeight: isHeader ? FontWeight.w600 : FontWeight.w400,
              ),
              isCode: false,
            ),
          ),
          textAlign: alignAt(c),
          textDirection: TextDirection.ltr,
        )..layout(
            minWidth: (colW[c] - padH * 2).clamp(0.0, double.infinity),
            maxWidth: (colW[c] - padH * 2).clamp(0.0, double.infinity),
          );
        painters.add(tp);
        rowH = rowH > tp.height + padV * 2 ? rowH : tp.height + padV * 2;
      }
      for (var c = 0; c < cols; c++) {
        cells.add(_TableCell(
          Rect.fromLTWH(colEdges[c], yy, colW[c], rowH),
          painters[c],
          isHeader,
          r,
          c,
          Offset(colEdges[c] + padH, yy + padV),
        ));
      }
      rowHandles.add(Rect.fromLTWH(x0, yy, 10, rowH));
      yy += rowH;
    }
    final gridBottom = yy;
    final gridHeight = gridBottom - gridTop;

    final colHandles = [
      for (var c = 0; c < cols; c++)
        Rect.fromLTWH(colEdges[c], top, colW[c], RenderDocument._tTopGutter),
    ];
    // Inner borders between columns, plus the table's right edge (c == cols)
    // so the LAST column is resizable too — drag resizes, a plain click on
    // the strip still adds a column.
    final colBorders = [
      for (var c = 1; c <= cols; c++)
        (rect: Rect.fromLTWH(colEdges[c] - 6, gridTop, 12, gridHeight), col: c),
    ];

    final layout = _NodeLayout(
      TextPainter(text: const TextSpan(text: ''), textDirection: TextDirection.ltr)
        ..layout(),
    )
      ..kind = 'table'
      ..boxLeft = EditorTheme.gutter
      ..nodeId = node.id
      ..contentLeft = 0
      ..boxTop = top
      ..textTop = gridTop
      ..textHeight = gridHeight
      ..boxHeight = RenderDocument._tTopGutter + gridHeight + RenderDocument._tBottomBar
      ..tableCells = cells
      ..tableTop = gridTop
      ..tableHeight = gridHeight
      ..tableHeader = table.header
      ..rowHandles = rowHandles
      ..colHandles = colHandles
      ..colBorders = colBorders
      // The add-column strip sits just past the table's right edge (clamped
      // inside the content area when the table is full-width).
      ..addColBar = Rect.fromLTWH(
        (colEdges.last + 2).clamp(0.0, maxWidth - RenderDocument._tEdge),
        gridTop,
        RenderDocument._tEdge,
        gridHeight,
      )
      ..addRowBar = Rect.fromLTWH(x0, gridBottom, availW, RenderDocument._tBottomBar)
      ..tableHandle = Rect.fromLTWH(x0, top, 16, RenderDocument._tTopGutter)
      ..tableDelete = Rect.fromLTWH(maxWidth - 18, top, 18, RenderDocument._tTopGutter);
    return layout;
  }

  @override
  void paint(
    RenderDocument host,
    Canvas canvas,
    Offset offset,
    _NodeLayout l,
    int index,
  ) {
    final border = Paint()
      ..color = const Color(0xFFCBD5E1)
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1;
    final headerBg = Paint()..color = const Color(0xFFF1F5F9);
    final editing = host._editingCell;
    for (final cell in l.tableCells) {
      final r = cell.rect.shift(offset);
      if (cell.header) canvas.drawRect(r, headerBg);
      canvas.drawRect(r, border);
      // Skip the text of the cell being edited (the overlay field shows it).
      if (editing != null &&
          editing.node == index &&
          editing.row == cell.row &&
          editing.col == cell.col) {
        continue;
      }
      cell.painter.paint(canvas, cell.textAt + offset);
    }

    if (host._hoverCode != index) return;

    final plusColor = EditorTheme.muted;

    // Hovered column border: an accent line so the resize target is obvious.
    final hb = host._hoverColBorder;
    if (hb != null && hb.node == index) {
      final b = l.colBorders.where((x) => x.col == hb.col);
      if (b.isNotEmpty) {
        final r = b.first.rect.shift(offset);
        canvas.drawRect(
          Rect.fromLTWH(r.center.dx - 1, r.top, 2, r.height),
          Paint()..color = const Color(0x802563EB),
        );
      }
    }

    // Row / column drag grips: a single soft rounded pill centered on the edge
    // line — a clean handle affordance rather than three scattered dots.
    final gripPaint = Paint()..color = const Color(0xFFC2CAD6);
    for (final h in l.rowHandles) {
      final r = h.shift(offset);
      final len = (r.height * 0.5).clamp(12.0, 18.0);
      canvas.drawRRect(
        RRect.fromRectAndRadius(
          Rect.fromCenter(
            center: Offset(r.left, r.center.dy),
            width: 3.5,
            height: len,
          ),
          const Radius.circular(2),
        ),
        gripPaint,
      );
    }
    for (final h in l.colHandles) {
      final r = h.shift(offset);
      final len = (r.width * 0.4).clamp(14.0, 24.0);
      canvas.drawRRect(
        RRect.fromRectAndRadius(
          Rect.fromCenter(
            center: Offset(r.center.dx, r.bottom),
            width: len,
            height: 3.5,
          ),
          const Radius.circular(2),
        ),
        gripPaint,
      );
    }

    void plusBar(Rect? rect, bool vertical) {
      if (rect == null) return;
      final r = rect.shift(offset);
      canvas.drawRRect(
        RRect.fromRectAndRadius(r.deflate(2), const Radius.circular(4)),
        Paint()..color = const Color(0x14000000),
      );
      final tp = TextPainter(
        text: TextSpan(
          text: String.fromCharCode(Icons.add.codePoint),
          style: TextStyle(
            fontFamily: Icons.add.fontFamily,
            package: Icons.add.fontPackage,
            fontSize: 14,
            color: plusColor,
          ),
        ),
        textDirection: TextDirection.ltr,
      )..layout();
      tp.paint(canvas, r.center - Offset(tp.width / 2, tp.height / 2));
      tp.dispose();
    }

    plusBar(l.addColBar, true);
    plusBar(l.addRowBar, false);

    void cornerIcon(Rect? rect, IconData icon, Color color) {
      if (rect == null) return;
      final r = rect.shift(offset);
      final tp = TextPainter(
        text: TextSpan(
          text: String.fromCharCode(icon.codePoint),
          style: TextStyle(
            fontFamily: icon.fontFamily,
            package: icon.fontPackage,
            fontSize: 14,
            color: color,
          ),
        ),
        textDirection: TextDirection.ltr,
      )..layout();
      tp.paint(canvas, r.center - Offset(tp.width / 2, tp.height / 2));
      tp.dispose();
    }

    cornerIcon(l.tableHandle, Icons.drag_indicator, EditorTheme.muted);
    cornerIcon(l.tableDelete, Icons.delete_outline, const Color(0xFFDC2626));
  }
}

// -----------------------------------------------------------------------------
// Mermaid (a code_block wearing a diagram)
// -----------------------------------------------------------------------------

/// Renders ```mermaid fenced blocks as diagrams. The node stays a plain
/// `code_block` in the document model (round-trips through Markdown
/// untouched) — only rendering changes, and only when ALL of: the language
/// is mermaid, the caret is outside the block (the focused block shows its
/// editable source, Typora-style), and the preview pipeline has produced an
/// image (no producer on this platform, a syntax error, or a pending render
/// all fall back to the highlighted source via null-decline).
class MermaidRenderer extends AtomicBlockRenderer {
  const MermaidRenderer();

  @override
  String get kind => 'code_block';

  @override
  _NodeLayout? layout(
    RenderDocument host,
    EditorNode node,
    int index,
    double y,
    double maxWidth,
  ) {
    if ((node.data['language'] as String?) != 'mermaid') return null;
    if (node.text.trim().isEmpty) return null;
    // The form is an EXPLICIT choice via the [code|preview] tabs — clicking
    // or selecting the rendered diagram must not flip it into source (that
    // visual jump was terrible). data.view == 'code' shows the source;
    // absent means preview, the default.
    if ((node.data['view'] as String?) == 'code') {
      // Source form: drop any transient zoom/pan so the preview returns at
      // its natural size (a leftover zoom read as "the diagram shrank").
      host._previewZoom.remove(node.id);
      host._previewPan.remove(node.id);
      return null;
    }
    final avail =
        (maxWidth - EditorTheme.gutter - 24).clamp(40.0, double.infinity);
    final img = host._previewImages['mermaid']?[node.text];
    if (img == null) {
      host.onRequestPreview?.call('mermaid', node.text, avail);
      return null;
    }
    // Fill the content width: diagrams read better large, so scale UP as
    // well as down (the producer rasterized at 2x of this width, so the
    // upscale stays crisp). The BLOCK keeps its zoom=1 size — ctrl+wheel
    // zoom only rescales the picture inside this fixed viewport (paint
    // clips and centers), so zooming never reflows the page.
    final zoom = host._previewZoom[node.id] ?? 1.0;
    final w = avail;
    final h = w * (img.height / img.width.clamp(1, 1 << 30));
    final wz = w * zoom;
    final hz = h * zoom;
    // The painter must be laid out: code_block is a TEXT kind, so pointer
    // hit-testing runs text-position math against it (unlike the atomic
    // kinds, whose clicks take the block path). Empty text → offset 0 →
    // the caret lands in the block → next layout declines → source form.
    return _NodeLayout(
      TextPainter(
        text: const TextSpan(text: ''),
        textDirection: TextDirection.ltr,
      )..layout(),
    )
      ..kind = 'code_block'
      ..nodeId = node.id
      ..langText = 'mermaid'
      ..boxLeft = EditorTheme.gutter
      ..contentLeft = EditorTheme.gutter +
          ((maxWidth - EditorTheme.gutter - w) / 2).clamp(0.0, double.infinity)
      ..mathImage = img
      ..mathSize = Size(wz, hz)
      ..boxTop = y
      // Visually it IS the picture: no card, near-zero block padding.
      ..textTop = y + 2
      ..textHeight = h
      ..boxHeight = h + 4
      // Bottom-right, mirroring the source form's toolbar corner: diagram
      // content usually anchors top-left, so this corner overlaps least.
      // Preview (the default) leads, code follows.
      ..viewCodeTab = Rect.fromLTWH(maxWidth - 8 - 44, y + h + 4 - 26, 44, 20)
      ..viewPreviewTab = Rect.fromLTWH(
          maxWidth - 8 - 44 - 2 - 56, y + h + 4 - 26, 56, 20);
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
    // No backdrop, no card: the user is here for the diagram, so the block
    // chrome disappears and the picture sits directly on the page. The block
    // rect is a fixed viewport — the (possibly zoomed) picture centers in it
    // and clips to it, so zoom never moves surrounding content.
    final viewport = Rect.fromLTWH(offset.dx + l.boxLeft, offset.dy + l.boxTop,
        host.size.width - l.boxLeft, l.boxHeight);
    final pan = host._previewPan[l.nodeId] ?? Offset.zero;
    final dst = Rect.fromCenter(
      center: viewport.center + pan,
      width: l.mathSize.width,
      height: l.mathSize.height,
    );
    canvas.save();
    canvas.clipRect(viewport);
    canvas.drawImageRect(
      img,
      Rect.fromLTWH(0, 0, img.width.toDouble(), img.height.toDouble()),
      dst,
      Paint()..filterQuality = FilterQuality.medium,
    );
    canvas.restore();
  }

  @override
  void paintOverlay(
    RenderDocument host,
    Canvas canvas,
    Offset offset,
    _NodeLayout l,
    int index,
  ) {
    // The [code|preview] switch only surfaces on hover, so at rest the
    // block reads as a plain picture.
    if (host._hoverCode != index) return;
    host._paintViewTabs(canvas, offset, l, active: 'preview');
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
    int index,
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
