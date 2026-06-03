import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';

import 'highlight.dart';
import 'marks.dart';
import 'model.dart';
import 'table.dart';

/// User-adjustable editor appearance (document font). Page width is applied by
/// the surrounding page layout, not here.
class EditorAppearance {
  const EditorAppearance({this.fontScale = 1.0, this.fontFamily});

  /// Multiplier applied to every block's font size (0.85–1.4 typically).
  final double fontScale;

  /// Optional font family override for prose (code blocks keep monospace).
  final String? fontFamily;

  /// Bundled CJK font, used as a fallback so Chinese/Japanese/Korean glyphs
  /// render immediately (Flutter Web otherwise downloads them on demand, which
  /// flashes ".notdef" boxes on the custom-painted surface).
  static const List<String> cjkFallback = ['CJKFallback'];

  TextStyle applyTo(TextStyle base, {required bool isCode}) {
    final scaled = (base.fontSize ?? 16) * fontScale;
    return base.copyWith(
      fontSize: scaled,
      fontFamily: (fontFamily != null && !isCode) ? fontFamily : base.fontFamily,
      fontFamilyFallback: cjkFallback,
    );
  }

  @override
  bool operator ==(Object other) =>
      other is EditorAppearance &&
      other.fontScale == fontScale &&
      other.fontFamily == fontFamily;

  @override
  int get hashCode => Object.hash(fontScale, fontFamily);
}

/// Visual constants for the editing surface. Kept here so the look stays in one
/// place; per-kind styling only changes inline typography, never block chrome
/// (see docs/editor.md).
class EditorTheme {
  static const Color text = Color(0xFF0F172A);
  static const Color muted = Color(0xFF475569);
  static const Color faint = Color(0xFF94A3B8);
  static const Color caret = Color(0xFF2563EB);
  static const Color selection = Color(0x332563EB);
  static const Color codeBg = Color(0xFFF1F5F9);
  static const Color quoteBar = Color(0xFFCBD5E1);

  static const double caretWidth = 2;
  static const double bottomPad = 96;

  /// Minimum height of the writing surface so a blank page still offers a large
  /// click-to-write area (Word/Typora feel) rather than a thin strip.
  static const double minSurfaceHeight = 420;
  static const double codePadH = 14;
  static const double codePadV = 12;

  /// Height of the toolbar row at the top of a code block (language selector on
  /// the left, copy button on the right).
  static const double codeToolbar = 26;

  static TextStyle styleFor(EditorNode node) {
    switch (node.kind) {
      case 'heading':
        switch (node.headingLevel) {
          case 1:
            return const TextStyle(color: text, fontSize: 30, height: 1.25, fontWeight: FontWeight.w700);
          case 2:
            return const TextStyle(color: text, fontSize: 24, height: 1.3, fontWeight: FontWeight.w700);
          case 3:
            return const TextStyle(color: text, fontSize: 20, height: 1.35, fontWeight: FontWeight.w600);
          default:
            return const TextStyle(color: text, fontSize: 17, height: 1.4, fontWeight: FontWeight.w600);
        }
      case 'quote':
        return const TextStyle(color: muted, fontSize: 16, height: 1.5, fontStyle: FontStyle.italic);
      case 'code_block':
        return const TextStyle(color: text, fontSize: 14, height: 1.5, fontFamily: 'monospace');
      case 'todo':
        if (node.todoChecked) {
          return const TextStyle(
            color: faint,
            fontSize: 16,
            height: 1.5,
            decoration: TextDecoration.lineThrough,
          );
        }
        return const TextStyle(color: text, fontSize: 16, height: 1.5);
      default:
        return const TextStyle(color: text, fontSize: 16, height: 1.5);
    }
  }

  /// Left inset where a node's text begins (room for bullets/checkboxes/bars).
  static double leadingInset(String kind) {
    switch (kind) {
      case 'bulleted_list':
      case 'numbered_list':
        return 26;
      case 'todo':
        return 30;
      case 'quote':
        return 16;
      case 'code_block':
        return codePadH;
      default:
        return 0;
    }
  }

  static bool _isList(String kind) =>
      kind == 'bulleted_list' || kind == 'numbered_list' || kind == 'todo';

  /// Vertical gap above a node, given the previous node's kind.
  static double gapAbove(String kind, String? prevKind) {
    if (prevKind == null) return 0;
    if (_isList(kind) && _isList(prevKind)) return kind == prevKind ? 2 : 6;
    if (kind == 'heading') return 22;
    if (kind == 'code_block' || prevKind == 'code_block') return 12;
    return 9;
  }
}

/// Per-node layout produced each [RenderDocument.performLayout].
class _NodeLayout {
  _NodeLayout(this.painter);

  final TextPainter painter;
  double contentLeft = 0; // x where text starts
  double textTop = 0; // y of text top
  double textHeight = 0;
  double boxTop = 0; // y of the node's full box (incl. code padding)
  double boxHeight = 0;
  int ordinal = 0; // for numbered lists
  Rect? checkbox; // todo checkbox rect (local), if any
  Rect? langLabel; // code-block language selector rect (local), if any
  Rect? copyButton; // code-block copy button rect (local), if any
  Rect? wrapButton; // code-block wrap toggle rect (local), if any
  String langText = ''; // resolved code language
  String nodeId = '';
  bool codeWrap = false; // whether this code block wraps
  double codeWidth = 0; // natural (unwrapped) text width, for code blocks
  double codeVisible = 0; // visible width of the code area
  Rect? scrollTrack; // horizontal scrollbar track (local), if code overflows
  String kind = 'paragraph';
  bool todoChecked = false;

  // Table layout (kind == 'table').
  List<_TableCell> tableCells = const [];
  double tableTop = 0;
  double tableHeight = 0;
  bool tableHeader = false;
  List<Rect> rowHandles = const [];
  List<Rect> colHandles = const [];
  List<({Rect rect, int col})> colBorders = const [];
  Rect? addColBar;
  Rect? addRowBar;
  Rect? tableHandle; // top-left block handle
  Rect? tableDelete; // top-right delete icon

  // Image layout (kind == 'image'): the destination rect (local) the decoded
  // image (or its placeholder) is painted into, plus hover affordances.
  Rect? imageDst;
  Rect? imageBar; // hover toolbar background
  List<Rect> imageButtons = const []; // toolbar buttons (see _imageActions)
  Rect? imageResize; // right-edge resize handle
}

/// Toolbar actions for an image, in painted order.
const List<String> _imageActions = [
  'expand',
  'left',
  'center',
  'right',
  'delete',
];

/// One laid-out table cell.
class _TableCell {
  _TableCell(this.rect, this.painter, this.header, this.row, this.col, this.textAt);
  final Rect rect; // full cell rect (local)
  final TextPainter painter;
  final bool header;
  final int row;
  final int col;
  final Offset textAt; // where to paint the cell text (local)
}

