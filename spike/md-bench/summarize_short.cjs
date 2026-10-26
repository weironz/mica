// Preserve all original run transcripts; write a verifiable follow-up summary
// and generate Markdown tables from the same measurements.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const dir=__dirname;
function load(file) {
 const lines=fs.readFileSync(path.join(dir,file),'utf8').split(/\r?\n/);
 return JSON.parse(lines[lines.indexOf('### Result')+1]);
}
const median=v=>[...v].sort((a,b)=>a-b)[Math.floor(v.length/2)];
const sizes=JSON.parse(fs.readFileSync(path.join(dir,'sizes-costs.json'),'utf8'));
const variants={};
for(const [variant,file] of [['opt-s','opts'],['opt-3','opt3']]) {
 const run=load(`short-${file}.txt`),served=load(`short-${file}-assets.txt`);
 for(const part of sizes.variants[variant].files) {
  assert.equal(served.artifacts[part.file].sha256,part.sha256);
  assert.equal(served.artifacts[part.file].rawBytes,part.rawBytes);
 }
 const inputs={};
 for(const key of ['p0_87','s0_87']) {
  const rounds=run.rounds.map(r=>{
   const v=r.inputs[key];
   assert.equal(v.characters,87);assert.equal(v.equivalence.equal,true);
   assert.equal(v.equivalence.differingBlocks,0);assert.equal(v.equivalence.blockCountDelta,0);
   assert.equal(JSON.parse(r.engineInfo).source_fingerprint,served.engineInfo.source_fingerprint);
   return {round:r.round,fullMs:v.stages.full.medianMs,dartMs:v.stages.dartMarkdown.medianMs,
    ratio:v.ratioFullVsDart,equivalence:v.equivalence};
  });
  inputs[key]={characters:87,rounds,medianRatio:median(rounds.map(r=>r.ratio)),
    fullMsMedian:median(rounds.map(r=>r.fullMs)),dartMsMedian:median(rounds.map(r=>r.dartMs)),
    allRoundsWithinBudget:rounds.every(r=>r.ratio<=1.2)};
 }
 variants[variant]={browser:run.browser,served,inputs};
}
const uuidRounds=[1,2,3].map(n=>load(`uuid-run${n}.txt`));
const uuid={};
for(const key of Object.keys(uuidRounds[0].inputs)) {
 uuid[key]={direction:'Sequential -> UUID; these probes differ in body extraction/root digest, so the delta is not pure RNG time',
  rounds:uuidRounds.map((r,i)=>({round:i+1,sequentialMs:r.inputs[key].stages.parseOnly.medianMs,
   uuidMs:r.inputs[key].stages.uuidParseOnly.medianMs}))};
}
const result={recommendation:'opt-3; all 6 new short-input measurements pass the unchanged 1.2x budget; G2 pending review, T2.2 not started',
 variants,uuid,notes:{stage3:'parseBody - parseOnly is only an estimate; parseOnly consumes a digest and returns f64; parseBody maps blocks, encodes JSON and returns a string. No isolated stage3 measurement exists.',
 unitCorrection:'base-z p0_100k echoJson = 1.56875 ms, not microseconds; p0_87 echoMd = 0.0007568359379 ms = 0.7568359379 microseconds',
 inputs:'s0_87 bytes match S0 hotPath87; s0_10k=9997 chars and s0_100k=99972 chars use line cuts, unlike S0 exact substring; approximate representative fixtures only',
 sizeMethod:sizes.method}};
fs.writeFileSync(path.join(dir,'summary-short.json'),JSON.stringify(result,null,2)+'\n');
const lines=['| 变体 | 输入（87 字符） | 轮次 | 完整路径 ms | Dart ms | 同轮比值 |',
 '| --- | --- | ---: | ---: | ---: | ---: |'];
for(const [variant,v] of Object.entries(variants)) for(const [key,input] of Object.entries(v.inputs))
 for(const r of input.rounds) lines.push(`| ${variant} | ${key} | ${r.round} | ${r.fullMs.toFixed(6)} | ${r.dartMs.toFixed(6)} | ${r.ratio.toFixed(6)} |`);
lines.push('','| 输入 | 轮次 | Sequential ms | UUID ms |','| --- | ---: | ---: | ---: |');
for(const [key,v] of Object.entries(uuid)) for(const r of v.rounds)
 lines.push(`| ${key} | ${r.round} | ${r.sequentialMs.toFixed(6)} | ${r.uuidMs.toFixed(6)} |`);
console.log(lines.join('\n'));
