// Real browser inputs against production widgets; all content/accounts synthetic.
// Build clients/mica_flutter/e2e/ui_experience_harness.dart into build/ui_experience.
import { chromium } from '@playwright/test';
import { createServer } from 'node:http';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../clients/mica_flutter/build/ui_experience/', import.meta.url));
const output = process.env.MICA_UI_RESULTS_DIR || fileURLToPath(new URL('../docs/code-optimization/assets/', import.meta.url));
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
await new Promise(done => server.listen(8093, '127.0.0.1', done));
const base = 'http://127.0.0.1:8093';
let browser;
for (const channel of [undefined, 'chrome', 'msedge']) {
  try { browser = await chromium.launch(channel ? {channel} : {}); break; } catch {}
}
if (!browser) { server.close(); throw new Error('No Chromium browser available'); }
const context = await browser.newContext({ locale: 'zh-CN', timezoneId: 'Asia/Shanghai', viewport: { width: 1280, height: 900 } });
const page = await context.newPage();
const errors = [];
page.on('pageerror', error => errors.push(String(error)));
const results = [];
function check(label, ok, detail = '') {
  results.push({label, ok, detail});
  console.log(`${ok ? 'ok' : 'FAIL'} ${label}${detail ? ': ' + detail : ''}`);
  if (!ok) throw new Error(label + ': ' + detail);
}
async function state() { return page.evaluate(() => JSON.parse(window.micaExperienceState())); }
async function semantics(target) {
  const placeholder = target.locator('flt-semantics-placeholder');
  if (await placeholder.count()) await placeholder.evaluate(element => element.click());
}
async function shot(name, target = page) { await target.screenshot({ path: resolve(output, name), fullPage: false }); }

