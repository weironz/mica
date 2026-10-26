// Aggregate the P0 cost experiment: per-input medians across rounds.
// Usage: node spike/md-bench/aggregate.cjs
const fs = require('node:fs');
const path = require('node:path');

const dir = __dirname;
const stageNames = ['loopOverhead', 'wasmNoop', 'echoMd', 'echoJson', 'parseOnly',
  'uuidParseOnly', 'parseBody', 'dartDecode', 'dartMap', 'full', 'dartMarkdown'];

function loadRun(file) {
  const lines = fs.readFileSync(path.join(dir, file), 'utf8').split('\n');
  const at = lines.indexOf('### Result');
  if (at < 0) throw new Error(`no result in ${file}`);
  return JSON.parse(lines[at + 1]);
}
function loadInit(file) {
  const lines = fs.readFileSync(path.join(dir, file), 'utf8').split('\n');
  const at = lines.indexOf('### Result');
  return JSON.parse(lines[at + 1]).map((x) => x.initMs);
}
function median(values) {
  const s = [...values].sort((a, b) => a - b);
  const m = Math.floor(s.length / 2);
  return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2;
}

function aggregate(files) {
  const runs = files.map(loadRun);
  const out = {};
  for (const key of Object.keys(runs[0].inputs)) {
    const stages = {};
    for (const name of stageNames) {
      const values = runs
        .map((r) => r.inputs[key].stages[name]?.medianMs)
        .filter((v) => typeof v === 'number');
      if (values.length) stages[name] = median(values);
    }
    const dart = stages.dartMarkdown;
    out[key] = {
      characters: runs[0].inputs[key].characters,
      blocks: runs[0].inputs[key].blocks,
      jsonBytes: runs[0].inputs[key].jsonBytes,
      stages,
      ratioFullVsDart: stages.full / dart,
      equivalence: runs.map((r) => r.inputs[key].equivalence),
    };
  }
  return out;
}

const result = {
  environment: {
    rust: '1.98.0 stable (rust-toolchain.toml), wasm-bindgen crate + CLI 0.2.129, wasm32-unknown-unknown',
    flutter: '3.47.1 / Dart 3.13.1, flutter build web --release --no-web-resources-cdn',
    browser: 'HeadlessChrome/156.0.0.0, Windows 11 build 26300',
    server: 'node static host, no COOP/COEP, Cache-Control: no-store; crossOriginIsolated=false',
    method: 'Each stage measured in isolated adaptive batches (15 samples, calibrated to ~15 ms per batch), alternating order; round value is the median across 3 independent release runs, then the median across rounds. Dart comparator is the production markdownToBlocks in the same page, same release build.',
  },
  sizesGzipBytes: {
    'base-z': 102184, 'opt-z': 102346, 'opt-s': 111224, 'opt-3': 123226,
    'opt-z-uuid': 104811, threshold: 204800,
  },
  initMs: {
    'base-z': loadInit('init-base-z.txt'),
    'opt-z': loadInit('init-opt-z.txt'),
    'opt-s': loadInit('init-opt-s.txt'),
    'opt-3': loadInit('init-opt3.txt'),
  },
  initMsMedian: {},
  variants: {
    'base-z': aggregate(['base-z-run1.txt', 'base-z-run2.txt', 'base-z-run3.txt']),
    'opt-z': aggregate(['opt-z-run1.txt', 'opt-z-run2.txt', 'opt-z-run3.txt']),
    'opt-s': aggregate(['opts-run1.txt', 'opts-run2.txt', 'opts-run3.txt']),
    'opt-3': aggregate(['opt3-run1.txt', 'opt3-run2.txt', 'opt3-run3.txt']),
    'opt-z-uuid': aggregate(['uuid-run1.txt', 'uuid-run2.txt', 'uuid-run3.txt']),
  },
};
for (const [k, v] of Object.entries(result.initMs)) result.initMsMedian[k] = median(v);
fs.writeFileSync(path.join(dir, 'summary-costs.json'), JSON.stringify(result, null, 2));
console.log(JSON.stringify({
  initMedian: result.initMsMedian,
  variantRatios: Object.fromEntries(Object.entries(result.variants).map(([v, inputs]) =>
    [v, Object.fromEntries(Object.entries(inputs).map(([k, i]) => [k, +i.ratioFullVsDart.toFixed(3)]))])),
}, null, 1));
