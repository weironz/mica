#!/usr/bin/env node
// Headless W2 cross-engine harness (no browser). Driven by the Rust side:
// crates/mica-core/tests/web_interop.rs spawns `node <this file> <base64>`.
//
//   argv[2]:     base64 of a yrs-written v1 state (the base)
//   stdout:      base64 of the yjs-re-encoded state after the W2 edits
//   exit != 0:   a yjs-side check failed (message on stderr)
//
// Loads the COMMITTED production bundle (web/yjs_bundle.js — the exact bytes
// the web client ships, same yjs version) rather than a fresh npm install, so
// what CI certifies is what users run. yjs is isomorphic; the IIFE just sets
// `globalThis.micaYjs` and needs no browser API.
//
// The write side below mirrors MicaYDoc (lib/web/mica_ydoc.dart)
// _updateBlock/_insertBlock — the same primitive sequence the web editor
// issues — NOT a convenience rewrite. If the doc layout changes there, change
// it here too.
const path = require('path');
require(path.join(__dirname, '..', '..', 'web', 'yjs_bundle.js'));
const Y = globalThis.micaYjs;

function fail(msg) {
  console.error(`w2_headless: ${msg}`);
  process.exit(1);
}

const baseB64 = process.argv[2];
if (!baseB64) fail('usage: node w2_headless.cjs <base64 yrs state>');
const doc = Y.docFromUpdate(new Uint8Array(Buffer.from(baseB64, 'base64')));

// ── direction 1: yjs reads what yrs wrote ────────────────────────────────────
const meta = Y.getMap(doc, 'meta');
const root = Y.mapGet(meta, 'root');
if (root !== 'root') fail(`meta.root: expected "root", got ${JSON.stringify(root)}`);

const blocks = Y.getMap(doc, 'blocks');
const seed = Y.mapGet(blocks, 'seed');
if (!seed || !Y.isMap(seed)) fail('seed block missing from yrs base');
const seedText = Y.mapGet(seed, 'text');
if (!seedText || !Y.isText(seedText)) fail('seed.text is not a Y.Text');
const s = Y.textToString(seedText);
if (s !== 'seed text here') fail(`seed text: got ${JSON.stringify(s)}`);

// The yrs-written italic mark must surface as Y.Text formatting: the first
// delta run is exactly the [0,4) italic span.
const delta = JSON.parse(Y.textDeltaJson(seedText));
const run0 = delta[0] || {};
if (run0.insert !== 'seed' || !run0.attributes || run0.attributes.italic !== true) {
  fail(`yrs-written italic mark not visible in yjs delta: ${JSON.stringify(delta)}`);
}

// The yrs-written int prop arrives as BigInt; mapEntriesJson must survive it
// (the exact path that crashed the web read side live in P4-2).
const seedProps = Y.mapGet(seed, 'props');
if (!seedProps || !Y.isMap(seedProps)) fail('seed.props is not a Y.Map');
const props = JSON.parse(Y.mapEntriesJson(seedProps));
if (props.indent !== 1) fail(`seed props.indent: got ${JSON.stringify(props)}`);

// ── W2 write side: the edits the browser self-test applies ──────────────────
// update_block on seed — text kept, marks replaced by bold [0,5), props
// rewritten from what was just read (mirrors _setTextAndMarks + _setProps).
Y.textDelete(seedText, 0, Y.textLength(seedText));
Y.textInsert(seedText, 0, s);
Y.textFormatJson(seedText, 0, 5, JSON.stringify({ bold: true }));
for (const k of Y.mapKeys(seedProps)) {
  if (!(k in props)) Y.mapDelete(seedProps, k);
}
for (const [k, v] of Object.entries(props)) {
  Y.mapSetJson(seedProps, k, JSON.stringify(v));
}

// insert_block of "w2new" under root at index 0 (mirrors _insertBlock).
const bm = Y.newMap();
Y.mapSet(blocks, 'w2new', bm);
Y.mapSet(bm, 'ty', 'paragraph');
const t = Y.newText('hello link');
Y.mapSet(bm, 'text', t);
Y.textFormatJson(t, 0, 5, JSON.stringify({ link: { href: 'http://x', title: 'T' } }));
const w2props = Y.newMap();
Y.mapSet(bm, 'props', w2props);
Y.mapSetJson(w2props, 'role', JSON.stringify('note'));
Y.mapSetJson(w2props, 'level', JSON.stringify(2));
Y.mapSet(bm, 'children', Y.newArray());

const rootBm = Y.mapGet(blocks, 'root');
if (!rootBm || !Y.isMap(rootBm)) fail('root block missing from yrs base');
const rootChildren = Y.mapGet(rootBm, 'children');
if (!rootChildren || !Y.isArray(rootChildren)) fail('root.children is not a Y.Array');
Y.arrayInsertJson(rootChildren, 0, JSON.stringify(['w2new']));

process.stdout.write(Buffer.from(Y.encodeState(doc)).toString('base64'));
