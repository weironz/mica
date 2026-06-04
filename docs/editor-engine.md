# Editor Engine Design

In-house editor engine for Mica. We build this ourselves rather than depend on a
third-party editor (`appflowy_editor`, `super_editor`, Quill, …). The backend
block model and sync are reused unchanged; this engine is a new client layer.

**It is built into Mica, not a separate project.** Unlike AppFlowy, which ships
its editor as a standalone reusable package (`appflowy_editor`), our engine is an
internal module of the existing Flutter app — no new repo, no new pub package, no
new dependency. It lives under `clients/mica_flutter/lib/editor/` (e.g.
`document.dart`, `selection.dart`, `input/`, `commands/`, `render/`, `markdown/`),
extracted out of today's single `main.dart` and called directly by the app UI.
Extracting it into a package later remains possible but is explicitly not a goal.

Read alongside [Editor Design Principles](editor.md) (the UX north star: the page
is block-based underneath but feels like one continuous document — Typora/Word,
no block chrome at rest). This document is the engineering spec: the full list of
capabilities to implement.

## Dependencies & layering

"Editor" and "rendering engine" are not two separate products — they are two
layers of the *same* in-house editor. What we own vs. what the platform provides:

| Layer | What it does | Owner | Third-party? |
| --- | --- | --- | --- |
| Pixel rendering engine | Rasterization, low-level text layout (draw to screen) | **Flutter** (Skia / Impeller) | Platform — not reimplemented |
| Editor view layer | Per-node layout, caret + selection painting, hit-testing | **Mica (in-house)** — `lib/editor/render/` | No |
| Editor core | Document model, selection, input, commands, transactions | **Mica (in-house)** — `lib/editor/` | No |

Dependency policy:

- **No third-party editor library** (no `appflowy_editor` / `super_editor` /
  `quill`). The current MVP is built on Flutter's built-in `TextField`; the
  engine will instead use Flutter's lower-level primitives (`RenderObject`,
  `TextPainter`, `TextInput`) — still the framework, not an external package.
- The whole app deliberately depends only on **Flutter** plus two Dart-team
  packages: `http` and `web_socket_channel`. Both are removable in favor of
  `dart:io` / `dart:html` built-ins if we ever want a zero-package client; the
  benefit is small, so they stay for now.
- Scope of the "no external dependency" principle: **do not adopt heavyweight,
  framework-level ready-made solutions** (a third-party editor core, a CRDT
  framework, etc.). It does **not** mean avoiding Flutter itself or basic
  building-block libraries. Rewriting the pixel renderer or replacing Flutter is
  out of scope unless explicitly decided otherwise.

## Why a single editing surface

The current MVP renders one Flutter `TextField` per block. Each `TextField` owns
its own selection, focus, gesture recognizer, and OS/IME input connection, and
Flutter has no API for a selection spanning multiple editable widgets. That makes
cross-block drag selection, unified caret movement, multi-block copy/paste, and
document-wide undo impossible.

The engine replaces N text fields with **one editing surface**: a single focus, a
single input connection, one document-wide selection model, and custom rendering
of each node. Nodes render themselves but do not own selection — the engine owns
caret, selection painting, input, and commands across the whole document.

## Document & selection model

### Document tree

- A document is an ordered tree of **nodes**. Each node has: `id`, `type`,
  `attributes` (map; e.g. heading level, todo checked, image url, code language),
  optional **inline content** (rich text), and ordered `children`.
- Two structural families:
  - **Text nodes** — carry editable inline content (paragraph, heading, list
    item, quote, todo, callout title, table cell, …).
  - **Void / atomic nodes** — no text caret inside (image, file, divider,
    embed/bookmark, equation, …). Selected and manipulated as a unit.
- Maps to the backend block model (see Sync integration). The engine's in-memory
  document is the source of truth while editing; it is serialized to blocks.

### Inline content (rich text) — DECIDED: marks over plain text

A text node's inline content is **plain text plus structured marks**, not raw
Markdown and not embedded markup. Marks are inline-style ranges over the text:
bold, italic, underline, strikethrough, inline code, link (href), and later
color/highlight.

