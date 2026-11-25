#!/usr/bin/env node
/**
 * Mica WebAssembly Client Automated Performance Gate (Playwright + Chromium CDP).
 *
 * Measures against a live Mica Web + API stack:
 *   1. Cold start: DOMContentLoaded, WASM bundle download size/time, first live synced page.
 *   2. 2,000-block mixed document open time & virtual-scroll FPS (rAF frame intervals + jank frames >50ms).
 *   3. 2,000-block document real browser keystroke-to-paint latency (P50 / P95 / P99 / Max).
 *   4. 50 repeated page switches: switch latency (P50 / P95) & JS heap slope after explicit GC.
 *
 * Usage:
 *   node benchmarks/web_perf.mjs --base http://127.0.0.1:18090 --email perf@mica.dev --password BenchPassword123!
 */
import assert from 'node:assert/strict';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, '..');
const require = createRequire(import.meta.url);
const { chromium } = require(path.join(repoRoot, 'e2e/node_modules/playwright'));

function parseArgs(argv) {
  const out = {
    base: 'http://127.0.0.1:18090',
    email: 'webperf@mica.dev',
    password: 'Mica#PerfTest99!',
    blocks: 2000,
    scrollSteps: 60,
    typeChars: 40,
    switches: 50,
    json: false,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a === '--base') out.base = argv[++i];
    else if (a === '--email') out.email = argv[++i];
    else if (a === '--password') out.password = argv[++i];
    else if (a === '--blocks') out.blocks = Number(argv[++i]);
    else if (a === '--scroll-steps') out.scrollSteps = Number(argv[++i]);
    else if (a === '--type-chars') out.typeChars = Number(argv[++i]);
    else if (a === '--switches') out.switches = Number(argv[++i]);
    else if (a === '--json') out.json = true;
  }
  out.base = out.base.replace(/\/+$/, '');
  return out;
}

function summarize(nums) {
  if (!nums.length) return { p50: 0, p95: 0, p99: 0, max: 0, mean: 0 };
  const s = [...nums].sort((a, b) => a - b);
  const pct = (p) => s[Math.min(s.length - 1, Math.round((s.length - 1) * p))];
  const mean = s.reduce((a, b) => a + b, 0) / s.length;
  return { p50: pct(0.5), p95: pct(0.95), p99: pct(0.99), max: s[s.length - 1], mean };
}

function generateMixedMarkdown(targetBlocks) {
  const parts = ['# WebAssembly 2000 块混排性能压测文档\n'];
  let idx = 1;
  while (idx < targetBlocks) {
    const k = idx % 8;
    if (k === 0) {
      parts.push(`\n## 章节 ${idx} — Rust WASM 虚拟滚动与实时编辑\n`);
    } else if (k === 1 || k === 2 || k === 3) {
      parts.push(
        `\n第 ${idx} 段正文内容：评估 **Mica** 纯 Rust WebAssembly 编辑器在超长混排文档下的 *实时输入延迟* 与 [虚拟列表](https://mica.dev) 帧率表现，包含 \`mica-editor\` 细粒度信号更新。\n`
      );
    } else if (k === 4) {
      parts.push(`\n- 列表条目 ${idx}：验证虚拟滚动回收与局部渲染\n`);
    } else if (k === 5) {
      parts.push(`\n> 引用段落 ${idx}：确保长文档中滚动条与视口高度计算无抖动。\n`);
    } else if (k === 6) {
      parts.push(`\n\`\`\`rust\nfn block_${idx}(x: usize) -> usize { x * 2 + 1 }\n\`\`\`\n`);
    } else {
      parts.push(`\n| 指标 | 数值 ${idx} | 结论 |\n| --- | ---: | --- |\n| 延迟 | ${idx % 16} ms | 达标 |\n`);
    }
    idx += 1;
  }
  return parts.join('');
}

