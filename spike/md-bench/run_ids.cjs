// Same parse API/root/mode/digest: BlockIds is the only variable, AB/BA.
async (page) => {
 await page.waitForFunction(()=>typeof globalThis.runMdP0==='function',{timeout:180000});
 const rounds=[];
 for(let round=1;round<=3;round++) rounds.push({round,...await page.evaluate(async()=>JSON.parse(await globalThis.runMdP0(
  ['p0_87','p0_10k','p0_100k'],['sequentialParse','uuidParse'])))});
 const hashes=await page.evaluate(async()=>{
  const bytes=await(await fetch('mica_md/mica_md_bg.wasm',{cache:'no-store'})).arrayBuffer();
  return {rawBytes:bytes.byteLength,sha256:Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',bytes)),x=>x.toString(16).padStart(2,'0')).join('')};
 });
 return {wasm:hashes,rounds};
}
