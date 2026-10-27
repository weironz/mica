// Replay the checked-in default wire vectors against the actual shipped WASM.
// Node validates the binding artifact; it does not replace Dart/browser tests.
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const assert = require('node:assert/strict');
const root = path.resolve(__dirname, '..');
const bundle = path.join(root, 'clients/mica_flutter/web/mica_md');
const vectors = path.join(root, 'crates/md-core/tests/fixtures/wire');
const bindings = {
  parse_body_json: ['parseBody', ['markdown', 'fragment']],
  parse_inline_batch_json: ['parseInlineBatch', ['sources_json']],
  decode_cells_json: ['decodeCells', ['cells_json']],
  encode_cells_json: ['encodeCells', ['inlines_json']],
  serialize_body_json: ['serializeBody', ['blocks_json', 'assets_json']],
  serialize_inline_json: ['serializeInline', ['inline_json']],
  render_html_body_json: ['renderHtmlBody', ['blocks_json', 'image_srcs_json', 'profile']],
  math_run_spans_json: ['mathRunSpans', ['source']],
  parse_inline_math_json: ['parseInlineMath', ['source']],
};
async function main() {
  vm.runInThisContext(fs.readFileSync(path.join(bundle, 'mica_md.js'), 'utf8'));
  await wasm_bindgen({module_or_path: fs.readFileSync(path.join(bundle, 'mica_md_bg.wasm'))});
  let functions = 0;
  let cases = 0;
  for (const [wire, [name, args]] of Object.entries(bindings)) {
    const gold = JSON.parse(fs.readFileSync(path.join(vectors, wire.replace(/_json$/, '') + '.json'), 'utf8'));
    assert.equal(gold.function, wire);
    assert.ok(gold.cases.length > 0, `${wire}: empty corpus`);
    for (const item of gold.cases) {
      assert.equal(wasm_bindgen[name](...args.map(arg => item.args[arg])), item.output, `${wire}: ${item.id}`);
      cases++;
    }
    functions++;
    console.log(`${wire}: ${gold.cases.length} byte-exact cases passed`);
  }
  assert.equal(functions, 9);
  const info = JSON.parse(wasm_bindgen.engineInfo());
  assert.equal(info.source_fingerprint, fs.readFileSync(path.join(bundle, 'SOURCE_FINGERPRINT'), 'utf8').trim());
  assert.equal(info.schema_version, 1);
  for (const [name, args] of [
    ['parseInlineBatch', ['{']], ['decodeCells', ['{']], ['encodeCells', ['{']],
    ['serializeBody', ['{', '{}']], ['serializeInline', ['{']],
    ['renderHtmlBody', ['[]', '{}', 'invalid']],
  ]) {
    assert.throws(() => wasm_bindgen[name](...args), error => error instanceof Error && String(error).includes('invalid input'), name);
  }
  assert.equal(JSON.parse(wasm_bindgen.parseBody('# hello', false))[0].text, 'hello');
  console.log(`receipt: ${functions} golden functions, ${cases} cases, engineInfo fingerprint, 6 errors and recovery passed`);
}
main().catch(error => { console.error(error); process.exitCode = 1; });
