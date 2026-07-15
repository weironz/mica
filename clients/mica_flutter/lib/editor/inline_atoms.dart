part of 'render.dart';

/// Renderer for one inline atom type — a marked run of a node's text that
/// paints as a single typeset object (math today) instead of as characters.
///
/// The block-level sibling of this registry is [AtomicBlockRenderer]; the
/// contract is deliberately the same shape. [measure] returning null DECLINES:
/// the run stays styled source (exactly the block registry's `layout() → null`
/// fall-through), which is what happens while a raster is still being
/// produced, when it failed, or when the run is empty.
///
/// The atom occupies ONE placeholder code unit (U+FFFC) in the node's
/// TextPainter while the underlying document text keeps all N source
/// characters — the mapping between the two lives in [FoldPlan], and only
/// render.dart ever sees painter offsets. Nothing about the document model,
/// storage, or markdown round-trip changes: folding is a paint-time affair.
abstract class InlineAtomRenderer {
  const InlineAtomRenderer();

  /// The mark type this renderer folds ('math').
  String get markType;

  /// Size + baseline (logical px, top-to-alphabetic) the atom wants at
  /// [fontSize] text, at most [maxWidth] wide. Null declines the fold.
  ({Size size, double? baseline})? measure(
    RenderDocument host,
    String source,
    double fontSize,
    double maxWidth,
  );

  /// Paint the atom into [rect] (already positioned by the text layout).
  void paint(RenderDocument host, Canvas canvas, Rect rect, String source);
}

/// Inline math: reuses the block previewer whole — same 'math' id, same
/// source-keyed raster cache, same offstage flutter_math_fork capture — and
/// scales the 18pt raster to the surrounding font size. One cache serves the
/// block renderer, the hover card, and every inline size, which is why the
/// cache key needs no font size in it.
class MathInlineAtomRenderer extends InlineAtomRenderer {
  const MathInlineAtomRenderer();

  @override
  String get markType => 'math';

  @override
  ({Size size, double? baseline})? measure(
    RenderDocument host,
    String source,
    double fontSize,
    double maxWidth,
  ) {
    if (source.trim().isEmpty) return null;
    final img = host._previewImages['math']?[source];
    if (img == null) {
      // Deferred-safe (request() only registers; rebuilds are post-frame),
      // and the raster landing relayouts via the previewImages setter.
      host.onRequestPreview?.call('math', source, 0);
      return null;
    }
    const dpr = EditorTheme.mathPixelRatio;
    var scale = (fontSize / EditorTheme.mathRasterFontSize) / dpr;
    var w = img.width * scale;
    if (w > maxWidth && w > 0) {
      // A formula wider than the wrap width would silently overflow the line
      // box (placeholders don't clip); shrink it to fit instead.
      scale *= maxWidth / w;
      w = maxWidth;
    }
    final h = img.height * scale;
    final rawBaseline = host._previewBaselines['math']?[source];
    // Baseline is logical at capture size; the image is captured at dpr×, so
    // the scale from capture-logical to draw-logical is (scale * dpr).
    final baseline = rawBaseline == null ? null : rawBaseline * scale * dpr;
    return (size: Size(w, h), baseline: baseline);
  }

  @override
  void paint(RenderDocument host, Canvas canvas, Rect rect, String source) {
    final img = host._previewImages['math']?[source];
    if (img == null) return; // evicted between layout and paint: skip a frame
    canvas.drawImageRect(
      img,
      Rect.fromLTWH(0, 0, img.width.toDouble(), img.height.toDouble()),
      rect,
      Paint()..filterQuality = FilterQuality.medium,
    );
  }
}

/// Inline atom registry, keyed by mark type. Adding an inline-rendered mark
/// means adding a renderer here — render.dart must not grow `if (type == …)`
/// branches (docs/render-architecture.md).
const Map<String, InlineAtomRenderer> _inlineAtomRenderers = {
  'math': MathInlineAtomRenderer(),
};

/// One folded run within a node: [docStart, docEnd) of the node's text
/// rendered as a single placeholder at [painterIndex] in the display text.
class InlineAtom {
  InlineAtom({
    required this.docStart,
    required this.docEnd,
    required this.source,
    required this.size,
    required this.baseline,
    required this.renderer,
    this.underline = false,
    this.painterIndex = -1,
  });

  final int docStart;
  final int docEnd;
  final String source;

  /// A link mark spans this run — Flutter won't draw a TextDecoration across a
  /// placeholder, so the atom paints its own underline segment to keep a link
  /// that wraps a formula visually continuous. A generic mark-property pass-
  /// through, not a renderer-type branch.
  final bool underline;

  /// Offset of this atom's U+FFFC in the display text — assigned by
  /// [FoldPlan]'s constructor from the accumulated shifts.
  int painterIndex;
  final Size size;
  final double? baseline;
  final InlineAtomRenderer renderer;

