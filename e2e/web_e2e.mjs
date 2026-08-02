// Web end-to-end: the assertions only a real browser against a real stack can make.
//
// Why this exists: the 14 integration tests in CI run on Windows against native
// FFI. Every one imports `frb_generated.dart` or `LocalOffline`, neither of which
// exists on web — so none can be pointed at Chrome, and the web half of the
// product had zero end-to-end coverage. That is not a thoroughness gap: the web
// build is a categorically different binary running a DIFFERENT CRDT
// implementation (yjs in JS, not Rust yrs), a different storage backend
// (IndexedDB, not SQLite) and a different renderer path.
//
// It deliberately does NOT drive the UI. Flutter web paints to a canvas and this
// app wires almost no `Semantics`, so there is essentially no DOM to select —
// clicking through it would be building on sand. Instead it drives the
// `window.micaYjs*` hooks the app already registers (`lib/web/yjs_probe_web.dart`,
// registered unconditionally so they exist in the release bundle), plus the HTTP
// layer, which is where the bugs that have actually shipped twice live.
//
//   node web_e2e.mjs --base http://127.0.0.1:8090 \
//                    --email e2e@mica.test --password e2epassword123
//
// Exits non-zero on any failed assertion, and prints what it saw.

import { chromium } from '@playwright/test';

const arg = (name, fallback) => {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
};

const BASE = arg('base', 'http://127.0.0.1:8090');
const EMAIL = arg('email', 'e2e@mica.test');
const PASSWORD = arg('password', 'e2epassword123');

const failures = [];
const check = (name, ok, detail) => {
  console.log(`${ok ? 'ok  ' : 'FAIL'}  ${name}${detail ? `  — ${detail}` : ''}`);
  if (!ok) failures.push(name);
};

// A stack that is not up must FAIL, never skip. `cloud_sync_test.dart` skips-as-pass
// when it cannot reach a server, and justfile:128 records the four days of false
// green that cost. Same trap, same answer: assert the stack before anything else.
const health = await fetch(`${BASE}/api/health`).catch(() => null);
if (!health || !health.ok) {
  console.error(`the stack is not up at ${BASE} — refusing to report a pass`);
  process.exit(1);
}

// Playwright's bundled Chromium, or the system browser if it was never downloaded.
//
// Not a convenience: from CN, `playwright install chromium` cannot reach
// playwright.azureedge.net and the npmmirror mirror fails on the headless shell
// too — the same class of problem CLAUDE.md records for Flutter and pub. Without
// this fallback the harness is unrunnable on the machine it was written on, which
// is how a test ends up trusted-but-never-run. CI (ubuntu-latest, outside CN) gets
// the bundled build and never reaches the fallback.
async function launch() {
  try {
    return await chromium.launch();
  } catch (bundled) {
    for (const channel of ['chrome', 'msedge']) {
      try {
        const browser = await chromium.launch({ channel });
        console.log(`note: bundled Chromium unavailable, using system ${channel}`);
        return browser;
      } catch {
        /* try the next one */
      }
    }
    throw bundled;
  }
}

const browser = await launch();
// Pin the locale and timezone. An e2e should not inherit whatever the machine
// happens to report — but this one is also load-bearing: headless Chromium on the
// CI runner reports locale information that makes the Flutter engine throw
// `ArgumentError: Incorrect locale information provided` before `main()` gets far
// enough to register anything, so the app never starts at all. Pinning makes the
// harness deterministic; the app's own fragility there is a separate finding, filed
// in docs/roadmap.md rather than papered over here.
const context = await browser.newContext({
  locale: 'en-US',
  timezoneId: 'UTC',
});
const page = await context.newPage();
const ownHost = new URL(BASE).host;

