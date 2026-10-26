// Re-measure the RETAINED bundles, not handwritten table numbers.
// Each browser resource is compressed separately: gzip -n -9 -c < bytes.
// Keep concat-gzip as the historical P0 metric, never call it transfer total.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const zlib = require('node:zlib');
const {spawnSync} = require('node:child_process');
const variants = process.argv.slice(2);
const names = variants.length ? variants : ['base-z','opt-z','opt-s','opt-3','opt-z-uuid'];
const dir = path.resolve(__dirname, '../artifacts');
function gzip(bytes) {
  const p = spawnSync('gzip', ['-n','-9','-c'], {input:bytes, maxBuffer:16*1024*1024});
  if (p.error || p.status !== 0) throw p.error || new Error(p.stderr.toString());
  return p.stdout.length;
}
const records = Object.fromEntries(names.map(variant => {
  const parts = ['mica_md_bg.wasm','mica_md.js'].map(file => {
    const bytes = fs.readFileSync(path.join(dir,variant,file));
    return {file,rawBytes:bytes.length,gzipBytes:gzip(bytes),
      sha256:crypto.createHash('sha256').update(bytes).digest('hex'), bytes};
  });
  const record = {variant, files:parts.map(({bytes,...rest})=>rest),
    transferGzipBytes:parts.reduce((n,p)=>n+p.gzipBytes,0),
    historicalConcatGzipBytes:gzip(Buffer.concat(parts.map(p=>p.bytes))),
    historicalNodeZlib:{version:process.versions.zlib,
      wasmGzipBytes:zlib.gzipSync(parts[0].bytes,{level:9}).length,
      jsGzipBytes:zlib.gzipSync(parts[1].bytes,{level:9}).length,
      transferGzipBytes:parts.reduce((n,p)=>n+zlib.gzipSync(p.bytes,{level:9}).length,0),
      concatGzipBytes:zlib.gzipSync(Buffer.concat(parts.map(p=>p.bytes)),{level:9}).length}};
  return [variant,record];
}));
if (!variants.length) {
  const result = {method:'gzip -n -9 -c on each resource separately; raw file SHA-256 identifies the exact retained artifact',
    hardBudgetBytes:200*1024, softTargetBytes:120*1024, variants:records};
  fs.writeFileSync(path.join(__dirname,'sizes-costs.json'),JSON.stringify(result,null,2)+'\n');
}
const table=['| 配置 | wasm gzip (B) | JS gzip (B) | 分文件传输合计 (B) | 旧拼接 gzip (B) |',
  '| --- | ---: | ---: | ---: | ---: |',
  ...Object.values(records).map(r=>`| ${r.variant} | ${r.files[0].gzipBytes} | ${r.files[1].gzipBytes} | ${r.transferGzipBytes} | ${r.historicalConcatGzipBytes} |`)];
console.log(table.join('\n'));
