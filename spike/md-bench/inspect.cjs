async (page) => {
  return await page.evaluate(() => ({bench:typeof globalThis.runMdP0,old:globalThis.__benchResult,init:globalThis.micaMdInitMs,error:globalThis.micaMdError,isolated:crossOriginIsolated}));
}
