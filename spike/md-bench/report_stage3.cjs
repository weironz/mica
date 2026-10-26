// Append this independent experiment without rewriting the prior P0 record.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),crypto=require('node:crypto');
const dir=__dirname;
const l=fs.readFileSync(path.join(dir,'stage3-opt3-raw.txt'),'utf8').split(/\r?\n/);
const raw=JSON.parse(l[l.indexOf('### Result')+1]);
const med=v=>[...v].sort((a,b)=>a-b)[Math.floor(v.length/2)];
const out={method:'Rust internal performance.now intervals; parsing/reference wire/clone prep untimed; projection normalizes owned cloned blocks; serde_json::to_string measured separately; full output consumed and compared outside timers; JS return excluded',
 mirror:'spike-only wire projection mirror, production core unchanged; every sample byte-equal to real wire JSON',
 compiler:'Rust 1.98.0 stable, wasm32-unknown-unknown, opt-level 3, LTO, one codegen unit; wasm-bindgen 0.2.129',
 browser:raw.browser,crossOriginIsolated:raw.crossOriginIsolated,artifacts:raw.artifacts,
 timerRounds:raw.rounds.map(r=>r.timer),inputs:{}};
for(const name of ['mica_md.js','mica_md_bg.wasm']) {
 const data=fs.readFileSync(path.resolve(dir,'../artifacts/stage3-opt3',name));
 assert.equal(crypto.createHash('sha256').update(data).digest('hex'),raw.artifacts[name].sha256);
}
for(const key of ['p0_87','p0_10k','p0_100k']) {
 const rounds=raw.rounds.map(r=>{
  const c=r.cases[key];
  assert(c.samples.every(x=>x.wireBytesEqual));
  const expected=fs.readFileSync(path.resolve(dir,'../inputs',key+'.md'),'utf8');
  assert.equal(c.characters,expected.length);
  return {round:r.round,iterations:c.iterations,projectionMs:c.projectionMedianMs,encodeMs:c.encodeMedianMs,
   wireByteEqual:c.samples.every(x=>x.wireBytesEqual),jsonBytes:c.samples[0].jsonBytes,
   blocks:c.samples[0].blocks,engineInfo:JSON.parse(c.samples[0].engineInfo)};
 });
 out.inputs[key]={characters:raw.rounds[0].cases[key].characters,rounds,
 projectionMedianMs:med(rounds.map(x=>x.projectionMs)),encodeMedianMs:med(rounds.map(x=>x.encodeMs))};
}
const receipt=fs.readFileSync(path.join(dir,'receipt-stage3.txt'),'utf8');
assert(receipt.includes('receipt: 6 passed (floor 6)'));
fs.writeFileSync(path.join(dir,'summary-stage3.json'),JSON.stringify(out,null,2)+'\n');
const f=n=>n.toFixed(9);
const lines=['### P0 Stage 3 孤立计时补充（T2.1，2026-10-27）','',
'此前 Stage 3 的差值估计不足以完成原任务，本节新增独立计时，不覆盖 f706c9b 的原始观测。只改 spike 与执行记录，core6594f12没有新增变化；正式S2仍停，G2待审核。','',
'**实现/方法**：spike/md-wasm-p0/src/stage3.rs 复现私有 wire 投影helper（投影镜像探针，非生产改动），将拥有所有权的Block移成BlockJson，执行相同null→{}与稳定marks排序。每次调用先parse_body一次准备被测blocks；另调真实wire生成参考JSON。两次解析、深clone输入准备、外层预分配均在计时外。内部以导入performance.now量两段：①投影/规范化（含内部Vec分配、id/children销毁），②serde_json::to_string编码（含String分配）；编码返回的完整String保留，计时结束后每字节参与checksum并与真实wire JSON逐字节相等才接受数据。JS只收到小metrics JSON；完整文档字符串跨JS返回不在编码计时中。没有缓存解析结果替代生产路径。','',
'真实Flutter release仍由preview_start静态托管、Playwright Chrome156打开，noCOOP/COEP且crossOriginIsolated=false。本轮只调用JS诊断导出进入Rust，不改Flutter Dart生产路径。opt-level=3/LTO/单codegen unit、stable1.98.0/wasm-bindgen0.2.129；原P0输入87/10000/100000字节保持。每档每轮3个预热调用+15个计时调用，各调用内87字符重复8192次、1万128次、10万16次；三轮原始数据全部保存。','',
'| 输入/实际字符 | 轮次 | 每调用批次数 | 投影/规范化 ms/次 | Rust JSON编码 ms/次 | wire字节等价 |',
'| --- | ---: | ---: | ---: | ---: | --- |'];
for(const [key,c]of Object.entries(out.inputs))for(const r of c.rounds)
 lines.push(`| ${key}/${c.characters} | ${r.round} | ${r.iterations} | ${f(r.projectionMs)} | ${f(r.encodeMs)} | 通过 |`);
lines.push('','| 输入 | 三轮投影中位 ms | 三轮编码中位 ms |','| --- | ---: | ---: |');
for(const[k,v]of Object.entries(out.inputs))lines.push(`| ${k} | ${f(v.projectionMedianMs)} | ${f(v.encodeMedianMs)} |`);
lines.push('','**计时器开销原始数据**（不扣减）：每轮在Rust循环调用performance.now 100000次，包含导入调用/循环/累加，总时长与摊薄值如下。','',
'| 轮次 | 总时长 ms | call+loop ms/次 | 最小非零tick ms |','| --- | ---: | ---: | ---: |');
out.timerRounds.forEach((t,i)=>lines.push(`| ${i+1} | ${f(t.totalMs)} | ${f(t.callLoopMs)} | ${f(t.minNonzeroTickMs)} |`));
lines.push('',
'**限制**：这是相同投影的镜像探针，不能代替未来正式wire代码防漂移；host测试7个输入逐字节比真实wire，每次浏览器调用也全量比对。内部批量保留多个投影/JSON输出，内存驻留、allocator/缓存/GC与单次完整调用不同；clone准备不计时但会改变缓存热度。只对稳定batch取中位，不能把这些数与parse/boundary/Dart阶段简单相加。performance.now约0.1ms量化，通过批量摊薄，不把底噪中位数相减作精确校正。所有映射均消费，JSON String为完整字节比较/校验，不是只数长度；计时段不包括JSON输出销毁与JS返回。','',
'**验证/产物关联**：receipt-stage3.txt为**6 passed（floor6）**（先前4项+7输入投影镜像wire字节等价+null/tie排序检查），shellcheck/structure/ignored-source门禁通过。实际served wasm/JS SHA-256与保留stage3-opt3产物核对一致，core fingerprint记录在summary-stage3.json。新增诊断导出产物仅用于stage3，未冒充原123073B预算产物。','',
'数据入口：spike/md-bench/stage3-opt3-raw.txt、summary-stage3.json、receipt-stage3.txt；复现脚本run_stage3.cjs/report_stage3.cjs、探针src/stage3.rs。Stage3归因缺口至此由独立测量补齐；正式S2仍未启动。','');
const doc=path.resolve(dir,'../../docs/single-source/stages/S2-bindings.md');
const text=fs.readFileSync(doc,'utf8'),marker='### P0 Stage 3 孤立计时补充';
const start=text.indexOf(marker);
fs.writeFileSync(doc,(start<0?text.trimEnd()+'\n\n':text.slice(0,start))+lines.join('\n'));
console.log(JSON.stringify({inputs:out.inputs,timer:out.timerRounds},null,2));
