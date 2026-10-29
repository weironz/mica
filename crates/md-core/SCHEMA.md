# The block-document schema

This is the normative description of the document model `mica-md` parses
Markdown into and serializes it back from. It is enforced against the code by
`tests/schema_doc_sync.rs`, which parses the tables below and asserts they
match the constants in `src/schema.rs` one-for-one — a change on either side
fails the test.

The model: a **flat block table** plus **UTF-16 inline marks**. There is no
nested syntax tree; structure (list depth, quote depth) rides on `data` keys.
This is the same shape block editors (ProseMirror-class editors) keep, which
is why the round trip Markdown → blocks → Markdown can be made lossless.

## Schema version

`SCHEMA_VERSION = 1`.

Version policy:

- The version increments **only** when a document that already exists would be
  *read differently* under new rules — that is, when old data changes meaning.
- Adding a block kind, data key, or mark type does **not** increment the
  version; readers are required to ignore what they don't know (the exporters
  already do). Additions go in `CHANGELOG.md`.
- Bumping the version means `parse` rejects (or migrates) older payloads and
  is a **breaking change** under the changelog rules.

## Block kinds (`Block::kind`)

| kind | text holds | data keys |
| --- | --- | --- |
| `page` | usually empty | `front_matter`, `title` |
| `paragraph` | inline text | `marks`, `li`, `quote`, `qbreak` |
| `heading` | inline text | `level` (1–6), `marks`, `quote`, `qbreak` |
| `bulleted_list` | inline text | `indent`, `loose`, `marker`, `marks`, `quote`, `qbreak` |
| `numbered_list` | inline text | `indent`, `loose`, `marker`, `start`, `marks` |
| `todo` | inline text | `checked`, `indent`, `loose`, `marks` |
| `quote` | inline text | `quote` (depth), `qbreak`, `alert`, `li`, `marks` |
| `code_block` | raw source | `language`, `raw`, `li`, `quote`, `qbreak` |
| `math_block` | LaTeX source | — |
| `image` | alt text | `url`, `title`, `file_id`, `name` |
| `table` | — | `rows`, `header`, `align`, `aligns`, `widths`, `width` |
| `divider` | — | `li` |
| `footnote_def` | definition body | `label`, `marks` |

Read-only aliases the exporters accept but the parser never emits:
`bullet_list` = `bulleted_list`, `number_list` = `numbered_list`,
`code` = `code_block`.

The parser emits a root block of kind `paragraph` (Mica's convention is
`page`; the adapter rewrites it). Consumers must identify the root by
`root_block_id`, not by kind.

## Mark types (`Mark::kind`)

`bold`, `italic`, `strike`, `code`, `link` (`href`, optional `title`;
zero-width labels allowed), `image` (inline image), `math`, `html` (raw inline
HTML), `footnote` (a reference).

Mark offsets are **UTF-16 code units**, half-open `[start, end)` — the same
coordinate system as JavaScript and Dart strings, so browser editors and this
engine agree without conversion. Rust callers slicing `str` must convert
boundaries (see the `inline` example).

## Data keys

| key | on | meaning |
| --- | --- | --- |
| `front_matter` | root | raw YAML front matter |
| `title` | root | page title (Mica adapter) |
| `marks` | text blocks | inline marks array |
| `indent` | list items | list nesting level |
| `loose` | list items | blank line inside the list (GFM loose) |
| `marker` | list items | bullet character |
| `start` | numbered list | first item number |
| `checked` | todo | checkbox state |
| `quote` | quote / any | quote depth |
| `qbreak` | quote | quote-group break |
| `alert` | quote | GFM alert type (`note`, `tip`, `important`, `warning`, `caution`) |
| `li` | container children | list-item container level |
| `level` | heading | 1–6 |
| `language` | code_block | fence language |
| `raw` | code_block | raw HTML block: no fences on export |
| `url` | image | destination |
| `title` | image | title |
| `file_id` | image | Mica content-addressed asset id |
| `name` | image | display name |
| `rows` | table | cell storage (below) |
| `header` | table | header row present |
| `align` | table | whole-table cell alignment |
| `aligns` | table | per-column alignment from separator colons |
| `widths` / `width` | table | editor-only column weights / table width fraction |
| `label` | footnote_def | definition label |

Unknown keys and unknown kinds are **ignored**, not errors — embedders may
store their own keys alongside these (Mica's editor stores table geometry).

## Table cell storage (`rows`)

`data.rows` is `[[cell, …], …]`. Each cell is one of:

| shape | meaning | reading |
| --- | --- | --- |
| JSON string | unmarked cell, or legacy documents: **inline Markdown source** | `decode_cell` parses it |
| `{"text": …, "marks": […]}` | marked cell, or an unmarked cell whose source would not read back exactly (`marks: []`) | taken directly |
| `null` / absent / other | empty cell | empty |

`encode_cell` writes the object form when the cell has marks. An unmarked cell
is written as escaped source — so a literal `*` round-trips without becoming
emphasis — but only when re-reading that source gives the same cell; a bare
URL (which would gain a link mark) or trailing whitespace (which would be
trimmed) is written as `{"text": …, "marks": []}` instead. So
`decode_cell(encode_cell(x)) == x` for every `x`.