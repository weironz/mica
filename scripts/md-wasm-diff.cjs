// Recheck the browser capture against the real host wire, preserving raw diffs.
const fs = require('node:fs');
const path = require('node:path');
const cp = require('node:child_process');
const crypto = require('node:crypto');
const root = path.resolve(__dirname, '..');
const cases = JSON.parse(fs.readFileSync(path.join(root, 'target/md-m2-diff-inputs.json'), 'utf8'));
const canon = value => JSON.stringify(value, (_key, v) => v && typeof v === 'object' && !Array.isArray(v) ? Object.fromEntries(Object.entries(v).sort()) : v);
function normalize(value) {
  if (Array.isArray(value)) return value.map(normalize);
  if (!value || typeof value !== 'object') return value;
  const out = Object.fromEntries(Object.entries(value).map(([k, v]) => [k, normalize(v)]));
  if (Array.isArray(out.marks)) out.marks.sort((a,b) => a.start-b.start || a.end-b.end || (a.type < b.type ? -1 : a.type > b.type ? 1 : 0));
  return out;
}
const digest = s => crypto.createHash('sha256').update(s).digest('hex');
const output = cases.map(item => {
  const host = cp.execFileSync(path.join(root, 'target/debug/examples/wire_stdin.exe'), [], {input:item.source, encoding:'utf8', maxBuffer:16*1024*1024});
  const wire = JSON.parse(item.wire);
  const diffs = [];
  for (let i=0;i<Math.max(wire.length,item.dart.length);i++) {
    if (canon(wire[i]) !== canon(item.dart[i])) diffs.push({index:i,wire:wire[i],dart:item.dart[i],equalAfterFrozenMarkOrdering:canon(normalize(wire[i]))===canon(normalize(item.dart[i]))});
  }
  return {id:item.id,utf16Length:item.source.length,sourceSha256:digest(item.source),hostWireSha256:digest(host),wasmWireSha256:digest(item.wire),hostWasmByteIdentical:host===item.wire,allEqualAfterFrozenMarkOrdering:canon(normalize(wire))===canon(normalize(item.dart)),tail:item.source.slice(-180),diffs};
});
fs.writeFileSync(path.join(root,'target/md-m2-diff-summary.json'),JSON.stringify(output,null,2));
console.log(JSON.stringify(output.map(({diffs,...rest})=>({...rest,diffCount:diffs.length,firstIndices:diffs.slice(0,4).map(x=>x.index)})),null,2));
if(output.some(x=>!x.hostWasmByteIdentical))process.exitCode=1;