const consoleErrors = [];
page.on('console', (m) => m.type() === 'error' && consoleErrors.push(m.text()));
const badResponses = [];
const requestedHosts = new Set();
page.on('response', (r) => {
  let url;
  try {
    url = new URL(r.url());
  } catch {
    return;
  }
  requestedHosts.add(url.host);
  // Only our own origin counts: the app's UI probes its build-time default API
  // base on load, and in this harness that is a different port whose failure is
  // the harness's, not the app's.
  if (r.status() >= 400 && url.host === ownHost) {
    badResponses.push(`${r.status()} ${r.url()}`);
  }
});

// Uncaught exceptions are NOT console messages; without this a Dart error during
// startup is invisible and the only symptom is the probe never appearing.
const pageErrors = [];
page.on('pageerror', (e) => pageErrors.push(String(e)));

await page.goto(BASE, { waitUntil: 'load' });
try {
  await page.waitForFunction(() => typeof window.micaYjsWebSyncTest === 'function', null, {
    timeout: 60_000,
  });
} catch (timeout) {
  // A harness that times out without saying what it saw makes the next attempt a
  // guess. Everything known about the page goes to the log before giving up.
  console.error('the app never registered its probes — what the page reported:');
  console.error(`  url        : ${page.url()}`);
  console.error(`  title      : ${await page.title().catch(() => '(unavailable)')}`);
  const present = await page
    .evaluate(() => ({
      selfTest: typeof window.micaYjsSelfTest,
      w2: typeof window.micaYjsW2Test,
      webSync: typeof window.micaYjsWebSyncTest,
      flutterReady: typeof window._flutter,
      bodyChildren: document.body ? document.body.children.length : -1,
    }))
    .catch((e) => ({ evaluateFailed: String(e) }));
  console.error(`  window     : ${JSON.stringify(present)}`);
  console.error(`  pageerrors : ${pageErrors.length ? pageErrors.join(' | ') : '(none)'}`);
  console.error(`  console    : ${consoleErrors.length ? consoleErrors.join(' | ') : '(none)'}`);
  console.error(`  bad reqs   : ${badResponses.length ? badResponses.join(' | ') : '(none)'}`);
  console.error(`  hosts      : ${[...requestedHosts].join(', ')}`);
  await browser.close();
  throw timeout;
}

// Snapshot the load-time observations BEFORE anything below deliberately asks for
// a 400 or a 404. The first version of this asserted console cleanliness at the
// end and failed on the very errors its own routing probes had just requested —
// a test complaining about the thing it did on purpose.
const loadConsoleErrors = [...consoleErrors];
const loadBadResponses = [...badResponses];

// ── 1. The bundle is self-contained ───────────────────────────────────────────
// `--no-web-resources-cdn` is asserted nowhere at runtime. If it regressed the app
// would silently fetch CanvasKit from www.gstatic.com — unreachable from CN, and
// dependent on gstatic continuing to serve COOP/COEP. Only a real browser's
// network log can prove the load stayed same-origin.
const cdn = [...requestedHosts].filter((h) => h.includes('gstatic') || h.includes('googleapis'));
check('no CDN fetch for engine resources', cdn.length === 0, cdn.join(', ') || 'same-origin only');

// ── 2. Release-mode load is clean ─────────────────────────────────────────────
// Both halves of "it loaded without complaining": no failed request from our own
// origin, and nothing on the console. Release is the only mode the web build has,
// so an assert that debug would have caught never runs here.
check(
  'no 4xx/5xx from our own origin on load',
  loadBadResponses.length === 0,
  loadBadResponses.join('; '),
);
const ownLoadErrors = loadConsoleErrors.filter((t) => t.includes(ownHost) || !t.includes('http'));
check(
  'no console errors on load',
  ownLoadErrors.length === 0,
  ownLoadErrors.slice(0, 3).join(' | '),
);

