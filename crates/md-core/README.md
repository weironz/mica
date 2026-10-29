# mica-md

A Markdown ⇄ block-document engine. `parse` reads Markdown into a flat block
table (the model block editors keep); `serialize` writes it back. The round
trip is the product: **export → re-import loses nothing** — structure,
footnote labels, list tightness, even literal `*` in an unmarked table cell
come back exactly as they went in.

One implementation powers every surface of [Mica](https://github.com/weironz/mica):
the server renders and exports, the CLI imports, MCP writes documents, the
desktop editor calls it over FFI and the web editor over wasm. This crate is
that engine, extracted.

## Conformance

- **CommonMark 0.31.2: 641/641** (11 GFM-era waivers excluded — see the
  scoreboard test)
- **GFM extensions: 24/24** (tables, strikethrough, task lists, autolinks)
- Dialect extensions: GFM footnotes, YAML front matter, Pandoc `$…$`/`$$…$$`
  math, alert callouts

The scoreboard runs as a regression test
(`cargo test -p mica-markdown --test commonmark_scoreboard`); the per-section
report is [docs/architecture/commonmark-scoreboard.md](../../docs/architecture/commonmark-scoreboard.md)
in the monorepo.

## Quick start

```rust
use mica_md::{parse_body, parse_inline, render_html, serialize_body, HtmlOptions, ParseMode};

// Markdown → blocks → Markdown (round trip).
let blocks = parse_body("# Title\n\nSome **bold** text.", ParseMode::Document);
assert_eq!(blocks[0].kind, "heading");
let md = serialize_body(&blocks, &Default::default());
assert_eq!(md, "# Title\n\nSome **bold** text.");

// Inline: text + marks, not source.
let inline = parse_inline("run `kubectl` now");
assert_eq!(inline.text, "run kubectl now");
assert_eq!(inline.marks[0].kind, "code");

// HTML.
let html = render_html(&parse_body("# Hi", ParseMode::Document).into_iter().collect(), &HtmlOptions::default());
```

`ParseMode::Document` treats a leading `---` as front matter;
`ParseMode::Fragment` treats it as a thematic break (paste semantics).

## The document model

[SCHEMA.md](SCHEMA.md) is the normative schema: block kinds, data keys, mark
types, the table-cell storage shapes, and the version policy.

**Read this twice: mark offsets are UTF-16 code units**, `[start, end)`, not
byte offsets — the same coordinate system as JavaScript and Dart strings, so
browser editors and this engine agree without conversion. Rust callers slicing
`str` must convert boundaries (`str::encode_utf16`, see the `inline` example).

## Feature flags

- `uuid` (default): block ids are `block_<uuid-v4>`. Without it, ids come from
  the caller-supplied `BlockIds::Sequential` strategy — deterministic, no RNG.
- `render` (default off): Mermaid → SVG (merman) and math → SVG (ratex). Adds
  heavy dependencies; server-side rendering only. Clipboard HTML never needs
  it.

## Guarantees

- **Round-trip stability**: for the inputs the conformance corpus covers,
  `serialize(parse(md))` is byte-stable across versions. A change to
  serialized output is a **breaking change** (see CHANGELOG) unless it is a
  bug fix, in which case the CHANGELOG names the input and both outputs.
- **Never panics on input**: the parsers are property-tested against
  arbitrary input (`never_panic.rs`); panics are bugs.
- **HTML sanitization, and its honest limits**: both profiles strip `on*`
  event-handler attributes and `<meta http-equiv=refresh>`, run URLs through a
  `safe_url` check (no `javascript:`), and apply the GFM `tagfilter` to raw
  HTML. But a raw **type-1** block (`<script>`, `<style>`, `<pre>`,
  `<textarea>`) is passed through verbatim so the round trip is exact — the
  output is inert only under a Content-Security-Policy, and a CSP is the
  **embedder's** job (Mica ships one; `SAFE_HTML_CSP` lives in the
  `mica-markdown` adapter, not this crate). `HtmlProfile::Clipboard`
  additionally keeps source math and Mermaid as data attributes instead of
  rendering them.
- **Unknown keys are ignored**: embedders may add their own `data` keys; this
  engine round-trips them untouched.

## Bindings

- **wasm / npm**: `packages/mica-md-js` wraps the same engine
  (`crates/md-wasm`), `--target web` and `--target nodejs` builds.
- **FFI**: no binding ships here; Mica's desktop app bridges it through
  flutter_rust_bridge. The wire format (`mica_md::wire`) is JSON-in/JSON-out
  and shared by wasm and FFI, so any language can bind the same surface.
- **Dart / Flutter**: intentionally not published (see the monorepo's
  single-source project docs) — Mica's `lib/markdown/markdown_codec.dart`
  facade is the in-repo consumer and the future starting point if a package
  is ever wanted.

## MSRV, License

MSRV: Rust 1.85 (workspace `rust-version`). License: `MIT OR Apache-2.0`.
Dependency licenses: [THIRD_PARTY.md](THIRD_PARTY.md) — both feature sets are
permissive (MIT / Apache-2.0 families), no copyleft.