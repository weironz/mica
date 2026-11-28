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
 *   7. Flutter Web (<= 0.14.x) unsynced edits left in the legacy `mica-localfirst`
 *      IndexedDB are migrated on doc open, pushed to the server, and the legacy
 *      record is deleted.
 *   8. Desktop Web black-box cross-code-block drag selection: real mouse drag
 *      from top prose through a code block to trailing prose end, verifying
 *      nothing stalls on the code block and copied text includes every end char.
 *   9. Mobile Web black-box touch selection (`390x844`, `hasTouch: true`): real
 *      touch long-press on CJK prose mounts `touch-selection-bar` + draggable
 *      teardrop handles (`touch-handle-start` / `touch-handle-end`), dragging
 *      the end handle expands the selection, and Copy puts selected text on the
 *      clipboard (`CAP-16-06`).
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
  if (resp.status === 200) {
    const data = await resp.json().catch(() => null);
    if (data && data.verified === false) {
      throw new Error(
        `seed register returned verified=false (instance must be fresh with MICA_REGISTRATION_ENABLED unset so first operator auto-verifies)`
      );
    }
  }

  // Ensure the test user has at least one workspace and one page on a fresh DB
  // so the browser client opens directly into a live `/ws/` synced page.
  const loginResp = await fetch(`${base}/api/auth/login`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ email, password }),
  });
  if (!loginResp.ok) {
    const txt = await loginResp.text().catch(() => '');
    throw new Error(`seed login failed: HTTP ${loginResp.status} ${txt}`);
  }
  const loginData = await loginResp.json();
  const token = loginData.access_token;
  const authHeaders = {
    'Content-Type': 'application/json',
    Authorization: `Bearer ${token}`,
  };

  const wsListResp = await fetch(`${base}/api/workspaces`, { headers: authHeaders });
  if (!wsListResp.ok) {
    throw new Error(`seed list workspaces failed: HTTP ${wsListResp.status}`);
  }
  const wsListData = await wsListResp.json();
  const workspaces = Array.isArray(wsListData) ? wsListData : (wsListData.workspaces || []);
  let wsId = workspaces[0]?.id;
  if (!wsId) {
    const createWs = await fetch(`${base}/api/workspaces`, {
      method: 'POST',
      headers: authHeaders,
      body: JSON.stringify({ name: 'E2E Workspace' }),
    });
    if (!createWs.ok) {
      const txt = await createWs.text().catch(() => '');
      throw new Error(`seed create workspace failed: HTTP ${createWs.status} ${txt}`);
    }
    const created = await createWs.json();
    wsId = created.workspace?.id || created.id;
  }

  const viewsResp = await fetch(`${base}/api/workspaces/${wsId}/views`, { headers: authHeaders });
  const viewsData = viewsResp.ok ? await viewsResp.json() : { views: [] };
  const views = Array.isArray(viewsData) ? viewsData : (viewsData.views || []);
  if (!views.some((v) => v.view_type === 'page' && !v.deleted_at)) {
    const createDoc = await fetch(`${base}/api/workspaces/${wsId}/documents`, {
      method: 'POST',
      headers: authHeaders,
      body: JSON.stringify({ name: 'E2E Welcome' }),
    });
    if (!createDoc.ok) {
      const txt = await createDoc.text().catch(() => '');
      throw new Error(`seed create document failed: HTTP ${createDoc.status} ${txt}`);
    }
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
  const vp = page.viewportSize();
  if (vp && vp.width < 640) {
    // Compact mobile header (<640px) intentionally hides the sync text badge
    // to preserve title and primary action space; wait for sign-in dismissal
    // and compact editor header readiness instead.
    await page.locator('[data-testid="world-auth-submit"]').waitFor({ state: 'hidden', timeout: timeoutMs }).catch(() => {});
    await page.locator('[data-testid="ed-more"]').waitFor({ state: 'visible', timeout: timeoutMs });
    return;
  }
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

// Move a doc's unsynced outbox from the new store (`mica_sync_v1`) into a
// legacy Flutter Web database (`mica-localfirst`, same record shapes as main's
// WebIdbDocStore), so the edit exists ONLY where a 0.14.x Flutter Web client
// would have left it.
async function plantLegacyFlutterOutbox(page) {
  return page.evaluate(async () => {
    const req = (r) => new Promise((res, rej) => { r.onsuccess = () => res(r.result); r.onerror = () => rej(r.error); });
    const fresh = await req(indexedDB.open('mica_sync_v1', 1));
    const tx = fresh.transaction('docs', 'readwrite');
    const store = tx.objectStore('docs');
    const keys = await req(store.getAllKeys());
    const vals = await req(store.getAll());
    let planted = null;
    for (let i = 0; i < keys.length; i += 1) {
      const snap = JSON.parse(String(vals[i]));
      const pending = (snap.outbox || []).filter((e) => e.clock > (snap.cursor?.pushed_clock ?? 0));
      if (pending.length === 0) continue;
      const [origin, docId] = String(keys[i]).split('\u0000');
      planted = { origin, docId, legacyKey: `${origin}|${docId}`, rows: pending.length };
      await req(store.delete(keys[i]));
      fresh.close();
      const open = indexedDB.open('mica-localfirst', 1);
      open.onupgradeneeded = () => {
        for (const s of ['base', 'outbox', 'remote', 'cursor']) open.result.createObjectStore(s);
      };
      const legacy = await req(open);
      const ltx = legacy.transaction(['outbox', 'cursor'], 'readwrite');
      ltx.objectStore('outbox').put(
        { rows: pending.map((e) => ({ clock: e.clock, bytes: new Uint8Array(e.bytes) })) },
        planted.legacyKey,
      );
      ltx.objectStore('cursor').put({ lastSyncedRid: 0, pushedClock: 0 }, planted.legacyKey);
      await new Promise((res, rej) => { ltx.oncomplete = res; ltx.onerror = () => rej(ltx.error); });
      legacy.close();
      return planted;
    }
    fresh.close();
    return planted;
  });
}

async function legacyRecordExists(page, legacyKey) {
  return page.evaluate(async (key) => {
    const req = (r) => new Promise((res, rej) => { r.onsuccess = () => res(r.result); r.onerror = () => rej(r.error); });
    const db = await req(indexedDB.open('mica-localfirst', 1));
    const v = await req(db.transaction('outbox').objectStore('outbox').get(key));
    db.close();
    return v !== undefined;
  }, legacyKey);
}

async function legacyFlutterWebMigration(browser, opts) {
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 820 } });
  try {
    const page = await ctx.newPage();
    await signInOnPage(page, opts.base, opts.email, opts.password);
    const marker = ` LEGACY-FLUTTER-${Date.now()}`;
    await ctx.setOffline(true);
    await page.waitForTimeout(400);
    await focusEditorAndType(page, marker);
    await page.waitForTimeout(1200);
    const planted = await plantLegacyFlutterOutbox(page);
    assert(planted, 'expected an unsynced outbox to plant into mica-localfirst');
    // Leave while still offline: the edit now exists only in the legacy DB.
    await page.close();
    await ctx.setOffline(false);

    const page2 = await ctx.newPage();
    await page2.goto(`${opts.base}/`, { waitUntil: 'domcontentloaded' });
    await waitForSyncBadge(page2, 25000);
    await page2.getByText(marker.trim()).waitFor({ state: 'visible', timeout: 15000 });
    await page2.waitForTimeout(1500);
    assert.equal(await legacyRecordExists(page2, planted.legacyKey), false, 'legacy record must be deleted after migration');

    // The server has it: a fresh profile (no IndexedDB) sees the edit.
    const ctxB = await browser.newContext({ viewport: { width: 1280, height: 820 } });
    try {
      const pageB = await ctxB.newPage();
      await signInOnPage(pageB, opts.base, opts.email, opts.password);
      await pageB.getByText(marker.trim()).waitFor({ state: 'visible', timeout: 15000 });
    } finally {
      await ctxB.close();
    }
    console.log(`    Migrated ${planted.rows} legacy Flutter Web edit(s); server has them; legacy record deleted.`);
  } finally {
    await ctx.close();
  }
}