/// Which code-block toolbar icon the pointer is hovering.
enum _CodeIcon { none, lang, copy, wrap }

/// The single editing surface: one leaf render object that lays out and paints
/// every node, the caret, and the selection, and maps screen points to document
/// positions. There is exactly one of these per open document — no per-block
/// editable widgets — so the caret and selection are document-wide.
class RenderDocument extends RenderBox {
  RenderDocument({
    required List<EditorNode> nodes,
    required DocSelection? selection,
    required bool showCaret,
    required bool caretOn,
    required EditorAppearance appearance,
  }) : _nodes = nodes,
       _selection = selection,
       _showCaret = showCaret,
       _caretOn = caretOn,
       _appearance = appearance;

  List<EditorNode> _nodes;
  set nodes(List<EditorNode> value) {
    _nodes = value;
    markNeedsLayout();
  }

  DocSelection? _selection;
  set selection(DocSelection? value) {
    if (_selection == value) return;
    _selection = value;
    markNeedsPaint();
  }

  bool _showCaret;
  set showCaret(bool value) {
    if (_showCaret == value) return;
    _showCaret = value;
    markNeedsPaint();
  }

  bool _caretOn;
  set caretOn(bool value) {
    if (_caretOn == value) return;
    _caretOn = value;
    markNeedsPaint();
  }

  EditorAppearance _appearance;
  set appearance(EditorAppearance value) {
    if (_appearance == value) return;
    _appearance = value;
    markNeedsLayout();
  }

  final List<_NodeLayout> _layouts = [];

  /// Per-code-block horizontal scroll offset, keyed by node id. Code blocks do
  /// not wrap; long lines scroll left/right within the block.
  final Map<String, double> _codeScroll = {};

  /// Decoded images keyed by `file_id`, painted directly onto the canvas. The
  /// host (`MicaEditor`) populates this as images load and calls [setImages].
  Map<String, ui.Image> _images = {};
  set images(Map<String, ui.Image> value) {
    _images = value;
    markNeedsLayout();
  }

  /// `file_id`s that failed to load — painted as a broken-image placeholder.
  Set<String> _imageErrors = {};
  set imageErrors(Set<String> value) {
    _imageErrors = value;
    markNeedsPaint();
  }

  /// Called during layout when an image node's bytes aren't decoded yet, so the
  /// host can fetch + decode them. Must not synchronously mutate layout.
  void Function(String fileId)? onRequestImage;

  /// `nodeId:offset` of the caret at the last layout. Auto-scroll-to-caret only
  /// runs when this changes (a real caret move), so caret blink / unrelated
  /// re-layouts don't fight a manual horizontal scroll.
  String _lastCaretKey = '';

  // Code-block toolbar hover state (icons only show when hovering the block).
  int? _hoverCode;
  _CodeIcon _hoverIcon = _CodeIcon.none;

  // Image hover state (toolbar + resize handle only show when hovering).
  int? _hoverImage;

  // The table cell currently being edited inline (its underlying painted text is
  // hidden so it doesn't ghost behind the overlay field).
  ({int node, int row, int col})? _editingCell;
  set editingCell(({int node, int row, int col})? value) {
    if (_editingCell == value) return;
    _editingCell = value;
    markNeedsPaint();
  }

  /// Update which code block / icon the pointer is over. Drives hover reveal.
  void setHover(Offset? local) {
    int? node;
    var icon = _CodeIcon.none;
    int? image;
    if (local != null) {
      for (var i = 0; i < _layouts.length; i++) {
        final l = _layouts[i];
        if (l.kind == 'image') {
          if (local.dy >= l.boxTop && local.dy <= l.boxTop + l.boxHeight) {
            image = i;
          }
          continue;
        }
        if (l.kind != 'code_block' && l.kind != 'table') continue;
        if (local.dy >= l.boxTop && local.dy <= l.boxTop + l.boxHeight) {
          node = i;
          if (l.copyButton?.contains(local) ?? false) {
            icon = _CodeIcon.copy;
          } else if (l.wrapButton?.contains(local) ?? false) {
            icon = _CodeIcon.wrap;
          } else if (l.langLabel?.contains(local) ?? false) {
            icon = _CodeIcon.lang;
          }
          break;
        }
      }
    }
    if (node != _hoverCode || icon != _hoverIcon || image != _hoverImage) {
      _hoverCode = node;
      _hoverIcon = icon;
      _hoverImage = image;
      markNeedsPaint();
    }
  }

  @override
  void attach(PipelineOwner owner) {
    super.attach(owner);
    // Re-layout when web fonts finish loading; otherwise glyphs that weren't
    // ready at first paint stay as ".notdef" boxes until an interaction.
    PaintingBinding.instance.systemFonts.addListener(_onSystemFontsChanged);
  }

  @override
  void detach() {
    PaintingBinding.instance.systemFonts.removeListener(_onSystemFontsChanged);
    super.detach();
  }

  void _onSystemFontsChanged() => markNeedsLayout();

  @override
  bool hitTestSelf(Offset position) => true;

  /// Bottom y of the last node's box (excludes the click-below padding).
  double get contentBottom =>
      _layouts.isEmpty ? 0 : _layouts.last.boxTop + _layouts.last.boxHeight;

