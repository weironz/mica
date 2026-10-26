// Run 3 AB/BA-paired full-path rounds of all six inputs; no large matrix.
async (page) => {
 await page.waitForFunction(()=>typeof globalThis.runMdP0==='function',{timeout:180000});
 const rounds=[];
 for(let round=1;round<=3;round++) {
  rounds.push({round,...await page.evaluate(async()=>JSON.parse(await globalThis.runMdP0(
   ['p0_87','p0_10k','p0_100k','s0_87','s0_10k','s0_100k'],['full','dartMarkdown'])))});
 }
 const assets=await page.evaluate(async()=>{
  const out={};
  for(const file of ['mica_md.js','mica_md_bg.wasm']) {
   const bytes=await(await fetch('mica_md/'+file,{cache:'no-store'})).arrayBuffer();
   out[file]={rawBytes:bytes.byteLength,sha256:Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',bytes)),x=>x.toString(16).padStart(2,'0')).join('')};
  }
  return out;
 });
 return {browser:await page.evaluate(()=>navigator.userAgent),assets,rounds};
}