async function seedPerfCorpus(opts) {
  let login = await fetch(`${opts.base}/api/auth/login`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ email: opts.email, password: opts.password }),
  });
  if (!login.ok) {
    const reg = await fetch(`${opts.base}/api/auth/register`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ email: opts.email, password: opts.password, display_name: 'Web Runner' }),
    });
    if (![200, 201, 409].includes(reg.status)) {
      const txt = await reg.text().catch(() => '');
      throw new Error(`register failed: HTTP ${reg.status} ${txt}`);
    }
    login = await fetch(`${opts.base}/api/auth/login`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ email: opts.email, password: opts.password }),
    });
  }
  if (!login.ok) throw new Error(`login failed: HTTP ${login.status}`);
  const { access_token: token } = await login.json();
  const headers = { 'Content-Type': 'application/json', Authorization: `Bearer ${token}` };

  const wsListResp = await fetch(`${opts.base}/api/workspaces`, { headers });
  if (!wsListResp.ok) throw new Error(`list ws failed: HTTP ${wsListResp.status}`);
  const wsListData = await wsListResp.json();
  const workspaces = Array.isArray(wsListData) ? wsListData : (wsListData.workspaces || []);
  let wsId = workspaces[0]?.id;
  if (!wsId) {
    const wsResp = await fetch(`${opts.base}/api/workspaces`, {
      method: 'POST',
      headers,
      body: JSON.stringify({ name: `WebPerf-${Date.now()}` }),
    });
    if (!wsResp.ok) throw new Error(`create ws failed: HTTP ${wsResp.status}`);
    const wsData = await wsResp.json();
    wsId = wsData.workspace?.id || wsData.id;
  }

  const runTag = String(Date.now()).slice(-6);
  const createDocWithMd = async (name, markdown) => {
    const r = await fetch(`${opts.base}/api/workspaces/${wsId}/documents`, {
      method: 'POST',
      headers,
      body: JSON.stringify({ name }),
    });
    if (!r.ok) throw new Error(`create doc ${name} failed: HTTP ${r.status}`);
    const d = await r.json();
    const docId = d.document?.id || d.id;
    const put = await fetch(`${opts.base}/api/workspaces/${wsId}/documents/${docId}/markdown`, {
      method: 'PATCH',
      headers,
      body: JSON.stringify({ markdown, mode: 'replace_all' }),
    });
    if (!put.ok) {
      const t = await put.text().catch(() => '');
      throw new Error(`patch markdown ${name} failed: HTTP ${put.status} ${t}`);
    }
    return { docId, name };
  };

  const largeMd = generateMixedMarkdown(opts.blocks);
  const largeDoc = await createDocWithMd(`Perf-Large-${opts.blocks}-${runTag}`, largeMd);
  const switchDocs = [];
  for (let i = 0; i < 5; i += 1) {
    const md = generateMixedMarkdown(120).replace('WebAssembly 2000 块', `切页测试文档 #${i + 1}-${runTag}`);
    switchDocs.push(await createDocWithMd(`Perf-Switch-${i + 1}-${runTag}`, md));
  }
  return { wsId, runTag, largeDoc, switchDocs, largeMdBytes: Buffer.byteLength(largeMd, 'utf8') };
}

async function fillTestInput(page, testId, value) {
  const host = page.locator(`[data-testid="${testId}"]`);
  await host.waitFor({ state: 'visible', timeout: 20000 });
  const inner = host.locator('input, textarea').first();
  if (await inner.count()) {
    await inner.click();
    await inner.fill(value);
  } else {
    await host.click();
    await page.keyboard.type(value);
  }
}

