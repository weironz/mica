// Real production dialogs behind Dart deferred imports; no external account.
// Reuse CI's unified bundle, or --directory <isolated web build> for a local run.
import { expect } from '@playwright/test';
import { launchBrowser } from './browser.mjs';
import { createServer } from 'node:http';
import { access, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

function argument(name) {
  const index = process.argv.indexOf(name);
  if (index < 0) return undefined;
  const value = process.argv[index + 1];
  if (!value || value.startsWith('--')) throw new Error(`${name} requires a value`);
  return value;
}
const externalBase = argument('--base');
const directory = argument('--directory');
if (externalBase && directory) throw new Error('Use either --base or --directory');
const root = resolve(directory || fileURLToPath(new URL('../clients/mica_flutter/build/web_harness/', import.meta.url)));
const output = resolve(argument('--results') || fileURLToPath(new URL('../clients/mica_flutter/build/deferred-modules-results/', import.meta.url)));
await mkdir(output, { recursive: true });
// Only remove artifacts this probe owns; a rerun must not upload an old failure
// screenshot or treat an old result receipt as evidence for the current run.
await Promise.all([
  'deferred-ai.png', 'deferred-history.png', 'deferred-graph.png',
  'deferred-download-error.png', 'deferred-download-recovered.png',
  'deferred-failure.png', 'deferred-modules-results.json',
].map(name => rm(resolve(output, name), {force: true})));
const mime = {'.html': 'text/html', '.js': 'application/javascript', '.wasm': 'application/wasm', '.json': 'application/json', '.ttf': 'font/ttf', '.otf': 'font/otf', '.woff2': 'font/woff2', '.png': 'image/png', '.svg': 'image/svg+xml'};
let server;
let browser;
let currentPage;
const results = [];
const errors = [];
const traces = [];
const isPart = url => /\.part\.js$/.test(new URL(url).pathname);
function check(label, ok, detail) {
  results.push({label, ok, ...(detail === undefined ? {} : {detail: structuredClone(detail)})});
  console.log(`${ok ? 'ok' : 'FAIL'} ${label}${detail === undefined ? '' : ': ' + JSON.stringify(detail)}`);
  if (!ok) throw new Error(label);
}
async function state(page) { return page.evaluate(() => JSON.parse(window.micaDeferredState())); }
async function ready(page, base) {
  currentPage = page;
  page.on('pageerror', error => errors.push(String(error)));
  await page.goto(base);
  await page.waitForFunction(() => typeof window.micaDeferredState === 'function', null, {timeout: 60000});
  const semantics = page.locator('flt-semantics-placeholder');
  await semantics.waitFor({state: 'attached'});
  await semantics.evaluate(element => element.click());
  await page.getByRole('button', {name: 'ai', exact: true}).waitFor();
  await page.waitForLoadState('networkidle');
}
const titles = {ai: '问 AI', history: '版本历史', graph: '关系图'};
async function open(page, kind) {
  await page.getByRole('button', {name: kind, exact: true}).click();
  await expect.poll(async () => (await state(page)).active, {message: `${kind} dialog active`, timeout: 30000}).toBe(kind);
  await expect.poll(async () => (await state(page)).dialogMounted, {message: `${kind} production widget mounted`}).toBe(true);
  if (kind === 'ai') await page.getByText(titles[kind], {exact: true}).waitFor();
  else if (kind === 'history') await page.getByRole('button', {name: '保存检查点', exact: true}).waitFor();
  else {
    // The current graph close IconButton has no accessible name. Its real
    // widget mount plus its visible sole foreground button proves the route;
    // this test does not pretend that the missing name is an a11y pass.
    await expect(page.getByRole('button')).toHaveCount(1);
    await page.getByRole('button').waitFor();
  }
  await page.waitForLoadState('networkidle');
}
async function close(page) {
  await page.keyboard.press('Escape');
  await expect.poll(async () => (await state(page)).active, {message: 'dialog closed by Escape'}).toBeNull();
}

try {
  let origin = externalBase;
  if (!origin) {
    await access(resolve(root, 'index.html'));
    server = createServer(async (req, res) => {
      try {
        const pathname = decodeURIComponent(new URL(req.url, 'http://localhost').pathname);
        const file = resolve(root, '.' + (pathname === '/' ? '/index.html' : pathname));
        if (!file.startsWith(root + sep)) { res.writeHead(403).end(); return; }
        const bytes = await readFile(file);
        res.writeHead(200, {'content-type': mime[extname(file)] || 'application/octet-stream', 'cache-control': 'no-store'}).end(bytes);
      } catch { res.writeHead(404).end(); }
    });
    await new Promise((done, reject) => {server.once('error', reject);server.listen(0, '127.0.0.1', done);});
    origin = `http://127.0.0.1:${server.address().port}`;
  }
  const base = new URL(origin);
  base.searchParams.set('harness', 'deferred_modules');
  browser = await launchBrowser();
  const context = await browser.newContext({locale: 'zh-CN', viewport: {width: 1280, height: 900}, serviceWorkers: 'block'});
  const page = await context.newPage();
  const parts = [];
  context.on('request', request => {if (isPart(request.url())) parts.push(request.url());});
  await ready(page, base.toString());
  check('initial harness does not request any deferred part', parts.length === 0, parts);
  for (const kind of ['ai', 'history', 'graph']) {
    const before = parts.length;
    await open(page, kind);
    const requested = parts.slice(before);
    check(`${kind} real dialog appears after its first deferred download`, requested.length > 0, requested.map(url => new URL(url).pathname));
    const semantics = await page.locator('flt-semantics').evaluateAll(nodes => nodes.map(node => ({
      role: node.getAttribute('role'), label: node.getAttribute('aria-label'), text: node.textContent,
    })));
    traces.push({kind, requested, semantics});
    await page.screenshot({path: resolve(output, `deferred-${kind}.png`)});
    await close(page);
    const loaded = parts.length;
    await open(page, kind);
    check(`${kind} reopening does not request a part again`, parts.length === loaded, parts.length - loaded);
    await close(page);
  }
  await context.close();

  // A fresh context has neither registered libraries nor a service-worker cache.
  const cold = await browser.newContext({locale: 'zh-CN', viewport: {width: 1280, height: 900}, serviceWorkers: 'block'});
  const failed = await cold.newPage();
  const blocked = [];
  const recovered = [];
  const partRoute = url => isPart(url.href);
  await failed.route(partRoute, async route => {blocked.push(route.request().url());await route.abort('failed');});
  cold.on('requestfinished', request => {if (isPart(request.url())) recovered.push(request.url());});
  await ready(failed, base.toString());
  check('cold error fixture also has no eager part request', blocked.length === 0, blocked);
  await failed.getByRole('button', {name: 'ai', exact: true}).click();
  await failed.locator('flt-semantics').getByText('组件加载失败，请联网后重试。', {exact: true}).last().waitFor({timeout: 30000});
  check('blocked download shows localized error and keeps the page intact', blocked.length > 0 && (await state(failed)).active === null, blocked.length);
  await failed.screenshot({path: resolve(output, 'deferred-download-error.png')});
  await failed.unroute(partRoute);
  await open(failed, 'ai');
  check('reopening after the network is restored downloads and shows the dialog', recovered.length > 0, recovered.map(url => new URL(url).pathname));
  await failed.screenshot({path: resolve(output, 'deferred-download-recovered.png')});
  await close(failed);
  const recoveredCount = recovered.length;
  await open(failed, 'ai');
  check('recovered library also remains loaded on subsequent reopening', recovered.length === recoveredCount, recovered.length - recoveredCount);
  await close(failed);
  check('no uncaught browser exceptions', errors.length === 0, errors);
  await cold.close();
  await writeFile(resolve(output, 'deferred-modules-results.json'), JSON.stringify({results, traces, errors, blocked, recovered}, null, 2));
} catch (error) {
  if (currentPage && !currentPage.isClosed()) console.log(await currentPage.locator('flt-semantics').allTextContents());
  if (currentPage && !currentPage.isClosed()) await currentPage.screenshot({path: resolve(output, 'deferred-failure.png')}).catch(() => {});
  await writeFile(resolve(output, 'deferred-modules-results.json'), JSON.stringify({results, traces, errors, failure: String(error), stack: error.stack}, null, 2));
  throw error;
} finally {
  if (browser) await browser.close();
  if (server) await new Promise(done => server.close(done));
}
