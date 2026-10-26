// Follow-up only: keep both 87-character inputs together, 3 same-page rounds.
async (page) => {
  await page.waitForFunction(() => typeof globalThis.runMdP0 === 'function', {timeout:180000});
  const rounds=[];
  for (let round=1; round<=3; round++) {
    const result=await page.evaluate(async () => JSON.parse(await globalThis.runMdP0(
      ['p0_87','s0_87'], ['full','dartMarkdown','parseBody','parseOnly'])));
    rounds.push({round,...result});
  }
  const artifacts = await page.evaluate(async () => {
    const result = {};
    for (const file of ['mica_md.js','mica_md_bg.wasm']) {
      const bytes=await (await fetch('mica_md/'+file,{cache:'no-store'})).arrayBuffer();
      const hash=Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',bytes)),
        b=>b.toString(16).padStart(2,'0')).join('');
      result[file]={rawBytes:bytes.byteLength,sha256:hash};
    }
    return result;
  });
  return {browser:await page.evaluate(()=>navigator.userAgent),artifacts,
    initMs:await page.evaluate(()=>globalThis.micaMdInitMs),rounds};
}
