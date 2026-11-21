// Automated Playwright benchmark & CJK IME probe for the Option C board spike.
// Measures:
//   1. Dart deferred .part.js split & compressed transfer sizes (gzip / Brotli)
//   2. 500-node CanvasKit A/B performance (naive vs optimized ui.Picture + RepaintBoundary)
//   3. Zoomed-canvas CJK IME editing at 0.45x, 1.0x, and 2.0x (DOM input rect drift,
//      CDP Input.imeSetComposition, multiline auto-grow, Escape commit)
import { expect } from '@playwright/test';
import { launchBrowser } from './browser.mjs';
import { createServer } from 'node:http';
import { access, mkdir, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import { extname, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { brotliCompressSync, constants as zlibConstants, gzipSync } from 'node:zlib';

const root = resolve(
  fileURLToPath(new URL('../clients/mica_flutter/build/board_spike/', import.meta.url)),
);
const output = resolve(
  fileURLToPath(new URL('../clients/mica_flutter/build/board-spike-results/', import.meta.url)),
);
await mkdir(output, { recursive: true });

const mime = {
  '.html': 'text/html',
  '.js': 'application/javascript',
  '.wasm': 'application/wasm',
  '.json': 'application/json',
  '.ttf': 'font/ttf',
  '.otf': 'font/otf',
  '.woff2': 'font/woff2',
  '.png': 'image/png',
  '.svg': 'image/svg+xml',
};

function percentile(sorted, p) {
  if (!sorted.length) return 0;
  const idx = Math.min(sorted.length - 1, Math.floor((sorted.length - 1) * p));
  return sorted[idx];
}

function summarizeMs(values) {
  if (!values.length) return { count: 0, avgMs: 0, p50Ms: 0, p95Ms: 0, maxMs: 0 };
  const sorted = [...values].sort((a, b) => a - b);
  const avg = values.reduce((s, v) => s + v, 0) / values.length;
  return {
    count: values.length,
    avgMs: Number(avg.toFixed(2)),
    p50Ms: Number(percentile(sorted, 0.5).toFixed(2)),
    p95Ms: Number(percentile(sorted, 0.95).toFixed(2)),
    maxMs: Number(sorted[sorted.length - 1].toFixed(2)),
  };
}

async function measureBundleSizes() {
  const entries = await readdir(root);
  const files = entries.filter((f) => f === 'main.dart.js' || f.endsWith('.part.js')).sort();
  const report = {};
  for (const name of files) {
    const raw = await readFile(resolve(root, name));
    const gz = gzipSync(raw, { level: 9 });
    const br = brotliCompressSync(raw, {
      params: { [zlibConstants.BROTLI_PARAM_QUALITY]: 11 },
    });
    report[name] = {
      rawBytes: raw.length,
      gzipBytes: gz.length,
      brotliBytes: br.length,
      brotliKB: Number((br.length / 1024).toFixed(1)),
    };
  }
  return report;
}

async function getState(page) {
  return page.evaluate(() => JSON.parse(window.micaBoardSpikeState()));
}

async function getNodeRect(page, nodeId) {
  return page.evaluate((id) => JSON.parse(window.micaBoardSpikeNodeRect(id)), nodeId);
}

async function runBenchmarkWorkload(page, label, gestureFn) {
  await page.evaluate(() => {
    window.micaBoardSpikeResetCounters();
    window.__spikeRafDeltas = [];
    window.__spikeRafActive = true;
    let prev = performance.now();
    function tick(now) {
      if (!window.__spikeRafActive) return;
      window.__spikeRafDeltas.push(now - prev);
      prev = now;
      requestAnimationFrame(tick);
    }
    requestAnimationFrame(tick);
  });

  const wallStart = Date.now();
  await gestureFn();
  await page.waitForTimeout(120);
  const wallMs = Date.now() - wallStart;

  const { rafDeltas, timings, state } = await page.evaluate(() => {
    window.__spikeRafActive = false;
    return {
      rafDeltas: window.__spikeRafDeltas.slice(1), // drop initial frame
      timings: JSON.parse(window.micaBoardSpikeFrameTimings()),
      state: JSON.parse(window.micaBoardSpikeState()),
    };
  });

  const buildMs = timings.map((t) => t.buildUs / 1000);
  const rasterMs = timings.map((t) => t.rasterUs / 1000);
  const totalMs = timings.map((t) => t.totalUs / 1000);
  const raf = summarizeMs(rafDeltas);
  const effectiveFps = raf.avgMs > 0 ? Number((1000 / raf.avgMs).toFixed(1)) : 0;

  const res = {
    label,
    wallMs,
    effectiveFps,
    raf,
    flutterBuild: summarizeMs(buildMs),
    flutterRaster: summarizeMs(rasterMs),
    flutterTotal: summarizeMs(totalMs),
    counters: {
      staticPaint: state.staticPaintCount,
      pictureRebuild: state.pictureRebuildCount,
      activePaint: state.activePaintCount,
      naivePaint: state.naivePaintCount,
    },
  };
  console.log(
    `[BENCH] ${label}: FPS=${effectiveFps} | rAF p50=${raf.p50Ms}ms p95=${raf.p95Ms}ms max=${raf.maxMs}ms | ` +
      `Flutter build p50=${res.flutterBuild.p50Ms}ms raster p50=${res.flutterRaster.p50Ms}ms total p95=${res.flutterTotal.p95Ms}ms | ` +
      `paints=${JSON.stringify(res.counters)}`,
  );
  return res;
}

let server;
let browser;
const pageErrors = [];

try {
  await access(resolve(root, 'index.html'));
  const bundles = await measureBundleSizes();
  console.log('[BUNDLES]', JSON.stringify(bundles, null, 2));

  server = createServer(async (req, res) => {
    try {
      const pathname = decodeURIComponent(new URL(req.url, 'http://localhost').pathname);
      const file = resolve(root, '.' + (pathname === '/' ? '/index.html' : pathname));
      if (!file.startsWith(root + sep)) {
        res.writeHead(403).end();
        return;
      }
      const bytes = await readFile(file);
      res
        .writeHead(200, {
          'content-type': mime[extname(file)] || 'application/octet-stream',
          'cache-control': 'no-store',
        })
        .end(bytes);
    } catch {
      res.writeHead(404).end();
    }
  });
  await new Promise((done, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', done);
  });
  const origin = `http://127.0.0.1:${server.address().port}`;

  browser = await launchBrowser();
  const context = await browser.newContext({
    locale: 'zh-CN',
    viewport: { width: 1280, height: 820 },
    serviceWorkers: 'block',
  });
  const page = await context.newPage();
  page.on('pageerror', (err) => pageErrors.push(String(err)));

  const requestedParts = [];
  context.on('request', (req) => {
    const pathname = new URL(req.url()).pathname;
    if (pathname.endsWith('.part.js')) requestedParts.push(pathname);
  });

  await page.goto(origin, { waitUntil: 'load' });
  await page.waitForFunction(() => typeof window.micaBoardSpikeState === 'function', null, {
    timeout: 60_000,
  });
  await page.waitForTimeout(400);

  // 1. Verify deferred chunk was NOT downloaded before opening board.
  const initialState = await getState(page);
  expect(initialState.loaded).toBe(false);
  expect(requestedParts.length).toBe(0);
  console.log('[DEFERRED] Initial load parts:', requestedParts.length, '(expected 0)');

  // Load deferred board library.
  await page.evaluate(() => window.micaBoardSpikeLoad());
  await expect
    .poll(async () => (await getState(page)).loaded, { timeout: 30_000 })
    .toBe(true);
  await page.waitForTimeout(500);
  console.log('[DEFERRED] Parts fetched on demand:', requestedParts);
  expect(requestedParts.length).toBeGreaterThan(0);

  await page.screenshot({ path: resolve(output, '01-overview-100.png') });

  // 2. A/B Benchmarks: naive vs optimized on 500 nodes + ~460 edges.
  const benchmarks = [];

  for (const mode of ['optimized', 'naive']) {
    await page.evaluate((m) => window.micaBoardSpikeSetMode(m), mode);
    await page.evaluate(() => window.micaBoardSpikeSetViewport(1.0, 160, 120));
    await page.waitForTimeout(250);

    // Workload A: Drag hero node n0 (5 incident edges) for 50 frames at 100% zoom.
    const rect100 = await getNodeRect(page, 'n0');
    const startX = rect100.left + rect100.width / 2;
    const startY = rect100.top + rect100.height / 2;
    benchmarks.push(
      await runBenchmarkWorkload(page, `${mode} / drag-100% (500 nodes)`, async () => {
        await page.mouse.move(startX, startY);
        await page.mouse.down();
        for (let i = 1; i <= 48; i++) {
          const angle = (i / 48) * Math.PI * 2;
          const dx = Math.cos(angle) * 120;
          const dy = Math.sin(angle) * 80;
          await page.mouse.move(startX + dx, startY + dy);
          await page.waitForTimeout(16);
        }
        await page.mouse.move(startX, startY);
        await page.mouse.up();
      }),
    );

    // Workload B: Zoomed out to 0.35x (hundreds of nodes visible) & drag n0 for 40 steps.
    await page.evaluate(() => window.micaBoardSpikeSetViewport(0.35, 240, 80));
    await page.waitForTimeout(250);
    if (mode === 'optimized') {
      await page.screenshot({ path: resolve(output, '02-zoomout-35.png') });
    }
    const rect35 = await getNodeRect(page, 'n0');
    const sx35 = rect35.left + rect35.width / 2;
    const sy35 = rect35.top + rect35.height / 2;
    benchmarks.push(
      await runBenchmarkWorkload(page, `${mode} / drag-35%-all-visible (500 nodes)`, async () => {
        await page.mouse.move(sx35, sy35);
        await page.mouse.down();
        for (let i = 1; i <= 40; i++) {
          await page.mouse.move(sx35 + i * 4, sy35 + Math.sin(i * 0.3) * 40);
          await page.waitForTimeout(16);
        }
        await page.mouse.move(sx35, sy35);
        await page.mouse.up();
      }),
    );

    // Workload C: Continuous pan across dense board at 0.5x zoom (40 steps).
    await page.evaluate(() => window.micaBoardSpikeSetViewport(0.5, 200, 80));
    await page.waitForTimeout(200);
    benchmarks.push(
      await runBenchmarkWorkload(page, `${mode} / pan-50% (500 nodes)`, async () => {
        await page.mouse.move(100, 100);
        await page.mouse.down();
        for (let i = 1; i <= 40; i++) {
          await page.mouse.move(100 + i * 8, 100 - i * 4);
          await page.waitForTimeout(16);
        }
        await page.mouse.up();
      }),
    );
  }

  // 3. Zoomed-canvas CJK IME Editing Probe at 0.45x, 1.0x, and 2.0x.
  await page.evaluate(() => window.micaBoardSpikeSetMode('optimized'));
  const cdp = await page.context().newCDPSession(page);
  const imeResults = [];

  for (const zoom of [0.45, 1.0, 2.0]) {
    // Center n0 nicely in viewport at each zoom level.
    const panX = 240;
    const panY = zoom >= 1.5 ? 20 : 140;
    await page.evaluate(([z, px, py]) => window.micaBoardSpikeSetViewport(z, px, py), [
      zoom,
      panX,
      panY,
    ]);
    await page.waitForTimeout(250);

    const beforeRect = await getNodeRect(page, 'n0');
    const clickX = beforeRect.left + beforeRect.width / 2;
    const clickY = beforeRect.top + beforeRect.height / 2;

    // Real double-click on the node to enter inline editing.
    await page.mouse.click(clickX, clickY);
    await page.waitForTimeout(80);
    await page.mouse.click(clickX, clickY);
    await page.waitForTimeout(300);

    const editState = await getState(page);

    // Measure Flutter Web's hidden/positioned DOM <textarea>/<input> bounding rect.
    // This rect is where Chrome/OS anchors the native CJK IME candidate popup!
    const domInputInfo = await page.evaluate(() => {
      const inputs = [...document.querySelectorAll('textarea, input')];
      const active = document.activeElement;
      return {
        activeTag: active?.tagName ?? null,
        inputs: inputs.map((el) => {
          const r = el.getBoundingClientRect();
          const cs = getComputedStyle(el);
          return {
            tag: el.tagName,
            isActive: el === active,
            left: Number(r.left.toFixed(1)),
            top: Number(r.top.toFixed(1)),
            width: Number(r.width.toFixed(1)),
            height: Number(r.height.toFixed(1)),
            transform: cs.transform,
            fontSize: cs.fontSize,
          };
        }),
      };
    });

    // Drive real Chrome CJK IME composition via CDP.
    const phrase = `缩放${Math.round(zoom * 100)}中文`;
    await page.keyboard.press('Enter');
    await page.waitForTimeout(60);
    for (let i = 1; i <= phrase.length; i++) {
      await cdp.send('Input.imeSetComposition', {
        text: phrase.slice(0, i),
        selectionStart: i,
        selectionEnd: i,
      });
      await page.waitForTimeout(35);
    }
    await cdp.send('Input.insertText', { text: phrase });
    await page.waitForTimeout(250);

    const duringRect = await getNodeRect(page, 'n0');
    const afterComposeState = await getState(page);
    await page.screenshot({
      path: resolve(output, `04-ime-zoom-${Math.round(zoom * 100)}.png`),
    });

    // Commit & exit edit mode via Escape.
    await page.keyboard.press('Escape');
    await page.waitForTimeout(250);
    const afterExitState = await getState(page);

    const activeDom = domInputInfo.inputs.find((i) => i.isActive) ?? domInputInfo.inputs[0] ?? null;
    const dxDrift = activeDom ? Number((activeDom.left - beforeRect.left).toFixed(1)) : null;
    const dyDrift = activeDom ? Number((activeDom.top - beforeRect.top).toFixed(1)) : null;
    const widthRatio =
      activeDom && beforeRect.width > 0
        ? Number((activeDom.width / beforeRect.width).toFixed(2))
        : null;

    const record = {
      zoom,
      enteredEditMode: editState.editingNodeId === 'n0',
      committedPhrasePresent: (afterComposeState.editingText ?? '').includes(phrase),
      exitedOnEscape: afterExitState.editingNodeId === null,
      worldHeightBefore: Number(beforeRect.worldH.toFixed(1)),
      worldHeightAfter: Number(duringRect.worldH.toFixed(1)),
      autoGrew: duringRect.worldH > beforeRect.worldH,
      nodeScreenRect: {
        left: Number(beforeRect.left.toFixed(1)),
        top: Number(beforeRect.top.toFixed(1)),
        width: Number(beforeRect.width.toFixed(1)),
        height: Number(beforeRect.height.toFixed(1)),
      },
      activeDomInput: activeDom,
      imeAnchorDeltaPx: { dx: dxDrift, dy: dyDrift, widthRatio },
    };
    imeResults.push(record);
    console.log(`[IME zoom=${zoom}]`, JSON.stringify(record));
  }

  // Final post-edit overview screenshot.
  await page.evaluate(() => window.micaBoardSpikeSetViewport(1.0, 160, 120));
  await page.waitForTimeout(250);
  await page.screenshot({ path: resolve(output, '05-final-committed.png') });

  const report = {
    timestamp: new Date().toISOString(),
    bundles,
    deferredPartsFetched: requestedParts,
    benchmarks,
    imeResults,
    pageErrors,
  };
  await writeFile(resolve(output, 'board-spike-report.json'), JSON.stringify(report, null, 2));
  console.log('[DONE] Report saved to', resolve(output, 'board-spike-report.json'));
  if (pageErrors.length) {
    console.error('[PAGE ERRORS]', pageErrors);
    process.exitCode = 1;
  }
} finally {
  if (browser) await browser.close();
  if (server) server.close();
}
