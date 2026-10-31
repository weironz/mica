// The package's own gate: the nodejs wasm build through the JS wrapper.
// Asserts the same three promises the README makes — round trip, UTF-16 mark
// offsets, and sanitized HTML — so a broken wasm packaging fails here rather
// than in a downstream app.
import assert from 'node:assert/strict';
import test from 'node:test';

import {
  decodeCells,
  encodeCells,
  engineInfo,
  parseBody,
  parseInline,
  properties,
  renderHtml,
  serializeBody,
} from '../index.js';

test('parseBody / serializeBody round-trip a document', () => {
  const markdown = '# Title\n\nSome **bold** text.\n\n- one\n- two';
  const blocks = parseBody(markdown);
  assert.equal(blocks[0].type, 'heading');
  assert.equal(blocks[0].data.level, 1);
  assert.equal(blocks[1].text, 'Some bold text.');
  assert.deepEqual(
    blocks[1].data.marks.map((m) => m.type),
    ['bold'],
  );
  // No trailing newline: the writer's normalized subset, byte-stable.
  assert.equal(serializeBody(blocks), markdown);
  assert.equal(serializeBody(parseBody(serializeBody(blocks))), markdown);
});

test('mark offsets are UTF-16 code units over the text', () => {
  // CJK before the marked run makes byte offsets and UTF-16 units diverge.
  const inline = parseInline('中文 then `kubectl` here');
  assert.equal(inline.text, '中文 then kubectl here');
  const code = inline.marks.find((m) => m.type === 'code');
  assert.equal(inline.text.slice(code.start, code.end), 'kubectl');
});

test('renderHtml strips event-handler attributes and unsafe URLs', () => {
  const blocks = parseBody('# Hi\n\n<img src="https://e.dev/x" onerror="steal()">');
  const html = renderHtml(blocks);
  assert.ok(html.includes('<h1>Hi</h1>'));
  assert.ok(!html.includes('onerror'), `on* attr must be stripped: ${html}`);
  assert.ok(html.includes('https://e.dev/x'));
});

test("renderHtml: fenced code escapes, raw type-1 HTML passes through", () => {
  // Fenced: code_block, escaped on render — always inert.
  const fenced = parseBody('```\n<script>a</script>\n```');
  assert.ok(renderHtml(fenced).includes('&lt;script&gt;'));
  // Raw type-1 block (starts the LINE, CommonMark HTML block type 1):
  // passes through verbatim by design — CSP is the embedder's defense.
  const raw = parseBody('<script>a</script>\n');
  assert.ok(renderHtml(raw).includes('<script>a</script>'));
});

test('table cells: source with a literal * round-trips without emphasis', () => {
  const stored = encodeCells([{ text: '2 * 3 * 4', marks: [] }]);
  assert.equal(typeof stored[0], 'string');
  assert.ok(stored[0].includes('\\*'), 'literal * must be escaped in source form');
  const back = decodeCells(stored);
  assert.equal(back[0].text, '2 * 3 * 4');
  assert.equal(back[0].marks.length, 0);
});

test('engineInfo identifies the build', () => {
  const info = engineInfo();
  assert.equal(info.schema_version, 1);
  assert.ok(info.crate_version);
  assert.ok(info.source_fingerprint);
});

test('properties parse and edit front matter without touching other lines', () => {
  const fm = '# keep\ntitle: Old\ntags: [a, b]';
  assert.deepEqual(properties({ op: 'parse', front_matter: fm }), [
    { key: 'title', value: { type: 'text', value: 'Old' } },
    { key: 'tags', value: { type: 'list', value: ['a', 'b'] } },
  ]);
  const edited = properties({
    op: 'upsert',
    front_matter: fm,
    key: 'tags',
    value: { type: 'list', value: ['a', 'b', 'c'] },
  });
  assert.equal(edited, '# keep\ntitle: Old\ntags: [a, b, c]');
  assert.equal(properties({ op: 'remove', front_matter: edited, key: 'title' }), '# keep\ntags: [a, b, c]');
  assert.deepEqual(properties({ op: 'infer', raw: '2026-07-22' }), { type: 'date', value: '2026-07-22' });
});
