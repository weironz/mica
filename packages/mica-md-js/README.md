# mica-md (npm)

The Markdown ⇄ block-document engine from [Mica](https://github.com/weironz/mica),
compiled to WebAssembly. Same engine as the Rust crate
([`mica-md`](https://crates.io/crates/mica-md)) — CommonMark 0.31.2 (641/641) +
GFM (24/24) + footnotes, front matter and Pandoc math, with a round-trip
guarantee. This package is a thin object wrapper over the JSON wire format; it
contains no Markdown logic of its own.

## Install

```bash
npm install mica-md
```

## Use

```js
import { parseBody, serializeBody, renderHtml, parseInline } from 'mica-md';

const blocks = parseBody('# Title\n\nSome **bold** text.');
// [{ type: 'heading', text: 'Title', data: { level: 1 } },
//  { type: 'paragraph', text: 'Some bold text', data: { marks: [...] } }]

serializeBody(blocks);            // '# Title\n\nSome **bold** text.\n'
renderHtml(blocks);               // sanitized share HTML
parseInline('run `kubectl` now'); // { text: 'run kubectl now', marks: [...] }
```

Mark offsets are **UTF-16 code units** over the returned `text` (same as JS
string indices), so `text.slice(mark.start, mark.end)` is the marked run.

In a browser or bundler, initialize the WebAssembly build once:

```js
import { initWeb } from 'mica-md';
await initWeb();          // or initWeb(new URL('...wasm', import.meta.url))
```

Node needs no init: the nodejs build loads on first call.

## API

| function | returns |
| --- | --- |
| `parseBody(markdown, { fragment })` | `Block[]` (`fragment: true` = paste semantics) |
| `serializeBody(blocks, { assets })` | Markdown |
| `parseInline(source)` / `parseInlineBatch(sources)` | `{ text, marks }` |
| `serializeInline({ text, marks })` | inline Markdown |
| `renderHtml(blocks, { imageSrcs, profile })` | HTML (`export` \| `clipboard`) |
| `decodeCells(cells)` / `encodeCells(cells)` | table-cell storage shapes |
| `mathRunSpans(source)` / `parseInlineMath(source)` | math spans / inline math |
| `properties({ op, ... })` | front-matter properties: `parse` / `upsert` / `remove` / `infer` |
| `engineInfo()` | crate + schema version, source fingerprint |

The document model (block kinds, data keys, mark types) is specified in
[SCHEMA.md](https://github.com/weironz/mica/blob/main/crates/md-core/SCHEMA.md).

## License

`MIT OR Apache-2.0`.