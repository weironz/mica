# Block rendering architecture (P2 refactor design)

Status: IMPLEMENTED (Phase 1+2). The renderer registry lives in
`lib/editor/block_renderers.dart` (a part-file of render.dart — renderers
stay inside the library boundary instead of forcing layout internals
public); the preview pipeline in `lib/editor/preview_raster.dart`. Body
paint dispatches on the layout's producer (`renderedBy`); backdrops
dispatch by kind (block identity, shown on the fallen-through source form
too).

The SHIPPED dispatch differs from Decision 1's sketch below (kept as the
historical record): registration is a const `List<AtomicBlockRenderer>`
and a renderer declines by returning null from `layout()` — not
`canHandle` order.

**P3-1 closed (2026-08-03).** The registry used to flatten into a
kind→renderer `Map`, so a second registration for a kind silently
REPLACED the first with no compile error. `DetailsRenderer` is that second
`code_block` renderer, so the map became kind→`List`: within a kind the
renderers are tried in registration order and the first non-null `layout()`
wins. Order is a tie-breaker, not a design — two renderers that could both
claim one node is still a bug, and the fix is to be strict in `layout()`.
`test/details_fold_test.dart` pins that both `code_block` renderers survive
registration.

## Decision 5: a renderer can absorb the nodes after it — zero-height, not skipped

A `<details>` written the way GitHub's docs recommend (blank lines, so the
body is Markdown) is not one block: the parser emits the opening tags, the
body as ordinary blocks, and a bare `</details>` (pinned in
`crates/markdown/tests/details_fold.rs`). Folding it means hiding a RANGE.

`_layouts[i]` means "node i" in roughly forty places in render.dart —
selection painting, caret rects, click and hover hit-testing, drop indices,
comment anchors. **Skipping nodes in the layout pass would shift every index
after the fold**, so the mechanism is the opposite: absorbed nodes still get
a `_NodeLayout`, marked `hidden`, of zero height.

- A renderer declares what it took in `_NodeLayout.absorbs` (forward indices
  only). `performLayout` collects them as it goes and emits the placeholders.
  This is the only new concept in the loop; there are no per-kind branches,
  and any future renderer that owns a run of blocks (a real toggle block, a
  columns layout) gets it for free.
- `_nodeVisible` — already the single cull predicate every paint layer runs
  through — returns false for `hidden`, so *nothing* paints: no quote bar, no
  selection wash, no scrollbar, no backdrop.