**Storage decision (resolves Open Question 1).** Keep the backend block's `text`
as a clean plain string (unchanged), and store formatting additively in
`data.marks` as ranges, e.g.:

```json
{ "id": "…", "type": "paragraph", "text": "hello world",
  "data": { "marks": [ { "start": 6, "end": 11, "type": "bold" },
                       { "start": 0, "end": 5, "type": "link", "href": "https://…" } ] } }
```

Rationale:

- Matches the established architecture: internal state is a structured block
  model; **Markdown is import/export only**, never the stored representation
  (see [architecture.md](architecture.md)). This is also how ProseMirror / Quill
  / AppFlowy / Google Docs work internally.
- `text` stays plain, so search, plain-text display, and existing Markdown export
  for unformatted text are unaffected; formatting is purely additive in `data`.
- Clean HTML export (`<strong>` etc.) and a natural anchor for future
  range-based features (comments, highlight colors). Storing Markdown markers in
  `text` (the rejected option) leaks markup into the plain field, complicates
  HTML export, and is ambiguous to round-trip.

Cost (accepted): mark offsets must be shifted/recomputed as text is inserted or
deleted; this is a standard, well-understood approach (e.g. Yjs uses text +
marks). The editor's in-memory model may use runs (`{text, marks}`) for
convenience and serialize to/from the `text` + `data.marks` form on load/save.

The backend Markdown and HTML exporters/importers are extended to apply and
parse these marks; an unformatted block (no `data.marks`) behaves exactly as
today.

### Positions and selection

- A **position** is `(nodePath, offset)` for text nodes, or a **node anchor** for
  void nodes (before/on/after the node).
- A **selection** is a range `(anchor, focus)` that may span:
  - within a single text node,
  - across sibling and nested text nodes (multi-block),
  - across/over void nodes (image fully inside the range is included as a unit),
  - **table cells** (rectangular cell range — see Tables).
- Selection states to support: collapsed caret, ranged text selection,
  single void-node selected, multi-node selection, table-cell-range selection,
  and "whole node" selection (e.g. a code block selected as a unit).
- The engine paints caret and selection highlight itself across all node types.

## Block / node types

Each type must define: how it renders, how the caret enters/leaves it, what
Enter/Backspace/Delete do, how it is selected, and how it (de)serializes to
Markdown and to backend blocks.

### Text nodes (have inline content)

- Paragraph
- Heading 1–6 (`attributes.level`)
- Bulleted list item
- Numbered list item (ordinal computed by position)
- To-do / checkbox (`attributes.checked`)
- Quote / blockquote
- Code block (`attributes.language`; plain text content, no inline marks; Enter
  inserts a newline, does not split)
- Callout (icon/color attribute + inline content) — later
- Toggle / collapsible heading (has children, collapsible) — later

### Void / atomic nodes (no inner text caret)

- Divider / horizontal rule
- Image (`attributes.url`, `fileId`, `width`, `alt`, `caption?`) — backed by the
  files API
- File / attachment (`fileId`, name, size)
- Embed / bookmark (url, preview) — later
- Equation / math (latex) — later

### Container nodes (have child nodes)

- Document root
- List grouping (logical; rendering groups consecutive list items)
- **Table** (see dedicated section)
- Columns / column layout — later
- Toggle children — later

## Tables (first-class, plan for it now)

Tables are the hardest case for selection/deletion; design the model to support
them even if implemented later.

- **Structure:** `table` node → `table_row` children → `table_cell` children;
  each cell is a container of text nodes (usually one paragraph). Column count,
  column widths, and header row/column flags live in `table.attributes`.
- **Caret & navigation:** caret lives inside a cell's text. `Tab` / `Shift+Tab`
  move to next/previous cell (creating a new row when tabbing past the last
  cell, like Word). `Arrow` keys move within a cell, then to the adjacent cell
  on the same visual row/column.
- **Selection:**
  - Text selection within one cell behaves normally.
  - Dragging across cells selects a **rectangular cell range** (not raw text);
    rendered as highlighted cells.
  - Selecting the whole table (e.g. via the table's drag handle / clicking its
    corner) selects the table node as a unit.
