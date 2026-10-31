// The npm face of the same engine the rest of Mica runs: wasm-bindgen exports
// speak JSON strings (one batch = one call), and this module turns them into
// ordinary object/array interfaces. No parsing logic lives here — if something
// in this file ever looks like Markdown grammar, it belongs in the engine.
//
// Two builds ship, both produced by scripts/build-md-npm.sh from crates/md-wasm:
//   dist/node/  --target nodejs   (CommonJS glue, loaded by Node)
//   dist/web/   --target web      (ESM glue with an async init that fetches the
//                                  .wasm; for bundlers/browsers)
// Node resolves the nodejs build; the web build is exposed for bundler users
// through `initWeb`.

import { createRequire } from 'node:module';

/**
 * @typedef {{start: number, end: number, type: string, href?: string, title?: string}} Mark
 * @typedef {{text: string, marks: Mark[]}} InlineText
 * @typedef {{type: string, text: string, data: object}} Block
 * @typedef {'export'|'clipboard'} HtmlProfile
 */

const require = createRequire(import.meta.url);

/** @type {any} */
let native = null;

/**
 * Load the Node build. Idempotent; `parseBody` and friends call it on first use
 * so most callers never need to.
 */
export function init() {
  if (native) return native;
  native = require('./dist/node/mica_md.js');
  return native;
}

/**
 * Load the Web build (fetches `dist/web/mica_md_bg.wasm`). Call once before
 * any other function in a browser/bundler context.
 * @param {string} [wasmUrl] optional explicit URL to the .wasm file
 */
export async function initWeb(wasmUrl) {
  const mod = await import('./dist/web/mica_md.js');
  await mod.default({ module_or_path: wasmUrl });
  native = mod;
  return native;
}

function engine() {
  return native ?? init();
}

/**
 * Parse Markdown into blocks.
 * @param {string} markdown
 * @param {{fragment?: boolean}} [options] `fragment: true` treats a leading
 *   `---` as a thematic break instead of front matter (paste semantics).
 * @returns {Block[]}
 */
export function parseBody(markdown, options = {}) {
  return JSON.parse(engine().parseBody(markdown, Boolean(options.fragment)));
}

/**
 * Parse inline Markdown into text plus marks.
 * Mark offsets are UTF-16 code units over the returned `text`.
 * @param {string} source
 * @returns {InlineText}
 */
export function parseInline(source) {
  return parseInlineBatch([source])[0];
}

/**
 * Batch inline parse — one engine call for many strings (table cells, a
 * paste's lines).
 * @param {string[]} sources
 * @returns {InlineText[]}
 */
export function parseInlineBatch(sources) {
  return JSON.parse(engine().parseInlineBatch(JSON.stringify(sources)));
}

/**
 * Serialize blocks back to Markdown.
 * @param {Block[]} blocks
 * @param {{assets?: Record<string, string>}} [options] file id → destination.
 * @returns {string}
 */
export function serializeBody(blocks, options = {}) {
  return engine().serializeBody(JSON.stringify(blocks), JSON.stringify(options.assets ?? {}));
}

/**
 * Serialize one text+marks run back to inline Markdown.
 * @param {InlineText} inline
 * @returns {string}
 */
export function serializeInline(inline) {
  return engine().serializeInline(JSON.stringify(inline));
}

/**
 * Render blocks to HTML.
 * @param {Block[]} blocks
 * @param {{imageSrcs?: Record<string, string>, profile?: HtmlProfile}} [options]
 *   `profile: 'clipboard'` keeps source math/Mermaid as data attributes;
 *   the default `'export'` is sanitized share HTML.
 * @returns {string}
 */
export function renderHtml(blocks, options = {}) {
  return engine().renderHtmlBody(
    JSON.stringify(blocks),
    JSON.stringify(options.imageSrcs ?? {}),
    options.profile ?? 'export',
  );
}

/**
 * Decode stored table cells (`string | {text, marks} | null`) to text + marks.
 * @param {Array<string|object|null>} cells
 * @returns {InlineText[]}
 */
export function decodeCells(cells) {
  return JSON.parse(engine().decodeCells(JSON.stringify(cells)));
}

/**
 * Encode text + marks back to the stored cell shapes.
 * @param {InlineText[]} cells
 * @returns {Array<string|object>}
 */
export function encodeCells(cells) {
  return JSON.parse(engine().encodeCells(JSON.stringify(cells)));
}

/**
 * Character spans of every inline math run in `source` (UTF-16 units).
 * @param {string} source
 * @returns {Array<[number, number]>}
 */
export function mathRunSpans(source) {
  return JSON.parse(engine().mathRunSpans(source));
}

/**
 * Inline text plus math marks from a source line (paste detection).
 * @param {string} source
 * @returns {InlineText}
 */
export function parseInlineMath(source) {
  return JSON.parse(engine().parseInlineMath(source));
}

/**
 * Front-matter properties. `request` is one of
 * `{op: "parse", front_matter}` → `Property[]`,
 * `{op: "upsert", front_matter, key, value}` / `{op: "remove", front_matter, key}`
 * → the new front matter (string), `{op: "infer", raw}` → `PropertyValue`.
 * Front matter is the raw inner text, without the `---` fences.
 * @param {object} request
 * @returns {unknown}
 */
export function properties(request) {
  return JSON.parse(engine().properties(JSON.stringify(request)));
}

/**
 * Engine identity: crate version, schema version, source fingerprint.
 * @returns {{crate_version: string, schema_version: number, source_fingerprint: string}}
 */
export function engineInfo() {
  return JSON.parse(engine().engineInfo());
}