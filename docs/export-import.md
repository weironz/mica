# Export / Import Design

How Mica moves content in and out: single pages, whole workspaces, and
archives from other tools (Notion). Everything is dependency-free — the ZIP
writer/reader, the DEFLATE inflater and the GBK table are all in-house.

## Architecture: the `mica-interchange` engine

All heavy data work is Rust. The pure, I/O-free engine lives in
`crates/interchange` (zip reader/writer, inflate, name decoding, archive
normalization, Notion adaptation, ordering, `plan_import`); the api-server
executes its plans (database writes, S3 uploads). The Flutter client only
picks/pack-uploads archives and polls progress — bulk imports of tens of
thousands of pages never depend on the browser tab.

```
client: pick files/folder ──pack STORE zip──┐
client: pick .zip ──────────as-is───────────┤
                                            ▼
                    POST /api/workspaces/import   (job id)
                                            │
              interchange::plan_import (pure: tree, order, titles)
                                            │
        api-server executes: pages (pre-generated view ids), assets→S3,
        image refs → {file_id,name}, .md links → mica://page/<id>
                                            │
                    GET /api/import/jobs/{id}     (progress)
```

## Formats at a glance

| Action | Format | Entry point (UI) |
|---|---|---|
| Export one page | `.md` / `.zip` (md + `assets/`) | page title menu (▾) |
| Export a workspace | hierarchical Markdown ZIP | workspace ⋮ → Export (ZIP) |
| Import into an existing workspace | `.md` files, `.zip`, or a whole folder | workspace ⋮ → Import ▸ Files / Folder |
| Import as a new workspace | `.zip` (Mica or Notion export) | dropdown footer → Import workspace ▸, or Settings → Data |

## Workspace export (`GET /api/workspaces/{id}/export.zip`)

Implemented in `crates/api-server/src/routes/documents.rs`
(`export_workspace_zip`).

- **Layout mirrors the page tree.** Each page is `<ancestors…>/<page>.md`; a
  page with children also names a folder (`Guide.md` + `Guide/Setup.md`).
  Pre-order traversal; siblings ordered by view `position`.
- **`manifest.json`** is the first entry: every page path in tree order plus
  its title. This is what lets import restore sibling order — folder layout
  alone cannot carry it.

  ```json
  {
    "version": 1,
    "generator": "mica",
    "pages": [
      {"path": "Guide.md", "title": "Guide"},
      {"path": "Guide/Setup.md", "title": "Setup"}
    ]
  }
  ```

- **Images** are de-duplicated globally (by object key) into a root
  `assets/` folder, referenced with the right relative depth
  (`../assets/x.png` from `Guide/Setup.md`). Original filenames are kept,
  sanitized by `safe_file_name` (unicode letters/digits and `-_.` kept,
  other runs collapsed to `_`) and uniqued with `-2`, `-3` suffixes.
- **Page content** is `# <title>\n\n<body>`; the body comes from
  `export_markdown_with_assets` (`crates/app-core/src/documents.rs`), which
  prefers the bundled asset path for an image, then its name, then its URL.
  Images that were never re-hosted (external links) stay standard
  `![alt](https://…)` markdown.
