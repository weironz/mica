# Block rendering architecture (P2 refactor design)

Status: IMPLEMENTED (Phase 1+2). The renderer registry lives in
`lib/editor/block_renderers.dart` (a part-file of render.dart — renderers
stay inside the library boundary instead of forcing layout internals
public); the preview pipeline in `lib/editor/preview_raster.dart`. Body
paint dispatches on the layout's producer (`renderedBy`); backdrops
dispatch by kind (block identity, shown on the fallen-through source form
too). Still open: collapsing _NodeLayout's per-kind fields into a
rendererData slot, and hit-test dispatch — both deferred until a renderer
actually needs them.

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

## Decision 3: animated images are played by hand, off the canvas's own cache

GIFs (and animated WebP) can't ride Flutter's `Image` widget — the canvas paints
raw `ui.Image`s. `ui.instantiateImageCodec` + a single `getNextFrame()` gives you
frame 0 and nothing else, which is exactly what a GIF used to look like here:
still.

`image_animator.dart` plays the loop instead, modelled on Flutter's own
`MultiFrameImageStreamCompleter`. Two things it copies, both load-bearing:

- **Frames are emitted from a scheduler frame callback**, not straight off a
  `Timer`. The engine stops producing frames when the window is hidden, so the
  animation stalls by itself rather than decoding for nobody.
- **A 0ms delay means "unspecified"**, not "flat out" — substitute 100ms, as
  browsers do, or the loop pegs a core.

Two things it does NOT copy, because the canvas is not a widget tree:

- **Frame swaps go through `RenderDocument.replaceImage`, not the `images`
  setter** — every frame is the same size, so the box never moves and a relayout
  of the whole document per frame would be pure waste. A frame that somehow does
  differ in size falls back to a relayout.
- **Loops are stopped by the paint, not by the model.** `ImageRenderer.paint`
  reports each image it draws (`onImagePainted`); a loop whose last frame nobody
  drew is paused (block deleted, source replaced) and revives when the canvas
  paints it again. There is no separate liveness bookkeeping to drift out of
  sync with the document.

Ownership is the sharp edge: each emitted frame belongs to the host from the
moment it lands, the host disposes the frame it has moved past, and `RawImage`
clones what it is handed — which is why the fullscreen viewer can hold a frame
the editor is busy replacing.

## Non-goals

- No renderer interface for text blocks (see Decision 1).
- No plugin discovery/dynamic loading — the registry is a hardcoded list.
- No server-side diagram rendering for now (revisit if desktop needs previews).
