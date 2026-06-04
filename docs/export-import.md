# Export / Import Design

How Mica moves content in and out: single pages, whole workspaces, and
archives from other tools (Notion). Everything is dependency-free — the ZIP
writer/reader, the DEFLATE inflater, SHA-256 and the GBK table are all
in-house (see `prefer in-house over dependencies` in the project philosophy).

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
- **ZIP writer** (`crates/api-server/src/zip.rs`): STORE-only (markdown and
  already-compressed images don't benefit from deflate), UTF-8 name flag
  (0x0800) set so CJK names survive.

Naming collisions among siblings are resolved by `unique_zip_path`
(`Page.md`, `Page-2.md`). Path segments go through `safe_segment`.

## Import pipeline

All three client entry points feed one shared core,
`_importEntriesInto` in `clients/mica_flutter/lib/main.dart`:

```
ZIP file ──readZip──┐
loose .md files ────┤→ normalizeZipEntries → expandNestedZips → tree import
picked folder ──────┘   (peel wrappers,       (inner Part-N.zip)
                         drop OS junk)
```

The tree import then:

1. **Reads `manifest.json`** if present and orders pages with
   `orderPagePaths` (`lib/upload/import_order.dart`): manifest order first
   (restores sibling order — view `position` is UUIDv7, so creation order
   *is* sibling order), then unknown files parents-first + natural-sorted
   (`2 < 10`, digit runs compared numerically).
2. **Walks each page's folder chain.** A folder maps to the page exported as
   `<folder>.md` (via `folderPageIndex`); a folder with no matching page
   becomes an **empty directory page**, so hand-organized trees keep their
   hierarchy.
3. **Resolves image references** per page with `resolveZipPath`
   (`lib/upload/unzip.dart`): relative to the md's own folder first, then
   the archive root; `./`, `../` chains and percent-encoded names all work.
   Matching files are uploaded **lazily, once each** (server dedups again by
   sha256) and the block is rewired to Mica's `{file_id, name}` image form.
   External URLs (`http(s):`, `data:`) stay links.
4. **Skips a leading `# H1`** that duplicates the page title (the export
   prepends one; round-trips must not double it).

### Archive normalization (`lib/upload/unzip.dart`)

- `normalizeZipEntries` — drops `__MACOSX/`, AppleDouble `._*`, `.DS_Store`,
  `Thumbs.db`; then repeatedly peels a single top-level folder when no file
  sits beside it (zipped-folder habit, macOS Finder archives, Notion's
  `Export-<id>/` shell). Mica exports are never peeled: `manifest.json`
  always sits at the root.
- `expandNestedZips` — unpacks archives nested one level deep (Notion's
  whole-workspace exports ship `Part-N.zip` inside the outer ZIP).

### ZIP reader capabilities

`readZip` is central-directory-driven and handles what real-world tools
produce:

| Feature | Why it matters |
|---|---|
| STORE + DEFLATE | Mica exports are STORE; everything else (zip CLI, Explorer, Finder, Notion) is DEFLATE. Inflater: `lib/upload/inflate.dart`, RFC 1951, puff.c-style |
| data descriptors (flag bit 3) | streamed writers leave local sizes zero; central dir has the truth |
| ZIP64 markers | server-generated archives (Notion) write `0xffffffff` sizes/offsets with the real values in the `0x0001` extra and the zip64 EOCD |
| entry-name encodings | precedence: UTF-8 flag → Info-ZIP Unicode Path extra (0x7075) → strict UTF-8 → **GBK** (`lib/upload/gbk.dart`, generated cp936 table) — Windows Explorer on a CJK locale sets no flag. GBK pairs that masquerade as valid UTF-8 are caught by rejecting decodes landing in U+0180–U+03FF when GBK decodes cleanly |

Web platform note: no 64-bit shifts anywhere (dart2js); u64 reads use
multiplication.

## Notion adaptation (`lib/upload/notion.dart`)

Notion's "Markdown & CSV" export is itself a markdown ZIP, so it shares the
pipeline. Everything Notion-specific is isolated in one module and applied
only when **Notion mode** is on:

- forced by the "From Notion" menu entry, or
- auto-detected by `looksLikeNotionExport`: at least half the md files (and
  ≥1) carry an ID suffix — a standard archive with one hash-named file is
  not mangled.

What the mode changes:

- `stripNotionId` removes the trailing 32-hex or dashed-UUID ID from page
  titles, directory-page names and the workspace name.
- `folderPageIndex(notion: true)` matches folders to pages **modulo IDs**:
  Notion names folders plainly (`apple/`) but pages with the suffix
  (`apple 31f5<…>.md`). Without this every folder imported twice (an empty
  directory page plus a duplicate root page).

Not adapted (by design, for now): database CSVs are ignored; inter-page
links inside md bodies stay as exported.

## Permanent image links

Import/export interacts with the image architecture
(`docs/architecture.md`): blocks store `file_id` + `name`, never URLs.
Copying content out of Mica emits permanent capability URLs
(`GET /api/workspaces/{ws}/files/{id}/blob` → 307 to a fresh presigned URL),
so pasted markdown keeps working outside Mica.

## Known limitations

- ZIP import reads the whole archive in memory (browser); multi-GB exports
  are untested.
- Nested archives are expanded one level only.
- Non-UTF-8, non-GBK entry-name code pages (Shift-JIS, EUC-KR…) fall back to
  lossy UTF-8.
- Notion CSV databases don't become tables/pages yet.

## Tests

- Rust: `cargo test -p mica-api-server` (zip writer, `safe_segment`,
  `unique_zip_path`, …)
- Dart: `flutter test` — `unzip_test.dart` (reader incl. ZIP64 /
  data-descriptor / GBK / nested fixtures, wrapper peeling, ref resolution),
  `inflate_test.dart` (against Python zlib output), `import_order_test.dart`
  (manifest ordering, natural sort), `notion_test.dart` (detection,
  ID stripping, folder↔page matching in both modes).
