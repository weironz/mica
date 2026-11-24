// Usage: node warm.js <url> <latency-ms> <mbps>
// Repeat visit: first load primes the HTTP cache (ETag) and V8's wasm code cache,
// then reload under throttling with the cache enabled.
const { chromium } = require('playwright');
(async () => {
  const [,, url, latency, mbps] = process.argv;
  const b = await chromium.launch();
  const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
  const page = await ctx.newPage();
  const ready = () => page.waitForFunction(() => {
    const s = document.getElementById('status'), app = document.getElementById('app');
    // Old index.html had #status; the splash version has #mica-splash outside #app.
    return (!s || s.style.display === 'none') && app && app.innerText.trim().length > 0;
  }, null, { timeout: 120000, polling: 50 });
  await page.goto(url); await ready(); await page.waitForTimeout(1500);
  const cdp = await ctx.newCDPSession(page);
  await cdp.send('Network.enable');
  await cdp.send('Network.emulateNetworkConditions', { offline: false, latency: +latency, downloadThroughput: +mbps * 1e6 / 8, uploadThroughput: +mbps * 1e6 / 8 });
  let bytes = 0; cdp.on('Network.loadingFinished', e => { bytes += e.encodedDataLength; });
  const t0 = Date.now(); await page.goto(url, { waitUntil: 'commit' }); await ready();
  console.log(JSON.stringify({ latency: +latency, mbps: +mbps, warmFirstUiSeconds: (Date.now() - t0) / 1000, transferredKiB: Math.round(bytes / 1024) }));
  await b.close();
})();
