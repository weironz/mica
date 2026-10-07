// Real Flutter editor keyboard path → browser ClipboardItem, in a tiny fixture.
// The fixture exposes only focus; these assertions use actual Ctrl+A/C events.
import { chromium } from '@playwright/test';

const baseArg = process.argv.indexOf('--base');
const base = baseArg >= 0 ? process.argv[baseArg + 1] : 'http://127.0.0.1:8091';
const source = 'print(1)\nprint(2)';
const crossBlock = `${source}\n\nafter`;

async function launch() {
  try {
    return await chromium.launch();
  } catch (bundled) {
    for (const channel of ['chrome', 'msedge']) {
      try {
        return await chromium.launch({ channel });
      } catch {
        // Try the next installed browser; CI uses the bundled Chromium.
      }
    }
    throw bundled;
  }
}

const browser = await launch();
const context = await browser.newContext({
  locale: 'en-US',
  timezoneId: 'UTC',
  permissions: ['clipboard-read', 'clipboard-write'],
});
const page = await context.newPage();
const pageErrors = [];
page.on('pageerror', error => pageErrors.push(String(error)));

async function clipboard() {
  return page.evaluate(async () => {
    const items = await navigator.clipboard.read();
    if (items.length !== 1) throw new Error(`expected one ClipboardItem, got ${items.length}`);
    const item = items[0];
    return {
      types: item.types,
      plain: await (await item.getType('text/plain')).text(),
      html: await (await item.getType('text/html')).text(),
    };
  });
}

async function expectCopy(label, expected, rich = '<pre><code class="language-py">', key = 'Control+C') {
  if (key) await page.keyboard.press(key);
  let value;
  const deadline = Date.now() + 10_000;
  do {
    value = await clipboard().catch(() => null);
    if (value?.plain.replace(/\r\n/g, '\n') === expected && value.html.includes(rich)) break;
    await page.waitForTimeout(100);
  } while (Date.now() < deadline);
  if (!value) throw new Error(`${label}: clipboard has no rich item`);
  // Chromium's Windows ClipboardItem blob may use CRLF while readText() gives
  // LF. Compare their normalized content and reject any lone carriage return.
  const normalized = value.plain.replace(/\r\n/g, '\n');
  if (normalized !== expected || normalized.includes('\r') ||
      !value.types.includes('text/plain') ||
      !value.types.includes('text/html') ||
      !value.html.includes(rich)) {
    throw new Error(`${label}: unexpected clipboard ${JSON.stringify(value)}`);
  }
  console.log(`ok  ${label}: ${JSON.stringify(value.plain)}`);
  return value;
}

async function openFixture(fixture = '', legacy = false) {
  await page.goto(`${base}${fixture ? `?fixture=${fixture}` : ''}`, { waitUntil: 'load' });
  await page.waitForFunction(() => typeof window.micaClipboardHarnessFocus === 'function', null, {
    timeout: 60_000,
  });
  if (legacy) {
    // Exercise production execCommand fallback while leaving clipboard.read
    // available to verify what really landed in the OS clipboard.
    await page.evaluate(() => {
      Object.defineProperty(window, 'ClipboardItem', { value: undefined, configurable: true });
    });
  }
  await page.evaluate(() => window.micaClipboardHarnessFocus());
  await page.waitForTimeout(500);
  await page.screenshot({ path: `clipboard-fixture-${fixture || 'code'}.png`, fullPage: true });
}

async function pasteDestinations(expected) {
  const target = await context.newPage();
  try {
    // A separate page has no Mica paste capture handler. Both targets consume
    // the same real system clipboard through native browser Ctrl+V.
    await target.setContent('<textarea id="plain"></textarea><div id="rich" contenteditable="true"></div>');
    await target.locator('#plain').focus();
    await target.keyboard.press('Control+V');
    const plain = await target.locator('#plain').inputValue();
    if (plain.replace(/\r\n/g, '\n') !== expected) throw new Error(`plain paste: ${JSON.stringify(plain)}`);
    await target.locator('#rich').focus();
    await target.keyboard.press('Control+V');
    const code = await target.locator('#rich code').textContent();
    if (code !== 'command') throw new Error(`rich paste lost code: ${code}`);
  } finally {
    await target.close();
    await page.bringToFront();
  }
}

try {
  await openFixture();
  await page.keyboard.press('Control+A');
  await expectCopy('single code block copies literal source without fences', source);
  await page.keyboard.press('Control+A');
  await expectCopy('cross-block copy has literal source and rich code', crossBlock);

  for (const legacy of [false, true]) {
    const mode = legacy ? 'legacy' : 'ClipboardItem';
    await openFixture('inline', legacy);
    // Real pointer double click selects the word within the inline-code mark.
    await page.mouse.dblclick(85, 16);
    await expectCopy(`${mode}: double-click inline code`, 'command', '<code>command</code>');
    await pasteDestinations('command');
    await page.screenshot({ path: `clipboard-${mode}-inline.png`, fullPage: true });

    await page.evaluate(() => window.micaClipboardHarnessFocus());
    await page.keyboard.press('Home');
    await page.keyboard.press('Shift+End');
    await expectCopy(`${mode}: whole line with code`, 'run command then `literal`', '<code>command</code>');
    await pasteDestinations('run command then `literal`');

    await page.evaluate(() => window.micaClipboardHarnessFocus());
    await page.keyboard.press('Control+A');
    await expectCopy(`${mode}: cross-block inline code`, 'run command then `literal`\n\nafter', '<code>command</code>');
    await page.evaluate(() => window.micaClipboardHarnessFocus());
    await page.keyboard.press('Home');
    await page.keyboard.press('Shift+End');
    await expectCopy(`${mode}: cut uses same flavors`, 'run command then `literal`', '<code>command</code>', 'Control+X');

    await openFixture('table', legacy);
    // Click the first cell to open its real WYSIWYG TextField.
    await page.mouse.click(85, 30);
    await page.waitForTimeout(500);
    await page.keyboard.press('Control+A');
    await expectCopy(`${mode}: cell copy clips stored code marks`, 'command then `literal`', '<code>command</code>');
    await pasteDestinations('command then `literal`');
    await page.screenshot({ path: `clipboard-${mode}-cell.png`, fullPage: true });
  }
  if (pageErrors.length) throw new Error(`page errors: ${pageErrors.join(' | ')}`);
  console.log('clipboard e2e: all assertions passed');
} catch (error) {
  await page.screenshot({ path: 'clipboard-e2e-failure.png', fullPage: true }).catch(() => {});
  console.error(`clipboard e2e failed: ${error}`);
  console.error(`page errors: ${pageErrors.join(' | ') || '(none)'}`);
  process.exitCode = 1;
} finally {
  await browser.close();
}
