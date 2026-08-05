// Does typing Chinese with an IME actually work in the web editor?
//
// The roadmap has carried "Web IME/光标滚动实况调优 — Milestone 1 遗留(合成态/
// 游离换行、caret scroll-into-view)" as a one-line entry with NO measurement
// behind it, since Milestone 1. This measures it.
//
// Composition is driven through CDP `Input.imeSetComposition` — the same path
// real Chrome uses for a real IME. Hand-dispatching CompositionEvent from page
// script is NOT the same test: it bypasses the browser's own composition state
// machine, which is exactly the layer a "合成态" bug would live in.
//
//   node web_ime_probe.mjs --base http://127.0.0.1:8090 --api http://127.0.0.1:8081
import { chromium } from 'playwright';

const arg = (name, fallback) => {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 ? process.argv[i + 1] : fallback;
};
const BASE = arg('base', 'http://127.0.0.1:8090');
const API = arg('api', 'http://127.0.0.1:8081');
const SHOT = arg('shot', 'ime-probe.png');

const browser = await chromium.launch({ channel: 'chrome' });
const page = await browser.newPage({ viewport: { width: 1280, height: 860 } });
const errors = [];
page.on('pageerror', (e) => errors.push(String(e)));
page.on('console', (m) => m.type() === 'error' && errors.push(m.text()));

// The bundle's build-time API base is not this port, so tell the app where the
// API is the same way the app itself stores it.
await page.addInitScript((api) => {
  window.localStorage.setItem('mica.apiBase', api);
  window.localStorage.setItem('mica_api_base', api);
}, API);

await page.goto(BASE, { waitUntil: 'load' });
await page.waitForFunction(() => typeof window.micaYjsWebSyncTest === 'function', null, {
  timeout: 90_000,
});
console.log('app booted');

const cdp = await page.context().newCDPSession(page);

/// Type `text` as an IME composition: a preedit built up character by character,
/// then committed — the shape every CJK IME produces.
async function composeAndCommit(text) {
  for (let i = 1; i <= text.length; i++) {
    await cdp.send('Input.imeSetComposition', {
      text: text.slice(0, i),
      selectionStart: i,
      selectionEnd: i,
    });
    await page.waitForTimeout(40);
  }
  await cdp.send('Input.insertText', { text });
  await page.waitForTimeout(120);
}

// This probe deliberately does NOT assert a login flow: if the app parks on the
// login screen, typing Chinese into its fields still exercises the same Flutter
// web text stack, and the report says which surface it measured.
await page.waitForTimeout(1500);
const before = await page.evaluate(() => document.body.innerText.slice(0, 400));

const box = page.viewportSize();
await page.mouse.click(box.width / 2, box.height / 2);
await page.waitForTimeout(300);

await composeAndCommit('中文输入');
await page.waitForTimeout(400);
await page.screenshot({ path: '01-compose.png' });
console.log('[1] plain compose+commit -> 01-compose.png');

// 2. Enter WHILE composing. A real IME commits the preedit on Enter; the bug the
// roadmap calls 游离换行 is a newline landing as well, so the paragraph breaks
// where the user only meant to accept the candidate.
await cdp.send('Input.imeSetComposition', {
  text: '换行测试',
  selectionStart: 4,
  selectionEnd: 4,
});
await page.waitForTimeout(120);
await page.keyboard.press('Enter');
await page.waitForTimeout(300);
await cdp.send('Input.insertText', { text: '换行测试' });
await page.waitForTimeout(300);
await page.screenshot({ path: '02-enter-during-composition.png' });
console.log('[2] Enter during composition -> 02-enter-during-composition.png');

// 3. caret scroll-into-view: fill past one screen and see whether the caret is
// still on screen at the end. This is the half a user notices immediately —
// typing into a place you cannot see.
//
// At HUMAN cadence (~80 ms/keystroke) on purpose. A first version fired the
// keystrokes as fast as CDP would send them and the lines came out scrambled;
// slowing down produced perfectly ordered text, so the scrambling was the
// harness outrunning the editor, not a bug in it. The scroll finding below
// survives both speeds — which is what makes it a finding.
for (let i = 0; i < 28; i++) {
  await page.keyboard.press('Enter');
  await page.waitForTimeout(120);
  await page.keyboard.type(`SLOW-${String(i).padStart(2, '0')}`, { delay: 80 });
  await page.waitForTimeout(120);
}
await page.keyboard.type('<<<CARET-IS-HERE>>>', { delay: 60 });
await page.waitForTimeout(800);
await page.screenshot({ path: '03-caret-scroll.png' });
console.log('[3] 28 lines at human cadence -> 03-caret-scroll.png');
console.log('    EXPECTED (2026-08-05): the marker is NOT visible — the viewport');
console.log('    never followed the caret. That is the confirmed half of the entry.');

const after = await page.evaluate(() => document.body.innerText.slice(0, 400));
await page.screenshot({ path: SHOT, fullPage: false });

console.log('--- what the DOM says ---');
console.log('before:', JSON.stringify(before.slice(0, 160)));
console.log('after :', JSON.stringify(after.slice(0, 160)));
console.log('pageerrors:', errors.length ? errors.join(' | ').slice(0, 400) : '(none)');
console.log(`screenshot: ${SHOT}`);

await browser.close();