// ── 3. yjs (browser) ↔ yrs (server) actually converge ─────────────────────────
// The heart of it: two live sessions over a real WebSocket, folded by the real
// server into real Postgres. `mica-core`'s web_interop test shells out to node
// headlessly — it has never run in a browser, or against the live server.
const sync = JSON.parse(
  await page.evaluate(([b, e, p]) => window.micaYjsWebSyncTest(b, e, p), [BASE, EMAIL, PASSWORD]),
);
check('two web sessions converge through the real server', sync.ok === true, JSON.stringify(sync));
check(
  'the edit made in session A arrived in session B',
  Array.isArray(sync.bBlocks) && sync.bBlocks.includes('wp1'),
  JSON.stringify(sync.bBlocks),
);

// ── 4. Server-rendered routes beat the SPA fallback ───────────────────────────
// Pure HTTP, invisible to every Dart test, and a bug class that has shipped twice.
// A route that falls through to index.html looks like "the app loaded" — the user
// never learns their confirmation link or share link did nothing. (This assertion
// found `/s/` missing from the dev nginx config the first time it ran.)
const serverRendered = ['/verify-email?token=bogus', '/reset-password?token=bogus', '/s/not-a-real-token'];
const routed = await page.evaluate(async (paths) => {
  const out = {};
  for (const p of paths) {
    const r = await fetch(p);
    const body = await r.text();
    out[p] = { status: r.status, isSpa: body.includes('flutter_bootstrap.js') };
  }
  return out;
}, [...serverRendered, '/deep/app/route']);

for (const p of serverRendered) {
  check(`${p} is server-rendered, not the SPA`, routed[p].isSpa === false, JSON.stringify(routed[p]));
}
check(
  'an unknown app route still falls back to the SPA',
  routed['/deep/app/route'].isSpa === true,
  JSON.stringify(routed['/deep/app/route']),
);

// ── 5. The entry files are not cacheable ──────────────────────────────────────
// v0.5.2 "looked not deployed" because a fixed-name entry file came from cache.
// These four names never change, so they must never be reused; everything else is
// content-hashed and may be.
const cache = await page.evaluate(async (files) => {
  const out = {};
  for (const f of files) out[f] = (await fetch(f)).headers.get('cache-control');
  return out;
}, ['/', '/main.dart.js', '/flutter_bootstrap.js', '/flutter_service_worker.js']);
for (const [f, value] of Object.entries(cache)) {
  // The dev nginx says `no-store`, production says `no-cache`. Either is a refusal
  // to reuse a stale copy; what must never appear is a max-age letting one linger.
  check(`${f} refuses stale caching`, /no-store|no-cache/.test(value ?? ''), value ?? '(none)');
}

// ── 7. A POSIX locale does not stop the app from starting ─────────────────────
// The engine throws `RangeError: Incorrect locale information provided` from
// `new Locale` when the browser reports "C" — inside its own startup, before
// `main()`, so `runZonedGuarded` never sees it and the console stays clean. The
// only symptom is a blank page. `web/index.html` sanitises the value before
// flutter_bootstrap.js runs; this assertion is what keeps that guard honest.
//
// Its own context, because a context's locale is fixed at creation. Runs last so a
// failure here cannot be confused with the assertions above.
{
  const odd = await browser.newContext({ locale: 'C', timezoneId: 'UTC' });
  const oddPage = await odd.newPage();
  const oddErrors = [];
  oddPage.on('pageerror', (e) => oddErrors.push(String(e)));
  await oddPage.goto(BASE, { waitUntil: 'load' });
  let booted = true;
  try {
    await oddPage.waitForFunction(() => typeof window.micaYjsSelfTest === 'function', null, {
      timeout: 30_000,
    });
  } catch {
    booted = false;
  }
  check(
    'the app still starts when the browser reports a POSIX locale ("C")',
    booted,
    oddErrors[0] ?? (booted ? 'booted' : 'no pageerror captured'),
  );
  await odd.close();
}

await browser.close();

if (failures.length) {
  console.error(`\n${failures.length} failed: ${failures.join(', ')}`);
  process.exit(1);
}
console.log('\nweb e2e: all assertions passed');