  @override
  void performLayout() {
    final maxWidth = constraints.maxWidth.isFinite ? constraints.maxWidth : 600.0;
    for (final l in _layouts) {
      l.painter.dispose();
      for (final cell in l.tableCells) {
        cell.painter.dispose();
      }
    }
    _layouts.clear();

    final sel = _selection;
    final caretKey = (sel != null && sel.isCollapsed && sel.focus.node < _nodes.length)
        ? '${_nodes[sel.focus.node].id}:${sel.focus.offset}'
        : '';
    final caretMoved = caretKey != _lastCaretKey;
    _lastCaretKey = caretKey;

    double y = 0;
    String? prevKind;
    var consecutiveNumbered = 0;
    for (final node in _nodes) {
      y += EditorTheme.gapAbove(node.kind, prevKind);

      if (node.kind == 'table') {
        final layout = _layoutTable(node, y, maxWidth);
        _layouts.add(layout);
        y += layout.boxHeight;
        prevKind = node.kind;
        continue;
      }

      if (node.kind == 'divider') {
        final layout = _NodeLayout(TextPainter(textDirection: TextDirection.ltr))
          ..kind = 'divider'
          ..nodeId = node.id
          ..contentLeft = 0
          ..boxTop = y
          ..textTop = y
          ..textHeight = _dividerHeight
          ..boxHeight = _dividerHeight;
        _layouts.add(layout);
        y += layout.boxHeight;
        prevKind = node.kind;
        continue;
      }

      if (node.kind == 'image') {
        final layout = _layoutImage(node, y, maxWidth);
        _layouts.add(layout);
        y += layout.boxHeight;
        prevKind = node.kind;
        continue;
      }

      final style = _appearance.applyTo(
        EditorTheme.styleFor(node),
        isCode: node.isCode,
      );
      final contentLeft = EditorTheme.leadingInset(node.kind);
      final isCode = node.isCode;
      final textWidth = (maxWidth - contentLeft - (isCode ? EditorTheme.codePadH : 0))
          .clamp(0.0, double.infinity);

      final String? codeLang = isCode
          ? resolveCodeLanguage(node.text, node.data['language'] as String?)
          : null;
      final TextSpan span;
      if (isCode && node.text.isNotEmpty) {
        span = buildCodeSpan(node.text, codeLang!, style);
      } else {
        span = buildMarkedSpan(node.text, marksFromData(node.data), style);
      }

      final codeWrap = isCode && node.data['wrap'] == true;
      final painter = TextPainter(
        text: span,
        textDirection: TextDirection.ltr,
        textWidthBasis: TextWidthBasis.parent,
      )..layout(maxWidth: (isCode && !codeWrap) ? double.infinity : textWidth);

      final layout = _NodeLayout(painter)
        ..contentLeft = contentLeft
        ..kind = node.kind
        ..nodeId = node.id
        ..todoChecked = node.todoChecked
        ..langText = codeLang ?? ''
        ..codeWrap = codeWrap
        ..codeWidth = isCode ? painter.width : 0;

      // Code blocks pad symmetrically; their controls float at the bottom-right
      // on hover (no reserved top toolbar row).
      final innerTop = isCode ? EditorTheme.codePadV : 0.0;
      layout.boxTop = y;
      layout.textTop = y + innerTop;
      layout.textHeight = painter.height;
      layout.boxHeight = painter.height + (isCode ? 2 * EditorTheme.codePadV : 0);

      if (node.kind == 'numbered_list') {
        consecutiveNumbered += 1;
        layout.ordinal = consecutiveNumbered;
      } else {
        consecutiveNumbered = 0;
      }
      if (node.kind == 'todo') {
        final lh = painter.preferredLineHeight;
        const box = 18.0;
        layout.checkbox = Rect.fromLTWH(2, layout.textTop + (lh - box) / 2, box, box);
      }
      if (isCode) {
        // Keep the caret visible by auto-scrolling the code horizontally — but
        // only when the caret actually moved, so blink/re-layout or a manual
        // scrollbar drag is never yanked back to the caret.
        // The current node's index is `_layouts.length` (not yet appended).
        final visible = (maxWidth - contentLeft - EditorTheme.codePadH)
            .clamp(0.0, double.infinity);
        var scroll = _codeScroll[node.id] ?? 0;
        if (caretMoved &&
            sel != null &&
            sel.isCollapsed &&
            sel.focus.node == _layouts.length) {
          final caretX = painter
              .getOffsetForCaret(
                TextPosition(offset: sel.focus.offset.clamp(0, node.text.length)),
                Rect.zero,
              )
              .dx;
          if (caretX - scroll > visible - 12) scroll = caretX - visible + 12;
          if (caretX - scroll < 0) scroll = caretX;
        }
        final maxScroll = (layout.codeWidth - visible).clamp(0.0, double.infinity);
        _codeScroll[node.id] = scroll.clamp(0.0, maxScroll);

        layout.codeVisible = visible;
        if (maxScroll > 0) {
          // Reserve space for a horizontal scrollbar at the bottom of the block.
          const barH = 8.0;
          const barGap = 5.0;
          layout.boxHeight += barH + barGap;
          layout.scrollTrack = Rect.fromLTWH(
            contentLeft,
            layout.boxTop + layout.boxHeight - barH - 2,
            visible,
            barH,
          );
        }

        // Controls float at the bottom-right (above the scrollbar if present):
        // language selector + wrap + copy, right-aligned.
        const iconBox = 22.0;
        final marker = TextPainter(
          text: TextSpan(
            text: '${layout.langText}  ▾',
            style: const TextStyle(fontSize: 11, color: EditorTheme.muted),
          ),
          textDirection: TextDirection.ltr,
        )..layout();
        final labelW = marker.width + 14;
        final labelH = marker.height + 6;
        marker.dispose();
        final bottomLimit = layout.scrollTrack?.top ?? (layout.boxTop + layout.boxHeight);
        final iconY = bottomLimit - iconBox - 4;
        layout.copyButton =
            Rect.fromLTWH(maxWidth - iconBox - 8, iconY, iconBox, iconBox);
        layout.wrapButton =
            Rect.fromLTWH(maxWidth - 2 * iconBox - 12, iconY, iconBox, iconBox);
        layout.langLabel = Rect.fromLTWH(
          maxWidth - 2 * iconBox - 12 - labelW - 6,
          iconY + (iconBox - labelH) / 2,
          labelW,
          labelH,
        );
      }

      _layouts.add(layout);
      y += layout.boxHeight;
      prevKind = node.kind;
    }

    y += EditorTheme.bottomPad;
    size = constraints.constrain(
      Size(maxWidth, y < EditorTheme.minSurfaceHeight ? EditorTheme.minSurfaceHeight : y),
    );
  }

  // The grid spans the full content width (aligned with body text on both
  // sides); handles/add-buttons overlay the edges on hover instead of taking
  // horizontal space. Only vertical space is reserved (top gutter for column
  // handles, bottom bar for add-row).
  static const double _tTopGutter = 16;
  static const double _tBottomBar = 16;
  static const double _tEdge = 16; // overlay handle thickness

  // Vertical box reserved for a divider (horizontal rule centered within it).
  static const double _dividerHeight = 26;

  // Image placeholder size (before the real image decodes) and vertical gap.
  static const double _imagePlaceholderH = 180;
  static const double _imageGap = 6;

  _NodeLayout _layoutImage(EditorNode node, double top, double maxWidth) {
    // Images load by file_id (our storage) or external url. The host decides
    // whether to actually fetch a url (it skips fetching when auto-re-hosting,
    // since the url will soon become a file_id).
    final key = (node.data['file_id'] ?? node.data['url']) as String?;
    final img = key == null ? null : _images[key];
    if (img == null && key != null && !_imageErrors.contains(key)) {
      onRequestImage?.call(key);
    }

    final maxW = maxWidth.clamp(1.0, double.infinity);
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
      h = _imagePlaceholderH;
    }

    final left = switch (align) {
      1 => (maxW - w) / 2,
      2 => maxW - w,
      _ => 0.0,
    };

    // Hover toolbar (top-right inside the image) + right-edge resize handle.
    const btn = 26.0, pad = 4.0;
    final barW = _imageActions.length * btn + pad * 2;
    final barH = btn + pad * 2;
    final barLeft = (left + w - barW - 6).clamp(left + 2, left + w);
    final barTop = top + 6;
    final buttons = [
      for (var k = 0; k < _imageActions.length; k++)
        Rect.fromLTWH(barLeft + pad + k * btn, barTop + pad, btn, btn),
    ];

