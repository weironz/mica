// Read-only verification after the 3 short-input rounds; no timing rerun.
async (page) => {
  return await page.evaluate(async () => {
    const artifacts={};
    for (const file of ['mica_md.js','mica_md_bg.wasm']) {
      const bytes=await (await fetch('mica_md/'+file,{cache:'no-store'})).arrayBuffer();
      artifacts[file]={rawBytes:bytes.byteLength,sha256:Array.from(new Uint8Array(
        await crypto.subtle.digest('SHA-256',bytes)),b=>b.toString(16).padStart(2,'0')).join('')};
    }
    return {engineInfo:JSON.parse(globalThis.micaMd.engineInfo()),artifacts};
  });
}