- Zero height is not enough on its own. Four paths needed explicit skips
  because they answer with an index rather than by geometry, and each has a
  red test in `test/details_fold_test.dart`: `positionAt`'s "past the last
  block" fallback (a document ending in a fold parked the caret in the hidden
  closer), `dropIndexAt` (a zero-height midpoint swallows the drop aimed at
  the block below), `_stepToNode` (it answers `DocPosition(i, 0)` outright for
  an ATOMIC node, so Down off the header landed on a folded-away divider), and
  `caretRectFor` (a remote cursor would be drawn on the fold's seam).

The escape hatch is the same one the tight form already had, narrowed: the
renderer declines when the selection touches the tags **or anything the fold
hides** — so the source is always reachable — but NOT when the caret is in an
expanded body, which is the normal place to type.

Still open: collapsing _NodeLayout's per-kind fields into a rendererData
slot (still unneeded); hit-test dispatch is NOT "unneeded" anymore —
TableRenderer needs it today and does it ad-hoc via RenderDocument methods
(`tableCellAt`/`tableCellRect`/`tableHandleAt`/`tableDeleteAt`,
render.dart ~2908-2966) outside the registry (code-review P3-2, open).

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

编辑器的 `RasterPreviewer` 异步产出图像和可选基线。`RasterPreviewPipeline` 只为可见内容发请求，
按渲染器 id 和源码缓存；主题切换、视口变化或编辑器关闭后，过期结果不会写回，迟到的图像会被释放。
失败分两种，处理不同：

- **源码渲染不了**（语法错、未知命令，或这条源码让渲染器崩溃）：记为失败，编辑器继续显示可编辑源码，不再重试。
- **渲染器没能运行**（`PreviewUnavailable`：渲染器下载失败、离线，或别的源码让网页渲染器崩溃时它正在排队）：
  不记失败，这个渲染器暂停 1 秒后重试，连续失败时暂停时长翻倍，最长 30 秒；成功一次后复位。
  网页渲染器加载失败时 worker 会自行关闭，因为浏览器会在同一个 worker 里记住失败的模块导入，只有新 worker 能重新加载。

Mermaid 和公式在网页经 Web Worker 调用 Rust wasm，在桌面和手机经 FFI 调用同一份 Rust。
两端共用 SVG 光栅化；Mermaid 保留主程序主题角色和 CSS 内联，RaTeX 分别给行内、块级排版。
HTML/PDF 直接使用同一 Rust 引擎。Markdown 与文档 wasm 负责启动，渲染 wasm 只在预览请求时下载，
见[实测及预算](../reviews/redundancy-and-home-review-2026-11-10.md#前三项实施记录2026-11-11)。

Mermaid 仍是 `language: mermaid` 的 `code_block`，公式的块型和标记也未变；只有预览路径变了。

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

## Decision 4: inline atoms fold at paint time; formulas are atoms you click to edit

Inline math (`$…$`, a `math` mark over N source characters) typesets in-line
without touching Decision 1's load-bearing wall, via a fold: every atom-marked
run becomes **one U+FFFC placeholder** in its node's TextPainter
(`TextPainter.setPlaceholderDimensions` — probed on desktop **Skia** and
verified on CanvasKit with real rendering), and the typeset raster is drawn into
the placeholder's box. This holds unconditionally — a formula is never entered.

> ⚠️ **That "desktop Skia" is now a claim about a renderer we no longer ship.**
> Flutter 3.47 made **Impeller** the default on Windows
> ([#188140](https://github.com/flutter/flutter/pull/188140)), and 3.47.1 is the
> pinned toolchain (`.fvmrc`), so every desktop build from 0.13.29 on is
> Impeller. The behaviour almost certainly did not change — placeholder metrics
> are a framework-side `TextPainter` concern, above the raster backend — but the
> EVIDENCE behind this sentence was gathered somewhere we do not build any more,
> and an unverified claim that reads as verified is what this file exists to
> prevent. Re-probe on Impeller before leaning on it again.
>
> The one place in this renderer worth an actual eyeball is the math hover
> card's shadow (`render.dart`, `MaskFilter.blur`): it is the ONLY exotic canvas
> op in the whole editor — no `saveLayer`, no `drawVertices`, no
> `BackdropFilter`, no `ImageFilter` — and mask blur is where Impeller and Skia
> have historically differed most.

A typeset formula is an **atom** (AppFlowy / AFFiNE / Notion all landed here):

- The caret rests on either edge but never inside the source. The controller's
  single `setSelection` choke point snaps any selection endpoint out of a run's
  interior, so every caret move, click and drag inherits it for free.
- Clicking a formula opens the same **source editor dialog** the block form
  uses (`_editInlineMath`), writing back through `setInlineMathSource`, which
  rides the `math` mark's length change through `shiftMarks` (empty source
  deletes the run).
- Backspace/Delete at an edge removes the whole formula
  (`deleteMathAtom{Backward,Forward}`) — the source is indivisible.

The document model, storage, and markdown round-trip see none of it: folding is
strictly a render-layer affair. (The earlier Zed model — the caret's node shows
editable source in place — is preserved in `_paintMathPreview`'s hover card,
which typesets the formula you are editing; it is now a preview aid, not the
edit surface.)

The machinery (`lib/editor/inline_atoms.dart`, a part-file like the block
registry):

- **`InlineAtomRenderer` registry, keyed by mark type** — the inline sibling of
  `AtomicBlockRenderer`, same contract shape: `measure() → null` DECLINES and
  the run stays styled source (raster pending, capture failed, empty source).
  Adding an inline-rendered mark type means adding a renderer here, never an
  `if (type == …)` in render.dart.
- **`FoldPlan`** — the doc↔painter offset mapping for one folded node. Doc
  offsets strictly inside a run collapse to the placeholder's edges
  (floor/ceil pair, so ranges always cover the whole placeholder); painter
  offsets map back to run edges, never inside. The invariant that keeps this
  sane: **painter offsets never leave render.dart unmapped**. Crossings today:
  `positionAt`, `caretRectFor` (remote cursors reach folded nodes),
  `_paintSelection`, `_paintInlineCode`, `lineStart`/`lineEnd`.
- **The selection setter is paint-only** — fold state no longer depends on the
  selection (folding is unconditional). Note this is not a layout fast-path in
  practice: the `nodes` setter relayouts the whole document on every controller
  notification (nodes is mutated in place, so it can't cheaply detect change);
  measured ~6ms for 200 nodes, folding included, and folding makes layout
  *cheaper* (one placeholder per run instead of the glyphs). Don't build on an
  assumption that caret moves skip layout.
- **Math reuses the block previewer whole** — same `'math'` id, same
  source-keyed raster cache; the 18pt raster scales by (text font size / 18).
  The pipeline additionally captures each preview's **baseline** (a transparent
  `RenderProxyBox` inside the RepaintBoundary reads `getDistanceToBaseline`
  during its own performLayout — within 0.004px of a bare layout) so atoms sit
  on the text baseline via `PlaceholderAlignment.baseline`; a missing baseline
  degrades to `middle`, never crashes.

## Non-goals

- No renderer interface for text blocks (see Decision 1).
- No plugin discovery/dynamic loading — the registry is a hardcoded list.
- No server-side diagram rendering for now (revisit if desktop needs previews).
- No in-place editing of a typeset inline formula (clicking it unfolds the
  node to source instead — AppFlowy/AFFiNE reached the same conclusion via
  popovers).