    final layout = _NodeLayout(TextPainter(textDirection: TextDirection.ltr))
      ..kind = 'image'
      ..nodeId = node.id
      ..contentLeft = left
      ..boxTop = top
      ..textTop = top
      ..textHeight = h
      ..boxHeight = h + _imageGap
      ..imageDst = Rect.fromLTWH(left, top, w, h)
      ..imageBar = Rect.fromLTWH(barLeft, barTop, barW, barH)
      ..imageButtons = buttons
      ..imageResize = Rect.fromLTWH(left + w - 5, top + h / 2 - 18, 10, 36);
    return layout;
  }

  _NodeLayout _layoutTable(EditorNode node, double top, double maxWidth) {
    final table = TableData.fromBlock(node.data);
    final cols = table.columns.clamp(1, 64);
    final textAlign = switch (table.align) {
      'center' => TextAlign.center,
      'right' => TextAlign.right,
      _ => TextAlign.left,
    };
    final gridTop = top + _tTopGutter;
    const x0 = 0.0;
    final availW = maxWidth.clamp(60.0, double.infinity);

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

    const padH = 8.0;
    const padV = 7.0;
    const minRowH = 34.0;
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
          text: TextSpan(
            text: raw.isEmpty ? ' ' : raw,
            style: _appearance.applyTo(
              TextStyle(
                color: EditorTheme.text,
                fontSize: 15,
                height: 1.4,
                fontWeight: isHeader ? FontWeight.w600 : FontWeight.w400,
              ),
              isCode: false,
            ),
          ),
          textAlign: textAlign,
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
      rowHandles.add(Rect.fromLTWH(0, yy, _tEdge, rowH));
      yy += rowH;
    }
    final gridBottom = yy;
    final gridHeight = gridBottom - gridTop;

    final colHandles = [
      for (var c = 0; c < cols; c++)
        Rect.fromLTWH(colEdges[c], top, colW[c], _tTopGutter),
    ];
    final colBorders = [
      for (var c = 1; c < cols; c++)
        (rect: Rect.fromLTWH(colEdges[c] - 4, gridTop, 8, gridHeight), col: c),
    ];

    final layout = _NodeLayout(
      TextPainter(text: const TextSpan(text: ''), textDirection: TextDirection.ltr)
        ..layout(),
    )
      ..kind = 'table'
      ..nodeId = node.id
      ..contentLeft = 0
      ..boxTop = top
      ..textTop = gridTop
      ..textHeight = gridHeight
      ..boxHeight = _tTopGutter + gridHeight + _tBottomBar
      ..tableCells = cells
      ..tableTop = gridTop
      ..tableHeight = gridHeight
      ..tableHeader = table.header
      ..rowHandles = rowHandles
      ..colHandles = colHandles
      ..colBorders = colBorders
      ..addColBar = Rect.fromLTWH(maxWidth - _tEdge, gridTop, _tEdge, gridHeight)
      ..addRowBar = Rect.fromLTWH(x0, gridBottom, availW, _tBottomBar)
      ..tableHandle = Rect.fromLTWH(0, top, _tEdge, _tTopGutter)
      ..tableDelete = Rect.fromLTWH(maxWidth - 18, top, 18, _tTopGutter);
    return layout;
  }

  // ---------------------------------------------------------------------------
  // Painting
  // ---------------------------------------------------------------------------

  @override
  void paint(PaintingContext context, Offset offset) {
    final canvas = context.canvas;
    // Block backgrounds first, so the selection highlight (next) is not hidden
    // behind a code block's fill.
    _paintBlockBackgrounds(canvas, offset);
    _paintSelection(canvas, offset);
    for (var i = 0; i < _layouts.length; i++) {
      _paintNode(canvas, offset, i);
    }
    for (var i = 0; i < _layouts.length; i++) {
      if (_layouts[i].kind == 'image') _paintImageOverlay(canvas, offset, _layouts[i], i);
    }
    _paintAtomicSelection(canvas, offset);
    _paintScrollbars(canvas, offset);
    _paintCaret(canvas, offset);
  }

  /// Highlight selected atomic nodes (image/divider/table) on top of their
  /// opaque content: a whole-block caret stop (collapsed) gets a tint + border;
  /// atomic nodes inside a ranged selection get the selection tint.
  void _paintAtomicSelection(Canvas canvas, Offset offset) {
    final sel = _selection;
    if (sel == null) return;
    if (sel.isCollapsed) {
      if (_isAtomicNode(sel.focus.node) && sel.focus.node < _layouts.length) {
        _drawAtomicHighlight(canvas, offset, sel.focus.node, border: true);
      }
      return;
    }
    for (var i = sel.start.node; i <= sel.end.node && i < _layouts.length; i++) {
      if (_isAtomicNode(i) && (i != sel.start.node || i != sel.end.node)) {
        _drawAtomicHighlight(canvas, offset, i, border: false);
      }
    }
  }

  void _drawAtomicHighlight(Canvas canvas, Offset offset, int i, {required bool border}) {
    final l = _layouts[i];
    final box = (l.kind == 'image' && l.imageDst != null)
        ? l.imageDst!.shift(offset)
        : Rect.fromLTWH(offset.dx, offset.dy + l.boxTop, size.width, l.boxHeight);
    final rr = RRect.fromRectAndRadius(
      box.inflate(border ? 2 : 0),
      const Radius.circular(6),
    );
    canvas.drawRRect(rr, Paint()..color = EditorTheme.selection);
    if (border) {
      canvas.drawRRect(
        rr,
        Paint()
          ..color = EditorTheme.caret
          ..style = PaintingStyle.stroke
          ..strokeWidth = 2,
      );
    }
  }

  void _paintBlockBackgrounds(Canvas canvas, Offset offset) {
    for (var i = 0; i < _layouts.length; i++) {
      final l = _layouts[i];
      if (l.kind == 'code_block') {
        canvas.drawRRect(
          RRect.fromRectAndRadius(
            Rect.fromLTWH(offset.dx, offset.dy + l.boxTop, size.width, l.boxHeight),
            const Radius.circular(6),
          ),
          Paint()..color = EditorTheme.codeBg,
        );
      } else if (l.kind == 'quote') {
        canvas.drawRect(
          Rect.fromLTWH(offset.dx + 2, offset.dy + l.textTop, 3, l.textHeight),
          Paint()..color = EditorTheme.quoteBar,
        );
      }
      // Toolbar (language left, wrap + copy right) — only while hovering the block.
      if (l.kind == 'code_block' && _hoverCode == i) {
        _paintCodeToolbar(canvas, offset, l);
      }
    }
  }

  void _paintCodeToolbar(Canvas canvas, Offset offset, _NodeLayout l) {
    final label = l.langLabel;
    if (label != null) {
      final r = label.shift(offset);
      canvas.drawRRect(
        RRect.fromRectAndRadius(r, const Radius.circular(5)),
        Paint()..color = _hoverIcon == _CodeIcon.lang
            ? const Color(0xFFCBD5E1)
            : const Color(0xFFE2E8F0),
      );
      final marker = TextPainter(
        text: TextSpan(
          text: '${l.langText}  ▾',
          style: const TextStyle(fontSize: 11, color: EditorTheme.muted),
        ),
        textDirection: TextDirection.ltr,
      )..layout();
      marker.paint(canvas, r.topLeft + const Offset(7, 3));
      marker.dispose();
    }

    final wrap = l.wrapButton;
    if (wrap != null) {
      _paintIconButton(
        canvas,
        wrap.shift(offset),
        Icons.wrap_text,
        hovered: _hoverIcon == _CodeIcon.wrap,
        active: l.codeWrap,
        tooltip: l.codeWrap ? 'No wrap' : 'Wrap',
      );
    }

    final copy = l.copyButton;
    if (copy != null) {
      _paintIconButton(
        canvas,
        copy.shift(offset),
        Icons.content_copy,
        hovered: _hoverIcon == _CodeIcon.copy,
        active: false,
        tooltip: 'Copy',
      );
    }
  }

  void _paintIconButton(
    Canvas canvas,
    Rect rect,
    IconData icon, {
    required bool hovered,
    required bool active,
    required String tooltip,
  }) {
    if (hovered) {
      canvas.drawRRect(
        RRect.fromRectAndRadius(rect, const Radius.circular(5)),
        Paint()..color = const Color(0xFFCBD5E1),
      );
    }
    final color = active
        ? EditorTheme.caret
        : (hovered ? EditorTheme.text : EditorTheme.muted);
    final glyph = TextPainter(
      text: TextSpan(
        text: String.fromCharCode(icon.codePoint),
        style: TextStyle(
          fontFamily: icon.fontFamily,
          package: icon.fontPackage,
          fontSize: 15,
          color: color,
        ),
      ),
      textDirection: TextDirection.ltr,
    )..layout();
    glyph.paint(
      canvas,
      rect.topLeft + Offset((rect.width - glyph.width) / 2, (rect.height - glyph.height) / 2),
    );
    glyph.dispose();

    if (hovered) {
      _paintTooltip(canvas, rect, tooltip);
    }
  }

  void _paintTooltip(Canvas canvas, Rect anchor, String text) {
    final tp = TextPainter(
      text: TextSpan(
        text: text,
        style: const TextStyle(fontSize: 11, color: Color(0xFFFFFFFF)),
      ),
      textDirection: TextDirection.ltr,
    )..layout();
    const padH = 6.0;
    const padV = 3.0;
    final w = tp.width + padH * 2;
    final h = tp.height + padV * 2;
    var left = anchor.center.dx - w / 2;
    left = left.clamp(4.0, size.width - w - 4);
    final top = anchor.top - h - 4;
    final bg = Rect.fromLTWH(left, top, w, h);
    canvas.drawRRect(
      RRect.fromRectAndRadius(bg, const Radius.circular(4)),
      Paint()..color = const Color(0xFF0F172A),
    );
    tp.paint(canvas, Offset(left + padH, top + padV));
    tp.dispose();
  }

  /// Node index whose code-language selector contains [local], or null.
  int? codeLanguageAt(Offset local) {
    for (var i = 0; i < _layouts.length; i++) {
      if (_layouts[i].langLabel?.contains(local) ?? false) return i;
    }
    return null;
  }

  /// Node index whose code copy button contains [local], or null.
  int? codeCopyAt(Offset local) {
    for (var i = 0; i < _layouts.length; i++) {
      if (_layouts[i].copyButton?.contains(local) ?? false) return i;
    }
    return null;
  }

  /// Node index whose code wrap toggle contains [local], or null.
  int? codeWrapAt(Offset local) {
    for (var i = 0; i < _layouts.length; i++) {
      if (_layouts[i].wrapButton?.contains(local) ?? false) return i;
    }
    return null;
  }

  /// Scroll the code block under [local] horizontally by [deltaX]. Returns true
  /// if a code block consumed the scroll.
  bool scrollCodeAt(Offset local, double deltaX) {
    for (final l in _layouts) {
      if (l.kind != 'code_block') continue;
      if (local.dy < l.boxTop || local.dy > l.boxTop + l.boxHeight) continue;
      final visible = (size.width - l.contentLeft - EditorTheme.codePadH)
          .clamp(0.0, double.infinity);
      final maxScroll = (l.codeWidth - visible).clamp(0.0, double.infinity);
      if (maxScroll <= 0) return false;
      final current = _codeScroll[l.nodeId] ?? 0;
      final next = (current + deltaX).clamp(0.0, maxScroll);
      if (next != current) {
        _codeScroll[l.nodeId] = next;
        markNeedsPaint();
      }
      return true;
    }
    return false;
  }

  ({double left, double width, double maxScroll}) _thumb(_NodeLayout l) {
    final track = l.scrollTrack!;
    final visible = l.codeVisible;
    final maxScroll = (l.codeWidth - visible).clamp(0.0, double.infinity);
    final thumbWidth = (visible * visible / l.codeWidth).clamp(28.0, visible);
    final scroll = _codeScroll[l.nodeId] ?? 0;
    final usable = (visible - thumbWidth).clamp(0.0, double.infinity);
    final left = maxScroll <= 0
        ? track.left
        : track.left + (scroll / maxScroll) * usable;
    return (left: left, width: thumbWidth, maxScroll: maxScroll);
  }

  /// Node index whose horizontal scrollbar track contains [local], or null.
  int? scrollbarAt(Offset local) {
    for (var i = 0; i < _layouts.length; i++) {
      final track = _layouts[i].scrollTrack;
      // Generous vertical hit slop so the thin bar is easy to grab.
      if (track != null && track.inflate(6).contains(local)) return i;
    }
    return null;
  }

  /// Set a code block's scroll so the scrollbar thumb centers on track x.
  void setCodeScrollByTrackX(int index, double localX) {
    final l = _layouts[index];
    final track = l.scrollTrack;
    if (track == null) return;
    final t = _thumb(l);
    final usable = track.width - t.width;
    if (usable <= 0 || t.maxScroll <= 0) return;
    final pos = (localX - track.left - t.width / 2).clamp(0.0, usable);
    _codeScroll[l.nodeId] = (pos / usable) * t.maxScroll;
    markNeedsPaint();
  }

  void _paintScrollbars(Canvas canvas, Offset offset) {
    for (final l in _layouts) {
      final track = l.scrollTrack;
      if (track == null) continue;
      final r = track.shift(offset);
      final radius = Radius.circular(track.height / 2);
      canvas.drawRRect(
        RRect.fromRectAndRadius(r, radius),
        Paint()..color = const Color(0x14000000),
      );
      final t = _thumb(l);
      canvas.drawRRect(
        RRect.fromRectAndRadius(
          Rect.fromLTWH(offset.dx + t.left, r.top, t.width, track.height),
          radius,
        ),
        Paint()..color = const Color(0xFF94A3B8),
      );
    }
  }

  void _paintNode(Canvas canvas, Offset offset, int i) {
    final l = _layouts[i];
    if (l.kind == 'table') {
      _paintTable(canvas, offset, l, i);
      return;
    }
    if (l.kind == 'divider') {
      final cy = offset.dy + l.boxTop + l.boxHeight / 2;
      canvas.drawLine(
        Offset(offset.dx, cy),
        Offset(offset.dx + size.width, cy),
        Paint()
          ..color = EditorTheme.quoteBar
          ..strokeWidth = 1.5,
      );
      return;
    }
    if (l.kind == 'image') {
      _paintImage(canvas, offset, l);
      return;
    }
    final origin = offset + Offset(l.contentLeft, l.textTop);

    switch (l.kind) {
      case 'bulleted_list':
        canvas.drawCircle(
          origin + Offset(-14, l.painter.preferredLineHeight * 0.5),
          2.6,
          Paint()..color = EditorTheme.text,
        );
      case 'numbered_list':
        final marker = TextPainter(
          text: TextSpan(
            text: '${l.ordinal}.',
            style: TextStyle(
              color: EditorTheme.text,
              fontSize: 16 * _appearance.fontScale,
              height: 1.5,
            ),
          ),
          textDirection: TextDirection.ltr,
        )..layout();
        marker.paint(canvas, origin + Offset(-marker.width - 6, 0));
        marker.dispose();
      case 'todo':
        final box = l.checkbox;
        if (box != null) {
          final r = box.shift(offset);
          final rr = RRect.fromRectAndRadius(r, const Radius.circular(4));
          if (l.todoChecked) {
            canvas.drawRRect(rr, Paint()..color = EditorTheme.caret);
            final tick = Path()
              ..moveTo(r.left + 4, r.center.dy)
              ..lineTo(r.left + 7.5, r.bottom - 4.5)
              ..lineTo(r.right - 4, r.top + 4.5);
            canvas.drawPath(
              tick,
              Paint()
                ..color = const Color(0xFFFFFFFF)
                ..style = PaintingStyle.stroke
                ..strokeWidth = 2
                ..strokeJoin = StrokeJoin.round
                ..strokeCap = StrokeCap.round,
            );
          } else {
            canvas.drawRRect(
              rr,
              Paint()
                ..color = EditorTheme.faint
                ..style = PaintingStyle.stroke
                ..strokeWidth = 1.5,
            );
          }
        }
    }

    if (l.kind == 'code_block') {
      // Code does not wrap; clip to the block and offset by its scroll.
      final scroll = _codeScroll[l.nodeId] ?? 0;
      final visible = (size.width - l.contentLeft - EditorTheme.codePadH)
          .clamp(0.0, double.infinity);
      canvas.save();
      canvas.clipRect(
        Rect.fromLTWH(origin.dx, origin.dy, visible, l.textHeight),
      );
      l.painter.paint(canvas, origin - Offset(scroll, 0));
      canvas.restore();
    } else {
      l.painter.paint(canvas, origin);
    }
  }

  void _paintImage(Canvas canvas, Offset offset, _NodeLayout l) {
    final dst = l.imageDst;
    if (dst == null) return;
    final r = dst.shift(offset);
    final rr = RRect.fromRectAndRadius(r, const Radius.circular(6));
    final key = _imageKey(l);
    final decoded = key == null ? null : _images[key];
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
    final isError = key != null && _imageErrors.contains(key);
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

  void _paintImageOverlay(Canvas canvas, Offset offset, _NodeLayout l, int i) {
    if (_hoverImage != i) return;
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

  String? _imageKey(_NodeLayout l) {
    if (l.kind != 'image') return null;
    final node = _nodes.firstWhere(
      (n) => n.id == l.nodeId,
      orElse: () => EditorNode(id: '', kind: 'paragraph', text: ''),
    );
    return (node.data['file_id'] ?? node.data['url']) as String?;
  }

  void _paintSelection(Canvas canvas, Offset offset) {
    final sel = _selection;
    if (sel == null || sel.isCollapsed) return;
    final start = sel.start;
    final end = sel.end;
    final paint = Paint()..color = EditorTheme.selection;
    for (var i = start.node; i <= end.node && i < _layouts.length; i++) {
      final l = _layouts[i];
      // Atomic nodes (image/table/divider) are opaque and painted after the
      // selection layer, so their highlight is drawn on top in
      // _paintAtomicSelection instead — skip them here.
      if (EditorNode.isAtomicKind(l.kind)) continue;
      final from = i == start.node ? start.offset : 0;
      final to = i == end.node ? end.offset : _nodes[i].text.length;
      final isCode = l.kind == 'code_block';
      final scroll = isCode ? (_codeScroll[l.nodeId] ?? 0) : 0.0;
      final origin = offset + Offset(l.contentLeft - scroll, l.textTop);
      if (from == to) {
        if (i != end.node) {
          // Empty / fully-included blank line: show a thin marker.
          canvas.drawRect(
            Rect.fromLTWH(origin.dx, origin.dy, 6, l.painter.preferredLineHeight),
            paint,
          );
        }
        continue;
      }
      final boxes = l.painter.getBoxesForSelection(
        TextSelection(baseOffset: from, extentOffset: to),
      );
      if (isCode) {
        final visible = (size.width - l.contentLeft - EditorTheme.codePadH)
            .clamp(0.0, double.infinity);
        canvas.save();
        canvas.clipRect(Rect.fromLTWH(
          offset.dx + l.contentLeft,
          offset.dy + l.textTop,
          visible,
          l.textHeight,
        ));
      }
      for (final b in boxes) {
        canvas.drawRect(b.toRect().shift(origin), paint);
      }
      if (isCode) canvas.restore();
    }
  }

  void _paintTable(Canvas canvas, Offset offset, _NodeLayout l, int index) {
    final border = Paint()
      ..color = const Color(0xFFCBD5E1)
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1;
    final headerBg = Paint()..color = const Color(0xFFF1F5F9);
    final editing = _editingCell;
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

    if (_hoverCode != index) return;

    final handlePaint = Paint()..color = const Color(0xFFCBD5E1);
    final plusColor = EditorTheme.muted;

    // Row handles (left gutter) + column handles (top gutter): little grips.
    for (final h in l.rowHandles) {
      final r = h.shift(offset);
      canvas.drawRRect(
        RRect.fromRectAndRadius(
          Rect.fromLTWH(r.left + 3, r.center.dy - 9, 6, 18),
          const Radius.circular(3),
        ),
        handlePaint,
      );
    }
    for (final h in l.colHandles) {
      final r = h.shift(offset);
      canvas.drawRRect(
        RRect.fromRectAndRadius(
          Rect.fromLTWH(r.center.dx - 9, r.top + 3, 18, 6),
          const Radius.circular(3),
        ),
        handlePaint,
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

  /// Table cell under [local], or null.
  ({int node, int row, int col, Rect rect})? tableCellAt(Offset local) {
    for (var i = 0; i < _layouts.length; i++) {
      final l = _layouts[i];
      if (l.kind != 'table') continue;
      for (final cell in l.tableCells) {
        if (cell.rect.contains(local)) {
          return (node: i, row: cell.row, col: cell.col, rect: cell.rect);
        }
      }
    }
    return null;
  }

  /// Local rect of a specific table cell, or null.
  Rect? tableCellRect(int node, int row, int col) {
    if (node < 0 || node >= _layouts.length) return null;
    final l = _layouts[node];
    if (l.kind != 'table') return null;
    for (final cell in l.tableCells) {
      if (cell.row == row && cell.col == col) return cell.rect;
    }
    return null;
  }

  /// Row handle (left gutter) under [local] → (node, row), or null.
  ({int node, int row})? tableRowHandleAt(Offset local) {
    for (var i = 0; i < _layouts.length; i++) {
      final l = _layouts[i];
      if (l.kind != 'table') continue;
      for (var r = 0; r < l.rowHandles.length; r++) {
        if (l.rowHandles[r].contains(local)) return (node: i, row: r);
      }
    }
    return null;
  }

  /// Column handle (top gutter) under [local] → (node, col), or null.
  ({int node, int col})? tableColHandleAt(Offset local) {
    for (var i = 0; i < _layouts.length; i++) {
      final l = _layouts[i];
      if (l.kind != 'table') continue;
      for (var c = 0; c < l.colHandles.length; c++) {
        if (l.colHandles[c].contains(local)) return (node: i, col: c);
      }
    }
    return null;
  }

  /// Table block handle (top-left) under [local] → node, or null.
  int? tableHandleAt(Offset local) {
    for (var i = 0; i < _layouts.length; i++) {
      if (_layouts[i].kind != 'table') continue;
      if (_layouts[i].tableHandle?.contains(local) ?? false) return i;
    }
    return null;
  }

  /// Table delete icon (top-right) under [local] → node, or null.
  int? tableDeleteAt(Offset local) {
    for (var i = 0; i < _layouts.length; i++) {
      if (_layouts[i].kind != 'table') continue;
      if (_layouts[i].tableDelete?.contains(local) ?? false) return i;
    }
    return null;
  }

  /// Add-row / add-column bar under [local] → (node, isColumn), or null.
  ({int node, bool column})? tableAddAt(Offset local) {
    for (var i = 0; i < _layouts.length; i++) {
      final l = _layouts[i];
      if (l.kind != 'table') continue;
      if (l.addColBar?.contains(local) ?? false) return (node: i, column: true);
      if (l.addRowBar?.contains(local) ?? false) return (node: i, column: false);
    }
    return null;
  }

  /// Column border under [local] for resize → (node, rightCol), or null.
  ({int node, int col})? tableColBorderAt(Offset local) {
    for (var i = 0; i < _layouts.length; i++) {
      final l = _layouts[i];
      if (l.kind != 'table') continue;
      for (final b in l.colBorders) {
        if (b.rect.contains(local)) return (node: i, col: b.col);
      }
    }
    return null;
  }

  /// Current normalized column weights of a table node (for resize math).
  List<double> tableWeights(int index) {
    if (index < 0 || index >= _nodes.length) return const [];
    return TableData.fromBlock(_nodes[index].data).widths;
  }

  /// Pixel width available to columns (full content width).
  double tableAvailWidth() => size.width.clamp(60.0, double.infinity);

  void _paintCaret(Canvas canvas, Offset offset) {
    final sel = _selection;
    if (sel == null || !sel.isCollapsed || !_showCaret || !_caretOn) return;
    final rect = caretRectFor(sel.focus);
    if (rect == null) return;
    // In a horizontally-scrolled code block, don't draw the caret once it has
    // scrolled out of the visible area (otherwise it leaks past the page edge).
    final l = _layouts[sel.focus.node];
    if (l.kind == 'code_block') {
      if (rect.left < l.contentLeft - 1 ||
          rect.left > l.contentLeft + l.codeVisible) {
        return;
      }
    }
    canvas.drawRRect(
      RRect.fromRectAndRadius(rect.shift(offset), const Radius.circular(1)),
      Paint()..color = EditorTheme.caret,
    );
  }

  // ---------------------------------------------------------------------------
  // Geometry / hit testing (used by the widget for caret nav and pointers)
  // ---------------------------------------------------------------------------

  /// Caret rectangle (local coords) for a position, or null if out of range.
  Rect? caretRectFor(DocPosition pos) {
    if (pos.node < 0 || pos.node >= _layouts.length) return null;
    final l = _layouts[pos.node];
    if (EditorNode.isAtomicKind(l.kind)) return null; // no inline caret
    final off = pos.offset.clamp(0, _nodes[pos.node].text.length);
    final caret = l.painter.getOffsetForCaret(TextPosition(offset: off), Rect.zero);
    final scroll = l.kind == 'code_block' ? (_codeScroll[l.nodeId] ?? 0) : 0.0;
    return Rect.fromLTWH(
      l.contentLeft + caret.dx - scroll,
      l.textTop + caret.dy,
      EditorTheme.caretWidth,
      l.painter.preferredLineHeight,
    );
  }

  /// Document position nearest a local point (clamped into the document).
  DocPosition positionAt(Offset local) {
    if (_layouts.isEmpty) return const DocPosition(0, 0);
    final y = local.dy.clamp(0.0, size.height);
    var idx = _layouts.length - 1;
    for (var i = 0; i < _layouts.length; i++) {
      final l = _layouts[i];
      if (y < l.boxTop + l.boxHeight) {
        idx = i;
        break;
      }
    }
    // The caret can't sit inside an atomic node (table/divider/image); snap by
    // the pointer's side of the node's vertical midline: above → end of the
    // previous node, below → start of the next node. This lets a drag-select
    // include the atomic block (highlighting it) once it crosses the midline.
    if (_nodes[idx].isAtomic) {
      final l = _layouts[idx];
      final belowMid = y >= l.boxTop + l.boxHeight / 2;
      if (belowMid && idx + 1 < _nodes.length) {
        return DocPosition(idx + 1, 0);
      }
      var alt = idx - 1;
      while (alt >= 0 && _nodes[alt].isAtomic) {
        alt--;
      }
      if (alt >= 0) {
        return DocPosition(alt, _nodes[alt].text.length);
      }
      // No text node above: fall back to the next node, else this one.
      if (idx + 1 < _nodes.length) return DocPosition(idx + 1, 0);
      return DocPosition(idx, 0);
    }

    final l = _layouts[idx];
    final scroll = l.kind == 'code_block' ? (_codeScroll[l.nodeId] ?? 0) : 0.0;
    final localText = Offset(local.dx - l.contentLeft + scroll, y - l.textTop);
    final tp = l.painter.getPositionForOffset(localText);
    final offset = _nodes[idx].text.isEmpty
        ? 0
        : tp.offset.clamp(0, _nodes[idx].text.length);
    return DocPosition(idx, offset);
  }

  /// Position one visual line above [pos], tracking [goalX] (a local x).
  ///
  /// Probes just above the current line; if that does not move (the caret is on
  /// the first line of its node), it steps explicitly to the previous node's
  /// last line. The explicit step is what lets the caret cross the larger gap
  /// above a heading instead of getting trapped on it.
  DocPosition? positionAbove(DocPosition pos, double? goalX) {
    // An atomic node (image/divider/table) is a whole-block caret stop; from it,
    // step straight to the node above.
    if (_isAtomicNode(pos.node)) {
      return _stepToNode(pos.node - 1, goalX, fromBottom: true) ?? pos;
    }
    final rect = caretRectFor(pos);
    if (rect == null) return null;
    final x = goalX ?? rect.left;
    final probe = positionAt(Offset(x, rect.top - rect.height * 0.5));
    if (probe != pos && probe.node == pos.node) return probe; // moved up a line
    return _stepToNode(pos.node - 1, x, fromBottom: true) ?? const DocPosition(0, 0);
  }

  /// Position one visual line below [pos], tracking [goalX] (a local x).
  DocPosition? positionBelow(DocPosition pos, double? goalX) {
    if (_isAtomicNode(pos.node)) {
      return _stepToNode(pos.node + 1, goalX, fromBottom: false) ?? pos;
    }
    final rect = caretRectFor(pos);
    if (rect == null) return null;
    final x = goalX ?? rect.left;
    final probe = positionAt(Offset(x, rect.bottom + rect.height * 0.5));
    if (probe != pos && probe.node == pos.node) return probe; // moved down a line
    final last = _layouts.length - 1;
    return _stepToNode(pos.node + 1, x, fromBottom: false) ??
        DocPosition(last, _nodes[last].text.length);
  }

  bool _isAtomicNode(int index) =>
      index >= 0 && index < _nodes.length && _nodes[index].isAtomic;

  /// Land the caret on node [index] when crossing a node boundary: an atomic
  /// node becomes a whole-block stop (offset 0); a text node gets the line
  /// nearest goal-x ([fromBottom] picks its last vs first line).
  DocPosition? _stepToNode(int index, double? x, {required bool fromBottom}) {
    if (index < 0 || index >= _layouts.length) return null;
    if (_isAtomicNode(index)) return DocPosition(index, 0);
    final l = _layouts[index];
    final y = fromBottom ? l.textTop + l.textHeight - 1 : l.textTop + 1;
    return positionAt(Offset(x ?? 0, y));
  }

  /// Start of the visual line containing [pos].
  DocPosition lineStart(DocPosition pos) {
    final l = _layouts[pos.node];
    final lb = l.painter.getLineBoundary(TextPosition(offset: pos.offset));
    return DocPosition(pos.node, lb.start);
  }

  /// End of the visual line containing [pos].
  DocPosition lineEnd(DocPosition pos) {
    final l = _layouts[pos.node];
    final lb = l.painter.getLineBoundary(TextPosition(offset: pos.offset));
    return DocPosition(pos.node, lb.end);
  }

  /// Local top y of the node with [nodeId], or null if absent (for scroll-to).
  double? nodeBoxTop(String nodeId) {
    for (final l in _layouts) {
      if (l.nodeId == nodeId) return l.boxTop;
    }
    return null;
  }

  /// Node index of the image whose painted rect contains [local], or null.
  int? imageAt(Offset local) {
    for (var i = 0; i < _layouts.length; i++) {
      final l = _layouts[i];
      if (l.kind == 'image' && (l.imageDst?.contains(local) ?? false)) return i;
    }
    return null;
  }

  /// Toolbar action under [local] for the hovered image, or null. Gated on hover
  /// so the (invisible) bar can't be triggered when not shown.
  ({int node, String action})? imageActionAt(Offset local) {
    final i = _hoverImage;
    if (i == null || i >= _layouts.length) return null;
    final l = _layouts[i];
    for (var k = 0; k < l.imageButtons.length; k++) {
      if (l.imageButtons[k].contains(local)) {
        return (node: i, action: _imageActions[k]);
      }
    }
    return null;
  }

  /// Image node whose right-edge resize handle is under [local] (hover-gated).
  int? imageResizeAt(Offset local) {
    final i = _hoverImage;
    if (i == null || i >= _layouts.length) return null;
    return (_layouts[i].imageResize?.contains(local) ?? false) ? i : null;
  }

  /// Width the image at [index] would take if its right edge were dragged to
  /// local x [dx] (clamped to a sane range within the content width).
  double imageWidthFor(int index, double dx) {
    if (index < 0 || index >= _layouts.length) return 0;
    final l = _layouts[index];
    final left = l.contentLeft;
    return (dx - left).clamp(40.0, size.width);
  }

  /// Node index whose checkbox contains [local], or null.
  int? checkboxAt(Offset local) {
    for (var i = 0; i < _layouts.length; i++) {
      if (_layouts[i].checkbox?.contains(local) ?? false) return i;
    }
    return null;
  }

  @override
  void dispose() {
    for (final l in _layouts) {
      l.painter.dispose();
      for (final cell in l.tableCells) {
        cell.painter.dispose();
      }
    }
    _layouts.clear();
    super.dispose();
  }
}

/// Widget wrapper for [RenderDocument].
class DocumentSurface extends LeafRenderObjectWidget {
  const DocumentSurface({
    required this.nodes,
    required this.selection,
    required this.showCaret,
    required this.caretOn,
    required this.appearance,
    this.images = const {},
    this.imageErrors = const {},
    this.onRequestImage,
    super.key,
  });

  final List<EditorNode> nodes;
  final DocSelection? selection;
  final bool showCaret;
  final bool caretOn;
  final EditorAppearance appearance;
  final Map<String, ui.Image> images;
  final Set<String> imageErrors;
  final void Function(String fileId)? onRequestImage;

  @override
  RenderDocument createRenderObject(BuildContext context) => RenderDocument(
    nodes: nodes,
    selection: selection,
    showCaret: showCaret,
    caretOn: caretOn,
    appearance: appearance,
  )
    ..onRequestImage = onRequestImage
    ..imageErrors = imageErrors
    ..images = images;

  @override
  void updateRenderObject(BuildContext context, RenderDocument renderObject) {
    renderObject
      ..nodes = nodes
      ..selection = selection
      ..showCaret = showCaret
      ..caretOn = caretOn
      ..appearance = appearance
      ..onRequestImage = onRequestImage
      ..imageErrors = imageErrors
      ..images = images;
  }
}
