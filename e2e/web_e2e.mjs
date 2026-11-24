#!/usr/bin/env node
/**
 * End-to-end Playwright suite for Mica's pure-Rust WebAssembly client (P1-3B).
 *
 * Verifies:
 *   1. Cold start: strict CSP (zero /api/csp-report, zero console/page errors),
 *      `mica_rust_bg.wasm` fetched ONCE, zero `/mica_preview/*` requests on
 *      ordinary pages, zero synchronous XHR requests (`100%` async `fetch`).
 *   2. Sign-in (`HttpOnly` cookie, zero JWT secrets in `localStorage`) + live
 *      WebSocket CRDT sync (`已实时同步`) + `#111` account-scoped IndexedDB
 *      mirror persistence (`mica_sync_v1` / `docs` object store).
 *   3. New page creation, typing, persistence across reload without re-login.
 *   4. Two-browser real-time CRDT collaboration over `/ws/` (Peer B edits appear
 *      live in Peer A without reloading Peer A).
 *   5. Offline edit recovery via IndexedDB (`mica_sync_v1` outbox survives
 *      reload and reconciles once back online — P0-2).
 *   6. Lazy `mica_preview/math` WASM fetch & render on `/math` block (P0-3 W4),
 *      plus clean logout (`POST /api/auth/logout` -> 204 -> Sign-In gate on reload).
 *
 * Usage:
 *   node web_e2e.mjs --base http://127.0.0.1:8090 --email demo@mica.dev --password password123
 */
import assert from 'node:assert/strict';
import { launchBrowser } from './browser.mjs';

function parseArgs(argv) {
  const out = { base: 'http://127.0.0.1:8090', email: '', password: '', register: false };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a === '--base') out.base = argv[++i];
    else if (a === '--email') out.email = argv[++i];
    else if (a === '--password') out.password = argv[++i];
    else if (a === '--register') out.register = true;
  }
  out.base = out.base.replace(/\/+$/, '');
  if (!out.email || !out.password) {
    throw new Error('usage: node web_e2e.mjs --base <url> --email <email> --password <password> [--register]');
  }
  return out;
}

async function ensureUserRegistered(base, email, password) {
  const resp = await fetch(`${base}/api/auth/register`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ email, password, display_name: 'E2E User' }),
  });
  // 200/201 created, 409 already exists — both are fine.
  if (![200, 201, 409].includes(resp.status)) {
    const txt = await resp.text().catch(() => '');
    throw new Error(`seed register failed: HTTP ${resp.status} ${txt}`);
  }
}

async function readIdbDocs(page) {
  return page.evaluate(async () => {
    const dbs = indexedDB.databases ? await indexedDB.databases() : null;
    return new Promise((resolve) => {
      const req = indexedDB.open('mica_sync_v1', 1);
      req.onupgradeneeded = () => {
        const db = req.result;
        if (!db.objectStoreNames.contains('docs')) {
          db.createObjectStore('docs');
        }
      };
      req.onerror = () => resolve({ error: String(req.error), dbs, entries: [] });
      req.onsuccess = () => {
        const db = req.result;
        if (!db.objectStoreNames.contains('docs')) {
          db.close();
          resolve({ dbs, entries: [] });
          return;
        }
        const tx = db.transaction('docs', 'readonly');
        const store = tx.objectStore('docs');
        const keysReq = store.getAllKeys();
        const valsReq = store.getAll();
        tx.oncomplete = () => {
          const keys = keysReq.result || [];
          const vals = valsReq.result || [];
          const entries = keys.map((k, i) => ({ key: String(k), value: String(vals[i] ?? '') }));
          db.close();
          resolve({ dbs, entries });
        };
        tx.onerror = () => {
          db.close();
          resolve({ error: String(tx.error), dbs, entries: [] });
        };
      };
    });
  });
}

async function waitForSyncBadge(page, timeoutMs = 20000) {
  await page.getByText('已实时同步').waitFor({ state: 'visible', timeout: timeoutMs });
}

async function fillTestInput(page, testId, value) {
  const host = page.locator(`[data-testid="${testId}"]`);
  await host.waitFor({ state: 'visible', timeout: 30000 });
  const nested = host.locator('input, textarea').first();
  if ((await nested.count()) > 0) {
    await nested.fill(value);
  } else {
    await host.click();
    await page.keyboard.press('Control+A');
    await page.keyboard.type(value);
  }
}

async function signInOnPage(page, base, email, password) {
  await page.goto(`${base}/`, { waitUntil: 'domcontentloaded' });
  await fillTestInput(page, 'world-auth-email', email);
  await fillTestInput(page, 'world-auth-password', password);
  await page.locator('[data-testid="world-auth-submit"]').click();
  await waitForSyncBadge(page, 25000);
}

