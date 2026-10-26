async (page) => {
  await page.waitForFunction(() => typeof globalThis.runMdP0 === 'function', {timeout:120000});
  const result = await page.evaluate(async () => JSON.parse(await globalThis.runMdP0()));
  console.log(JSON.stringify(result));
  return {browser: await page.evaluate(() => navigator.userAgent), ...result};
}