async function getHeapMb(cdp) {
  try {
    await cdp.send('HeapProfiler.collectGarbage');
  } catch {
    // optional
  }
  const { metrics } = await cdp.send('Performance.getMetrics');
  const map = Object.fromEntries(metrics.map((m) => [m.name, m.value]));
  return {
    jsHeapUsedMb: (map.JSHeapUsedSize || 0) / (1024 * 1024),
    jsHeapTotalMb: (map.JSHeapTotalSize || 0) / (1024 * 1024),
    domNodes: map.Nodes || 0,
  };
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  const seed = await seedPerfCorpus(opts);

  const browser = await chromium.launch({ headless: true });
  const results = {};
  try {
    const ctx = await browser.newContext({ viewport: { width: 1280, height: 820 } });
    const page = await ctx.newPage();
    const cdp = await ctx.newCDPSession(page);
    await cdp.send('Performance.enable');
    await cdp.send('HeapProfiler.enable');

    let wasmRespBytes = 0;
    let wasmRespMs = 0;
    const wasmStart = new Map();
    page.on('request', (req) => {
      if (req.url().includes('mica_rust_bg.wasm')) {
        wasmStart.set(req.url(), performance.now());
      }
    });
    page.on('response', async (resp) => {
      if (resp.url().includes('mica_rust_bg.wasm')) {
        const started = wasmStart.get(resp.url()) ?? performance.now();
        const body = await resp.body().catch(() => null);
        wasmRespMs = performance.now() - started;
        wasmRespBytes = body ? body.length : 0;
      }
    });

    // ── 1. Cold start + sign-in ─────────────────────────────────────────────
    const tNav = performance.now();
    await page.goto(`${opts.base}/`, { waitUntil: 'domcontentloaded' });
    const domReadyMs = performance.now() - tNav;
    await fillTestInput(page, 'world-auth-email', opts.email);
    const firstRenderMs = performance.now() - tNav;
    await fillTestInput(page, 'world-auth-password', opts.password);
    const tLogin = performance.now();
    await page.locator('[data-testid="world-auth-submit"]').click();
    await page.getByText('已实时同步').first().waitFor({ state: 'visible', timeout: 30000 });
    const loginToSyncedMs = performance.now() - tLogin;
    const initialMem = await getHeapMb(cdp);

    results.cold_start = {
      dom_ready_ms: Number(domReadyMs.toFixed(2)),
      wasm_download_ms: Number(wasmRespMs.toFixed(2)),
      wasm_bytes: wasmRespBytes,
      first_interactive_ms: Number(firstRenderMs.toFixed(2)),
      login_to_synced_ms: Number(loginToSyncedMs.toFixed(2)),
      initial_js_heap_mb: Number(initialMem.jsHeapUsedMb.toFixed(2)),
      initial_dom_nodes: initialMem.domNodes,
    };

    // Switch to our freshly seeded workspace if multiple exist, or click the seeded large page directly.
    const largeItem = page.getByText(seed.largeDoc.name, { exact: false }).first();
    if (!(await largeItem.isVisible().catch(() => false))) {
      // Open workspace switcher if needed
      const wsBtn = page.getByText('WebPerf-', { exact: false }).first();
      if (await wsBtn.isVisible().catch(() => false)) {
        await wsBtn.click();
      }
    }

    // ── 2. Open 2,000-block mixed document & measure virtual scroll FPS ─────
    await largeItem.waitFor({ state: 'visible', timeout: 20000 });
    const tOpenLarge = performance.now();
    await largeItem.click();
    await page.getByText('WebAssembly 2000 块混排性能压测文档').first().waitFor({ state: 'visible', timeout: 25000 });
    const openLargeMs = performance.now() - tOpenLarge;
    await page.waitForTimeout(300);

    // Move mouse over the editor pane and sample rAF frames during smooth wheel scrolling.
    const editorBox = await page.locator('[data-testid="block-row-0"]').boundingBox();
    assert(editorBox, 'expected block-row-0 bounding box');
    await page.mouse.move(editorBox.x + 200, editorBox.y + 180);

    await page.evaluate(() => {
      window.__perfFrames = [];
      window.__perfSampling = true;
      let last = performance.now();
      const tick = (now) => {
        if (!window.__perfSampling) return;
        window.__perfFrames.push(now - last);
        last = now;
        requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    });

    for (let i = 0; i < opts.scrollSteps; i += 1) {
      await page.mouse.wheel(0, 260);
      await page.waitForTimeout(16);
    }
    await page.waitForTimeout(120);

    const frameDeltas = await page.evaluate(() => {
      window.__perfSampling = false;
      return window.__perfFrames.slice(2);
    });
    const frameStats = summarize(frameDeltas);
    const avgFps = frameStats.mean > 0 ? 1000 / frameStats.mean : 0;
    const p95Fps = frameStats.p95 > 0 ? 1000 / frameStats.p95 : 0;
    const jankCount = frameDeltas.filter((d) => d > 50).length;
    const jankRatioPct = frameDeltas.length ? (jankCount / frameDeltas.length) * 100 : 0;

    results.large_doc_scroll = {
      blocks: opts.blocks,
      markdown_bytes: seed.largeMdBytes,
      open_large_doc_ms: Number(openLargeMs.toFixed(2)),
      frames_sampled: frameDeltas.length,
      avg_fps: Number(avgFps.toFixed(1)),
      p95_low_fps: Number(p95Fps.toFixed(1)),
      frame_p50_ms: Number(frameStats.p50.toFixed(2)),
      frame_p95_ms: Number(frameStats.p95.toFixed(2)),
      frame_p99_ms: Number(frameStats.p99.toFixed(2)),
      frame_max_ms: Number(frameStats.max.toFixed(2)),
      jank_frames_over_50ms: jankCount,
      jank_ratio_pct: Number(jankRatioPct.toFixed(2)),
    };

    // Scroll back to top before typing benchmark.
    for (let i = 0; i < 8; i += 1) {
      await page.mouse.wheel(0, -3000);
      await page.waitForTimeout(20);
    }
    await page.waitForTimeout(200);

    // ── 3. Keystroke-to-Paint latency inside 2,000-block doc ────────────────
    const row1 = page.locator('[data-testid="block-row-1"]');
    await row1.waitFor({ state: 'visible', timeout: 15000 });
    await row1.click();
    await page.waitForTimeout(200);

    await page.evaluate(() => {
      window.__keyLatencies = [];
      window.__keyListener = () => {
        const t0 = performance.now();
        requestAnimationFrame(() => {
          requestAnimationFrame(() => {
            window.__keyLatencies.push(performance.now() - t0);
          });
        });
      };
      window.addEventListener('keydown', window.__keyListener, true);
    });

    const sampleChars = 'abcdefghij0123456789xyzrustmicabenchtest';
    for (let i = 0; i < opts.typeChars; i += 1) {
      await page.keyboard.press(sampleChars[i % sampleChars.length]);
      await page.waitForTimeout(55);
    }
    await page.waitForTimeout(150);

    const keyLats = await page.evaluate(() => {
      window.removeEventListener('keydown', window.__keyListener, true);
      return window.__keyLatencies;
    });
    const keyStats = summarize(keyLats);
    results.large_doc_typing = {
      chars_typed: keyLats.length,
      p50_ms: Number(keyStats.p50.toFixed(2)),
      p95_ms: Number(keyStats.p95.toFixed(2)),
      p99_ms: Number(keyStats.p99.toFixed(2)),
      max_ms: Number(keyStats.max.toFixed(2)),
    };

    // ── 4. 50 repeated page switches: latency & JS heap slope ───────────────
    const memStart = await getHeapMb(cdp);
    const switchLatencies = [];
    const heapSamplesMb = [memStart.jsHeapUsedMb];
    const nodeSamples = [memStart.domNodes];

    for (let i = 0; i < opts.switches; i += 1) {
      const target = seed.switchDocs[i % seed.switchDocs.length];
      const marker = `切页测试文档 #${(i % seed.switchDocs.length) + 1}-${seed.runTag}`;
      const navItem = page.getByText(target.name, { exact: false }).first();
      const t0 = performance.now();
      await navItem.click();
      await page.getByText(marker, { exact: false }).first().waitFor({ state: 'visible', timeout: 15000 });
      switchLatencies.push(performance.now() - t0);
      if ((i + 1) % 10 === 0) {
        const m = await getHeapMb(cdp);
        heapSamplesMb.push(m.jsHeapUsedMb);
        nodeSamples.push(m.domNodes);
      }
    }
    const swStats = summarize(switchLatencies);
    const heapDeltaMb = heapSamplesMb[heapSamplesMb.length - 1] - heapSamplesMb[0];
    const domDelta = nodeSamples[nodeSamples.length - 1] - nodeSamples[0];

    results.page_switches = {
      switches: opts.switches,
      switch_p50_ms: Number(swStats.p50.toFixed(2)),
      switch_p95_ms: Number(swStats.p95.toFixed(2)),
      switch_p99_ms: Number(swStats.p99.toFixed(2)),
      heap_start_mb: Number(heapSamplesMb[0].toFixed(2)),
      heap_end_mb: Number(heapSamplesMb[heapSamplesMb.length - 1].toFixed(2)),
      heap_delta_mb: Number(heapDeltaMb.toFixed(2)),
      heap_slope_kb_per_switch: Number(((heapDeltaMb * 1024) / opts.switches).toFixed(2)),
      dom_nodes_start: nodeSamples[0],
      dom_nodes_end: nodeSamples[nodeSamples.length - 1],
      dom_nodes_delta: domDelta,
    };

    if (opts.json) {
      console.log(JSON.stringify(results, null, 2));
    } else {
      console.log('Mica WebAssembly Performance Gate (Playwright + Chromium CDP)');
      console.log('\n=== [1/4] Cold Start ===');
      console.log(`  dom_ready_ms           ${results.cold_start.dom_ready_ms} ms`);
      console.log(`  wasm_download          ${results.cold_start.wasm_download_ms} ms (${(results.cold_start.wasm_bytes / 1024).toFixed(1)} KiB)`);
      console.log(`  first_interactive_ms   ${results.cold_start.first_interactive_ms} ms`);
      console.log(`  login_to_synced_ms     ${results.cold_start.login_to_synced_ms} ms`);
      console.log(`  initial_memory         ${results.cold_start.initial_js_heap_mb} MiB JS heap, ${results.cold_start.initial_dom_nodes} DOM nodes`);

      console.log(`\n=== [2/4] Large Document Virtual Scroll (${opts.blocks} blocks) ===`);
      console.log(`  open_large_doc_ms      ${results.large_doc_scroll.open_large_doc_ms} ms`);
      console.log(`  scroll_fps             avg=${results.large_doc_scroll.avg_fps} FPS  p95_low=${results.large_doc_scroll.p95_low_fps} FPS (${results.large_doc_scroll.frames_sampled} frames)`);
      console.log(`  frame_interval         p50=${results.large_doc_scroll.frame_p50_ms} ms  p95=${results.large_doc_scroll.frame_p95_ms} ms  p99=${results.large_doc_scroll.frame_p99_ms} ms  max=${results.large_doc_scroll.frame_max_ms} ms`);
      console.log(`  jank_frames (>50ms)    ${results.large_doc_scroll.jank_frames_over_50ms} (${results.large_doc_scroll.jank_ratio_pct}%)`);

      console.log(`\n=== [3/4] Large Document Typing Latency (${opts.blocks} blocks, ${results.large_doc_typing.chars_typed} keys) ===`);
      console.log(`  key_to_paint           p50=${results.large_doc_typing.p50_ms} ms  p95=${results.large_doc_typing.p95_ms} ms  p99=${results.large_doc_typing.p99_ms} ms  max=${results.large_doc_typing.max_ms} ms`);

      console.log(`\n=== [4/4] ${opts.switches} Repeated Page Switches & Memory Slope ===`);
      console.log(`  switch_latency         p50=${results.page_switches.switch_p50_ms} ms  p95=${results.page_switches.switch_p95_ms} ms  p99=${results.page_switches.switch_p99_ms} ms`);
      console.log(`  js_heap_slope          ${results.page_switches.heap_start_mb} MiB -> ${results.page_switches.heap_end_mb} MiB (delta ${results.page_switches.heap_delta_mb >= 0 ? '+' : ''}${results.page_switches.heap_delta_mb} MiB, ${results.page_switches.heap_slope_kb_per_switch} KiB/switch)`);
      console.log(`  dom_nodes_delta        ${results.page_switches.dom_nodes_start} -> ${results.page_switches.dom_nodes_end} (${results.page_switches.dom_nodes_delta >= 0 ? '+' : ''}${results.page_switches.dom_nodes_delta})`);
      console.log('\nWeb performance benchmark completed.');
    }
  } finally {
    await browser.close();
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
