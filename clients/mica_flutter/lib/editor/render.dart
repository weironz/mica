import 'dart:ui' as ui;

import 'package:flutter/foundation.dart' show listEquals;
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';

import '../cjk_fonts.dart';
import '../l10n/locale_controller.dart';
import 'highlight.dart';
import 'marks.dart';
import 'model.dart';
import 'table.dart';

part 'block_renderers.dart';
part 'inline_atoms.dart';

/// User-adjustable editor appearance (document font). Page width is applied by
/// the surrounding page layout, not here.
class EditorAppearance {
  const EditorAppearance({this.fontScale = 1.0, this.fontFamily});

  /// Multiplier applied to every block's font size (0.85–1.4 typically).
  final double fontScale;

  /// Optional font family override for prose (code blocks keep monospace).
  final String? fontFamily;

  /// CJK fallback chain: crisp system fonts on desktop (Windows 微软雅黑 etc.),
  /// the bundled font on web. See [cjkFontFallback].
  static List<String> get cjkFallback => cjkFontFallback;

  TextStyle applyTo(TextStyle base, {required bool isCode}) {
    final scaled = (base.fontSize ?? 16) * fontScale;
    return base.copyWith(
      fontSize: scaled,
      fontFamily: (fontFamily != null && !isCode)
          ? fontFamily
          : base.fontFamily,
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
  // A soft near-black (GitHub's ink) rather than a hard, cool slate-900 — reads
  // as calmer/warmer ink on the page while keeping ~13:1 contrast.
  static const Color text = Color(0xFF24292F);
  static const Color muted = Color(0xFF57606A);
  static const Color faint = Color(0xFF9AA4AF);
  static const Color caret = Color(0xFF2563EB);
  static const Color selection = Color(0x332563EB);
  static const Color codeBg = Color(0xFFF4F4F6);

  /// Inline `code` span pill — a soft neutral chip behind the mono text (drawn
  /// in _paintInlineCode). Translucent so a text selection tints through it.
  static const Color inlineCodeBg = Color(0x1A64748B);
  static const Color quoteBar = Color(0xFFCBD5E1);
  static const Color dropLine = Color(0xFF2563EB);

  /// Left rail reserved for the block drag handle (every block shifts right).
  static const double gutter = 24.0;

  /// Pixel ratio formulas are rasterized at (capture and draw agree).
  static const double mathPixelRatio = 2.0;

  /// Font size formulas are rasterized at ([MathPreviewer]'s Math.tex).
  /// Inline atoms scale the raster by (text font size / this).
  static const double mathRasterFontSize = 18.0;

  /// Keep-off distance for the inline-math preview card (_paintMathPreview).
  static const double mathCardMargin = 8.0;

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
            return const TextStyle(
              color: text,
              fontSize: 30,
              height: 1.3,
              fontWeight: FontWeight.w700,
              letterSpacing: -0.5,
            );
          case 2:
            return const TextStyle(
              color: text,
              fontSize: 24,
              height: 1.35,
              fontWeight: FontWeight.w700,
              letterSpacing: -0.3,
            );
          case 3:
            return const TextStyle(
              color: text,
              fontSize: 20,
              height: 1.4,
              fontWeight: FontWeight.w600,
              letterSpacing: -0.2,
            );
          case 4:
            return const TextStyle(
              color: text,
              fontSize: 18,
              height: 1.45,
              fontWeight: FontWeight.w600,
              letterSpacing: -0.1,
            );
          case 5:
            return const TextStyle(
              color: text,
              fontSize: 16,
              height: 1.5,
              fontWeight: FontWeight.w600,
            );
          default: // H6 — smallest; a muted ink keeps it distinct from body.
            return const TextStyle(
              color: muted,
              fontSize: 15,
              height: 1.5,
              fontWeight: FontWeight.w600,
            );
        }
      case 'quote':
        // Upright, not italic: the left bar + muted ink already mark a quote
        // (Notion/GitHub style). Baking italic into the base style meant an
        // italic *mark* had nothing to toggle — you could never un-italic quoted
        // text — so emphasis is left entirely to marks (marks-over-plain-text).
        return const TextStyle(color: muted, fontSize: 16, height: 1.6);
      case 'footnote_def':
        // Small muted body, mirroring quote — the `[label]` marker is painted
        // in the gutter (see _paintNode), so the text itself stays plain.
        return const TextStyle(color: muted, fontSize: 13, height: 1.5);
      case 'code_block':
        return const TextStyle(
          color: text,
          fontSize: 14,
          height: 1.5,
          fontFamily: kMonoFont,
        );
      case 'math_block':
        return const TextStyle(
          color: Color(0xFF7C3AED),
          fontSize: 15,
          height: 1.6,
          fontFamily: kMonoFont,
          fontStyle: FontStyle.italic,
        );
      case 'todo':
        if (node.todoChecked) {
          return const TextStyle(
            color: faint,
            fontSize: 16,
            height: 1.65,
            decoration: TextDecoration.lineThrough,
          );
        }
        return const TextStyle(color: text, fontSize: 16, height: 1.65);
      default:
        return const TextStyle(color: text, fontSize: 16, height: 1.65);
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
      case 'footnote_def':
        // Room for the `[label]` marker painted in the gutter.
        return 34;
      case 'code_block':
        return codePadH;
      case 'math_block':
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
    if (_isList(kind) && _isList(prevKind)) return kind == prevKind ? 3 : 8;
    // Headings open a section, so give them air above — a touch less between two
    // consecutive headings (title + subtitle stay related).
    if (kind == 'heading') return prevKind == 'heading' ? 20 : 30;
    if (kind == 'code_block' || prevKind == 'code_block') return 16;
    return 13; // paragraph breathing room
  }
}

/// Per-node layout produced each [RenderDocument.performLayout].
class _NodeLayout {
  _NodeLayout(this.painter);

  final TextPainter painter;

  /// The atomic renderer that produced this layout, if any. Paint passes
  /// dispatch on this (not on [kind]): a math/mermaid block whose preview
  /// isn't ready falls through to the text pipeline and must be painted as
  /// text even though its kind says otherwise.
  AtomicBlockRenderer? renderedBy;

  /// Inline atoms folded into [painter]'s text, or null when the node shows
  /// plain source (no atom marks, node under the selection, rasters pending).
  /// Non-null means painter offsets differ from doc offsets: every crossing
  /// must go through [FoldPlan.docToPainter]/[FoldPlan.painterToDoc].
  FoldPlan? fold;

  double contentLeft = 0; // x where text starts
  double textTop = 0; // y of text top
  double textHeight = 0;
  double boxTop = 0; // y of the node's full box (incl. code padding)
  double boxHeight = 0;
  int ordinal = 0; // for numbered lists
  int indentLevel = 0; // list nesting depth (bullet glyph variants)
  Rect? checkbox; // todo checkbox rect (local), if any
  Rect? langLabel; // code-block language selector rect (local), if any
  Rect? askAiButton; // code-block Ask-AI button rect (local), if any
  Rect? copyButton; // code-block copy button rect (local), if any
  Rect? moreButton; // code-block ⋯ overflow-menu rect (local), if any
  Rect? viewCodeTab; // mermaid view switch: source tab (local), if any
  Rect? viewPreviewTab; // mermaid view switch: preview tab (local), if any
  String langText = ''; // resolved code language
  bool langAuto = false; // resolved by detection, not pinned by the author

  /// The language chip's text — just the resolved language name (e.g. `yaml`),
  /// no `auto ·` prefix. An auto block still re-detects live as content changes;
  /// the prefix was dropped for a cleaner chip (whether a block is pinned vs
  /// auto now lives in the ⋯ menu / language picker, not the chip face).
  String get langChipText {
    final t = langText.isEmpty ? 'text' : langText;
    return t;
  }
  String footnoteLabel = ''; // `[label]` gutter marker (kind == 'footnote_def')
  String nodeId = '';
  bool codeWrap = false; // whether this code block wraps
  double codeWidth = 0; // natural (unwrapped) text width, for code blocks
  double codeVisible = 0; // visible width of the code area
  Rect? scrollTrack; // horizontal scrollbar track (local), if code overflows
  bool showLineNums = false; // code block: draw a left line-number gutter
  double lineNumGutter = 0; // width reserved (inside contentLeft) for line #s
  String codeTitle = ''; // code block: bottom caption, '' = none
  Rect? titleRect; // code-block title caption rect (local), if any
  bool codeCollapsed = false; // code block: currently folded (only shows head)
  bool codeCanCollapse = false; // long enough to offer folding at all
  double codeClipH = 0; // painter height to clip to when folded
  Rect? collapseButton; // fold/unfold hit target (local), if any
  String kind = 'paragraph';
  bool todoChecked = false;
  int quoteDepth = 0; // blockquote nesting (bars on the left)
  ui.Image? mathImage; // rendered formula (math_block), if captured
  Size mathSize = Size.zero;
  bool quoteBreak = false; // a blank separated this quote from the previous
  double boxLeft = 0; // where the node's box begins (item-child inset)

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
  Rect?
  tableGridRect; // the grid alone (no top gutter / bottom bar) — selection hugs this

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
  _TableCell(
    this.rect,
    this.painter,
    this.header,
    this.row,
    this.col,
    this.textAt,
  );
  final Rect rect; // full cell rect (local)
  final TextPainter painter;
  final bool header;
  final int row;
  final int col;
  final Offset textAt; // where to paint the cell text (local)
}