- **Deletion semantics (explicit):**
  - Delete/Backspace with a **text selection inside a cell** → deletes text only;
    never collapses the cell structure.
  - Delete with a **cell-range selection** → clears the contents of those cells,
    leaving the cells in place (Word/Notion behavior).
  - **Delete row / delete column** are explicit commands (toolbar/handle/menu),
    not a side effect of text deletion.
  - Deleting the **whole-table selection** removes the entire table node.
  - Backspace at the very start of the first cell does **not** merge the table
    into the previous block; it is a no-op (prevents accidental structure loss).
- **Editing:** insert/delete row above/below, insert/delete column left/right,
  toggle header row/column, set column width (drag), merge/split cells (later).
- **Markdown:** GitHub-Flavored Markdown pipe tables on export/import. Rich
  inline marks inside cells must round-trip; cell content that can't be
  expressed in GFM (block content inside a cell) is a documented limitation.

## Selection & deletion semantics (general)

The recurring problem the engine must get right for every node type:

- **Atomic/void node selected + Delete/Backspace** → remove the whole node
  (e.g. selected image is deleted in one keystroke). Typing with a void node
  selected replaces it with a paragraph containing the typed text.
- **Caret immediately before/after a void node** → Backspace/Delete selects the
  void node first (one press to select, second to delete), or deletes it
  directly per a chosen convention; define and keep it consistent.
- **Arrow keys across a void node** → the caret treats it as a single stop; it
  cannot land "inside."
- **Ranged selection spanning text + void + text** → Delete removes the whole
  range, including void nodes, and merges the surrounding text nodes.
- **Select-all** → selects the whole document; Delete clears to a single empty
  paragraph (never zero nodes).
- **Multi-node selection + convert/format** → applies to all fully-covered text
  nodes (e.g. make 3 paragraphs into bullets).
- The document is **never empty**: deleting everything leaves one empty paragraph.

## Editing operations

- Insert character / text at caret (with IME composition).
- Insert block; **split** a text node at the caret (Enter); **merge** with
  previous (Backspace at start) / next (Delete at end).
- Enter behavior per type: paragraph → new paragraph; list item → next item;
  empty list item → exit list to paragraph; code block → newline; heading → new
  paragraph below.
- Convert block type (turn-into) for caret block or all selected text nodes.
- Indent / outdent (`Tab` / `Shift+Tab`) for nestable lists/toggles (and table
  cell movement where applicable).
