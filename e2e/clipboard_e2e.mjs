// Real Flutter editor keyboard path → browser ClipboardItem, in a tiny fixture.
// The fixture exposes only focus; these assertions use actual Ctrl+A/C events.
import { chromium } from '@playwright/test';

const baseArg = process.argv.indexOf('--base');
const base = baseArg >= 0 ? process.argv[baseArg + 1] : 'http://127.0.0.1:8091';
const source = 'print(1)\nprint(2)';
const crossBlock = `\`\`\`py\n${source}\n\`\`\`\n\nafter`;

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

async function expectCopy(label, expected) {
  await page.keyboard.press('Control+C');
  await page.waitForFunction(async text => {
    try { return (await navigator.clipboard.readText()) === text; }
    catch { return false; }
  }, expected, { timeout: 10_000 });
  const value = await clipboard();
  // Chromium's Windows ClipboardItem blob may use CRLF while readText() gives
  // LF. Compare their normalized content and reject any lone carriage return.
  const normalized = value.plain.replace(/\r\n/g, '\n');
  if (normalized !== expected || normalized.includes('\r') ||
      !value.types.includes('text/plain') ||
      !value.types.includes('text/html') ||
      !value.html.includes('<pre><code class="language-py">')) {
    throw new Error(`${label}: unexpected clipboard ${JSON.stringify(value)}`);
  }
  console.log(`ok  ${label}: ${JSON.stringify(value.plain)}`);
}

try {
  await page.goto(base, { waitUntil: 'load' });
  await page.waitForFunction(() => typeof window.micaClipboardHarnessFocus === 'function', null, {
    timeout: 60_000,
  });
  await page.evaluate(() => window.micaClipboardHarnessFocus());

  // The first Ctrl+A in a code block selects only its literal source. A second
  // Ctrl+A escalates to the whole document, which then copies as Markdown.
  await page.keyboard.press('Control+A');
  await expectCopy('single code block copies literal source without fences', source);
  await page.keyboard.press('Control+A');
  await expectCopy('cross-block copy retains Markdown fences', crossBlock);
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
