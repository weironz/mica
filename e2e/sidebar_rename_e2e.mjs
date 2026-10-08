// Browser mouse and keyboard against the real WorkspaceView and rename dialog.
import { chromium } from '@playwright/test';
import { createServer } from 'node:http';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../clients/mica_flutter/build/sidebar_rename/', import.meta.url));
const output = process.env.MICA_RENAME_RESULTS_DIR || fileURLToPath(new URL('../clients/mica_flutter/build/sidebar-rename-results/', import.meta.url));
await mkdir(output, { recursive: true });
const mime = { '.html': 'text/html', '.js': 'application/javascript', '.wasm': 'application/wasm', '.json': 'application/json', '.ttf': 'font/ttf', '.otf': 'font/otf', '.woff2': 'font/woff2', '.png': 'image/png' };
const server = createServer(async (req, res) => {
  const pathname = decodeURIComponent(new URL(req.url, 'http://localhost').pathname);
  const path = resolve(root, `.${pathname === '/' ? '/index.html' : pathname}`);
  if (!path.startsWith(resolve(root) + sep)) { res.writeHead(403).end(); return; }
  try {
    const bytes = await readFile(path);
    res.writeHead(200, { 'content-type': mime[extname(path)] || 'application/octet-stream', 'cache-control': 'no-store' }).end(bytes);
  } catch { res.writeHead(404).end(); }
});
await new Promise(done => server.listen(8094, '127.0.0.1', done));
let browser;
for (const channel of [undefined, 'chrome', 'msedge']) {
  try { browser = await chromium.launch(channel ? { channel } : {}); break; } catch {}
}
if (!browser) { server.close(); throw new Error('No Chromium available'); }
const page = await browser.newPage({ locale: 'zh-CN', viewport: { width: 1280, height: 900 } });
const errors = [];
page.on('pageerror', error => errors.push(String(error)));
const state = () => page.evaluate(() => JSON.parse(window.micaRenameState()));
const check = (condition, message) => { if (!condition) throw new Error(message); console.log('ok ' + message); };
let sidebarPoint;
async function clickPoint(name) {
  // This fixture retains the same row/order throughout. Hover actions merge
  // its accessibility label, so retain the initial physical row coordinates.
  if (sidebarPoint) return sidebarPoint;
  // The same name appears in the breadcrumb/title after opening the page.
  // Identify the actual left tree row by its rendered position.
  for (const candidate of await page.getByText(name, { exact: true }).all()) {
    const box = await candidate.boundingBox();
    if (box && box.x < 220 && box.y > 200)
      return sidebarPoint = { x: box.x + Math.min(40, box.width / 2), y: box.y + box.height / 2 };
  }
  throw new Error('Sidebar row missing: ' + name);
}
async function pair(name) {
  const p = await clickPoint(name);
  await page.mouse.click(p.x, p.y);
  await page.waitForTimeout(80);
  await page.mouse.click(p.x, p.y);
  await page.getByText('重命名', { exact: true }).waitFor();
}
try {
  await page.goto('http://127.0.0.1:8094');
  await page.waitForFunction(() => typeof window.micaRenameState === 'function');
  await page.waitForLoadState('networkidle');
  const placeholder = page.locator('flt-semantics-placeholder');
  if (await placeholder.count()) await placeholder.evaluate(e => e.click());
  const first = await clickPoint('待重命名页面');
  // Semantics is for locating/snapshotting only; send physical mouse events
  // to the canvas rather than synthesizing a semantics tap with no timestamp.
  await page.addStyleTag({content: 'flt-semantics-host, flt-semantics-host * { pointer-events: none !important; }'});
  await page.mouse.click(first.x, first.y);
  // No second click and no double-click timeout are needed to open a page.
  await page.waitForFunction(() => JSON.parse(window.micaRenameState()).opens === 1);
  check((await state()).opens === 1, 'single click opens the page');
  await page.waitForTimeout(350);
  await pair('待重命名页面');
  check((await state()).opens === 2, 'double click opens once then renames');
  await page.screenshot({ path: resolve(output, 'sidebar-rename-dialog.png') });
  await page.keyboard.insertText('浏览器快速重命名');
  await page.keyboard.press('Enter');
  await page.waitForFunction(() => JSON.parse(window.micaRenameState()).name === '浏览器快速重命名');
  check((await state()).saves.length === 1, 'Enter saves the actual clicked page once');
  await page.waitForTimeout(350);
  await pair('浏览器快速重命名');
  await page.keyboard.insertText('不应保存');
  await page.keyboard.press('Escape');
  await page.getByText('重命名', { exact: true }).waitFor({ state: 'hidden' });
  check((await state()).saves.length === 1, 'Escape cancels without saving');
  const p = await clickPoint('浏览器快速重命名');
  await page.keyboard.down('Control');
  await page.mouse.click(p.x, p.y);
  await page.waitForTimeout(80);
  await page.mouse.click(p.x, p.y);
  await page.keyboard.up('Control');
  await page.waitForTimeout(350);
  check(await page.getByText('重命名', { exact: true }).count() === 0, 'Ctrl pair remains selection without a rename dialog');
  // A -> B -> A is not a double click, even if all three clicks are rapid.
  await page.mouse.click(p.x, p.y);
  await page.waitForTimeout(80);
  await page.mouse.click(p.x, p.y - 46);
  await page.waitForTimeout(80);
  await page.mouse.click(p.x, p.y);
  await page.waitForTimeout(350);
  check(await page.getByText('重命名', { exact: true }).count() === 0, 'another page breaks the click pair');
  check((await state()).opens === 6, 'rapid sequence opens all three page clicks');
  check(errors.length === 0, 'no uncaught browser errors');
  await page.screenshot({ path: resolve(output, 'sidebar-rename-saved.png') });
  await writeFile(resolve(output, 'sidebar-rename-results.json'), JSON.stringify({ ok: true, ...await state(), errors, scope: 'Real WorkspaceView/dialog, synthetic documents and callbacks; physical canvas mouse/keyboard; semantics only for labels and screenshots' }, null, 2));
} catch (error) {
  await page.screenshot({ path: resolve(output, 'sidebar-rename-failure.png') });
  console.error(await page.locator('flt-semantics').allTextContents());
  throw error;
} finally {
  await browser.close();
  await new Promise(done => server.close(done));
}