- Move block up / down; drag-and-drop reorder (with drop indicator).
- Delete: character, word, selection (see semantics above), whole block.
- Markdown **input rules** (auto-format while typing): `# `, `## `…, `- `, `* `,
  `1. `, `> `, `- [ ] `, ` ``` `, `---` (divider), `|...|` (table) at line start;
  inline rules `**b**`, `*i*`, `` `code` ``, `[text](url)`, `~~s~~`.
- Slash command (`/`) menu to insert any block type at the caret.
- Inline formatting commands: bold, italic, underline, strike, code, link
  (add/edit/remove), clear formatting.

## Text input & IME

- Implement a `TextInputClient` (or `DeltaTextInputClient`) so composition/IME
  (Chinese/Japanese/Korean, autocorrect, predictive) works on web, desktop, and
  mobile. The engine, not a `TextField`, owns the single input connection.
- Map incoming text edits/deltas to document transactions at the current
  selection.

## Keyboard

- Caret movement: Left/Right (char), Up/Down (visual line, crossing blocks),
  Home/End (line), Ctrl/Cmd+Left/Right (word), Ctrl/Cmd+Up/Down or Ctrl+Home/End
  (document start/end).
- Selection: Shift + all of the above; Ctrl/Cmd+A select-all (cell → table →
  document escalation).
- Editing: Enter, Shift+Enter (soft break), Backspace/Delete (+ word variants),
  Tab/Shift+Tab.
- Formatting: Ctrl/Cmd+B/I/U, Ctrl/Cmd+E (code), Ctrl/Cmd+K (link),
  Ctrl/Cmd+Shift+S (strike).
- History: Ctrl/Cmd+Z, Ctrl/Cmd+Shift+Z (or Ctrl+Y).
- Clipboard: Ctrl/Cmd+C/X/V.
- All bindings must respect platform modifiers (Cmd on macOS, Ctrl elsewhere).

## Pointer & touch

- Click to place caret (hit-test screen → document position).
- Click-drag to select across blocks; auto-scroll near viewport edges.
- Double-click selects word; triple-click selects block/paragraph.
- Drag across table cells → rectangular cell selection.
- Click a void node selects it.
- Hover reveals the block handle (drag-reorder + block menu) per editor.md.
- Touch: long-press to select word + selection handles; caret handle for
  repositioning. (Web is the priority surface; keep touch viable.)

## Clipboard

- **Copy/Cut** produces three flavors: internal rich format (lossless, for
  in-app paste), Markdown, and plain text; (HTML optional for external apps).
- **Paste:** prefer internal format; else parse Markdown/HTML into nodes; else
  plain text (split on blank lines into paragraphs). Paste replaces the current
  selection. Paste of an image → upload via files API, insert image node.
- Cut = copy + delete-selection.

## Undo / redo

- All mutations go through **transactions** (a list of reversible operations +
  selection before/after). Undo/redo restores document **and** selection.
- Coalesce consecutive typing into one undo step; structural ops are discrete.
- Interaction with remote sync: remote-applied changes must not be undoable by
  the local user (only the local user's own actions are on their undo stack).

## Markdown compatibility (hard requirement)

- Block-level and inline Markdown must round-trip: import file → document →
  export file is stable (idempotent), and matches what the backend's Markdown
  importer/exporter produces.
- Inline marks (bold/italic/code/strike/link) must survive round-trips.
- GFM tables, task lists, fenced code (with language), images, dividers.
- Keep the backend `export/markdown`, `export/html`, and `import/markdown`
  authoritative; the client engine's Markdown must agree with them (shared test
  fixtures).

## Backend & sync integration

- The engine edits an in-memory document; each transaction is translated to the
  existing backend block operations (`insert_block`, `update_block`,
  `delete_block`, `move_block`) and sent through the same REST/WebSocket path
  (see [Sync and API Draft](sync-and-api.md)). No new op types unless required.
- Void nodes serialize to blocks with `type` + `data` (image → `data.url`/
  `fileId`; table → row/cell child blocks). Images/files reference the files API
  records.
- Inline rich text requires a richer per-block text representation than today's
  plain string — see Open Questions.
- Remote updates reconcile into the live document **without** disturbing the
  local caret/selection or the node currently being edited (already the rule in
  the MVP; the engine must preserve it at position granularity, mapping remote
  block changes to document positions).
- Real concurrent-editing convergence (CRDT/OT) stays deferred per the MVP plan;
  the model should not preclude adding it later.

## Rendering & performance

- Custom caret + selection painting layered over node widgets/render objects.
- Virtualize long documents (build only visible nodes) while keeping selection
  and caret math correct off-screen.
- Smooth caret, selection that follows wrapping, correct hit-testing with mixed
  font sizes (headings) and inline marks.

## Accessibility & i18n

- Screen-reader semantics for nodes and selection; keyboard-only operability.
- RTL text and bidi; correct caret movement in mixed-direction text.

## Non-goals (for now)

- Infinite canvas / freeform layout.
- Real-time character-level CRDT merge (deferred).
- Comments/suggestions mode (post-MVP).

## Decisions

1. **Inline rich text storage — DECIDED.** Plain `text` stays clean; formatting
   is stored additively as ranges in `data.marks`; Markdown is import/export
   only. See "Inline content (rich text)" above for the rationale and shape.
   (Rejected: storing Markdown markers inside `text`.)

## Open questions / decisions needed

1. **Void-node deletion convention** — one-press delete vs select-then-delete
   when the caret is adjacent to a void node.
2. **Table scope for v1** — basic rows/cols + cell selection first; defer merge/
   split cells, column resize, header styling.
3. **Build order** — see Milestones.

## Milestones

1. **Core surface — IN PROGRESS.** single input connection + document model +
   caret across text nodes; replace the N-`TextField` editor for paragraph/
   heading/list/todo/quote/code. Cross-block arrow nav as a true unified caret.
   *Done:* the engine lives in `clients/mica_flutter/lib/editor/`
   (`model.dart`, `controller.dart`, `render.dart`, `editor.dart`); a single
   leaf `RenderDocument` paints all nodes + caret + selection and hit-tests
   screen→position; one `TextInputClient` owns IME; `MicaEditor` replaces
   `BlockEditor`. Caret moves L/R/Up/Down/Home/End across blocks; Enter splits,
   Backspace/Delete merge; click/drag place and extend selection across blocks;
   todo checkboxes toggle. *Needs live-browser tuning:* web IME/key interplay
   (composition, stray newlines), and caret scroll-into-view.
2. **Selection:** mouse drag + Shift-key ranged selection across blocks;
   select-all; delete-selection; copy/cut/paste (internal + Markdown + plain).
3. **Markdown input rules + slash menu — DONE.** on the new surface; inline
   marks (bold/italic/code/strike/link) stored additively over plain text in
   `data.marks` (`[start,end)` ranges; see `marks.dart`).
   *Done:* block-level input rules (`# `…`###### `, `- `/`* `, `1. `, `> `,
   ` ``` `, and `- ` then `[ ] `/`[x] ` → todo) convert the block and strip the
   marker as you type; the `/` slash menu (filter, ↑/↓, Enter/Tab to apply, Esc
   to dismiss) converts the focused block — both live in
   `controller.applyInputRules` / `applySlashCommand` and the editor's slash
   overlay. Inline marks: rendered via `buildMarkedSpan`; toggled with
   Ctrl/Cmd+B/I/E and Ctrl/Cmd+K (link) and a floating selection toolbar;
   Cmd/Ctrl+click opens a link. Inline input rules (`**b**`, `*i*`/`_i_`,
   `` `code` ``, `~~s~~`, `[t](url)`) convert and strip markers as you type
   (`controller._applyInlineRule`), preserving nested marks. Paste/AI/import run
   `parseInline`; copy/export round-trip via `inlineToMarkdown` (client) and the
   Rust mirror in `crates/app-core/src/documents.rs`.
4. **Undo/redo — DONE.** Snapshot history in `EditorController`: every committed
   change (the single `_send` choke point) pushes the prior document onto the
   undo stack and clears redo; `undo`/`redo` restore a snapshot and emit the
   block-op diff (`_diffOps`) so the backend follows — no `move_block` since the
   editor never reorders existing blocks. A debounced typing burst coalesces into
   one step (undo flushes pending first). Bound to Ctrl/Cmd+Z and
   Ctrl/Cmd+Shift+Z / Ctrl+Y in `editor._onKey`. *Pending:* toolbar buttons
   (state is controller-private; keyboard-only for now).
5. **Void nodes — DONE (divider + image).** Atomic nodes (`EditorNode.isAtomic`:
   `table`/`divider`/`image`) hold no inline caret: `caretRectFor` returns null,
   `positionAt` snaps clicks to the nearest text node, Up/Down make the block a
   whole-block caret stop (selected highlight; another press steps past), and
   Backspace/Delete from/on the block removes it. Dividers: `---`/`***`/`___`
   input rule or `/divider`. Images: content-addressed upload (sha256 object key,
   original name preserved) via `crates/api-server/.../files.rs` to RustFS;
   blocks store `file_id` (resolved to fresh signed URLs on load, never stale)
   or an external `url` (pasted Markdown links are re-hosted into our storage).
   Insert via `/image` picker, clipboard paste, or pasted URL; rendered on the
   canvas (`paintImage` + ui.Image cache); hover toolbar (expand/align/resize/
   delete), right-click menu (copy/download/delete), fullscreen viewer. Markdown
   `![alt](url)` round-trips (import/export).
6. **Tables:** structure, cell navigation/selection, row/col commands, GFM
   round-trip.
7. **Polish:** drag-reorder, virtualization, accessibility, RTL, touch.
   *Drag-reorder DONE (2026-06-05):* a 24px gutter rail on the left of every
   block (all layouts shifted: text, dividers, images, tables — todo
   checkboxes, table handles and quote bars included); hovering a block shows
   a ⠿ handle on its box edge (item children show it at their inset), grab
   cursor, drag paints a blue insertion line between blocks, drop calls
   `controller.moveBlock` → one `move_block` op (server-validated), caret
   follows the block, undo restores the order via the snapshot diff.
   *Still open:* virtualization, accessibility, RTL, touch.
8. **Markdown spec compliance** (engine roadmap; see `crates/markdown` and
   docs/export-import.md). North star: **read the full CommonMark + GFM
   spec, write a canonical subset** — full spec compliance is a *parsing*
   goal; the exporter intentionally emits only our normalized form, and
   round-trip stability (`export(import(x))` fixed point) stays the
   invariant. Phases:
   - **P1 — nested lists — DONE.** flat blocks gain a `data.indent` level (the
     editor engine stays a flat node list — a true tree would rewrite the
     render/selection core; Notion ships flat+indent too). Scope: engine
     parses leading indentation into levels and exports nested GFM (plus
     Dart-mirror parity + new conformance fixtures); editor gets
     Tab/Shift+Tab (clamped to prev sibling + 1), Enter keeps level,
     Backspace-at-start outdents first; render indents, varies bullet
     glyphs per level and restarts ordered counters per level. Imports
     stop flattening hierarchy. True container children (quote-with-code,
     callouts) come AFTER P1 on the same groundwork.
   - **P2 — backslash escapes + autolinks `<url>` — DONE.** Both parsers
     honor `\*` escapes (escape-aware emphasis closers; code spans stay
     literal) and `<uri>`/`<email>` autolinks. Both renderers escape
     literal segments and paragraph block-leaders (`\- x`, `1\. x`) so
     text can't change kind on re-import, keep code spans raw, and emit
     a link whose text equals its target as an autolink. The Dart
     renderer was rewritten to the nested form in the process (its old
     per-segment wrapping broke nested marks on copy).
   - **P3 — official CommonMark spec scoreboard — DONE.** The vendored
     0.31.2 spec (652 examples) runs through import→export_html in
     `crates/markdown/tests/commonmark_scoreboard.rs`; the per-section
     report lives in docs/commonmark-scoreboard.md (regenerate with
     GEN_SCOREBOARD=1). Baseline: **130/652 (19.9%)** — a regression
     floor assert keeps the number from ever dropping silently. Building
     it surfaced that export_html dropped inline marks entirely (now
     rendered as nested strong/em/code/del/a, +61 examples).
   - **P4 — data-driven long tail (IN PROGRESS).** Warm-up buckets DONE
     (2026-06-05): spec thematic breaks (`- - -`), setext headings,
     indented code blocks (top-level; inside lists indentation still
     means nesting), tab-stop column math, plus HTML-shape fixes
     (inner trailing newline in <pre><code>, language- class, <hr />).
     Links bucket DONE (2026-06-05): full inline grammar (angle-bracket
     destinations, balanced parens, titles — marks gained an optional
     `title` field), reference definitions (`[label]: dest "title"`,
     case-insensitive, full/collapsed/shortcut forms) with a defs
     pre-pass in both parsers; renderers emit titles and `<spaced
     dests>`. Emphasis bucket DONE (2026-06-05): the spec delimiter-stack
     algorithm (left/right flanking, `_` intraword restrictions, rule
     of 3, nearest-opener pairing with strong-before-em, delimiter
     deletion + offset remap) replaces the naive closer search in both
     parsers; parse output keeps nested same-type marks for
     `<em><em>` fidelity (editor ops still normalize). Scoreboard
     130 → 186 → 235 → 315 — Emphasis 7%→89%.
     List items bucket DONE (2026-06-05): multi-line items + lazy
     continuation (paragraph-like blocks stay "open"; indented or lazy
     lines join with a soft break, re-parsing inline marks over the
     joined source), blank + indented line → second paragraph inside
     the item (`\n\n` join, `data.loose`), loose lists (`<p>`-wrapped
     items in HTML), `+` bullets and `N)` ordered markers (≤9 digits),
     `<ol start>` via `data.start` on a run's first item, marker/delim
     changes break lists (`data.marker`), content-column nesting
     (` - b` is a sibling of `- a`, not a child), empty items, setext
     underlines convert the whole open multi-line paragraph (and
     export back in setext form). Markdown export emits loose blanks,
     continuation indentation, recorded markers/delims, and a blank
     between a list and the next block — the round-trip fixed point
     holds. Scoreboard **394/652 (60.4%)** — List items 8%→60%, Lists
     31%→69%, Paragraphs 88%. Fixtures 12–15 pin both parsers.
     Images bucket DONE (2026-06-05, 22/22 = 100%): inline images
     `![alt](url "title")` + all three reference forms ride the link
     bridge as `image` marks over the alt; inner markup is kept in the
     marks (markdown re-renders it) but flattens to plain text in the
     HTML alt attribute (spec alt rule). The whole-line direct form
     stays the image-block fast path (now with titles, angle dests,
     nested brackets); a paragraph that is exactly one image mark gets
     promoted to an image block post-pass. Ref-def labels reject
     unescaped brackets; a literal `!` before a rendered link escapes
     on export so it can't read back as an image. Image blocks export
     spec-shaped `<p><img src alt title /></p>` HTML and keep titles
     in markdown. Scoreboard 416/652 (63.8%). Fixtures 12–16 pin
     both parsers.
     Block quotes bucket DONE (2026-06-05, 25/25 = 100%) — WITHOUT
     container children: same flat-model trick as lists. Blocks inside
     quotes carry `data.quote` = marker depth (the `quote` kind IS the
     quoted paragraph, depth ≥ 2 recorded); `data.qbreak` marks a
     blank-separated new blockquote. Import strips `>` markers (one
     optional space each, ≤3 spaces between nested), joins marked AND
     lazy paragraph lines into the open quoted paragraph (partial
     markers continue the deepest paragraph, ex 251), handles fences /
     indented code / dividers / headings / lists / images inside
     quotes, `>`-only lines as paragraph breaks, and content-less
     groups as empty quote blocks. HTML rebuilds nested <blockquote>
     recursively from the depths (render_quote_group, the
     render_html_list pattern); markdown export re-prefixes every line
     with `> `×depth (blanks become bare `>`) plus a plain blank after
     each group. Editor degrade: non-paragraph blocks inside quotes
     render as their plain kinds (no quote border) until the editor
     learns `data.quote`. Also fixed: an indented (4+ col) marker can't
     start a top-level list (ex 238). Scoreboard 438/652 (67.2%),
     Block quotes 24%→100%, List items 62%. Fixtures 12–17 pin both
     parsers.
     Mid-tail sweep DONE (2026-06-05): ATX closing sequences + empty
     headings (#⇥ too) → 100%; autolinks tightened (scheme 2–32 chars,
     no backslash in emails) → 100%; spec backtick-run code spans
     (N opens/N closes, newlines→space, one-space strip; export picks
     a longer fence + pads) 77%; hard breaks (2+ trailing spaces or
     backslash-EOL canonicalize to `\`+newline in text, HTML renders
     <br />) 80%; `~~~` fences, info-string rules (first word, no
     backticks in backtick fences, entity/escape decode), fence-indent
     shedding, indented closers → Fenced 100%; indented-code blank/
     trailing-space fidelity → 100%; entity & numeric char refs (full
     numeric + curated ~170-name table both ends; unknown stay literal
     per spec) 71%; multi-line link reference definitions (dest/title
     on following lines, multi-line titles, fence-aware, can't
     interrupt a paragraph) → 81%; href percent-encoding (houdini
     set) and `'` no longer HTML-escaped; 4+-column lines are lazy
     continuation, never new blocks. Setext/Paragraphs/Soft breaks/
     Textual 100%, Emphasis 93%, Links 76%. Scoreboard 516/652
     (79.1%). Fixtures 12–19 pin both parsers.
     List-item containers DONE (2026-06-05): the flat-depth trick a
     third time — blocks inside a list item carry `data.li` = the
     owning item's indent level. Import recognizes, at or past the
     open item's content column: fenced code (running while the item
     does, content shed by the opener's column), indented code (4+
     past the content column), dividers, quotes (`data.li` composes
     with `data.quote`; the quote path keeps the list context), and —
     once an item has children — child paragraphs (text joins would
     render before the children, in the wrong order). Items whose TEXT
     opens a container (`- ```...`, `- * * *`, 4+ extra marker spaces)
     become an empty item plus a child. HTML renders children inside
     the `<li>` (quote runs rebuild <blockquote>; deeper runs recurse);
     markdown re-indents children to the content column, child
     paragraphs get a separating blank. Scoreboard 533/652 (81.7%)
     — List items 62%→88%, Lists 69%→81%, Tabs 82%. Fixture 20 pins
     both parsers.
     HTML degrade DONE (2026-06-05) — the decided policy (AFFiNE-style
     carrier + raw write-back): an HTML block (the spec's 7 start/end
     conditions, type 7 can't interrupt a paragraph) imports as
     `code_block { language: "html", raw: true }` — the editor shows
     source in the existing code-block UI and NEVER executes it; both
     exporters honor `data.raw` by writing the source back VERBATIM
     (no fences, no escaping), so foreign viewers still render the
     HTML and the spec passes. Inline raw HTML (valid tag / comment /
     PI / declaration / CDATA shapes, after autolinks) becomes an
     `html` mark over the verbatim text — HTML and markdown render it
     unescaped; invalid shapes stay escaped text. SECURITY: export
     passes raw HTML through — any Mica-side HTML RENDERING surface
     must sanitize (whitelist) before display; the editor itself only
     ever shows source. Scoreboard **598/652 (91.7%)** — HTML blocks
     0%→95%, Raw HTML 35%→100%; 16 sections at 100%. Fixture 21 pins
     both parsers.
     **GFM extensions COMPLETE (2026-06-05): 24/24 official extension
     examples** (vendored in fixtures/gfm-extensions.json, regression
     floor in tests/gfm_extensions.rs). Bare autolinks (http/https/ftp,
     `www.` → http://, bare emails; GFM boundary + trailing-punctuation
     trimming + domain rules; `www.` links write back bare), GFM
     strikethrough (one OR two tildes, run-matched), tagfilter (the nine
     disallowed tags escape on ALL raw HTML output — the security debt
     from the HTML degrade, now spec-shaped), spec tables (separator
     count must match the header, header defines the column count,
     pipe-less lazy rows, per-column `data.aligns` from separator colons
     — editor renders them, markdown writes them back, HTML emits
     GFM-shaped <table><thead><tbody> with align attrs and inline-parsed
     cells), task lists (GFM <input checkbox> shape; the `[x]` marker is
     content so two-space subtasks nest). The CommonMark scoreboard
     gains a GFM_DIALECT_WAIVERS list (608/611/612 bare autolinks,
     170–175 tagfilter) — examples cmark-gfm with extensions also fails
     — scored as 589/643 (91.6%).
     **P4 CLOSED (2026-06-05) at 598/652 = 91.7% CommonMark (now 589/643
     + 9 dialect waivers) and 24/24 GFM extensions.** The remaining 54
     are isolated precedence/exotic edges with no real-document impact
     — Links precedence (20), Emphasis rule-of-3 exotics (6), List
     items/Lists deep-container edges, code-span/entity/tab oddments —
     each would be a one-off special case; fix on demand when a real
     document hits one. The scoreboard stays a permanent regression
     floor (BASELINE_PASS = 598) and the 21 conformance fixtures pin
     both parsers; any future grammar change must keep or raise both. Decision point: keep extending
     the in-house parser vs adopting `comrak` for the *read side only*
     — decide when the in-house curve flattens.
