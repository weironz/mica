import { chromium } from '@playwright/test';

// An explicit executable must work: falling back would hide a broken local
// setup. Otherwise use Playwright's matching browser, then an installed one.
export async function launchBrowser() {
  if (process.env.CHROME_EXECUTABLE) {
    return chromium.launch({ executablePath: process.env.CHROME_EXECUTABLE });
  }
  let failure;
  for (const channel of [undefined, 'chrome', 'msedge']) {
    try {
      return await chromium.launch(channel ? { channel } : {});
    } catch (error) {
      failure ??= error;
    }
  }
  throw failure;
}
