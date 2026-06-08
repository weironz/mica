// P2-M4 (web→yjs): the browser-side CRDT engine for Mica web.
//
// Web can't run our Rust `yrs` core (the FFI isn't available in the browser), so
// the web client uses the JS `yjs` library instead — which is byte-for-byte
// wire-compatible with `yrs` at the update / state-vector / lib0 encoding level
// (verified: a real yrs-produced base decodes cleanly in yjs). This exposes a
// flat helper API on `globalThis.micaYjs` so Dart (`lib/web/yjs_interop.dart`)
// can drive yjs over `dart:js_interop` without binding every yjs class.
//
// Build (produces ../../web/yjs_bundle.js, committed):
//   cd tool/yjs && npm install && npm run build
import * as Y from 'yjs';

globalThis.micaYjs = {
  // ── lifecycle / sync primitives (mirror MicaDocument's FFI surface) ──
  newDoc: () => new Y.Doc(),
  docFromUpdate: (u8) => {
    const d = new Y.Doc();
    Y.applyUpdate(d, u8);
    return d;
  },
  applyUpdate: (doc, u8) => {
    try {
      Y.applyUpdate(doc, u8);
      return true;
    } catch (_) {
      return false;
    }
  },
  encodeState: (doc) => Y.encodeStateAsUpdate(doc),
  encodeStateVector: (doc) => Y.encodeStateVector(doc),
  encodeDiff: (doc, sv) => Y.encodeStateAsUpdate(doc, sv),
  transact: (doc, fn) => doc.transact(fn),

  // ── shared-type access (read side) ──
  getMap: (doc, name) => doc.getMap(name),
  mapGet: (m, k) => m.get(k),
  mapSet: (m, k, v) => m.set(k, v),
  mapHas: (m, k) => m.has(k),
  mapDelete: (m, k) => m.delete(k),
  mapKeys: (m) => Array.from(m.keys()),
  // Field-level props (P2-M4.7): set/read a nested Y.Map's entries as JSON, so
  // concurrent edits to different props keys converge. JSON.parse yields plain
  // JS values (yjs stores them like yrs `Any`).
  mapSetJson: (m, k, jsonStr) => m.set(k, JSON.parse(jsonStr)),
  mapEntriesJson: (m) => JSON.stringify(Object.fromEntries(m.entries())),

  // ── nested-type constructors (write side, for W2) ──
  newMap: () => new Y.Map(),
  newText: (s) => new Y.Text(s),
  newArray: () => new Y.Array(),

  // ── Y.Text ──
  textToString: (t) => (t && typeof t.toString === 'function' ? t.toString() : ''),
  textLength: (t) => (t ? t.length : 0),
  textInsert: (t, i, s) => t.insert(i, s),
  textDelete: (t, i, len) => t.delete(i, len),
  textFormat: (t, i, len, attrs) => t.format(i, len, attrs),
  // delta is [{insert, attributes?}, ...] — drives marks reconstruction in Dart.
  textDelta: (t) => t.toDelta(),
  // JSON-bridged variants so Dart passes/reads structured values as strings
  // instead of building/inspecting JS objects over js_interop.
  textDeltaJson: (t) => JSON.stringify(t.toDelta()),
  textFormatJson: (t, i, len, attrsJson) => t.format(i, len, JSON.parse(attrsJson)),

  // ── Y.Array ──
  arrayToList: (a) => (a && typeof a.toArray === 'function' ? a.toArray() : []),
  arrayLength: (a) => (a ? a.length : 0),
  arrayInsert: (a, i, items) => a.insert(i, items),
  arrayInsertJson: (a, i, itemsJson) => a.insert(i, JSON.parse(itemsJson)),
  arrayDelete: (a, i, len) => a.delete(i, len),

  // ── type guards ──
  isText: (x) => x instanceof Y.Text,
  isMap: (x) => x instanceof Y.Map,
  isArray: (x) => x instanceof Y.Array,
};
