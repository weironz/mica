// Rebuild final JSON + the S2 P0 execution record from retained raw runs.
// Only edits the existing execution-record subsection, never the stage plan.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const dir=__dirname;
const load=f=>{const l=fs.readFileSync(path.join(dir,f),'utf8').split(/\r?\n/);return JSON.parse(l[l.indexOf('### Result')+1]);};
const med=v=>[...v].sort((a,b)=>a-b)[Math.floor(v.length/2)];
const sizes=JSON.parse(fs.readFileSync(path.join(dir,'sizes-costs.json'),'utf8'));
const history=JSON.parse(fs.readFileSync(path.join(dir,'summary-costs.json'),'utf8'));
const full=load('final-opt3-paired.txt'),short=load('final-opts-short-paired.txt'),ids=load('final-ids-paired.txt');
const checkAssets=(run,variant)=>{
 for(const f of sizes.variants[variant].files) assert.equal((run.assets||run.artifacts)[f.file].sha256,f.sha256);
};
checkAssets(full,'opt-3');checkAssets(short,'opt-s');
function summarize(run){const out={};for(const key of Object.keys(run.rounds[0].inputs)) {
 const rounds=run.rounds.map(r=>{const i=r.inputs[key];assert(i.equivalence.equal);assert.equal(i.equivalence.differingBlocks,0);assert.equal(i.equivalence.blockCountDelta,0);
 return {round:r.round,fullMs:i.stages.full.medianMs,dartMs:i.stages.dartMarkdown.medianMs,ratio:i.ratioFullVsDart,equivalence:i.equivalence};});
 out[key]={characters:run.rounds[0].inputs[key].characters,rounds,medianRatio:med(rounds.map(x=>x.ratio)),medianFullMs:med(rounds.map(x=>x.fullMs)),medianDartMs:med(rounds.map(x=>x.dartMs)),allRoundsPass:rounds.every(x=>x.ratio<=1.2)};
}return out;}
const crypto=require('node:crypto');
const matchedWasm=fs.readFileSync(path.resolve(dir,'../artifacts/matched-z-uuid/mica_md_bg.wasm'));
assert.equal(ids.wasm.sha256,crypto.createHash('sha256').update(matchedWasm).digest('hex'));
const final={method:'AB/BA alternating batch samples; each run ratio is full median / Dart median; aggregate ratio is median of those per-run ratios, never ratio of aggregate stage medians.',opt3:summarize(full),optsShort:summarize(short),uuid:{},sizes,
 engineInfo:JSON.parse(full.rounds[0].engineInfo),servedOpt3Assets:full.assets,servedOptsAssets:short.artifacts,
 historicalUuidInvalid:true,sourceCoreCommit:'6594f12',baseCoreCommit:'12c7ff2',officialDelivery:false};
for(const key of Object.keys(ids.rounds[0].inputs)){
 const rounds=ids.rounds.map(r=>({round:r.round,sequentialMs:r.inputs[key].stages.sequentialParse.medianMs,uuidMs:r.inputs[key].stages.uuidParse.medianMs}));
 final.uuid[key]={direction:'Sequential -> UUID; identical parse API, Document mode, root, and digest including root; ID strategy (including uniqueness bookkeeping) only variable',rounds,
  medianSequentialMs:med(rounds.map(x=>x.sequentialMs)),medianUuidMs:med(rounds.map(x=>x.uuidMs))};
}
final.uuidArtifact={...ids.wasm,engineInfo:JSON.parse(ids.rounds[0].engineInfo)};
final.recommendation='opt-3; all six inputs pass all three rounds; G2 pending review; G1 unresolved; no T2.2+';
fs.writeFileSync(path.join(dir,'summary-final.json'),JSON.stringify(final,null,2)+'\n');
const fmt=n=>n.toFixed(6),ratio=n=>n.toFixed(3);
const lines=[`### P0 成本归因与候选优化（T2.1，2026-10-27，复核收尾）`,
'',`**结论：推荐 opt-level=3。** 修正采样与汇总方法后，opt-3 在原 P0 三档及三个代表性输入上，每档三轮全部满足完整路径 ≤同页 Dart×1.2；体积和初始化也满足硬预算。G2 仍待正式审核，G1 未裁决，未开始 T2.2+；本轮无新增 core 优化。`,
'',`**分支/提交**：\`ss/w/p0-costs\` 基于 S1 候选 \`12c7ff2\`；核心候选单独提交 \`6594f12\`，上一轮实验 \`8a2af3f\`。只修改 spike 与本执行记录；未 merge/push。`,
'',`#### 方法、输入与纠错`,
'',`- 真实 Flutter 3.47.1 / Dart 3.13.1 release，临时入口启动真实 app.main()；同页 Chrome 156 对照生产 markdownToBlocks。Rust 1.98.0 stable、wasm32-unknown-unknown、wasm-bindgen crate/CLI 0.2.129，无 nightly/-Z build-std。`,
`- 最终 full/Dart 采样采用 **AB/BA 交替的 15 组批次**，预热后校准批量；同页连续三轮。run_paired.cjs 为 opt-3 全六输入，run_short.cjs 为 opt-s 两个短输入。orderedSamples 保留原采样顺序。旧 run_stages 是整阶段固定顺序，旧报告声称交替不实，现纠正；旧原始数据保留。`,
`- 汇总比值为 median(run1 full/Dart, run2 full/Dart, run3 full/Dart)。aggregate.cjs 原先错误地计算“阶段中位数之比”，已重算全部历史 summary-costs.json，保留 perRunRatios 及 legacyRatioOfStageMedians；旧 opt-3 原 P0 比值 0.736/0.451/0.514 作废，更正为 0.782/0.447/0.531（仍是固定顺序历史数据，最终以本节 AB/BA 数据为准）。`,
`- 原 P0 输入保持 87/10,000/100,000 字符原字节，spike/inputs/p0_*.md 为证；s0_87 与 S0 hotPath87 原字节一致。s0_10k 实际 **9,997** 字符、s0_100k **99,972** 字符，按行边界截断；S0 使用精确 substring 截断，故两者仅为**近似同形代表性复核**，不是 S0 官方基线或完全同输入。`,
`- 性能样本外比较真实规范化块与 marks：丢弃 id/children、对象键排序、marks 按(start,end,type)排序。最终全部样本 differingBlocks=0、blockCountDelta=0；初轮基线对象键次序误报保留在原数据并明确作废，不修改原数据。尚不能外推其他语义输入。`,
`- 服务器由 preview_start(md-p0-static) 启动，无 COOP/COEP、Cache-Control:no-store；crossOriginIsolated=false。`,
'',`#### 最终完整路径：每轮实测（ms）`,
'',`| 配置 | 输入/实际字符 | 轮次 | full ms | Dart ms | 同轮比值 |`, `| --- | --- | ---: | ---: | ---: | ---: |`];
for(const [name,inputs] of [['opt-3',final.opt3],['opt-s',final.optsShort]]) for(const [key,i] of Object.entries(inputs)) for(const r of i.rounds)
 lines.push(`| ${name} | ${key} / ${i.characters} | ${r.round} | ${fmt(r.fullMs)} | ${fmt(r.dartMs)} | ${ratio(r.ratio)} |`);
