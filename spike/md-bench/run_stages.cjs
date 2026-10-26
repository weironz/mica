// Runs every isolated stage on the primary P0 inputs + the S0-shaped inputs.
async (page) => {
  await page.waitForFunction(() => typeof globalThis.runMdP0 === 'function', {timeout: 180000});
  const result = await page.evaluate(async () => JSON.parse(await globalThis.runMdP0()));
  return {
    browser: await page.evaluate(() => navigator.userAgent),
    isolated: await page.evaluate(() => crossOriginIsolated),
    initMs: await page.evaluate(() => globalThis.micaMdInitMs),
    ...result,
  };
}
