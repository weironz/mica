// The release-built editor with its real worker and bundled fonts.
import { chromium } from '@playwright/test';

const baseArg = process.argv.indexOf('--base');
const base = baseArg >= 0 ? process.argv[baseArg + 1] : 'http://127.0.0.1:8091';
const browser = await chromium.launch().catch(() => chromium.launch({ channel: 'chrome' }));

try {
  const context = await browser.newContext({ viewport: { width: 1100, height: 600 }, locale: 'zh-CN' });
  const page = await context.newPage();
  const errors = [];
  const assets = new Set();
  page.on('pageerror', error => errors.push(String(error)));
  context.on('request', request => {
    const path = new URL(request.url()).pathname;
    if (path.includes('/mica_preview') || path.endsWith('/mermaid.min.js')) assets.add(path);
  });

  async function open(fixture) {
    const url = new URL(base);
    url.searchParams.set('harness', 'clipboard');
    url.searchParams.set('fixture', fixture);
    await page.goto(url.toString(), { waitUntil: 'load' });
    await page.waitForFunction(() => typeof window.micaClipboardHarnessMathState === 'function');
  }

  await open('empty');
  if (assets.size) throw new Error(`Empty editor requested a preview engine: ${[...assets]}`);
  await open('render');
  await page.waitForFunction(() => {
    const state = JSON.parse(window.micaClipboardHarnessMathState());
    return state.previews.math === 1 && state.previews.mermaid === 1 &&
      state.folded.includes(String.raw`\frac{a}{b}`);
  });
  for (const path of ['/mica_preview/math_bg.wasm', '/mica_preview/mermaid_bg.wasm']) {
    if (!assets.has(path)) throw new Error(`No request for ${path}`);
  }
  if (assets.has('/mermaid.min.js')) throw new Error('The old Mermaid script was requested');
  await page.screenshot({ path: 'rich-preview-editor.png' });
  if (!await page.evaluate(() => window.micaPreviewHarnessHasHanGlyphs())) {
    throw new Error('Mermaid labels with different Han characters did not draw different glyphs');
  }

  // The renderers cannot load at first (offline, a failed download): the
  // source stays visible, and once they can load the previews appear by
  // themselves — a failed load must not be remembered as a bad formula.
  await page.route('**/mica_preview/**', route => route.abort());
  await open('render');
  await page.waitForTimeout(2500);
  const blocked = JSON.parse(await page.evaluate(() => window.micaClipboardHarnessMathState()));
  if (blocked.previews.math || blocked.previews.mermaid) {
    throw new Error(`Previews rendered while their renderers were blocked: ${JSON.stringify(blocked.previews)}`);
  }
  await page.unroute('**/mica_preview/**');
  await page.waitForFunction(() => {
    const state = JSON.parse(window.micaClipboardHarnessMathState());
    return state.previews.math === 1 && state.previews.mermaid === 1;
  }, null, { timeout: 15000 });

  if (errors.length) throw new Error(`Page errors: ${errors.join(' | ')}`);
  console.log('Rich previews: lazy wasm, inline and display math, Mermaid, Han glyphs and recovery after a failed load passed');
} finally {
  await browser.close();
}