try {
  await page.goto(base);
  await page.waitForFunction(() => typeof window.micaExperienceState === 'function', null, { timeout: 60000 });
  await page.waitForLoadState('networkidle');
  await page.evaluate(() => window.micaExperienceFocus());
  await page.waitForTimeout(300);
  const before = await state();
  for (let i = 0; i < 12; i++) { await page.keyboard.press('ArrowRight'); await page.waitForTimeout(20); }
  const after = await state();
  check('long document caret moves keep layout', after.layouts === before.layouts && after.selection.offset === before.selection.offset + 12,
    `${after.nodes} blocks, layouts ${before.layouts} → ${after.layouts}`);
  await page.keyboard.insertText('即时输入');
  await page.waitForTimeout(200);
  const typed = await state();
  check('real text input updates content and layout', typed.text.includes('即时输入') && typed.revision > after.revision && typed.layouts > after.layouts);
  await page.keyboard.press('Control+Z');
  await page.waitForTimeout(200);
  check('undo restores original content', (await state()).text === before.text);

  // Browser's actual text-input channel, including composing and committed text.
  const cdp = await context.newCDPSession(page);
  await cdp.send('Input.imeSetComposition', { text: '候选中文', selectionStart: 4, selectionEnd: 4 });
  await page.waitForTimeout(150);
  check('IME preedit renders once', (await state()).text.includes('候选中文'));
  await page.keyboard.insertText('候选中文');
  await page.waitForTimeout(150);
  const committed = await state();
  check('IME commit keeps one copy', committed.text.split('候选中文').length === 2);
  await page.keyboard.press('Control+Z');
  await page.waitForTimeout(200);
  await semantics(page);
  await page.keyboard.press('Home');
  await page.waitForTimeout(100);
  const dragStart = (await state()).caret;
  await page.evaluate(() => {
    window.micaFrameGaps = [];
    window.micaFrameTracking = true;
    let previous;
    const frame = now => {
      if (previous !== undefined) window.micaFrameGaps.push(now - previous);
      previous = now;
      if (window.micaFrameTracking) requestAnimationFrame(frame);
    };
    requestAnimationFrame(frame);
  });
  await page.mouse.move(dragStart.x + 2, dragStart.y + dragStart.height / 2);
  await page.mouse.down();
  await page.mouse.move(dragStart.x + 160, dragStart.y + dragStart.height / 2, {steps: 15});
  await page.waitForTimeout(150);
  check('drag selects text without showing format toolbar', !(await state()).selection.collapsed && await page.getByText('T1', {exact:true}).count() === 0);
  await page.mouse.up();
  await page.getByText('T1', {exact:true}).waitFor();
  check('format toolbar appears after drag release', await page.getByText('T1', {exact:true}).count() === 1);
  const frames = await page.evaluate(() => {
    window.micaFrameTracking = false;
    const gaps = window.micaFrameGaps.sort((a,b) => a-b);
    return {samples: gaps.length, medianMs: gaps[Math.floor(gaps.length / 2)], p95Ms: gaps[Math.floor(gaps.length * .95)]};
  });
  results.push({label: 'browser frame intervals during selection (observation, not an FPS guarantee)', ok: true, detail: frames});
  await page.keyboard.press('ArrowRight');
  await page.waitForTimeout(100);
  await shot('ui-experience-light.png');
  await page.getByRole('button', { name: '切换主题', exact: true }).click();
  await page.waitForTimeout(200);
  check('theme updates the actual editor', (await state()).dark);
  await shot('ui-experience-dark.png');

  await page.setViewportSize({ width: 390, height: 844 });
  await page.waitForTimeout(200);
  await shot('ui-experience-mobile.png');
  await page.getByRole('button', { name: '设置', exact: true }).click();
  await page.getByText('外观', { exact: false }).click();
  await page.getByRole('button', { name: '返回', exact: true }).waitFor();
  await shot('ui-experience-settings-mobile.png');
  await page.getByRole('button', { name: '返回', exact: true }).click({force: true});
  await page.getByRole('button', { name: '关闭', exact: true }).click({force: true});
  check('mobile settings navigation and close work', await page.getByText('排版预览', { exact: false }).count() === 0);

  // Full production app and private SettingsDialog, using a local fake session.
  const full = await context.newPage();
  await full.setViewportSize({ width: 1280, height: 900 });
  full.on('pageerror', error => errors.push(String(error)));
  const user = { id: 'ui-test-user', email: 'ui-test@mica.test', display_name: 'UI Test' };
  await full.addInitScript(({base, user}) => {
    localStorage.setItem('mica.cloudOrigin', base);
    localStorage.setItem('mica.activeOrigin', base);
    localStorage.setItem('mica.servers', JSON.stringify([base]));
    localStorage.setItem(`mica.authUser:${base}`, JSON.stringify(user));
    localStorage.setItem('mica.uiLanguage', 'zh');
  }, {base, user});
  const pendingAi = [];
  let aiMode = 'hold';
  let aiRequests = 0;
  await full.route('**/api/**', async route => {
    const path = new URL(route.request().url()).pathname;
    const json = body => route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
    if (path === '/api/ai/settings') {
      aiRequests++;
      if (aiMode === 'hold') { pendingAi.push(route); return; }
      if (aiMode === 'fail') return route.fulfill({ status: 503, contentType: 'application/json', body: JSON.stringify({error: 'UI test failure'}) });
      return json({ providers: [], has_key: false, model: '', base_url: '', protocol: 'openai' });
    }
    if (path === '/api/auth/refresh') return json({ access_token: 'local-ui-test-token', user });
    if (path === '/api/auth/me') return json({user});
    if (path === '/api/auth/me/settings') return json({settings: {}});
    if (path === '/api/workspaces') return json({workspaces: []});
    return json({registration_open: false, workspaces: [], tokens: [], imports: []});
  });
  await full.goto(`${base}/?fixture=app`);
  await full.waitForTimeout(1800);
  await semantics(full);
  await full.getByText('UI Test', {exact: false}).first().click({force: true});
  await full.getByText('设置', {exact: true}).last().click({force: true});
  await full.getByText('外观', { exact: false }).first().waitFor({timeout: 10000});
  check('production appearance opens while AI requests hang', aiRequests > 0 && pendingAi.length > 0 && await full.getByText('字号', { exact: true }).count() > 0);
  await shot('ui-experience-settings-desktop.png', full);
  await full.getByText('AI 服务商', { exact: false }).first().click({force: true});
  aiMode = 'fail';
  await Promise.all(pendingAi.splice(0).map(route => route.fulfill({status: 503, contentType: 'application/json', body: JSON.stringify({error: 'UI test failure'})})));
  await full.getByRole('button', {name: '重试', exact: true}).waitFor({timeout: 10000});
  check('production AI failure has local retry', await full.getByRole('button', {name: '重试', exact: true}).count() === 1);
  await shot('ui-experience-settings-ai-error.png', full);
  aiMode = 'ok';
  const requestsBefore = aiRequests;
  await full.getByRole('button', {name: '重试', exact: true}).click({force: true});
  await full.waitForTimeout(400);
  check('production AI retry issues new request and clears error', aiRequests > requestsBefore && await full.getByRole('button', {name: '重试', exact: true}).count() === 0);
  await full.getByText('外观', { exact: false }).first().click({force: true});
  await full.setViewportSize({width:390, height:844});
  await full.waitForTimeout(200);
  // On a breakpoint change a detail may stay open, or the shell may show its
  // category list. Either path must offer reachable navigation and close.
  if (await full.getByRole('button', {name: '返回', exact: true}).count() === 0)
    await full.getByText('外观', {exact: false}).first().click({force: true});
  await full.getByRole('button', {name:'返回', exact:true}).waitFor();
  await shot('ui-experience-production-settings-mobile.png', full);
  await full.getByRole('button', {name:'关闭', exact:true}).click({force: true});
  await full.getByRole('button', {name:'返回', exact:true}).waitFor({state: 'hidden'});
  check('production mobile settings closes', await full.getByRole('button', {name:'返回', exact:true}).count() === 0);
  check('no uncaught browser exceptions', errors.length === 0, errors.join(' | '));
  await writeFile(resolve(output, 'ui-experience-results.json'), JSON.stringify({results, errors}, null, 2));
} catch (error) {
  const failedPage = context.pages().at(-1) || page;
  console.log(await failedPage.locator('flt-semantics').allTextContents());
  await shot('ui-experience-failure.png', failedPage);
  await writeFile(resolve(output, 'ui-experience-results.json'), JSON.stringify({results, errors, failure: String(error)}, null, 2));
  throw error;
} finally {
  await browser.close();
  await new Promise(done => server.close(done));
}