  /// Placeholder rect from the painter, local to the node's text origin —
  /// filled after TextPainter.layout from inlinePlaceholderBoxes.
  Rect rect = Rect.zero;

  int get docLen => docEnd - docStart;
}

/// The doc↔painter offset mapping for one folded node.
///
/// Doc space is the node's stored text (marks, selection, IME, every offset
/// outside render.dart). Painter space is the display text where each atom's
/// run is one U+FFFC. Atoms are sorted and non-overlapping; everything between
/// them maps by a running shift. Painter offsets never leave render.dart
/// without passing back through [painterToDoc].
class FoldPlan {
  FoldPlan(this.atoms) {
    var shift = 0;
    for (final a in atoms) {
      a.painterIndex = a.docStart - shift;
      shift += a.docLen - 1;
    }
  }

  /// Sorted by [InlineAtom.docStart], non-overlapping.
  final List<InlineAtom> atoms;

  /// Doc → painter. Offsets strictly inside an atom's run collapse to the
  /// atom's placeholder: its leading edge, or the trailing edge with
  /// [ceilInsideAtom] — range mappers use the pair so a doc range that ends
  /// inside a run still covers the whole placeholder.
  int docToPainter(int doc, {bool ceilInsideAtom = false}) {
    var shift = 0;
    for (final a in atoms) {
      if (doc <= a.docStart) break;
      if (doc < a.docEnd) {
        return a.docStart - shift + (ceilInsideAtom ? 1 : 0);
      }
      shift += a.docLen - 1;
    }
    return doc - shift;
  }

  /// Painter → doc. The placeholder's leading edge maps to the run's start,
  /// its trailing edge to the run's end — a caret can land on either side of
  /// the atom but never inside it.
  int painterToDoc(int painter) {
    var shift = 0;
    for (final a in atoms) {
      final pos = a.docStart - shift;
      if (painter <= pos) return painter + shift;
      if (painter == pos + 1) return a.docEnd;
      shift += a.docLen - 1;
    }
    return painter + shift;
  }
}

/// Build the folded form of a node's text: the display TextSpan (with one
/// WidgetSpan per atom), the placeholder dimensions to hand to
/// [TextPainter.setPlaceholderDimensions] (same order), and the atoms.
///
/// Standalone from [buildMarkedSpan] on purpose: that builder is shared with
/// real EditableTexts (the table cell editor), where a placeholder span would
/// either assert or get its child built for real. This one is only ever
/// consumed by RenderDocument's own TextPainter.
({TextSpan span, List<PlaceholderDimensions> dims, List<InlineAtom> atoms})
buildFoldedSpan(
  String text,
  List<Mark> marks,
  TextStyle base,
  List<InlineAtom> atoms,
) {
  final len = text.length;
  final dims = <PlaceholderDimensions>[];
  final children = <InlineSpan>[];

  // Marks that survive folding: everything except the folded runs themselves.
  final styleMarks = [
    for (final m in marks)
      if (!atoms.any((a) => a.docStart == m.start && a.docEnd == m.end)) m,
  ];

  var cursor = 0;
  void emitText(int from, int to) {
    if (to <= from) return;
    // buildMarkedSpan styles a whole string by its marks; slice the segment's
    // marks into segment-local coordinates and reuse it, so bold/links/code
    // around atoms style exactly as they do unfolded.
    final localMarks = <Mark>[
      for (final m in styleMarks)
        if (m.end > from && m.start < to)
          Mark(
            (m.start - from).clamp(0, to - from),
            (m.end - from).clamp(0, to - from),
            m.type,
            href: m.href,
            title: m.title,
          ),
    ];
    final piece = buildMarkedSpan(text.substring(from, to), localMarks, base);
    children.add(piece);
  }

  for (final a in atoms) {
    emitText(cursor, a.docStart);
    children.add(
      WidgetSpan(
        // Never built: TextPainter only reads the placeholder, and this span
        // tree never enters a widget. The dimensions below carry the size.
        alignment: a.baseline != null
            ? ui.PlaceholderAlignment.baseline
            : ui.PlaceholderAlignment.middle,
        baseline: TextBaseline.alphabetic,
        child: const SizedBox.shrink(),
      ),
    );
    dims.add(
      PlaceholderDimensions(
        size: a.size,
        alignment: a.baseline != null
            ? ui.PlaceholderAlignment.baseline
            : ui.PlaceholderAlignment.middle,
        baseline: TextBaseline.alphabetic,
        baselineOffset: a.baseline,
      ),
    );
    cursor = a.docEnd;
  }
  emitText(cursor, len);

  return (
    span: TextSpan(style: base, children: children),
    dims: dims,
    atoms: atoms,
  );
}
