# Editor Design Principles

This is the UX north star. The full capability spec for the in-house editor —
selection model, all operations, tables, images/void nodes, clipboard, undo,
Markdown compatibility — lives in [Editor Engine Design](editor-engine.md).

## Core Principle

The document model is block-based, but the editing surface must not look or feel
block-based. In edit mode the page is one continuous document — like Typora or
Microsoft Word — and the user writes prose, not structured blocks.

Blocks are an internal data structure (for storage, operations, history, and
sync). They are an implementation detail of the backend and the sync protocol.
They are **not** a UI metaphor that the writer should ever have to think about.

> The reader/writer sees a document. The system sees blocks. The two must never
> leak into each other.

## What "no block trace" means

In the editing surface, by default there must be **no** persistent block chrome:

- No drag handles, grip dots, or `⋮` menus sitting next to every line.
- No "Add block" / "+" buttons occupying space in the page flow.
- No visible block borders, separators, backgrounds, or padding boxes that
  reveal where one block ends and the next begins.
- No per-block toolbars or type selectors shown at rest.
- No "block selected" outlines during normal text editing.

The cursor moves through the whole document as if it were a single rich text
field. Paragraphs, headings, lists, quotes, and code read as one flowing page
with normal document typography and spacing — nothing that says "this is a grid
of cells."

## How structure is created instead

Structure comes from writing, not from clicking block controls:

- **Markdown shortcuts** transform the current line as you type: `# ` → heading,
  `- ` / `* ` → bullet, `1. ` → numbered, `> ` → quote, `- [ ] ` → to-do,
  ` ``` ` → code. The marker text disappears and the line becomes that style.
- **Enter** continues the flow: new paragraph, or the next list item; an empty
  list item on Enter exits the list. **Backspace** at the start of a line merges
  it upward. This is exactly the Word/Typora muscle memory.
- Affordances that don't fit inline writing (changing an existing block's type,
  inserting an image, etc.) are reached through **transient, on-demand UI** —
  a slash command (`/`) at the cursor, or controls that appear only on hover and
  vanish otherwise — never persistent page furniture.

## Why

- Writing flow is the product. Every visible control is a small interruption;
  in aggregate they turn "writing" into "operating an outliner."
- The block model needs to stay free to evolve (nesting, new types, CRDT) without
  the UI promising users a particular block-grid mental model.
- It keeps Web, and later desktop/mobile, honest: the same seamless surface on
  every platform, with blocks living only beneath it.

## Implications for implementation

- The editor renders the block list as a continuous column with document
  typography; block kind only changes inline styling, not visible boundaries.
- Block-management controls (reorder, convert, delete, insert) must be
  hover-revealed or summoned (slash menu), not laid out in the page at rest.
- Selection, cursor movement, and keyboard behavior should approximate a single
  document: Enter/Backspace/markdown shortcuts handle structure so the writer
  rarely needs explicit block commands.
- Persistence and sync still operate per block (see
  [Sync and API Draft](sync-and-api.md)); none of that is exposed in the surface.

## Current status

The client now runs on the **in-house single-surface engine** (Milestone 1 of
[editor-engine.md](editor-engine.md)), replacing the old one-`TextField`-per-block
editor. A single `RenderDocument` paints the whole document — text, caret, and
selection — and one `TextInputClient` owns IME. The caret travels the document
as one continuous page:

- Unified caret across blocks: Left/Right/Up/Down/Home/End, click to place,
  click-drag to select **across** blocks (the long-standing single-line-only
  limitation is gone).
- Enter splits the current block (empty list item exits the list); Backspace at
  line start and Delete at line end merge blocks; todo checkboxes toggle.
- Per-kind typography with no resting block chrome.
- **Markdown shortcuts** convert the block as you type (`# `…`###### `, `- `/`* `,
  `1. `, `> `, ` ``` `, `- ` then `[ ] ` → to-do) and strip the marker.
- The **slash menu** (`/`) is summoned at the caret — filter by typing, ↑/↓ to
  move, Enter/Tab to apply, Esc to dismiss — to convert the current block.

Being layered on next: inline rich text (marks: bold/italic/code/link) and
inline Markdown rules, then undo/redo, void nodes (image/divider), tables — see
the milestones in [editor-engine.md](editor-engine.md). Web IME/key interplay and
caret scroll-into-view still need live-browser tuning.

## Out of scope (for now)

- Inline rich text (bold/italic/links as styled spans) is planned as **marks over
  plain text** on the in-house engine (Milestone 3), not via a third-party editor
  core. Block-level structure works today; inline formatting is the next layer.
- Conflict-free concurrent typing (CRDT/OT) is deferred per the MVP plan.
