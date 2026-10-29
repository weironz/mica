# Changelog

All notable changes to `mica-md`. Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning: [SemVer](https://semver.org/), with the round-trip guarantee below
counted as part of the public API.

## What counts as a breaking change

1. **A public signature changes** — anything pinned by `tests/public_api.rs`.
2. **A schema item is removed or renamed** — a kind, data key or mark type in
   `SCHEMA.md` / `schema.rs`. (Adding one is not breaking: readers must ignore
   unknown items, and this crate does.)
3. **`serialize` output changes for an input that already worked.** The round
   trip is the product, so byte-level output stability is API. Exception: a
   bug fix — then the `Fixed` entry MUST name the input and both the old and
   the new output.
4. **`SCHEMA_VERSION` increments** (`SCHEMA.md` version policy).

## [Unreleased]

### Added

- Nothing yet.

## [0.1.0] — unreleased

Initial SDK readiness cut (single-source project S5). Not published yet;
`publish = false` stays until the user approves a release.

- `parse` / `parse_body` / `parse_inline` / `parse_inline_math` — Markdown to
  the flat block model and to inline text + marks (UTF-16 offsets).
- `serialize` / `serialize_body` / `serialize_inline` — back to Markdown, the
  normalized subset described in `SCHEMA.md`.
- `render_html` / `render_html_body` — `HtmlProfile::Export` (sanitized share
  HTML) and `HtmlProfile::Clipboard` (paste flavor, source math/Mermaid kept
  as data attributes).
- `decode_cell` / `encode_cell` — table cell storage shapes; lossless for every
  cell (`decode_cell(encode_cell(x)) == x`, see `SCHEMA.md`).
- `wire` — the JSON in/out surface shared by the FFI and wasm bindings.
- `compat` — the Mica adapter's whole-document entries (`import_markdown`,
  `export_markdown`, `export_html`, `set_image_srcs`, `split_front_matter`).
  Not part of the SDK's recommended surface; kept so the adapter compiles
  unchanged.
- `render` feature (off by default): Mermaid → SVG and math → SVG.
- `properties` — front-matter property parsing/editing.
- Guarantees: CommonMark 0.31.2 641/641, GFM 24/24; round trip stable;
  never panics on untrusted input (property-tested).