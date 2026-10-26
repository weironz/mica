async (page) => {
  const samples = [];
  // Each fresh context has no HTTP cache/profile; same browser process may still
  // retain V8 wasm compilation cache. Server sends no-store and uncompressed bytes.
  for (let i = 0; i < 5; i++) {
    const context = await page.context().browser().newContext({locale:'en-US'});
    const p = await context.newPage();
    const response = await p.goto('http://127.0.0.1:8766/', {waitUntil:'domcontentloaded'});
    const measurement = await p.evaluate(async () => {
      await globalThis.micaMdReady;
      return {initMs: globalThis.micaMdInitMs, error:globalThis.micaMdError ?? null,
        isolated:crossOriginIsolated, available:typeof globalThis.micaMd?.parseBody,
        resources:performance.getEntriesByType('resource').filter(x=>x.name.includes('/mica_md/')).map(x=>({name:x.name,duration:x.duration,transferSize:x.transferSize}))};
    });
    samples.push({...measurement,headers:await response.allHeaders()});
    await context.close();
  }
  return samples;
}
