// Usage: node cold.js <url> <latency-ms> <mbps> [screenshot.png]
// Cold start under CDP throttling: time from navigation to the first rendered app UI
// (nsh mounts DOM under #app and hides #status). Cache disabled = first visit.
const { chromium } = require('playwright');
(async () => {
  const [,, url, latency, mbps, shot] = process.argv;
  const b = await chromium.launch();
  const ctx = await b.newContext({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 1 });
  const page = await ctx.newPage();
  const errs = [];
  page.on('pageerror', e => errs.push(e.message));
  const cdp = await ctx.newCDPSession(page);
  await cdp.send('Network.enable');
  await cdp.send('Network.setCacheDisabled', { cacheDisabled: true });
  await cdp.send('Network.emulateNetworkConditions', {
    offline: false, latency: +latency,
    downloadThroughput: (+mbps * 1e6) / 8, uploadThroughput: (+mbps * 1e6) / 8,
  });
  let bytes = 0;
  cdp.on('Network.loadingFinished', e => { bytes += e.encodedDataLength; });
  const t0 = Date.now();
  await page.goto(url, { waitUntil: 'commit' });
  await page.waitForFunction(() => {
    const s = document.getElementById('status'), app = document.getElementById('app');
    // Old index.html had #status; the splash version has #mica-splash outside #app.
    return (!s || s.style.display === 'none') && app && app.innerText.trim().length > 0;
  }, null, { timeout: 120000, polling: 50 });
  const t = (Date.now() - t0) / 1000;
  if (shot) await page.screenshot({ path: shot });
  console.log(JSON.stringify({ latency: +latency, mbps: +mbps, firstUiSeconds: t, transferredMiB: +(bytes / 1048576).toFixed(2), errors: errs.slice(0, 3) }));
  await b.close();
})();