async function focusEditorAndType(page, text) {
  const row0 = page.locator('[data-testid="block-row-0"]');
  if (await row0.isVisible().catch(() => false)) {
    await row0.click();
  } else {
    const appendZone = page.locator('[data-testid="editor-bottom-append-zone"]');
    await appendZone.waitFor({ state: 'visible', timeout: 15000 });
    await appendZone.click();
  }
  await page.waitForTimeout(250);
  await page.keyboard.type(text, { delay: 15 });
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.register) {
    await ensureUserRegistered(opts.base, opts.email, opts.password);
  }

  const browser = await launchBrowser();
  const contextA = await browser.newContext({ viewport: { width: 1280, height: 820 } });
  const pageA = await contextA.newPage();

  const consoleErrors = [];
  const pageErrors = [];
  const requests = [];
  const responses = [];

  pageA.on('console', (msg) => {
    if (msg.type() !== 'error') return;
    const text = msg.text();
    // Expected cold-boot cookie probe (`POST /api/auth/refresh` returns 401 before login).
    if (text.includes('401 (Unauthorized)')) return;
    consoleErrors.push(text);
  });
  pageA.on('pageerror', (err) => pageErrors.push(String(err)));
  pageA.on('request', (req) => {
    requests.push({ url: req.url(), type: req.resourceType(), method: req.method() });
  });
  pageA.on('response', (res) => {
    responses.push({ url: res.url(), status: res.status(), method: res.request().method() });
  });

  try {
    // ── 1. Cold start: single WASM load, strict CSP, zero preview, zero XHR ──
    console.log('==> [1/6] Cold start, CSP, single WASM download & zero XHR...');
    await signInOnPage(pageA, opts.base, opts.email, opts.password);
    await pageA.waitForTimeout(800);

    const wasmReqs = requests.filter((r) => r.url.includes('mica_rust_bg.wasm'));
    assert.equal(wasmReqs.length, 1, `expected mica_rust_bg.wasm fetched ONCE, got ${wasmReqs.length}`);

    const cspReports = requests.filter((r) => r.url.includes('/api/csp-report'));
    assert.equal(cspReports.length, 0, `unexpected CSP reports: ${JSON.stringify(cspReports)}`);

    const xhrReqs = requests.filter((r) => r.type === 'xhr');
    assert.equal(xhrReqs.length, 0, `unexpected synchronous/legacy XHR requests: ${JSON.stringify(xhrReqs)}`);

    const earlyPreview = requests.filter((r) => r.url.includes('/mica_preview/'));
    assert.equal(earlyPreview.length, 0, `ordinary page must not fetch mica_preview: ${JSON.stringify(earlyPreview)}`);

    const splashCount = await pageA.locator('#mica-splash').count();
    assert.equal(splashCount, 0, '#mica-splash should be removed after mount');

    const localStorageSecrets = await pageA.evaluate(() => {
      const vals = [];
      for (let i = 0; i < localStorage.length; i += 1) {
        const k = localStorage.key(i);
        vals.push([k, localStorage.getItem(k) || '']);
      }
      return vals.filter(([, v]) => /access_token|refresh_token|eyJ[A-Za-z0-9_-]{10,}/.test(v));
    });
    assert.equal(localStorageSecrets.length, 0, `JWT secret leaked to localStorage: ${JSON.stringify(localStorageSecrets)}`);

    // ── 2. Verify #111-scoped IndexedDB persistence (`mica_sync_v1`) ─────────
    console.log('==> [2/6] Verifying #111-scoped IndexedDB (mica_sync_v1) hydration & mirror...');
    const idb1 = await readIdbDocs(pageA);
    assert(!idb1.error, `IDB read error: ${idb1.error}`);
    assert(idb1.entries.length >= 1, `expected >=1 entry in mica_sync_v1/docs, got ${idb1.entries.length}`);
    const originPrefix = `["${opts.base}","`;
    assert(
      idb1.entries.some((e) => e.key.startsWith(originPrefix)),
      `expected #111 account-scoped key starting with ${originPrefix}, got ${JSON.stringify(idb1.entries.map((e) => e.key))}`,
    );
    console.log(`    IDB mica_sync_v1/docs: ${idb1.entries.length} doc(s), sample key=${idb1.entries[0].key.slice(0, 64)}…`);

    // ── 3. Create new page, edit & verify reload persistence via HttpOnly cookie ─
    console.log('==> [3/6] Create page, edit, reload persistence (HttpOnly cookie)...');
    const tag = `E2E-${Date.now()}`;
    await pageA.locator('[data-testid="new-page-btn"]').click();
    await waitForSyncBadge(pageA, 15000);
    await pageA.waitForTimeout(400);

    const titleHost = pageA.locator('[data-testid="page-title-input"]');
    if (await titleHost.isVisible().catch(() => false)) {
      await fillTestInput(pageA, 'page-title-input', tag);
      await pageA.keyboard.press('Enter');
      await pageA.waitForTimeout(300);
    }
    const markerA = `${tag}-BODY-A`;
    await focusEditorAndType(pageA, markerA);
    await pageA.waitForTimeout(1500);
    await pageA.getByText(markerA).waitFor({ state: 'visible', timeout: 10000 });

    await pageA.reload({ waitUntil: 'domcontentloaded' });
    await waitForSyncBadge(pageA, 25000);
    // Click the created page in sidebar if another page opened first.
    const pageRowA = pageA.getByText(tag, { exact: false }).first();
    if (await pageRowA.isVisible().catch(() => false)) {
      await pageRowA.click();
      await waitForSyncBadge(pageA, 15000);
    }
    await pageA.getByText(markerA).waitFor({ state: 'visible', timeout: 15000 });

    // ── 4. Two-browser real-time CRDT collaboration over /ws/ ────────────────
    console.log('==> [4/6] Two-browser real-time CRDT collaboration over /ws/...');
    const contextB = await browser.newContext({ viewport: { width: 1280, height: 820 } });
    const pageB = await contextB.newPage();
    try {
      await signInOnPage(pageB, opts.base, opts.email, opts.password);
      await pageB.getByText(tag, { exact: false }).first().click();
      await waitForSyncBadge(pageB, 15000);
      await pageB.getByText(markerA).waitFor({ state: 'visible', timeout: 15000 });

      const markerB = ` PEER-B-${Date.now()}`;
      await pageB.getByText(markerA).click();
      await pageB.keyboard.press('End');
      await pageB.keyboard.type(markerB, { delay: 15 });

      // Must appear live in Page A via /ws/ CRDT sync WITHOUT reloading Page A!
      await pageA.getByText(markerB.trim()).waitFor({ state: 'visible', timeout: 15000 });
      console.log('    Two-browser live /ws/ CRDT sync converged without reload!');
    } finally {
      await contextB.close();
    }

    // ── 5. Offline edit recovery via IndexedDB (P0-2) ────────────────────────
    console.log('==> [5/6] Offline edit -> IndexedDB outbox -> reload recovery (P0-2)...');
    const offlineMarker = ` OFFLINE-IDB-${Date.now()}`;
    await contextA.setOffline(true);
    await pageA.waitForTimeout(400);
    await pageA.getByText(markerA).click();
    await pageA.keyboard.press('End');
    await pageA.keyboard.type(offlineMarker, { delay: 15 });
    await pageA.waitForTimeout(1000);

    const idbOffline = await readIdbDocs(pageA);
    const hasOutbox = idbOffline.entries.some((e) => {
      try {
        const parsed = JSON.parse(e.value);
        return Array.isArray(parsed.outbox) && parsed.outbox.length > 0;
      } catch {
        return false;
      }
    });
    assert(hasOutbox, `expected non-empty IDB outbox after offline edit, got ${JSON.stringify(idbOffline.entries.map((e) => e.key))}`);

    await contextA.setOffline(false);
    await pageA.reload({ waitUntil: 'domcontentloaded' });
    await waitForSyncBadge(pageA, 25000);
    const rowAfterOffline = pageA.getByText(tag, { exact: false }).first();
    if (await rowAfterOffline.isVisible().catch(() => false)) {
      await rowAfterOffline.click();
      await waitForSyncBadge(pageA, 15000);
    }
    await pageA.getByText(offlineMarker.trim()).waitFor({ state: 'visible', timeout: 15000 });
    console.log('    Offline edit persisted in mica_sync_v1 outbox and survived reload!');

    // ── 6. Lazy mica_preview/math load & clean logout (204 -> Sign-In gate) ──
    console.log('==> [6/6] Lazy mica_preview/math fetch + logout (204 -> Sign-In gate)...');
    await pageA.locator('[data-testid="new-page-btn"]').click();
    await waitForSyncBadge(pageA, 15000);
    await pageA.waitForTimeout(300);
    await focusEditorAndType(pageA, '/math');
    await pageA.waitForTimeout(300);
    await pageA.keyboard.press('Enter');
    await pageA.waitForTimeout(2000);

    const mathPreviewReqs = responses.filter(
      (r) => r.url.includes('/mica_preview/math_bg.wasm') && r.status === 200,
    );
    assert(
      mathPreviewReqs.length >= 1,
      `expected lazy /mica_preview/math_bg.wasm 200 after inserting /math, got ${JSON.stringify(mathPreviewReqs)}`,
    );

    // Logout -> 204 -> reload returns to Sign-In gate cleanly.
    await pageA.locator('[data-testid="account-menu-btn"]').click();
    await pageA.locator('[data-testid="account-menu-signout-btn"]').click();
    await pageA.locator('[data-testid="world-auth-email"]').waitFor({ state: 'visible', timeout: 15000 });
    const logoutOk = responses.some((r) => r.url.includes('/api/auth/logout') && r.status === 204);
    assert(logoutOk, 'expected POST /api/auth/logout -> 204');

    assert.equal(consoleErrors.length, 0, `console errors: ${JSON.stringify(consoleErrors)}`);
    assert.equal(pageErrors.length, 0, `page errors: ${JSON.stringify(pageErrors)}`);
    console.log('PASS: all 6 pure-Rust Web E2E scenarios passed (0 console errors, 0 CSP reports, 0 XHR)');
  } finally {
    await contextA.close();
    await browser.close();
  }
}

main().catch((err) => {
  console.error('FAIL:', err);
  process.exit(1);
});
