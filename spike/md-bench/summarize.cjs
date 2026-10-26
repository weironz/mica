// Compact a playwright-cli run transcript into the numbers we report.
// Usage: node spike/md-bench/summarize.cjs <file> [<file> ...]
const fs = require('node:fs');

function load(file) {
  const lines = fs.readFileSync(file, 'utf8').split('\n');
  const at = lines.indexOf('### Result');
  if (at < 0) throw new Error(`no result in ${file}`);
  return JSON.parse(lines[at + 1]);
}

const stageNames = ['loopOverhead', 'wasmNoop', 'echoMd', 'echoJson', 'parseOnly',
  'uuidParseOnly', 'parseBody', 'dartDecode', 'dartMap', 'full', 'dartMarkdown'];

for (const file of process.argv.slice(2)) {
  const data = load(file);
  const rows = {};
  for (const [key, input] of Object.entries(data.inputs)) {
    const stages = {};
    for (const name of stageNames) {
      if (input.stages[name]) stages[name] = input.stages[name].medianMs;
    }
    rows[key] = {
      characters: input.characters,
      blocks: input.blocks,
      jsonBytes: input.jsonBytes,
      stages,
      ratio: input.ratioFullVsDart,
      eq: {
        equal: input.equivalence.equal,
        differingBlocks: input.equivalence.differingBlocks,
        blockCountDelta: input.equivalence.blockCountDelta,
      },
    };
  }
  console.log(JSON.stringify({
    file,
    browser: data.browser,
    isolated: data.isolated,
    initMs: data.initMs,
    uuidAvailable: data.uuidAvailable,
    engineInfo: data.engineInfo,
    inputs: rows,
  }, null, 1));
}