lines.push('',`| 配置 | 输入 | 三轮同轮比值中位数 | 每轮均 ≤1.2 |`,`| --- | --- | ---: | --- |`);
for(const [name,inputs] of [['opt-3',final.opt3],['opt-s',final.optsShort]]) for(const [key,i] of Object.entries(inputs))
 lines.push(`| ${name} | ${key} | ${ratio(i.medianRatio)} | ${i.allRoundsPass?'通过':'否（保留超线单轮）'} |`);
lines.push('',`opt-s 短输入的前一次固定窗口补测出现 p0_87=1.204819 的单轮，原记录 short-opts.txt 保留；最终 AB/BA 数字如上。推荐 opt-3 的延迟余量，不再要求用户在 s/3 间重新选择。`,
'',`#### 历史配置比较（纠正为同轮比值中位数，固定窗口采样）`,
'',`| 配置 | p0_87 | p0_10k | p0_100k | s0_87 | s0_10k（近似） | s0_100k（近似） |`,`| --- | ---: | ---: | ---: | ---: | ---: | ---: |`);
for(const name of ['base-z','opt-z','opt-s','opt-3']) lines.push(`| ${name} | ${['p0_87','p0_10k','p0_100k','s0_87','s0_10k','s0_100k'].map(k=>history.variants[name][k]?ratio(history.variants[name][k].ratioFullVsDart):'未测').join(' | ')} |`);
lines.push('',`#### 阶段成本与单位修正（历史独立探针，非可加和账本）`,
'',`- base-z 的 p0_100k：parseOnly **37.2 ms**；parseBody **44.0 ms**；echoJson **1.56875 ms（不是 µs）**；Dart jsonDecode **1.3375 ms**；BlockSpec 映射 **0.659375 ms**；full **45.8 ms**。87 字符 echoMd=0.000756835938 ms=**0.756835938 µs**。空循环约0.00002 ms，noop约0.000012 ms。`,
`- opt-z p0_100k：parseOnly 20.1 ms、parseBody 23.5 ms、full 25.8 ms；历史 opt-3 parseOnly 10.95 ms、parseBody 12.7 ms、full 15.1 ms。opt-3 没有独立测 echo/decode/map，旧表沿用 opt-z 值至 opt-3 的列现撤销，不当实测。`,
`- **Stage 3 尚无孤立测量。** parseBody−parseOnly 仅是粗估：parseOnly 含结果 digest 并返回 f64，而 parseBody 包含 BlockJson 映射/排序/JSON 编码及 JSON 字符串返回，两者并不同构。差值不可称精确编码耗时，禁止与 echo/decode/map 相加。分配/GC/热身使各阶段不可加和。`,
'',`#### 匹配 UUID/Sequential（方向 Sequential → UUID）`,
'',`旧 +48% 使用了 parse_body（含索引/移动）对 parse（含root digest）的不对称探针，**纯 UUID 归因作废**，旧 uuid-run*.txt 原样保留。新 parseIdDigest 两边使用同一 parse、Document mode、root="root"、同一含 root digest，仅 BlockIds::Sequential{prefix:"b"} 与 Uuid 不同；AB/BA 三轮。差值是完整 ID 策略（含唯一性 bookkeeping）成本，仍不是随机数系统调用的单独成本。`,
'',`| 输入 | 轮次 | Sequential ms | UUID ms |`,`| --- | ---: | ---: | ---: |`);
for(const [key,v] of Object.entries(final.uuid)) for(const r of v.rounds) lines.push(`| ${key} | ${r.round} | ${fmt(r.sequentialMs)} | ${fmt(r.uuidMs)} |`);
lines.push('',`#### 体积与初始化（最终可复算口径）`,
'',`measure_sizes.cjs 对保留的每个实际产物执行 **GNU gzip 1.14 -n -9 -c**，按 wasm/JS **分别压缩后相加**作为分文件传输总量；sizes-costs.json 记录各文件 SHA-256、原始 bytes、Node zlib 历史值及拼接压缩值。下面表格由该 JSON 自动生成：`,
'',`| 配置 | wasm gzip B | JS gzip B | 分文件传输合计 B | 历史拼接 gzip B（GNU） | init 中位 ms |`,`| --- | ---: | ---: | ---: | ---: | ---: |`);
for(const [name,r] of Object.entries(sizes.variants)) lines.push(`| ${name} | ${r.files[0].gzipBytes} | ${r.files[1].gzipBytes} | ${r.transferGzipBytes} | ${r.historicalConcatGzipBytes} | ${history.initMsMedian[name]?.toFixed(1)||'未测'} |`);
const s=sizes.variants['opt-3'];
lines.push('',`旧表合计 102184/102346/111224/123226/104811 是 **Node 24.12.0 zlib 的拼接压缩**，而单列是各文件 Node 压缩大小，故不能相加对应。旧 Node 的 opt-3 分文件总量为 ${s.historicalNodeZlib.transferGzipBytes} B，旧拼接值 ${s.historicalNodeZlib.concatGzipBytes} B；本次 GNU 分文件总量 **${s.transferGzipBytes} B**（${(s.transferGzipBytes/1024).toFixed(3)} KiB），超过120×1024软目标 **${s.transferGzipBytes-120*1024} B**，仍远低于200×1024硬预算。压缩器实现差异也已分开，未宣称是产物版本变化。`,
`所有体积都含 echo/noop/parseOnlyDigest 等诊断导出，为**本 spike 的体积上界**，不是正式 §3.3 十API产物体积；正式 S2 必须重新验收。新 ID 探针只在 matched-z-uuid 产物中，最终性能复测 opt-3/opt-s 使用原保留产物，served SHA 与 sizes JSON 校验一致。`,
`核心源码 fingerprint=${final.engineInfo.source_fingerprint} 对应6594f12，字节哈希在summary-final.json；无源码/wasm不匹配。`,
'',`#### 回执、范围与待审核`,
'',`- 本轮新增/修正仅 spike 与本执行记录，核心6594f12未再改动。final host receipt **4 passed（floor 4）**：7输入原Rust wire JSON SHA-256黄金、marks排序、UUID/Sequential块等价、匹配探针消费值等价。shellcheck、just check-structure、check-ignored-sources通过（receipt-final.txt）。此前核心55 passed/1 ignored、facade185 passed保留，非本轮重复跑大套件。`,
`- **推荐 opt-3：硬预算全部达标，G2待审核正式判定；G1仍阻塞，T2.2未启动。** 不问用户重复选配置，不合并/推送/发版。Firefox/Safari、桌面FFI及正式十API wasm体积未验证；stage3独立绝对成本仍未验证，粗估仅供理解。`,
`- 本轮raw数据：final-opt3-paired.txt、final-opts-short-paired.txt、final-ids-paired.txt；summary-final.json与sizes-costs.json为最终汇总。旧数据不删除，不回写观测值。`,'');
const receipt=fs.readFileSync(path.join(dir,'receipt-final.txt'),'utf8');
assert(receipt.includes('receipt: 4 passed (floor 4)'));
assert(receipt.includes('file-size gate: passed'));
assert(receipt.includes('no ignored source files'));
const doc=path.resolve(dir,'../../docs/single-source/stages/S2-bindings.md');
const text=fs.readFileSync(doc,'utf8');
const marker='### P0 成本归因与候选优化';
const start=text.indexOf(marker);assert(start>=0);
fs.writeFileSync(doc,text.slice(0,start)+lines.join('\n'));
console.log(JSON.stringify({opt3:Object.fromEntries(Object.entries(final.opt3).map(([k,v])=>[k,v.medianRatio])),opts:Object.fromEntries(Object.entries(final.optsShort).map(([k,v])=>[k,v.medianRatio])),gzip:s.transferGzipBytes},null,2));
