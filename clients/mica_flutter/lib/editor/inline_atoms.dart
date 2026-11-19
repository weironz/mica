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

  PreviewResourceKey? previewKey(String source);

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

/// Inline formulas use RaTeX text style, with an independent cache from
/// display formulas. The same raster scales to any surrounding font size.
class MathInlineAtomRenderer extends InlineAtomRenderer {
  const MathInlineAtomRenderer();

  @override
  String get markType => 'math';

  @override
  PreviewResourceKey? previewKey(String source) =>
      source.trim().isEmpty ? null : (id: 'math-inline', source: source);

  @override
  ({Size size, double? baseline})? measure(
    RenderDocument host,
    String source,
    double fontSize,
    double maxWidth,
  ) {
    if (source.trim().isEmpty) return null;
    final pixels = host._previewSizes[(id: 'math-inline', source: source)];
    if (pixels == null) return null;
    const dpr = EditorTheme.mathPixelRatio;
    var scale = (fontSize / EditorTheme.mathRasterFontSize) / dpr;
    var w = pixels.width * scale;
    if (w > maxWidth && w > 0) {
      // A formula wider than the wrap width would silently overflow the line
      // box (placeholders don't clip); shrink it to fit instead.
      scale *= maxWidth / w;
      w = maxWidth;
    }
    final h = pixels.height * scale;
    final rawBaseline =
        host._retainedBaselines[(id: 'math-inline', source: source)];
    // Convert the logical raster baseline to the surrounding text size.
    final baseline = rawBaseline == null ? null : rawBaseline * scale * dpr;
    return (size: Size(w, h), baseline: baseline);
  }

  @override
  void paint(RenderDocument host, Canvas canvas, Rect rect, String source) {
    final img = host._previewImages['math-inline']?[source];
    if (img == null) return; // evicted between layout and paint: skip a frame
    host.onPreviewPainted?.call('math-inline', source);
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
/// branches (docs/architecture/render-architecture.md).
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

class _FoldSlot {
  _FoldSlot({
    required this.docStart,
    required this.docEnd,
    required this.attachAfter,
    this.atom,
    this.spacer,
  });
  final int docStart;
  final int docEnd;
  final bool attachAfter;
  final InlineAtom? atom;
  final InlineCodeSpacer? spacer;
  int painterIndex = -1;
  int get docLen => docEnd - docStart;
  int get _order => atom != null ? 1 : (attachAfter ? 2 : 0);
}

/// The doc↔painter offset mapping for one folded node (inline atoms + code
/// boundary spacers). Painter offsets never leave render.dart without passing
/// back through [painterToDoc].
class FoldPlan {
  FoldPlan(this.atoms, [this.spacers = const []]) {
    _slots = [
      for (final a in atoms)
        _FoldSlot(docStart: a.docStart, docEnd: a.docEnd, attachAfter: false, atom: a),
      for (final s in spacers)
        if (!atoms.any((a) => a.docStart < s.docOffset && s.docOffset < a.docEnd))
          _FoldSlot(docStart: s.docOffset, docEnd: s.docOffset, attachAfter: s.attachAfter, spacer: s),
    ]..sort((a, b) {
        final c = a.docStart.compareTo(b.docStart);
        return c != 0 ? c : a._order.compareTo(b._order);
      });
    var shift = 0;
    for (final slot in _slots) {
      slot.painterIndex = slot.docStart - shift;
      slot.atom?.painterIndex = slot.painterIndex;
      shift += slot.docLen - 1;
    }
  }

  final List<InlineAtom> atoms;
  final List<InlineCodeSpacer> spacers;
  late final List<_FoldSlot> _slots;

  int get placeholderCount => _slots.length;

  bool assignBoxes(List<ui.TextBox> boxes) {
    if (boxes.length != _slots.length) return false;
    for (var i = 0; i < _slots.length; i++) {
      _slots[i].atom?.rect = boxes[i].toRect();
    }
    return true;
  }

  /// Doc → painter. Offsets inside an atom collapse to its placeholder edges;
  /// leading code spacers sit before [docOffset] (`doc >= docOffset` shifts),
  /// trailing code spacers sit after (`doc > docOffset` shifts).
  int docToPainter(int doc, {bool ceilInsideAtom = false}) {
    var shift = 0;
    for (final slot in _slots) {
      if (slot.atom != null) {
        if (doc <= slot.docStart) break;
        if (doc < slot.docEnd) {
          return slot.docStart - shift + (ceilInsideAtom ? 1 : 0);
        }
        shift += slot.docLen - 1;
      } else {
        if (doc < slot.docStart || (!slot.attachAfter && doc == slot.docStart)) {
          break;
        }
        shift -= 1;
      }
    }
    return doc - shift;
  }

  /// Painter → doc. Either edge of an atom maps to its run boundary; either
  /// edge of a code spacer maps back to its [docOffset].
  int painterToDoc(int painter) {
    var shift = 0;
    for (final slot in _slots) {
      final pos = slot.painterIndex;
      if (painter <= pos) return painter + shift;
      if (painter == pos + 1) return slot.docEnd;
      shift += slot.docLen - 1;
    }
    return painter + shift;
  }
}

/// Build the display TextSpan (with WidgetSpans for atoms and code spacers)
/// and matching [PlaceholderDimensions] for RenderDocument's own TextPainter.
({TextSpan span, List<PlaceholderDimensions> dims}) buildFoldedSpan(
  String text,
  List<Mark> marks,
  TextStyle base,
  FoldPlan fold,
  MicaTokens tokens,
) {
  final len = text.length;
  final dims = <PlaceholderDimensions>[];
  final children = <InlineSpan>[];
  final styleMarks = [
    for (final m in marks)
      if (!fold.atoms.any((a) => a.docStart == m.start && a.docEnd == m.end)) m,
  ];
  var cursor = 0;
  void emitText(int from, int to) {
    if (to <= from) return;
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
    children.add(buildMarkedSpan(text.substring(from, to), localMarks, base, tokens));
  }

  for (final slot in fold._slots) {
    emitText(cursor, slot.docStart);
    final a = slot.atom;
    if (a != null) {
      final align = a.baseline != null
          ? ui.PlaceholderAlignment.baseline
          : ui.PlaceholderAlignment.middle;
      children.add(WidgetSpan(alignment: align, baseline: TextBaseline.alphabetic, child: const SizedBox.shrink()));
      dims.add(PlaceholderDimensions(size: a.size, alignment: align, baseline: TextBaseline.alphabetic, baselineOffset: a.baseline));
      cursor = a.docEnd;
    } else {
      children.add(const WidgetSpan(alignment: ui.PlaceholderAlignment.baseline, baseline: TextBaseline.alphabetic, child: SizedBox.shrink()));
      dims.add(PlaceholderDimensions(size: Size(slot.spacer!.width, 1), alignment: ui.PlaceholderAlignment.baseline, baseline: TextBaseline.alphabetic, baselineOffset: 1));
      cursor = slot.docStart;
    }
  }
  emitText(cursor, len);
  return (span: TextSpan(style: base, children: children), dims: dims);
}