/// What part of a table the pointer is over. Drives the hover-reveal of the
/// row/column handles and the add-row/add-column bars so each shows only for the
/// row/column you're actually pointing at (AppFlowy/AFFiNE-style), not the whole
/// table at once.
class _TableHover {
  const _TableHover({
    required this.node,
    this.row,
    this.col,
    this.onAddCol = false,
    this.onAddRow = false,
  });
  final int node;
  final int?
  row; // hovered cell's row (or the row whose left handle is under it)
  final int?
  col; // hovered cell's col (or the column whose top handle is under it)
  final bool onAddCol; // pointer sits on the add-column strip itself
  final bool onAddRow; // pointer sits on the add-row strip itself

  @override
  bool operator ==(Object other) =>
      other is _TableHover &&
      other.node == node &&
      other.row == row &&
      other.col == col &&
      other.onAddCol == onAddCol &&
      other.onAddRow == onAddRow;

  @override
  int get hashCode => Object.hash(node, row, col, onAddCol, onAddRow);
}

/// Which code-block toolbar icon the pointer is hovering.
enum _CodeIcon { none, lang, askAi, copy, more, viewCode, viewPreview }

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
    // Inline atoms fold unconditionally (a typeset formula is never entered —
    // click opens an editor, the caret snaps to its edges), so fold state does
    // not depend on the selection: a selection change genuinely only needs a
    // repaint. This is NOT a layout fast-path in practice, though — the `nodes`
    // setter runs first on every controller notification and relayouts the
    // whole document unconditionally (nodes is one mutated-in-place instance,
    // so it can't cheaply tell whether it changed). Measured cost is ~6ms for
    // 200 nodes, folding included; don't build on an assumption that caret
    // moves skip layout.
    markNeedsPaint();
  }

  List<RemoteCursor> _remoteCursors = const [];
  set remoteCursors(List<RemoteCursor> value) {
    if (listEquals(_remoteCursors, value)) return;
    _remoteCursors = value;
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

  /// Registered atomic-block renderers, dispatched by kind in the layout and
  /// paint passes. New block types register here (docs/render-architecture.md).
  static const List<AtomicBlockRenderer> atomicRenderers = [
    DividerRenderer(),
    ImageRenderer(),
    MathBlockRenderer(),
    MermaidRenderer(),
    TableRenderer(),
  ];

  static final Map<String, AtomicBlockRenderer> _renderersByKind = {
    for (final r in atomicRenderers) r.kind: r,
  };

  final List<_NodeLayout> _layouts = [];

  /// Per-code-block horizontal scroll offset, keyed by node id. Code blocks do
  /// not wrap; long lines scroll left/right within the block.
  final Map<String, double> _codeScroll = {};

  /// Per-block diagram preview zoom (ctrl+wheel) and pan (drag), keyed by
  /// node id. View state only — never written to the document.
  final Map<String, double> _previewZoom = {};
  final Map<String, Offset> _previewPan = {};

  /// The rendered-diagram block under [local], if any.
  int? diagramAt(Offset local) {
    for (var i = 0; i < _layouts.length; i++) {
      final l = _layouts[i];
      if (l.renderedBy is! MermaidRenderer) continue;
      if (local.dy >= l.boxTop && local.dy <= l.boxTop + l.boxHeight) return i;
    }
    return null;
  }

  /// Drag a diagram by [delta] inside its fixed viewport.
  void panPreviewBy(int node, Offset delta) {
    if (node < 0 || node >= _layouts.length) return;
    final id = _layouts[node].nodeId;
    _previewPan[id] = (_previewPan[id] ?? Offset.zero) + delta;
    markNeedsPaint();
  }

  /// Restore every diagram to its natural view (zoom and pan) — the page
  /// host calls this when a click lands outside the editor canvas entirely
  /// (the page margins), extending the click-outside reset below.
  void resetAllPreviewViews() {
    if (_previewZoom.isEmpty && _previewPan.isEmpty) return;
    _previewZoom.clear();
    _previewPan.clear();
    markNeedsLayout();
  }

  /// A click anywhere OUTSIDE the diagram blocks restores their natural
  /// view (zoom and pan). Walking the pointer away no longer resets — a
  /// zoomed/panned diagram stays put until the user clicks elsewhere.
  void resetPreviewViewsOutside(Offset local) {
    var changed = false;
    for (final l in _layouts) {
      if (l.renderedBy is! MermaidRenderer) continue;
      if (local.dy >= l.boxTop && local.dy <= l.boxTop + l.boxHeight) continue;
      if (_previewZoom.remove(l.nodeId) != null) changed = true;
      if (_previewPan.remove(l.nodeId) != null) changed = true;
    }
    if (changed) markNeedsLayout();
  }

  /// Ctrl+wheel / pinch over a rendered diagram: zoom it by [factor].
  /// Returns true when consumed.
  bool zoomPreviewBy(Offset local, double factor) {
    for (final l in _layouts) {
      if (l.renderedBy is! MermaidRenderer) continue;
      if (local.dy < l.boxTop || local.dy > l.boxTop + l.boxHeight) continue;
      final cur = _previewZoom[l.nodeId] ?? 1.0;
      _previewZoom[l.nodeId] = (cur * factor).clamp(0.3, 3.0);
      markNeedsLayout();
      return true;
    }
    return false;
  }

  /// Decoded images keyed by `file_id`, painted directly onto the canvas. The
  /// host (`MicaEditor`) populates this as images load and calls [setImages].
  Map<String, ui.Image> _images = {};
  set images(Map<String, ui.Image> value) {
    _images = value;
    markNeedsLayout();
  }

  /// Swap in a new frame of an animated image (GIF / animated WebP).
  ///
  /// Separate from [images] because every frame of an animation is the same
  /// size as the last, so the block's box never moves — going through the
  /// setter would relayout the whole document ten-plus times a second for a
  /// picture that hasn't changed shape. A frame that somehow *does* differ in
  /// size falls back to a relayout rather than painting into a stale box.
  void replaceImage(String key, ui.Image frame) {
    final old = _images[key];
    _images[key] = frame;
    if (old != null &&
        (old.width != frame.width || old.height != frame.height)) {
      markNeedsLayout();
    } else {
      markNeedsPaint();
    }
  }

  /// Called during paint for each image actually drawn. The host uses it to
  /// stop animating pictures nothing draws any more — the block was deleted, or
  /// its source replaced. See `_MicaEditorState._onImageFrame`.
  ///
  /// NOT an on-screen test: this render object paints the whole document, with
  /// no viewport culling, so a picture scrolled out of view still reports here
  /// and its animation keeps running. Hiding the window does stall it, but by a
  /// different route — the engine simply stops asking for frames.
  void Function(String key)? onImagePainted;

  /// Rasterized previews per previewer id ('math', 'mermaid', …), keyed by
  /// source. Fed by the host's RasterPreviewPipeline.
  Map<String, Map<String, ui.Image>> _previewImages = const {};
  set previewImages(Map<String, Map<String, ui.Image>> value) {
    _previewImages = value;
    markNeedsLayout();
  }

  /// See [DocumentSurface.previewBaselines]. No markNeedsLayout of its own:
  /// baselines land strictly alongside their images, and the images setter
  /// above already relayouts.
  Map<String, Map<String, double>> _previewBaselines = const {};
  set previewBaselines(Map<String, Map<String, double>> value) {
    _previewBaselines = value;
  }

  /// Ask the host pipeline for a preview of [source] under previewer [id],
  /// to be displayed [targetWidth] logical px wide.
  void Function(String id, String source, double targetWidth)? onRequestPreview;

  /// Which rectangular cell AREA of a table is block-selected (rows r0–r1 ×
  /// columns c0–c1, inclusive). Row select = one row × all columns; column
  /// select = all rows × one column; a cross-cell drag selects any rectangle
  /// (AFFiNE-style area selection). Kept separate from the document text
  /// selection so it highlights whole cells, never a cross-cell text range.
  ({int node, int r0, int c0, int r1, int c1})? _tableBlockSel;
  ({int node, int r0, int c0, int r1, int c1})? get tableBlockSelection =>
      _tableBlockSel;
  set tableBlockSelection(({int node, int r0, int c0, int r1, int c1})? value) {
    if (_tableBlockSel == value) return;
    _tableBlockSel = value;
    markNeedsPaint();
  }

  /// The cell of table [node] nearest [local] — the point is clamped into the
  /// grid first, so a drag that wanders past the table's edge still resolves to
  /// an edge cell (keeps a cross-cell area drag live outside the borders).
  ({int row, int col})? tableCellNear(int node, Offset local) {
    if (node < 0 || node >= _layouts.length) return null;
    final l = _layouts[node];
    if (l.kind != 'table' || l.tableCells.isEmpty) return null;
    final grid = l.tableGridRect;
    final p = grid == null
        ? local
        : Offset(
            local.dx.clamp(grid.left + 0.5, grid.right - 0.5),
            local.dy.clamp(grid.top + 0.5, grid.bottom - 0.5),
          );
    for (final cell in l.tableCells) {
      if (cell.rect.contains(p)) return (row: cell.row, col: cell.col);
    }
    // On a border line: fall back to the nearest cell center.
    _TableCell? best;
    var bestD = double.infinity;
    for (final cell in l.tableCells) {
      final d = (cell.rect.center - p).distanceSquared;
      if (d < bestD) {
        bestD = d;
        best = cell;
      }
    }
    return best == null ? null : (row: best.row, col: best.col);
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
          if (l.askAiButton?.contains(local) ?? false) {
            icon = _CodeIcon.askAi;
          } else if (l.copyButton?.contains(local) ?? false) {
            icon = _CodeIcon.copy;
          } else if (l.moreButton?.contains(local) ?? false) {
            icon = _CodeIcon.more;
          } else if (l.langLabel?.contains(local) ?? false) {
            icon = _CodeIcon.lang;
          } else if (l.viewCodeTab?.contains(local) ?? false) {
            icon = _CodeIcon.viewCode;
          } else if (l.viewPreviewTab?.contains(local) ?? false) {
            icon = _CodeIcon.viewPreview;
          }
          break;
        }
      }
    }
    final border = local == null ? null : tableColBorderAt(local);
    int? block;
    if (local != null) {
      for (var i = 0; i < _layouts.length; i++) {
        final l = _layouts[i];
        if (local.dy >= l.boxTop && local.dy < l.boxTop + l.boxHeight) {
          block = i;
          break;
        }
      }
    }
    // Which table cell / row-column handle / add-strip the pointer is over —
    // so each affordance reveals only for the row/column being pointed at.
    _TableHover? tableHover;
    if (local != null) {
      final cell = tableCellAt(local);
      final rowH = tableRowHandleAt(local);
      final colH = tableColHandleAt(local);
      final add = tableAddAt(local);
      final tnode = cell?.node ?? rowH?.node ?? colH?.node ?? add?.node;
      if (tnode != null) {
        tableHover = _TableHover(
          node: tnode,
          row: cell?.row ?? rowH?.row,
          col: cell?.col ?? colH?.col,
          onAddCol: add?.column == true,
          onAddRow: add != null && !add.column,
        );
      }
    }

    if (node != _hoverCode ||
        icon != _hoverIcon ||
        image != _hoverImage ||
        block != _hoverBlock ||
        tableHover != _hoverTable ||
        border?.node != _hoverColBorder?.node ||
        border?.col != _hoverColBorder?.col) {
      _hoverCode = node;
      _hoverIcon = icon;
      _hoverImage = image;
      _hoverBlock = block;
      _hoverTable = tableHover;
      _hoverColBorder = border;
      markNeedsPaint();
    }
  }

  ({int node, int col})? _hoverColBorder;
  _TableHover? _hoverTable; // which cell/row/col/edge of a table is hovered
  int? _hoverBlock; // block under the pointer (shows the drag handle)
  int? _dropIndex; // insertion index while a block drag is live

  /// The grab rect of block [i]'s drag handle (gutter rail, first line).
  Rect _handleRectFor(int i) {
    final l = _layouts[i];
    final cy =
        l.painter.text != null && l.kind != 'divider' && l.kind != 'image'
        ? l.textTop + l.painter.preferredLineHeight * 0.5
        : l.boxTop + (l.boxHeight < 40 ? l.boxHeight / 2 : 20.0);
    return Rect.fromCenter(
      center: Offset(l.boxLeft - 12, cy),
      width: 18,
      height: 22,
    );
  }

  /// The block whose box contains [local], if any.
  int? blockAt(Offset local) {
    for (var i = 0; i < _layouts.length; i++) {
      final l = _layouts[i];
      if (local.dy >= l.boxTop && local.dy < l.boxTop + l.boxHeight) {
        return i;
      }
    }
    return null;
  }

  /// The typeset inline formula under [local] — its node, source range and
  /// LaTeX — or null. A click on one opens the editor (formulas are atoms; you
  /// don't edit them in place).
  ({int node, int start, int end, String source})? inlineMathAt(Offset local) {
    for (var i = 0; i < _layouts.length; i++) {
      final l = _layouts[i];
      final fold = l.fold;
      if (fold == null) continue;
      final origin = Offset(l.contentLeft, l.textTop);
      for (final a in fold.atoms) {
        if (a.renderer is! MathInlineAtomRenderer) continue;
        if (a.rect.shift(origin).contains(local)) {
          return (node: i, start: a.docStart, end: a.docEnd, source: a.source);
        }
      }
    }
    return null;
  }

  /// The block whose drag handle sits under [local], if any.
  int? dragHandleAt(Offset local) {
    final h = _hoverBlock;
    if (h == null || h >= _layouts.length) return null;
    return _handleRectFor(h).inflate(2).contains(local) ? h : null;
  }

  /// Insertion index (0..nodes.length) for a drop at height [dy].
  int dropIndexAt(double dy) {
    for (var i = 0; i < _layouts.length; i++) {
      final l = _layouts[i];
      if (dy < l.boxTop + l.boxHeight / 2) return i;
    }
    return _layouts.length;
  }

  int? get dropIndex => _dropIndex;

  void setDropIndicator(int? index) {
    if (_dropIndex == index) return;
    _dropIndex = index;
    markNeedsPaint();
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
    final maxWidth = constraints.maxWidth.isFinite
        ? constraints.maxWidth
        : 600.0;
    for (final l in _layouts) {
      l.painter.dispose();
      for (final cell in l.tableCells) {
        cell.painter.dispose();
      }
    }
    _layouts.clear();

    final sel = _selection;
    final caretKey =
        (sel != null && sel.isCollapsed && sel.focus.node < _nodes.length)
        ? '${_nodes[sel.focus.node].id}:${sel.focus.offset}'
        : '';
    final caretMoved = caretKey != _lastCaretKey;
    _lastCaretKey = caretKey;

    double y = 0;
    String? prevKind;
    final numberedCounters = <int>[];
    for (var nodeIndex = 0; nodeIndex < _nodes.length; nodeIndex++) {
      final node = _nodes[nodeIndex];
      y += EditorTheme.gapAbove(node.kind, prevKind);

      // Atomic blocks dispatch to their registered renderer; a null return
      // (math waiting on its raster, empty source) falls through to the text
      // pipeline so the source stays visible and editable.
      final renderer = _renderersByKind[node.kind];
      if (renderer != null) {
        final layout = renderer.layout(this, node, nodeIndex, y, maxWidth);
        if (layout != null) {
          layout.renderedBy = renderer;
          _layouts.add(layout);
          y += layout.boxHeight;
          prevKind = node.kind;
          continue;
        }
      }

      final style = _appearance.applyTo(
        EditorTheme.styleFor(node),
        isCode: node.isCode,
      );
      // Item-child blocks (`data.li`) align under the owning item's text;
      // quoted blocks (`data.quote`) inset 16px per depth for the bars (the
      // `quote` kind's own leadingInset already covers the first bar).
      final liInset = node.liLevel != null ? 26.0 + 24.0 * node.liLevel! : 0.0;
      final quoteExtra =
          (16.0 * node.quoteDepth - (node.kind == 'quote' ? 16.0 : 0.0)).clamp(
            0.0,
            double.infinity,
          );
      final isCode = node.isCode;
      // A line-number gutter widens contentLeft (code shifts right; numbers are
      // painted in the reserved strip). Width scales with the digit count so a
      // 1000-line block still lines up. SiYuan-style per-block toggle.
      final showLineNums = isCode && node.data['lineNumbers'] == true;
      final lineNumGutter = showLineNums
          ? (node.text.split('\n').length.toString().length.clamp(1, 9) * 7.5 +
                12.0)
          : 0.0;
      final contentLeft =
          EditorTheme.gutter +
          EditorTheme.leadingInset(node.kind) +
          (node.isListKind ? 24.0 * node.indent : 0.0) +
          liInset +
          quoteExtra +
          lineNumGutter;
      final textWidth =
          (maxWidth - contentLeft - (isCode ? EditorTheme.codePadH : 0)).clamp(
            0.0,
            double.infinity,
          );

      final String? pinnedLang = isCode
          ? canonicalCodeLanguage((node.data['language'] as String?) ?? '')
          : null;
      final String? codeLang = isCode
          ? resolveCodeLanguage(node.text, node.data['language'] as String?)
          : null;
      final marks = isCode ? const <Mark>[] : marksFromData(node.data);

      // Inline atoms (math): marked runs render as one typeset object,
      // unconditionally. A formula is an atom — the caret never lands inside its
      // source (the controller snaps selections out of the run and click opens
      // an editor), so folding does not depend on where the selection is, and
      // the doc↔painter map only ever sees offsets on the run's edges.
      FoldPlan? fold;
      if (!isCode && marks.isNotEmpty) {
        final atoms = <InlineAtom>[];
        for (final m in marks) {
          final r = _inlineAtomRenderers[m.type];
          if (r == null) continue;
          final s = m.start.clamp(0, node.text.length);
          final e = m.end.clamp(0, node.text.length);
          if (e <= s) continue;
          final source = node.text.substring(s, e);
          final measured = r.measure(
            this,
            source,
            (style.fontSize ?? 16) * _appearance.fontScale,
            textWidth,
          );
          if (measured == null) continue; // declined: run stays styled source
          // A link wrapping the formula keeps its underline across the atom.
          final underline = marks.any(
            (l) => l.type == 'link' && l.start <= s && l.end >= e,
          );
          atoms.add(
            InlineAtom(
              docStart: s,
              docEnd: e,
              source: source,
              size: measured.size,
              baseline: measured.baseline,
              renderer: r,
              underline: underline,
            ),
          );
        }
        // The parser emits sorted, disjoint runs; concurrent-edit clamping can
        // break that, and an overlapping pair would corrupt the offset map —
        // decline the whole fold instead.
        atoms.sort((a, b) => a.docStart.compareTo(b.docStart));
        var disjoint = atoms.isNotEmpty;
        for (var k = 1; k < atoms.length; k++) {
          if (atoms[k].docStart < atoms[k - 1].docEnd) {
            disjoint = false;
            break;
          }
        }
        if (disjoint) fold = FoldPlan(atoms);
      }

      TextSpan span;
      List<PlaceholderDimensions>? dims;
      if (isCode && node.text.isNotEmpty) {
        span = buildCodeSpan(node.text, codeLang!, style);
      } else if (fold != null) {
        final folded = buildFoldedSpan(node.text, marks, style, fold.atoms);
        span = folded.span;
        dims = folded.dims;
      } else {
        span = buildMarkedSpan(node.text, marks, style);
      }

      final codeWrap = isCode && node.data['wrap'] == true;
      var painter = TextPainter(
        text: span,
        textDirection: TextDirection.ltr,
        textWidthBasis: TextWidthBasis.parent,
      );
      if (dims != null) painter.setPlaceholderDimensions(dims);
      painter.layout(
        maxWidth: (isCode && !codeWrap) ? double.infinity : textWidth,
      );
      if (fold != null) {
        final boxes = painter.inlinePlaceholderBoxes ?? const <ui.TextBox>[];
        if (boxes.length == fold.atoms.length) {
          for (var k = 0; k < boxes.length; k++) {
            fold.atoms[k].rect = boxes[k].toRect();
          }
        } else {
          // The engine dropped a placeholder (never observed — probed on this
          // Flutter). A folded painter without its geometry would misplace
          // every offset in the node; rebuild unfolded instead.
          fold = null;
          painter.dispose();
          painter = TextPainter(
            text: buildMarkedSpan(node.text, marks, style),
            textDirection: TextDirection.ltr,
            textWidthBasis: TextWidthBasis.parent,
          )..layout(maxWidth: textWidth);
        }
      }

      final layout = _NodeLayout(painter)
        ..fold = fold
        ..contentLeft = contentLeft
        ..kind = node.kind
        ..nodeId = node.id
        ..quoteDepth = node.quoteDepth
        ..quoteBreak = node.data['qbreak'] == true
        ..boxLeft = EditorTheme.gutter + liInset
        ..todoChecked = node.todoChecked
        ..langText = codeLang ?? ''
        ..langAuto = isCode && (pinnedLang!.isEmpty || pinnedLang == 'auto')
        ..footnoteLabel = node.kind == 'footnote_def'
            ? (node.data['label'] as String? ?? '')
            : ''
        ..codeWrap = codeWrap
        ..codeWidth = isCode ? painter.width : 0
        ..showLineNums = showLineNums
        ..lineNumGutter = lineNumGutter
        ..codeTitle = isCode ? (node.data['title'] as String? ?? '') : '';

      // Code blocks pad symmetrically; their controls float at the top-right on
      // hover (no reserved top toolbar row). The optional title caption is added
      // AFTER the scrollbar below (so it sits under the code + scrollbar).
      // A block past the threshold auto-folds (data.collapsed absent) to its
      // first few lines; data.collapsed false/true is the user's override.
      final codeLineCount = isCode ? node.text.split('\n').length : 0;
      final canCollapse = isCode && codeLineCount > _codeCollapseThreshold;
      final collapsed =
          canCollapse && ((node.data['collapsed'] as bool?) ?? true);
      final clipH = collapsed
          ? _codeCollapsedLines * painter.preferredLineHeight
          : painter.height;
      layout.codeCanCollapse = canCollapse;
      layout.codeCollapsed = collapsed;
      layout.codeClipH = clipH;

      final innerTop = isCode ? EditorTheme.codePadV : 0.0;
      layout.boxTop = y;
      layout.textTop = y + innerTop;
      layout.textHeight = clipH;
      layout.boxHeight = clipH + (isCode ? 2 * EditorTheme.codePadV : 0);

      if (node.isListKind) {
        final level = node.indent;
        layout.indentLevel = level;
        // Per-level ordered counters: deeper levels reset when we go back
        // up; a bullet/todo at a level resets numbering at that level and
        // below, without touching parent counters.
        if (numberedCounters.length > level + 1) {
          numberedCounters.removeRange(level + 1, numberedCounters.length);
        }
        if (node.kind == 'numbered_list') {
          while (numberedCounters.length <= level) {
            numberedCounters.add(0);
          }
          // An imported `data.start` pins the number — a list interrupted
          // by bullets/code/quotes resumes (`<ol start>` semantics) instead
          // of restarting at 1.
          final start = (node.data['start'] as num?)?.toInt();
          numberedCounters[level] = start ?? (numberedCounters[level] + 1);
          layout.ordinal = numberedCounters[level];
        } else if (numberedCounters.length > level) {
          numberedCounters.removeRange(level, numberedCounters.length);
        }
      } else if (node.liLevel == null) {
        // Container children (`data.li`) live INSIDE an item — they must
        // not reset the surrounding list's numbering.
        numberedCounters.clear();
      }
      if (node.kind == 'todo') {
        final lh = painter.preferredLineHeight;
        const box = 18.0;
        layout.checkbox = Rect.fromLTWH(
          EditorTheme.gutter + 2,
          layout.textTop + (lh - box) / 2,
          box,
          box,
        );
      }
      if (isCode) {
        // Keep the caret visible by auto-scrolling the code horizontally — but
        // only when the caret actually moved, so blink/re-layout or a manual
        // scrollbar drag is never yanked back to the caret.
        // The current node's index is `_layouts.length` (not yet appended).
        final visible = (maxWidth - contentLeft - EditorTheme.codePadH).clamp(
          0.0,
          double.infinity,
        );
        var scroll = _codeScroll[node.id] ?? 0;
        if (caretMoved &&
            sel != null &&
            sel.isCollapsed &&
            sel.focus.node == _layouts.length) {
          final caretX = painter
              .getOffsetForCaret(
                TextPosition(
                  offset: sel.focus.offset.clamp(0, node.text.length),
                ),
                Rect.zero,
              )
              .dx;
          if (caretX - scroll > visible - 12) scroll = caretX - visible + 12;
          if (caretX - scroll < 0) scroll = caretX;
        }
        final maxScroll = (layout.codeWidth - visible).clamp(
          0.0,
          double.infinity,
        );
        _codeScroll[node.id] = scroll.clamp(0.0, maxScroll);

        layout.codeVisible = visible;
        // A folded block clips to its head — no horizontal scrollbar there.
        if (!collapsed && maxScroll > 0) {
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

        // Fold/expand affordance strip (below the code + scrollbar) for any
        // block long enough to fold.
        if (canCollapse) {
          layout.collapseButton = Rect.fromLTWH(
            layout.boxLeft,
            layout.boxTop + layout.boxHeight,
            maxWidth - layout.boxLeft,
            _codeFoldBarH,
          );
          layout.boxHeight += _codeFoldBarH;
        }

        // Bottom caption row, below the code (and the scrollbar, if any).
        if (layout.codeTitle.isNotEmpty) {
          final titleLeft = contentLeft - lineNumGutter;
          layout.titleRect = Rect.fromLTWH(
            titleLeft,
            layout.boxTop + layout.boxHeight,
            (maxWidth - titleLeft - EditorTheme.codePadH).clamp(
              0.0,
              double.infinity,
            ),
            _codeTitleH,
          );
          layout.boxHeight += _codeTitleH;
        }

        // Controls float at the TOP-right on hover (GitHub/VSCode/Notion-style),
        // right-aligned: [language chip] [copy] [⋯ more]. Trimmed to the common
        // actions — wrap / line-numbers / title / collapse / delete live behind
        // the ⋯ menu. Overlaid, not a reserved row, so a resting block shows no
        // empty toolbar strip (04834a1); the overlay sits over the first line's
        // right edge (nearly always slack).
        const iconBox = 22.0;
        final marker = TextPainter(
          text: TextSpan(
            text: '${layout.langChipText}  ▾',
            style: const TextStyle(fontSize: 11, color: EditorTheme.muted),
          ),
          textDirection: TextDirection.ltr,
        )..layout();
        final labelW = marker.width + 14;
        final labelH = marker.height + 6;
        marker.dispose();
        final iconY = layout.boxTop + 4;
        layout.moreButton = Rect.fromLTWH(
          maxWidth - iconBox - 8,
          iconY,
          iconBox,
          iconBox,
        );
        layout.copyButton = Rect.fromLTWH(
          maxWidth - 2 * iconBox - 12,
          iconY,
          iconBox,
          iconBox,
        );
        // Ask AI stays on the toolbar (a primary action, per the design), left
        // of copy. The editor no-ops the tap when AI isn't configured.
        layout.askAiButton = Rect.fromLTWH(
          maxWidth - 3 * iconBox - 16,
          iconY,
          iconBox,
          iconBox,
        );
        layout.langLabel = Rect.fromLTWH(
          maxWidth - 3 * iconBox - 16 - labelW - 6,
          iconY + (iconBox - labelH) / 2,
          labelW,
          labelH,
        );
        // Diagram blocks: the [code|preview] switch joins the bottom-right
        // toolbar, left of the language label — the top-left overlapped the
        // first line of source. Same corner on both forms.
        if (codeLang == 'mermaid') {
          final tabY = iconY + (iconBox - 20) / 2;
          final right = layout.langLabel!.left - 8;
          layout.viewCodeTab = Rect.fromLTWH(right - 44, tabY, 44, 20);
          layout.viewPreviewTab = Rect.fromLTWH(
            right - 44 - 2 - 56,
            tabY,
            56,
            20,
          );
        }
      }

      _layouts.add(layout);
      y += layout.boxHeight;
      prevKind = node.kind;
    }

    y += EditorTheme.bottomPad;
    size = constraints.constrain(
      Size(
        maxWidth,
        y < EditorTheme.minSurfaceHeight ? EditorTheme.minSurfaceHeight : y,
      ),
    );
  }

  // The grid spans the full content width (aligned with body text on both
  // sides); handles/add-buttons overlay the edges on hover instead of taking
  // horizontal space. Only vertical space is reserved (top gutter for column
  // handles, bottom bar for add-row).
  static const double _tTopGutter = 10;
  static const double _tBottomBar = 16;
  static const double _tEdge = 16; // overlay handle thickness

  // Vertical box reserved for a divider (horizontal rule centered within it).
  static const double _dividerHeight = 26;

  // Height of a code block's optional bottom caption row.
  static const double _codeTitleH = 24;

  // Auto-fold a code block past this many lines (AFFiNE-style); when folded it
  // shows the first _codeCollapsedLines with a fade + expand affordance.
  static const int _codeCollapseThreshold = 20;
  static const int _codeCollapsedLines = 10;
  static const double _codeFoldBarH = 26; // fold/expand affordance strip

  // Image placeholder size (before the real image decodes) and vertical gap.
  static const double _imagePlaceholderH = 180;
  static const double _imageGap = 6;

  /// A table cell's display span. Cells store raw inline-Markdown source
  /// (`` `code` ``, `**bold**`, …); painting shows the rendered form while the
  /// overlay editor keeps showing the source (Typora-style). Parse failures
  /// can't happen — unmatched markers simply stay literal text.
  static TextSpan cellDisplaySpan(String raw, TextStyle base) {
    if (raw.isEmpty) return TextSpan(text: ' ', style: base);
    final parsed = parseInline(raw);
    return buildMarkedSpan(parsed.text, parsed.marks, base);
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
    _paintInlineCode(canvas, offset);
    _paintSelection(canvas, offset);
    for (var i = 0; i < _layouts.length; i++) {
      _paintNode(canvas, offset, i);
    }
    for (var i = 0; i < _layouts.length; i++) {
      _layouts[i].renderedBy?.paintOverlay(
        this,
        canvas,
        offset,
        _layouts[i],
        i,
      );
    }
    _paintAtomicSelection(canvas, offset);
    _paintScrollbars(canvas, offset);
    _paintCaret(canvas, offset);
    _paintRemoteCursors(canvas, offset);
    // Last: it floats over everything, including the caret it belongs to.
    _paintMathPreview(canvas, offset);
  }

  /// The typeset formula for the math run the caret is inside, floated above it.
  ///
  /// Deliberately NOT inline typesetting. The LaTeX stays exactly where it is —
  /// in the buffer, editable, and every offset in this file goes on meaning what
  /// it has always meant. Putting the formula *in* the line would need a
  /// doc↔painter offset mapping through the text pipeline, and
  /// docs/render-architecture.md calls that pipeline the load-bearing wall. The
  /// need this answers is "did I write that formula right?", not "make my notes
  /// pretty", and a card answers it for ~1% of the cost.
  ///
  /// It also keeps the `$` misfires loud: `--master_addr=$MASTER_ADDR` still
  /// reads as purple source, because nothing about the line is rewritten.
  ///
  /// Reuses the block previewer whole — same 'math' id, same source-keyed cache,
  /// same 18pt raster. A popup wants that size anyway, which is why this needs
  /// no previewer of its own and no font-size in the cache key.
  void _paintMathPreview(Canvas canvas, Offset offset) {
    final sel = _selection;
    // Only a resting caret. A drag is selecting text, not asking to look.
    if (sel == null || !sel.isCollapsed) return;
    final i = sel.focus.node;
    if (i < 0 || i >= _layouts.length || i >= _nodes.length) return;
    final l = _layouts[i];
    if (EditorNode.isAtomicKind(l.kind) ||
        l.kind == 'code_block' ||
        l.renderedBy != null) {
      return;
    }
    final node = _nodes[i];
    final len = node.text.length;
    final caret = sel.focus.offset.clamp(0, len);

    final run = mathRunAt(marksFromData(node.data), caret);
    if (run == null) return;
    final s = run.start.clamp(0, len);
    final e = run.end.clamp(0, len);
    if (e <= s) return;
    final source = node.text.substring(s, e);
    if (source.trim().isEmpty) return;

    final img = _previewImages['math']?[source];
    if (img == null) {
      // Safe from inside paint: request() only registers and schedules — every
      // rebuild it causes is behind an addPostFrameCallback.
      onRequestPreview?.call('math', source, 0);
      return; // the card appears on the frame the raster lands
    }

    final boxes = l.painter.getBoxesForSelection(
      TextSelection(baseOffset: s, extentOffset: e),
      boxHeightStyle: ui.BoxHeightStyle.tight,
    );
    if (boxes.isEmpty) return;
    final anchor = boxes.first.toRect().shift(
      offset + Offset(l.contentLeft, l.textTop),
    );

    const dpr = EditorTheme.mathPixelRatio;
    var w = img.width / dpr;
    var h = img.height / dpr;
    const pad = 8.0;
    // A long formula shrinks to fit rather than running off the pane.
    final avail = size.width - 2 * EditorTheme.mathCardMargin - 2 * pad;
    if (w > avail && w > 0) {
      h *= avail / w;
      w = avail;
    }

    final cardW = w + 2 * pad;
    final cardH = h + 2 * pad;
    // Above the run, nudged onto the pane; below it when there is no room up
    // there (the first line of a document).
    var left = anchor.left - pad;
    left = left.clamp(
      EditorTheme.mathCardMargin,
      (size.width - EditorTheme.mathCardMargin - cardW).clamp(
        EditorTheme.mathCardMargin,
        double.infinity,
      ),
    );
    var top = anchor.top - cardH - 6;
    if (top < offset.dy + EditorTheme.mathCardMargin) top = anchor.bottom + 6;
    final card = Rect.fromLTWH(left, top, cardW, cardH);
    final rr = RRect.fromRectAndRadius(card, const Radius.circular(6));

    canvas.drawRRect(
      rr.shift(const Offset(0, 1)),
      Paint()
        ..color = const Color(0x1A000000)
        ..maskFilter = const MaskFilter.blur(BlurStyle.normal, 4),
    );
    canvas.drawRRect(rr, Paint()..color = const Color(0xFFFFFFFF));
    canvas.drawRRect(
      rr,
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1
        ..color = const Color(0xFFE2E8F0),
    );
    canvas.drawImageRect(
      img,
      Rect.fromLTWH(0, 0, img.width.toDouble(), img.height.toDouble()),
      Rect.fromLTWH(card.left + pad, card.top + pad, w, h),
      Paint()..filterQuality = FilterQuality.medium,
    );
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
    for (
      var i = sel.start.node;
      i <= sel.end.node && i < _layouts.length;
      i++
    ) {
      if (_isAtomicNode(i) && (i != sel.start.node || i != sel.end.node)) {
        _drawAtomicHighlight(canvas, offset, i, border: false);
      }
    }
  }

  void _drawAtomicHighlight(
    Canvas canvas,
    Offset offset,
    int i, {
    required bool border,
  }) {
    final l = _layouts[i];
    // A selected table hugs the grid exactly — no tint in the top gutter (column
    // handles) or the bottom add-row bar, and no rounded spill past the outer
    // lines. The highlight lands between the grid's top and bottom lines.
    if (l.kind == 'table' && l.tableGridRect != null) {
      final box = l.tableGridRect!.shift(offset);
      canvas.drawRect(box, Paint()..color = EditorTheme.selection);
      if (border) {
        canvas.drawRect(
          box,
          Paint()
            ..color = EditorTheme.caret
            ..style = PaintingStyle.stroke
            ..strokeWidth = 2,
        );
      }
      return;
    }
    // Start at the block's own left edge — drawing from x=0 spilled the
    // tint into the drag-handle gutter, left of the page's text column.
    final box = (l.kind == 'image' && l.imageDst != null)
        ? l.imageDst!.shift(offset)
        : Rect.fromLTWH(
            offset.dx + l.boxLeft,
            offset.dy + l.boxTop,
            size.width - l.boxLeft,
            l.boxHeight,
          );
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
      // Atomic-block backdrops dispatch by kind: a block's identity tint
      // (math's lavender) shows on its fallen-through source form too.
      _renderersByKind[l.kind]?.paintBackground(this, canvas, offset, l, i);
      if (l.kind == 'code_block') {
        final bgLeft = l.boxLeft + 16.0 * l.quoteDepth;
        // The title caption sits OUTSIDE the panel (centered below it), so the
        // background stops short of the title row.
        final bgHeight =
            l.boxHeight - (l.titleRect != null ? _codeTitleH : 0.0);
        canvas.drawRRect(
          RRect.fromRectAndRadius(
            Rect.fromLTWH(
              offset.dx + bgLeft,
              offset.dy + l.boxTop,
              size.width - bgLeft,
              bgHeight,
            ),
            const Radius.circular(6),
          ),
          Paint()..color = EditorTheme.codeBg,
        );
      }
      // Blockquote bars: one per nesting depth, on any quoted block kind.
      // Within one quote group the bars run continuously across the gaps
      // between blocks (a `qbreak` starts a fresh group).
      final prev = i > 0 ? _layouts[i - 1] : null;
      for (var k = 0; k < l.quoteDepth; k++) {
        var top = l.boxTop;
        if (!l.quoteBreak &&
            prev != null &&
            prev.quoteDepth > k &&
            prev.boxLeft == l.boxLeft) {
          top = prev.boxTop + prev.boxHeight; // bridge the gap above
        }
        canvas.drawRect(
          Rect.fromLTWH(
            offset.dx + l.boxLeft + 2 + 16.0 * k,
            offset.dy + top,
            3,
            l.boxHeight + (l.boxTop - top),
          ),
          Paint()..color = EditorTheme.quoteBar,
        );
      }
      // Toolbar (language left, wrap + copy right) — only while hovering the block.
      if (l.kind == 'code_block' && _hoverCode == i) {
        _paintCodeToolbar(canvas, offset, l);
      }
    }
    // Drag handle (⠿) on the hovered block's gutter rail.
    final hover = _hoverBlock;
    if (hover != null && hover < _layouts.length) {
      final r = _handleRectFor(hover).shift(offset);
      final paint = Paint()..color = EditorTheme.faint;
      for (var row = 0; row < 3; row++) {
        for (var col = 0; col < 2; col++) {
          canvas.drawCircle(
            Offset(r.center.dx - 3 + col * 6.0, r.center.dy - 6 + row * 6.0),
            1.4,
            paint,
          );
        }
      }
    }
    // Drop indicator while a block drag is live.
    final drop = _dropIndex;
    if (drop != null && _layouts.isNotEmpty) {
      final y = drop < _layouts.length
          ? _layouts[drop].boxTop - 2
          : _layouts.last.boxTop + _layouts.last.boxHeight + 2;
      canvas.drawRect(
        Rect.fromLTWH(
          offset.dx + EditorTheme.gutter,
          offset.dy + y,
          size.width - EditorTheme.gutter,
          2.5,
        ),
        Paint()..color = EditorTheme.dropLine,
      );
    }
  }

  /// Line numbers in the reserved strip left of the code (contentLeft already
  /// includes [_NodeLayout.lineNumGutter]). Fixed horizontally — they don't
  /// scroll with the code. Only a logical line's FIRST visual row gets a number;
  /// a soft-wrapped line's continuation rows stay blank (SiYuan-style).
  void _paintLineNumbers(Canvas canvas, Offset origin, _NodeLayout l) {
    final metrics = l.painter.computeLineMetrics();
    if (metrics.isEmpty) return;
    final text = l.painter.text?.toPlainText() ?? '';
    final gutterRight = origin.dx - 4; // right-align 4px before the code text
    var lineNo = 0;
    for (final m in metrics) {
      final rowTopLocal = m.baseline - m.ascent;
      final pos = l.painter.getPositionForOffset(Offset(0, rowTopLocal + 1));
      final off = pos.offset;
      final isLogicalStart =
          off == 0 || (off > 0 && off <= text.length && text[off - 1] == '\n');
      if (!isLogicalStart) continue;
      lineNo++;
      final tp = TextPainter(
        text: TextSpan(
          text: '$lineNo',
          style: const TextStyle(
            fontSize: 11,
            height: 1.0,
            color: Color(0xFF94A3B8),
            fontFamily: kMonoFont,
          ),
        ),
        textDirection: TextDirection.ltr,
      )..layout();
      tp.paint(
        canvas,
        Offset(
          gutterRight - tp.width,
          origin.dy + rowTopLocal + (m.height - tp.height) / 2,
        ),
      );
      tp.dispose();
    }
  }

  /// The fold/expand affordance strip (centered) for a foldable code block.
  void _paintFoldBar(Canvas canvas, Offset offset, _NodeLayout l) {
    final r = l.collapseButton!.shift(offset);
    final label = l.codeCollapsed ? '⌄  Expand' : '⌃  Collapse';
    final tp = TextPainter(
      text: TextSpan(
        text: label,
        style: const TextStyle(
          fontSize: 12,
          color: EditorTheme.muted,
          fontWeight: FontWeight.w500,
        ),
      ),
      textDirection: TextDirection.ltr,
    )..layout();
    tp.paint(
      canvas,
      Offset(r.center.dx - tp.width / 2, r.center.dy - tp.height / 2),
    );
    tp.dispose();
  }

  /// The code block's caption — centered in the strip just BELOW the panel
  /// (AFFiNE-style), muted.
  void _paintCodeTitle(Canvas canvas, Offset offset, _NodeLayout l) {
    final r = l.titleRect!.shift(offset);
    final tp = TextPainter(
      text: TextSpan(
        text: l.codeTitle,
        style: const TextStyle(fontSize: 12, color: EditorTheme.muted),
      ),
      textDirection: TextDirection.ltr,
      maxLines: 1,
      ellipsis: '…',
    )..layout(maxWidth: r.width);
    tp.paint(
      canvas,
      Offset(
        r.left + (r.width - tp.width) / 2, // centered horizontally
        r.top + (r.height - tp.height) / 2,
      ),
    );
    tp.dispose();
  }

  void _paintCodeToolbar(Canvas canvas, Offset offset, _NodeLayout l) {
    final label = l.langLabel;
    if (label != null) {
      final r = label.shift(offset);
      canvas.drawRRect(
        RRect.fromRectAndRadius(r, const Radius.circular(5)),
        Paint()
          ..color = _hoverIcon == _CodeIcon.lang
              ? const Color(0xFFCBD5E1)
              : const Color(0xFFE2E8F0),
      );
      final marker = TextPainter(
        text: TextSpan(
          text: '${l.langChipText}  ▾',
          style: const TextStyle(fontSize: 11, color: EditorTheme.muted),
        ),
        textDirection: TextDirection.ltr,
      )..layout();
      marker.paint(canvas, r.topLeft + const Offset(7, 3));
      marker.dispose();
    }

    final askAi = l.askAiButton;
    if (askAi != null) {
      _paintIconButton(
        canvas,
        askAi.shift(offset),
        Icons.auto_awesome,
        hovered: _hoverIcon == _CodeIcon.askAi,
        active: false,
        tooltip: 'Ask AI',
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

    final more = l.moreButton;
    if (more != null) {
      _paintIconButton(
        canvas,
        more.shift(offset),
        Icons.more_horiz,
        hovered: _hoverIcon == _CodeIcon.more,
        active: false,
        tooltip: 'More',
      );
    }

    _paintViewTabs(canvas, offset, l, active: 'code');
  }

  /// The [code|preview] switch shown on diagram blocks while hovered — the
  /// same control on both forms, with the current form's tab highlighted.
  void _paintViewTabs(
    Canvas canvas,
    Offset offset,
    _NodeLayout l, {
    required String active,
  }) {
    void tab(Rect? r0, String text, _CodeIcon icon, bool isActive) {
      if (r0 == null) return;
      final r = r0.shift(offset);
      canvas.drawRRect(
        RRect.fromRectAndRadius(r, const Radius.circular(5)),
        Paint()
          ..color = isActive
              ? const Color(0xFF1E293B)
              : (_hoverIcon == icon
                    ? const Color(0xFFCBD5E1)
                    : const Color(0xFFE2E8F0)),
      );
      final tp = TextPainter(
        text: TextSpan(
          text: text,
          style: TextStyle(
            fontSize: 11,
            color: isActive ? const Color(0xFFF8FAFC) : EditorTheme.muted,
          ),
        ),
        textDirection: TextDirection.ltr,
      )..layout();
      tp.paint(canvas, r.center - Offset(tp.width / 2, tp.height / 2));
      tp.dispose();
    }

    tab(
      l.viewPreviewTab,
      'preview',
      _CodeIcon.viewPreview,
      active == 'preview',
    );
    tab(l.viewCodeTab, 'code', _CodeIcon.viewCode, active == 'code');
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
      rect.topLeft +
          Offset(
            (rect.width - glyph.width) / 2,
            (rect.height - glyph.height) / 2,
          ),
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

  /// Node index whose code Ask-AI button contains [local], or null.
  int? codeAskAiAt(Offset local) {
    for (var i = 0; i < _layouts.length; i++) {
      if (_layouts[i].askAiButton?.contains(local) ?? false) return i;
    }
    return null;
  }

  /// Node index whose code-block title caption contains [local], or null.
  int? codeTitleAt(Offset local) {
    for (var i = 0; i < _layouts.length; i++) {
      if (_layouts[i].titleRect?.contains(local) ?? false) return i;
    }
    return null;
  }

  /// Node index whose code-block fold/expand strip contains [local], or null.
  int? codeCollapseAt(Offset local) {
    for (var i = 0; i < _layouts.length; i++) {
      if (_layouts[i].collapseButton?.contains(local) ?? false) return i;
    }
    return null;
  }

  /// Node index whose code ⋯ overflow-menu button contains [local], or null.
  int? codeMoreAt(Offset local) {
    for (var i = 0; i < _layouts.length; i++) {
      if (_layouts[i].moreButton?.contains(local) ?? false) return i;
    }
    return null;
  }

  /// The mermaid view tab under [local]: switch to 'code' or 'preview'.
  /// Only the HOVERED block's tabs are live — they are only painted on
  /// hover, and an invisible hit target silently flipped blocks to the code
  /// view when a click landed in the corner.
  ({int node, String view})? viewTabAt(Offset local) {
    final i = _hoverCode;
    if (i == null || i >= _layouts.length) return null;
    if (_layouts[i].viewCodeTab?.contains(local) ?? false) {
      return (node: i, view: 'code');
    }
    if (_layouts[i].viewPreviewTab?.contains(local) ?? false) {
      return (node: i, view: 'preview');
    }
    return null;
  }

  /// Scroll the code block under [local] horizontally by [deltaX]. Returns true
  /// if a code block consumed the scroll.
  bool scrollCodeAt(Offset local, double deltaX) {
    for (final l in _layouts) {
      if (l.kind != 'code_block') continue;
      if (local.dy < l.boxTop || local.dy > l.boxTop + l.boxHeight) continue;
      final visible = (size.width - l.contentLeft - EditorTheme.codePadH).clamp(
        0.0,
        double.infinity,
      );
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
    // Layouts produced by an atomic renderer paint through it; everything
    // else (including a math block that fell through while its raster is
    // pending) is text-pipeline output.
    final renderer = l.renderedBy;
    if (renderer != null) {
      renderer.paint(this, canvas, offset, l, i);
      return;
    }
    final origin = offset + Offset(l.contentLeft, l.textTop);

    switch (l.kind) {
      case 'bulleted_list':
        final c = origin + Offset(-14, l.painter.preferredLineHeight * 0.5);
        switch (l.indentLevel % 3) {
          case 0: // ● filled
            canvas.drawCircle(c, 2.6, Paint()..color = EditorTheme.text);
          case 1: // ○ hollow
            canvas.drawCircle(
              c,
              2.6,
              Paint()
                ..color = EditorTheme.text
                ..style = PaintingStyle.stroke
                ..strokeWidth = 1.2,
            );
          default: // ▪ square
            canvas.drawRect(
              Rect.fromCenter(center: c, width: 4.6, height: 4.6),
              Paint()..color = EditorTheme.text,
            );
        }
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
      case 'footnote_def':
        // `[label]` gutter marker, mirroring the numbered-list marker — a
        // small muted chip that reads as the footnote's number.
        final marker = TextPainter(
          text: TextSpan(
            text: '[${l.footnoteLabel}]',
            style: TextStyle(
              color: EditorTheme.muted,
              fontSize: 12 * _appearance.fontScale,
              height: 1.5,
              fontWeight: FontWeight.w600,
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
      final visible = (size.width - l.contentLeft - EditorTheme.codePadH).clamp(
        0.0,
        double.infinity,
      );
      canvas.save();
      canvas.clipRect(
        Rect.fromLTWH(origin.dx, origin.dy, visible, l.textHeight),
      );
      l.painter.paint(canvas, origin - Offset(scroll, 0));
      canvas.restore();
      if (l.showLineNums) {
        _paintLineNumbers(canvas, origin, l);
      }
      if (l.codeCollapsed) {
        // Fade the last strip of the shown code into the background — hints
        // there's more (AFFiNE-style).
        const fadeH = 36.0;
        final fadeTop = origin.dy + l.textHeight - fadeH;
        canvas.drawRect(
          Rect.fromLTWH(
            offset.dx + l.boxLeft,
            fadeTop,
            size.width - l.boxLeft,
            fadeH,
          ),
          Paint()
            ..shader = ui.Gradient.linear(
              Offset(0, fadeTop),
              Offset(0, fadeTop + fadeH),
              [EditorTheme.codeBg.withAlpha(0), EditorTheme.codeBg],
            ),
        );
      }
      if (l.collapseButton != null) {
        _paintFoldBar(canvas, offset, l);
      }
      if (l.titleRect != null) {
        _paintCodeTitle(canvas, offset, l);
      }
    } else {
      l.painter.paint(canvas, origin);
      // Inline atoms: the painter reserved each placeholder's box; draw the
      // typeset object into it. Rects are painter-local, same origin as text.
      final fold = l.fold;
      if (fold != null) {
        for (final a in fold.atoms) {
          if (a.rect.isEmpty) continue;
          final box = a.rect.shift(origin);
          if (a.underline) {
            // Same blue marks.dart underlines link text with (_linkColor).
            canvas.drawRect(
              Rect.fromLTWH(box.left, box.bottom - 1, box.width, 1),
              Paint()..color = const Color(0xFF2563EB),
            );
          }
          a.renderer.paint(this, canvas, box, a.source);
        }
      }
    }
  }

  /// Rounded pills behind inline `code` spans. Painted under the selection +
  /// text layers, so a selection still tints them and the glyphs stay on top.
  void _paintInlineCode(Canvas canvas, Offset offset) {
    final paint = Paint()..color = EditorTheme.inlineCodeBg;
    for (var i = 0; i < _layouts.length; i++) {
      final l = _layouts[i];
      if (EditorNode.isAtomicKind(l.kind) ||
          l.kind == 'code_block' ||
          l.renderedBy != null) {
        continue;
      }
      final marks = marksFromData(_nodes[i].data);
      if (marks.isEmpty) continue;
      final origin = offset + Offset(l.contentLeft, l.textTop);
      final len = _nodes[i].text.length;
      for (final m in marks) {
        if (m.type != 'code') continue;
        final s = m.start.clamp(0, len);
        final e = m.end.clamp(0, len);
        if (e <= s) continue;
        // Code runs never overlap atom runs (the parser keeps them disjoint),
        // so on a folded node this is a pure shift — mapped anyway, because a
        // stale mark from a concurrent edit must misplace a pill, not corrupt.
        final ps = l.fold?.docToPainter(s) ?? s;
        final pe = l.fold?.docToPainter(e, ceilInsideAtom: true) ?? e;
        final boxes = l.painter.getBoxesForSelection(
          TextSelection(baseOffset: ps, extentOffset: pe),
          boxHeightStyle: ui.BoxHeightStyle.tight,
        );
        for (final b in boxes) {
          final r = b.toRect().shift(origin);
          final chip = Rect.fromLTRB(
            r.left - 3,
            r.top - 1.5,
            r.right + 3,
            r.bottom + 1.5,
          );
          canvas.drawRRect(
            RRect.fromRectAndRadius(chip, const Radius.circular(4)),
            paint,
          );
        }
      }
    }
  }

  void _paintSelection(Canvas canvas, Offset offset) {
    final sel = _selection;
    if (sel == null || sel.isCollapsed) return;
    final start = sel.start;
    final end = sel.end;
    final paint = Paint()..color = EditorTheme.selection;
    for (var i = start.node; i <= end.node && i < _layouts.length; i++) {
      final l = _layouts[i];
      // Atomic nodes (image/table/divider) and renderer-claimed layouts
      // (rendered mermaid) are opaque and painted after the selection layer,
      // so their highlight is drawn on top in _paintAtomicSelection instead.
      if (EditorNode.isAtomicKind(l.kind) || l.renderedBy != null) continue;
      final from = i == start.node ? start.offset : 0;
      final to = i == end.node ? end.offset : _nodes[i].text.length;
      final isCode = l.kind == 'code_block';
      final scroll = isCode ? (_codeScroll[l.nodeId] ?? 0) : 0.0;
      final origin = offset + Offset(l.contentLeft - scroll, l.textTop);
      if (from == to) {
        if (i != end.node) {
          // Empty / fully-included blank line: show a thin marker.
          canvas.drawRect(
            Rect.fromLTWH(
              origin.dx,
              origin.dy,
              6,
              l.painter.preferredLineHeight,
            ),
            paint,
          );
        }
        continue;
      }
      // A folded node's painter wants placeholder offsets. Floor the start and
      // ceil the end when they land inside an atom's run, so a selection that
      // clips a formula still highlights the whole placeholder — you cannot
      // select half of a typeset object.
      final pFrom = l.fold?.docToPainter(from) ?? from;
      final pTo = l.fold?.docToPainter(to, ceilInsideAtom: true) ?? to;
      // BoxHeightStyle.max stretches every run's box to the full line
      // height: CJK and Latin glyphs run at different intrinsic heights, and
      // per-run boxes give the highlight a jagged top edge on mixed lines.
      final boxes = l.painter.getBoxesForSelection(
        TextSelection(baseOffset: pFrom, extentOffset: pTo),
        boxHeightStyle: ui.BoxHeightStyle.max,
      );
      if (isCode) {
        final visible = (size.width - l.contentLeft - EditorTheme.codePadH)
            .clamp(0.0, double.infinity);
        canvas.save();
        canvas.clipRect(
          Rect.fromLTWH(
            offset.dx + l.contentLeft,
            offset.dy + l.textTop,
            visible,
            l.textHeight,
          ),
        );
      }
      for (final b in boxes) {
        canvas.drawRect(b.toRect().shift(origin), paint);
      }
      if (isCode) canvas.restore();
    }
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
      if (l.addRowBar?.contains(local) ?? false)
        return (node: i, column: false);
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

  /// EFFECTIVE per-column widths of table [index] as currently laid out, in
  /// pixels. In auto-fit mode (all stored weights equal) the rendered widths
  /// are content-derived and DIFFER from the stored weights — a column resize
  /// must seed from what's on screen, or its first tick snaps every column
  /// back to the stored (equal) split. Empty when the layout isn't a table.
  List<double> tableEffectiveWeights(int index) {
    if (index < 0 || index >= _layouts.length) return const [];
    final l = _layouts[index];
    if (l.kind != 'table' || l.tableCells.isEmpty) return const [];
    // Cells are built row-major: the first row's rects span every column.
    final widths = <double>[];
    for (final cell in l.tableCells) {
      if (cell.row != 0) break;
      widths.add(cell.rect.width);
    }
    return widths;
  }

  /// Pixel width available to columns (full content width).
  double tableAvailWidth() =>
      (size.width - EditorTheme.gutter).clamp(60.0, double.infinity);

  /// Current overall width fraction of a table node.
  double tableWidthFraction(int index) {
    if (index < 0 || index >= _nodes.length) return 1.0;
    return TableData.fromBlock(_nodes[index].data).tableWidth;
  }

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

  /// Paint other collaborators' carets (a colored bar + a small name flag).
  void _paintRemoteCursors(Canvas canvas, Offset offset) {
    if (_remoteCursors.isEmpty) return;
    for (final rc in _remoteCursors) {
      final idx = _nodes.indexWhere((n) => n.id == rc.blockId);
      if (idx < 0 || idx >= _layouts.length) continue;
      final rect = caretRectFor(DocPosition(idx, rc.offset));
      if (rect == null) continue;
      final caret = rect.shift(offset);
      canvas.drawRRect(
        RRect.fromRectAndRadius(caret, const Radius.circular(1)),
        Paint()..color = rc.color,
      );
      final tp = TextPainter(
        text: TextSpan(
          text: rc.label,
          style: const TextStyle(
            color: Color(0xFFFFFFFF),
            fontSize: 10,
            height: 1.0,
          ),
        ),
        textDirection: TextDirection.ltr,
        maxLines: 1,
      )..layout();
      const padH = 4.0;
      const padV = 2.0;
      final flag = Rect.fromLTWH(
        caret.left,
        caret.top - (tp.height + padV * 2) - 1,
        tp.width + padH * 2,
        tp.height + padV * 2,
      );
      canvas.drawRRect(
        RRect.fromRectAndRadius(flag, const Radius.circular(3)),
        Paint()..color = rc.color,
      );
      tp.paint(canvas, Offset(flag.left + padH, flag.top + padV));
    }
  }

  // ---------------------------------------------------------------------------
  // Geometry / hit testing (used by the widget for caret nav and pointers)
  // ---------------------------------------------------------------------------

  /// Caret rectangle (local coords) for a position, or null if out of range.
  Rect? caretRectFor(DocPosition pos) {
    if (pos.node < 0 || pos.node >= _layouts.length) return null;
    final l = _layouts[pos.node];
    if (EditorNode.isAtomicKind(l.kind)) return null; // no inline caret
    final doc = pos.offset.clamp(0, _nodes[pos.node].text.length);
    // Folding is unconditional now, so the caret's own node is folded too
    // (local and remote alike). docToPainter maps the caret onto the
    // placeholder's edge — run.start → leading, run.end → trailing — rather
    // than into the collapsed source that isn't displayed. The caret only ever
    // rests on those edges (setSelection snaps it out of a run's interior).
    final off = l.fold?.docToPainter(doc) ?? doc;
    final caret = l.painter.getOffsetForCaret(
      TextPosition(offset: off),
      Rect.zero,
    );
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
    // A folded painter answers in placeholder space; the caret (and everything
    // downstream) lives in doc space. Clicking an atom lands on whichever run
    // edge the hit snapped to — never inside it.
    final doc = l.fold?.painterToDoc(tp.offset) ?? tp.offset;
    final offset = _nodes[idx].text.isEmpty
        ? 0
        : doc.clamp(0, _nodes[idx].text.length);
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
    return _stepToNode(pos.node - 1, x, fromBottom: true) ??
        const DocPosition(0, 0);
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
    if (probe != pos && probe.node == pos.node)
      return probe; // moved down a line
    final last = _layouts.length - 1;
    return _stepToNode(pos.node + 1, x, fromBottom: false) ??
        DocPosition(last, _nodes[last].text.length);
  }

  /// Whole-block nodes for selection/caret purposes: atomic kinds, plus any
  /// node whose layout a renderer claimed (a rendered ```mermaid block is a
  /// code_block by kind but behaves as a picture — drag-selection paints the
  /// block tint and the caret steps over it as one stop).
  bool _isAtomicNode(int index) =>
      index >= 0 &&
      index < _nodes.length &&
      (_nodes[index].isAtomic ||
          (index < _layouts.length && _layouts[index].renderedBy != null));

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
  ///
  /// The caret's node is unfolded in practice (Home/End act on the selection
  /// node), so the fold mapping here is identity today — kept anyway so this
  /// stays correct if a caller ever asks about another node.
  DocPosition lineStart(DocPosition pos) {
    final l = _layouts[pos.node];
    final p = l.fold?.docToPainter(pos.offset) ?? pos.offset;
    final lb = l.painter.getLineBoundary(TextPosition(offset: p));
    return DocPosition(pos.node, l.fold?.painterToDoc(lb.start) ?? lb.start);
  }

  /// End of the visual line containing [pos]. See [lineStart] on folding.
  DocPosition lineEnd(DocPosition pos) {
    final l = _layouts[pos.node];
    final p = l.fold?.docToPainter(pos.offset) ?? pos.offset;
    final lb = l.painter.getLineBoundary(TextPosition(offset: p));
    return DocPosition(pos.node, l.fold?.painterToDoc(lb.end) ?? lb.end);
  }

  /// Local top y of the node with [nodeId], or null if absent (for scroll-to).
  double? nodeBoxTop(String nodeId) {
    for (final l in _layouts) {
      if (l.nodeId == nodeId) return l.boxTop;
    }
    return null;
  }

  /// The language chip's text for node [i] — the pinned/auto distinction is
  /// only ever painted onto the canvas, so a test has no other way to read it.
  @visibleForTesting
  String debugLangChipAt(int i) => _layouts[i].langChipText;

  /// Node index of an atomic block whose renderer takes a click as "select me"
  /// and whose box contains [local], or null.
  ///
  /// [positionAt] refuses to put the caret on an atomic node — it snaps to the
  /// neighbouring text — which leaves a block like a divider with no way to be
  /// selected at all. Renderers opt in via
  /// [AtomicBlockRenderer.selectsWholeBlockOnClick]; the host turns a hit here
  /// into a whole-block caret stop, which [_paintAtomicSelection] then tints.
  int? blockSelectAt(Offset local) {
    for (var i = 0; i < _layouts.length; i++) {
      final l = _layouts[i];
      if (!(_renderersByKind[l.kind]?.selectsWholeBlockOnClick ?? false)) {
        continue;
      }
      final box = Rect.fromLTWH(
        l.boxLeft,
        l.boxTop,
        size.width - l.boxLeft,
        l.boxHeight,
      );
      if (box.contains(local)) return i;
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
/// A remote collaborator's caret to paint: block id + UTF-16 offset + color +
/// name label (P2 awareness).
typedef RemoteCursor = ({
  String blockId,
  int offset,
  Color color,
  String label,
});

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
    this.onImagePainted,
    this.previewImages = const {},
    this.previewBaselines = const {},
    this.onRequestPreview,
    this.remoteCursors = const [],
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
  final void Function(String key)? onImagePainted;
  final List<RemoteCursor> remoteCursors;

  /// Rasterized formulas keyed by LaTeX source (captured by the editor via
  /// an offstage flutter_math_fork widget at device pixel ratio).
  final Map<String, Map<String, ui.Image>> previewImages;

  /// Top-to-baseline distances matching [previewImages] (logical px at the
  /// offstage widget's size). Inline atoms sit formulas on the text baseline
  /// with these; a missing entry degrades to middle alignment.
  final Map<String, Map<String, double>> previewBaselines;
  final void Function(String id, String source, double targetWidth)?
  onRequestPreview;

  @override
  RenderDocument createRenderObject(BuildContext context) =>
      RenderDocument(
          nodes: nodes,
          selection: selection,
          showCaret: showCaret,
          caretOn: caretOn,
          appearance: appearance,
        )
        ..onRequestImage = onRequestImage
        ..onImagePainted = onImagePainted
        ..onRequestPreview = onRequestPreview
        ..imageErrors = imageErrors
        ..previewImages = previewImages
        ..previewBaselines = previewBaselines
        ..remoteCursors = remoteCursors
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
      ..onImagePainted = onImagePainted
      ..onRequestPreview = onRequestPreview
      ..imageErrors = imageErrors
      ..previewImages = previewImages
      ..previewBaselines = previewBaselines
      ..remoteCursors = remoteCursors
      ..images = images;
  }
}