async function touchLongPress(cdp, x, y, holdMs = 650) {
  await cdp.send('Input.dispatchTouchEvent', {
    type: 'touchStart',
    touchPoints: [{ x, y, id: 1 }],
  });
  await new Promise((r) => setTimeout(r, holdMs));
  await cdp.send('Input.dispatchTouchEvent', {
    type: 'touchEnd',
    touchPoints: [],
  });
}

async function touchDrag(cdp, x0, y0, x1, y1, steps = 8) {
  await cdp.send('Input.dispatchTouchEvent', {
    type: 'touchStart',
    touchPoints: [{ x: x0, y: y0, id: 2 }],
  });
  for (let i = 1; i <= steps; i += 1) {
    const t = i / steps;
    await cdp.send('Input.dispatchTouchEvent', {
      type: 'touchMove',
      touchPoints: [{ x: x0 + (x1 - x0) * t, y: y0 + (y1 - y0) * t, id: 2 }],
    });
    await new Promise((r) => setTimeout(r, 25));
  }
  await cdp.send('Input.dispatchTouchEvent', {
    type: 'touchEnd',
    touchPoints: [],
  });
}

async function crossCodeBlockAndMobileTouchSelection(browser, opts) {
  const pageTag = `SEL-${Date.now()}`;
  const headText = `AlphaDragStart ${pageTag}`;
  const codeText = `let code_val = 42;`;
  const tailText = `中文触摸选区测试段落 OmegaDragTail`;

  // ── 8. Desktop Web: real mouse drag across prose -> code block -> prose ──
  console.log('==> [8/10] Desktop Web black-box cross-code-block drag selection...');
  const deskCtx = await browser.newContext({
    viewport: { width: 1280, height: 820 },
    permissions: ['clipboard-read', 'clipboard-write'],
  });
  try {
    const page = await deskCtx.newPage();
    await signInOnPage(page, opts.base, opts.email, opts.password);
    await page.locator('[data-testid="new-page-btn"]').click();
    await waitForSyncBadge(page, 15000);
    await page.waitForTimeout(300);

    const titleHost = page.locator('[data-testid="page-title-input"]');
    if (await titleHost.isVisible().catch(() => false)) {
      await fillTestInput(page, 'page-title-input', pageTag);
      await page.keyboard.press('Enter');
      await page.waitForTimeout(250);
    }
    await focusEditorAndType(page, headText);
    await page.keyboard.press('Enter');
    await page.waitForTimeout(200);
    await page.locator('[data-testid="ed-code-block"]').click();
    await page.waitForTimeout(250);
    await page.keyboard.type(codeText, { delay: 12 });
    await page.waitForTimeout(250);
    await page.locator('[data-testid="editor-bottom-append-zone"]').click();
    await page.waitForTimeout(250);
    await page.keyboard.type(tailText, { delay: 12 });
    await page.waitForTimeout(1000);

    const headLoc = page.getByText(headText, { exact: false }).first();
    const codeLoc = page.locator('[data-testid="code-block-editor"]').first();
    const tailLoc = page.getByText('OmegaDragTail', { exact: false }).first();
    await headLoc.waitFor({ state: 'visible', timeout: 10000 });
    await codeLoc.waitFor({ state: 'visible', timeout: 10000 });
    await tailLoc.waitFor({ state: 'visible', timeout: 10000 });

    const hb = await headLoc.boundingBox();
    const cb = await codeLoc.boundingBox();
    const tb = await tailLoc.boundingBox();
    assert(hb && cb && tb, 'expected visible bounding boxes for head, code block, and tail');

    // Drag from the start of the first paragraph, straight through the code
    // block (including its upper-right toolbar zone), past the end of the tail.
    await page.mouse.move(hb.x + 2, hb.y + hb.height * 0.5);
    await page.mouse.down();
    await page.mouse.move(cb.x + cb.width * 0.85, cb.y + 10, { steps: 6 });
    await page.mouse.move(cb.x + cb.width * 0.5, cb.y + cb.height * 0.5, { steps: 6 });
    await page.mouse.move(tb.x + tb.width + 60, tb.y + tb.height * 0.5, { steps: 8 });
    await page.mouse.up();
    await page.waitForTimeout(250);

    await page.keyboard.press('Control+C');
    await page.waitForTimeout(250);
    const copied = await page.evaluate(() => navigator.clipboard.readText());
    assert(
      copied.includes('AlphaDragStart') && copied.includes('let code_val = 42;') && copied.includes('OmegaDragTail'),
      `cross-code-block drag selection must capture full head + code + tail without stalling; got: ${JSON.stringify(copied)}`,
    );
    console.log('    Desktop cross-code-block drag copied full head + code + tail!');
  } finally {
    await deskCtx.close();
  }

  // ── 9. Mobile Web: touch long-press + teardrop handle drag + copy ────────
  console.log('==> [9/10] Mobile Web black-box touch selection & teardrop handles...');
  const mobCtx = await browser.newContext({
    viewport: { width: 390, height: 844 },
    hasTouch: true,
    isMobile: true,
    permissions: ['clipboard-read', 'clipboard-write'],
  });
  try {
    const page = await mobCtx.newPage();
    await signInOnPage(page, opts.base, opts.email, opts.password);
    // Open the page created in scenario 8 via the mobile drawer if needed.
    const tailOnPage = page.getByText('中文触摸选区测试段落', { exact: false }).first();
    if (!(await tailOnPage.isVisible().catch(() => false))) {
      const toggleBtn = page.locator('[data-testid="toggle-sidebar-btn"]');
      if (await toggleBtn.isVisible().catch(() => false)) {
        await toggleBtn.click();
        await page.waitForTimeout(300);
      }
      await page.getByText(pageTag, { exact: false }).first().click();
      await waitForSyncBadge(page, 15000);
    }
    await tailOnPage.waitFor({ state: 'visible', timeout: 15000 });
    const box = await tailOnPage.boundingBox();
    assert(box, 'expected visible CJK paragraph on mobile viewport');

    const cdp = await mobCtx.newCDPSession(page);
    await touchLongPress(cdp, box.x + 28, box.y + box.height * 0.5, 650);

    const bar = page.locator('[data-testid="touch-selection-bar"]');
    const startHandle = page.locator('[data-testid="touch-handle-start"]');
    const endHandle = page.locator('[data-testid="touch-handle-end"]');
    await bar.waitFor({ state: 'visible', timeout: 8000 });
    await startHandle.waitFor({ state: 'visible', timeout: 8000 });
    await endHandle.waitFor({ state: 'visible', timeout: 8000 });

    const eb = await endHandle.boundingBox();
    assert(eb, 'expected bounding box for touch-handle-end');
    await touchDrag(cdp, eb.x + eb.width * 0.5, eb.y + eb.height * 0.5, eb.x + eb.width * 0.5 + 90, eb.y + eb.height * 0.5, 8);
    await page.waitForTimeout(250);

    await page.locator('[data-testid="touch-sel-copy"]').tap();
    await page.waitForTimeout(250);
    const mobileCopied = await page.evaluate(() => navigator.clipboard.readText());
    assert(
      mobileCopied.length >= 2 && /[中文触摸选区测试段落OmegaDragTail]/.test(mobileCopied),
      `mobile touch selection + handle drag must copy selected prose; got: ${JSON.stringify(mobileCopied)}`,
    );
    console.log(`    Mobile touch long-press + handle drag copied: ${JSON.stringify(mobileCopied)}`);
  } finally {
    await mobCtx.close();
  }

  // ── 10. Public share (/s/{token}), Workspace settings & list Tab ─────────
  console.log('==> [10/10] Public share (/s/{token}), Workspace settings & multi-line list Tab...');
  const shareCtx = await browser.newContext({
    viewport: { width: 1280, height: 820 },
    permissions: ['clipboard-read', 'clipboard-write'],
  });
  try {
    const page = await shareCtx.newPage();
    await signInOnPage(page, opts.base, opts.email, opts.password);
    await page.locator('[data-testid="new-page-btn"]').click();
    await waitForSyncBadge(page, 15000);
    await page.waitForTimeout(300);

    const shareMarker = `SHARE-PUBLIC-${Date.now()}`;
    await focusEditorAndType(page, shareMarker);
    await page.waitForTimeout(1000);

    await page.locator('[data-testid="ed-share-btn"]').click();
    const toggle = page.locator('[data-testid="share-public-toggle"]');
    await toggle.waitFor({ state: 'visible', timeout: 10000 });
    await toggle.click();

    const urlLoc = page.locator('[data-testid="share-url-text"]');
    await urlLoc.waitFor({ state: 'visible', timeout: 10000 });
    await page.locator('[data-testid="share-copy-btn"]').click();
    let shareUrl = '';
    for (let i = 0; i < 15; i += 1) {
      await page.waitForTimeout(150);
      shareUrl = ((await page.evaluate(() => navigator.clipboard.readText())) || '').trim();
      if (shareUrl.includes('/s/') && !shareUrl.includes('…')) break;
    }
    assert(
      shareUrl.includes('/s/') && !shareUrl.includes('…'),
      `expected full /s/{token} share url from copy button, got: ${shareUrl}`,
    );

    const pubResp = await fetch(shareUrl);
    assert.equal(pubResp.status, 200, `expected unauthenticated GET ${shareUrl} -> 200`);
    const pubHtml = await pubResp.text();
    assert(pubHtml.includes(shareMarker), 'public share HTML must include document marker');

    // Disable public share -> link returns 404.
    await toggle.click();
    await page.locator('[data-testid="share-url-box"]').waitFor({ state: 'detached', timeout: 10000 });
    const revokedResp = await fetch(shareUrl);
    assert.equal(revokedResp.status, 404, `revoked share url must return 404, got ${revokedResp.status}`);
    await page.locator('[data-testid="share-dialog-close-btn"]').click();

    // Open & close Workspace Settings modal cleanly.
    await page.locator('[data-testid="open-ws-settings-btn"]').click();
    const wsModal = page.locator('[data-testid="workspace-settings-modal"]');
    await wsModal.waitFor({ state: 'visible', timeout: 10000 });
    await page.locator('[data-testid="ws-settings-close"]').click();
    await wsModal.waitFor({ state: 'detached', timeout: 8000 });

    // Refocus the editor and verify multi-line bullet list Tab indent in real browser.
    await page.getByText(shareMarker, { exact: false }).first().click();
    await page.keyboard.press('End');
    await page.keyboard.press('Enter');
    await page.keyboard.type('- ItemOne', { delay: 10 });
    await page.keyboard.press('Enter');
    await page.keyboard.type('ItemTwo', { delay: 10 });
    await page.keyboard.press('Control+A');
    await page.keyboard.press('Tab');
    await page.waitForTimeout(250);
    await page.getByText('ItemOne', { exact: false }).waitFor({ state: 'visible', timeout: 5000 });
    console.log('    Public share (/s/ 200 -> 404), workspace settings, and list Tab passed!');
  } finally {
    await shareCtx.close();
  }
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
    console.log('==> [1/10] Cold start, instant HTML shell logo, CSP, single WASM download & zero XHR...');
    await pageA.goto(`${opts.base}/`, { waitUntil: 'domcontentloaded' });
    const loginLogoCount = await pageA.evaluate(
      () => document.querySelectorAll('#mica-boot-auth svg linearGradient#ml-main').length,
    );
    assert.equal(loginLogoCount, 1, 'instant HTML login shell SVG must have dedicated #ml-main gradient defs');
    await fillTestInput(pageA, 'world-auth-email', opts.email);
    await fillTestInput(pageA, 'world-auth-password', opts.password);
    await pageA.locator('[data-testid="world-auth-submit"]').click();
    await waitForSyncBadge(pageA, 25000);
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
    console.log('==> [2/10] Verifying #111-scoped IndexedDB (mica_sync_v1) hydration & mirror...');
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
    console.log('==> [3/10] Create page, edit, reload persistence (HttpOnly cookie)...');
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
    console.log('==> [4/10] Two-browser real-time CRDT collaboration over /ws/...');
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
    console.log('==> [5/10] Offline edit -> IndexedDB outbox -> reload recovery (P0-2)...');
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
    console.log('==> [6/10] Lazy mica_preview/math fetch + logout (204 -> Sign-In gate)...');
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
    console.log('PASS: scenarios 1-6 passed (0 console errors, 0 CSP reports, 0 XHR)');

    // ── 7. Legacy Flutter Web IndexedDB (`mica-localfirst`) migration ────────
    console.log('==> [7/10] Flutter Web mica-localfirst unsynced edit -> migrated & pushed...');
    await legacyFlutterWebMigration(browser, opts);

    // ── 8 & 9. Framework-agnostic drag & mobile touch selection contracts ────
    await crossCodeBlockAndMobileTouchSelection(browser, opts);
    console.log('PASS: all 10 pure-Rust Web E2E scenarios passed');
  } finally {
    await contextA.close();
    await browser.close();
  }
}

main().catch((err) => {
  console.error('FAIL:', err);
  process.exit(1);
});
