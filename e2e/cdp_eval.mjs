// Playwright's predicate evaluator creates Functions in the page and therefore
// reports unsafe-eval under CSP Report-Only. DevTools evaluation can read the
// same predicate without changing the app policy or filtering its console.
const sessions = new WeakMap();
export async function evaluatePage(page, fn, arg) {
  let session = sessions.get(page);
  if (!session) {
    session = await page.context().newCDPSession(page);
    sessions.set(page, session);
  }
  const args = arg === undefined ? [] : [arg];
  const response = await session.send('Runtime.evaluate', {
    expression: `(${fn.toString()})(...${JSON.stringify(args)})`,
    awaitPromise: true,
    allowUnsafeEvalBlockedByCSP: false,
    returnByValue: true,
  });
  if (response.exceptionDetails) {
    throw new Error(response.exceptionDetails.exception?.description || response.exceptionDetails.text);
  }
  return response.result.value;
}
export async function waitForPage(page, predicate, timeout) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await evaluatePage(page, predicate)) return;
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw new Error(`Browser predicate did not match within ${timeout}ms`);
}
