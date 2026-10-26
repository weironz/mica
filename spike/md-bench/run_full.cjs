// Full-path only (fast): used for the opt-level comparison rounds.
async (page) => {
  await page.waitForFunction(() => typeof globalThis.runMdP0 === 'function', {timeout: 180000});
  const result = await page.evaluate(async () =>
    JSON.parse(await globalThis.runMdP0(
      ['p0_87', 'p0_10k', 'p0_100k', 's0_10k', 's0_100k'],
      ['full', 'dartMarkdown', 'parseBody', 'parseOnly'])));
  return {
    browser: await page.evaluate(() => navigator.userAgent),
    initMs: await page.evaluate(() => globalThis.micaMdInitMs),
    ...result,
  };
}