- **ZIP writer** (`crates/interchange/src/zip/writer.rs`): STORE-only (markdown and
  already-compressed images don't benefit from deflate), UTF-8 name flag
  (0x0800) set so CJK names survive.

Naming collisions among siblings are resolved by `unique_zip_path`
(`Page.md`, `Page-2.md`). Path segments go through `safe_segment`.

## Import pipeline (server-side)

All three client entry points (ZIP / loose files / folder) end up as one
archive upload; the engine (`crates/interchange/src/import.rs`) normalizes
(`normalize.rs`: peel wrappers, drop OS junk, expand nested `Part-N.zip`)
and plans; the executor (`crates/api-server/src/routes/import.rs`) then:

1. **Reads `manifest.json`** if present and orders pages (`order.rs`):
   manifest order first (restores sibling order — view `position` is
   UUIDv7, so creation order *is* sibling order), then unknown files
   parents-first + natural-sorted (`2 < 10`, digit runs compared
   numerically).
2. **Walks each page's folder chain.** A folder maps to the page exported as
   `<folder>.md` (via `folder_page_index`); a folder with no matching page
   becomes an **empty directory page**, so hand-organized trees keep their
   hierarchy. Every page's view id is pre-generated, so links can target
   pages created later (forward references).
3. **Resolves image references** per page with `resolve_ref`: relative to
   the md's own folder first, then the archive root; `./`, `../` chains and
   percent-encoded names all work. Matching files are uploaded **once each**
   straight to S3 (deduped again by sha256 object keys) and the block is
   rewired to Mica's `{file_id, name}` image form. External URLs stay
   links.
4. **Skips a leading `# H1`** that duplicates the page title (the export
   prepends one; round-trips must not double it).

### Archive normalization (`normalize.rs`)

- `normalize_entries` — drops `__MACOSX/`, `Thumbs.db` and **any path with
  a dot-segment** (`.obsidian/`, `.git/`, `.trash/`, `.DS_Store`, AppleDouble
  `._*` — an md inside a dot-folder must not become a page); then repeatedly
  peels a single top-level folder when no file sits beside it (zipped-folder
  habit, macOS Finder archives, Notion's `Export-<id>/` shell). Mica exports
  are never peeled: `manifest.json` always sits at the root.
- Non-`.md` files are never imported as content: they only sit in a lookup
  table and are uploaded when an image block references them by relative
  path. Unreferenced files are dropped. The client folder picker
  pre-filters to md/image/json extensions so huge unrelated files are never
  read from disk.
- `expand_nested_zips` — unpacks archives nested one level deep (Notion's
  whole-workspace exports ship `Part-N.zip` inside the outer ZIP).

### ZIP reader capabilities

`zip/reader.rs` is central-directory-driven and handles what real-world
tools produce:

| Feature | Why it matters |
|---|---|
| STORE + DEFLATE | Mica exports are STORE; everything else (zip CLI, Explorer, Finder, Notion) is DEFLATE. Inflater: `zip/inflate.rs`, RFC 1951, puff.c-style |
| data descriptors (flag bit 3) | streamed writers leave local sizes zero; central dir has the truth |
| ZIP64 markers | server-generated archives (Notion) write `0xffffffff` sizes/offsets with the real values in the `0x0001` extra and the zip64 EOCD |
| entry-name encodings | precedence: UTF-8 flag → Info-ZIP Unicode Path extra (0x7075) → strict UTF-8 → **GBK** (`zip/names.rs`, generated cp936 table) — Windows Explorer on a CJK locale sets no flag. GBK pairs that masquerade as valid UTF-8 are caught by rejecting decodes landing in U+0180–U+03FF when GBK decodes cleanly |

## Notion adaptation (`notion.rs`)

Notion's "Markdown & CSV" export is itself a markdown ZIP, so it shares the
pipeline. Everything Notion-specific is isolated in one module and applied
only when **Notion mode** is on:

- forced by the "From Notion" menu entry, or
- auto-detected by `looks_like_notion_export`: at least half the md files (and
  ≥1) carry an ID suffix — a standard archive with one hash-named file is
  not mangled.

What the mode changes:

- `strip_notion_id` removes the trailing 32-hex or dashed-UUID ID from page
  titles, directory-page names and the workspace name.
- `folder_page_index(notion: true)` matches folders to pages **modulo IDs**:
  Notion names folders plainly (`apple/`) but pages with the suffix
  (`apple 31f5<…>.md`). Without this every folder imported twice (an empty
  directory page plus a duplicate root page).

Not adapted (by design, for now): database CSVs are ignored; inter-page
links inside md bodies stay as exported.

## Page links

Inside the workspace, inter-page links are standard markdown link marks
whose href uses the internal scheme `mica://page/<viewId>` (id-based, so
renaming a page never breaks links). The `[[` picker is editor input UX
only — it inserts a normal link and never persists in the document.

At the interchange boundary the scheme disappears entirely:

- **Export** rewrites `mica://page/<id>` hrefs to relative `.md` paths
  (`[Setup](Guide/Setup.md)`, `[Alpha](../Alpha.md)`) via
  `rewrite_page_links` — final zip paths are decided before the content pass
  so forward links resolve. Links to pages outside the archive keep their
  `mica://` href.
- **Import** pre-generates every page's view id before inserting anything,
  so link rewiring sees the complete path → view map (forward references
  included). A link href that resolves (via `resolve_ref`, so `../` chains
  and percent-encoding work) to an imported `.md` becomes
  `mica://page/<newViewId>`. This also picks up Notion's internal links
  (`Page%20<id>.md`).

## Permanent image links

Import/export interacts with the image architecture
(`docs/architecture.md`): blocks store `file_id` + `name`, never URLs.
Copying content out of Mica emits permanent capability URLs
(`GET /api/workspaces/{ws}/files/{id}/blob` → 307 to a fresh presigned URL),
so pasted markdown keeps working outside Mica.

## Known limitations

- The archive is buffered in server memory during import (route body limit
  1 GiB); streaming ingestion is future work.
- Nested archives are expanded one level only.
- Non-UTF-8, non-GBK entry-name code pages (Shift-JIS, EUC-KR…) fall back to
  lossy UTF-8.
- Notion CSV databases don't become tables/pages yet.

## Tests

- Rust: `cargo test -p mica-interchange` — binary fixtures
  (`tests/fixtures/*.zip`, generated by Python zlib/zipfile) cover ZIP64,
  data descriptors, GBK names, the 0x7075 extra field, nested Part zips,
  wrapper peeling, Notion matching, ordering, ref resolution and full plan
  integration; `cargo test -p mica-api-server` covers the export side.
- E2E: import fixtures through `POST /workspaces/import`, export back, and
  verify order/links/image bytes round-trip.
