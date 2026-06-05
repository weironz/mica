# Block rendering architecture (P2 refactor design)

Status: DESIGN — implement after `feat/editor-interactions` and
`fix/table-inline-code` merge (the refactor touches the same files).

## Problem

`render.dart` (70KB) handles every block kind with scattered `if (kind == …)`
branches in **three** places — layout (~480), paint (~930/~1269), hit-test
(~1908) — and `_NodeLayout` accretes per-kind fields (`tableCells`, `imageDst`,
`mathImage`, …). Each new block type (Mermaid, Graphviz, footnote panel)
multiplies all of it. The math raster pipeline (offstage widget → `ui.Image`)
lives ad-hoc in `editor.dart` (~190-260) and can't be reused.

## Decision 1: text flow is the engine, atomic blocks are plugins

Text-bearing kinds (`paragraph`, `heading`, lists, `quote`, `todo`,
`code_block`) share one TextPainter pipeline — marks, caret, selection,
wrapping. That pipeline IS the editor; forcing it behind a generic interface
would abstract the load-bearing wall. It stays as-is.

Atomic kinds (`divider`, `image`, `math_block`, `table`, future `mermaid`)
each own their geometry, painting, and pointer behavior. These become
**plugins**:

```dart
abstract class BlockRenderer {
  bool canHandle(EditorNode node);
  NodeLayout layout(LayoutContext ctx, EditorNode node, double y, double maxWidth);
  void paint(PaintContext ctx, Canvas canvas, Offset offset, NodeLayout l);
  /// Pointer behavior at [local] within the block, or null for default.
  BlockHit? hitTest(NodeLayout l, Offset local);
}
```

- A registry `List<BlockRenderer>` (first `canHandle` wins) replaces the kind
  branches; the three main loops ask the registry, falling through to the text
  pipeline when no renderer claims the node.
- `_NodeLayout`'s per-kind fields collapse into one `Object? rendererData`
  slot that each renderer owns; the shared fields (boxTop/boxHeight/kind/
  nodeId) stay.
- Migration order, one commit each, tests green between: divider (smallest) →
  image → math_block → table (largest; drags `_TableCell` along).

## Decision 2: one raster-preview pipeline for "source → picture" blocks

Math today: offstage flutter_math widget → RepaintBoundary.toImage → cache by
source string → repaint. Mermaid/Graphviz want the same lifecycle (pending,
capture/render, cache, retry-limit, invalidate-on-edit) with a different
producer. Generalize the lifecycle, not the producer:

```dart
abstract class RasterPreviewer {
  String get id;                       // 'math' | 'mermaid' | …
  bool claims(EditorNode node);        // math_block, or code_block lang=mermaid
  /// Either an offstage widget the pipeline captures…
  Widget? buildOffstage(String source) => null;
  /// …or a direct async producer (JS interop, server render).
  Future<ui.Image?> render(String source, double maxWidth)? get producer => null;
}
```

The pipeline (extracted from editor.dart's math code) owns: the
`{previewerId, source} → ui.Image` cache, the pending/retry bookkeeping, and
the offstage host column. **Failure degrades gracefully** to the highlighted
code block / literal source per the dialect principle — a preview is an
enhancement, never a gate.

Math becomes the first `RasterPreviewer` (offstage form). Mermaid (P4) becomes
the second: on web, JS interop → mermaid.js → SVG → image; elsewhere,
`claims()` returns false and the fenced block renders as plain highlighted
code. No new block kind: ` ```mermaid ` stays a `code_block` in the document
model (round-trips through Markdown untouched); only rendering changes.

## Non-goals

- No renderer interface for text blocks (see Decision 1).
- No plugin discovery/dynamic loading — the registry is a hardcoded list.
- No server-side diagram rendering for now (revisit if desktop needs previews).
