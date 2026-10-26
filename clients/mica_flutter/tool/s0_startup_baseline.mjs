// S0 startup baseline: web first-frame time, measured from OUTSIDE the app.
//
// `main()` -> `runApp` cannot be instrumented in S0 (that would be a product-code
// change), so this probe measures the closest external equivalent: navigation
// start -> the `flutter-first-frame` event Flutter's web engine dispatches on
// `window`. Five cold runs (fresh browser profile each), median.
//
//   cd clients/mica_flutter
//   flutter build web --release --no-web-resources-cdn
//   CHROME_EXECUTABLE="C:/.../chrome.exe" node tool/s0_startup_baseline.mjs --dir=build/web
//
// Set CHROME_EXECUTABLE to a Chrome that is NOT already running: a second
// `chrome.exe` launched against the same default profile is handed to the
// running instance and the debug port never appears. (The repo's Playwright
// Chromium under ms-playwright works and is what the S0 record used.)
//
// No Playwright dependency: it talks CDP over Node's built-in WebSocket.

import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { readFile, stat, rm, mkdtemp } from 'node:fs/promises';
import { createReadStream } from 'node:fs';
import { extname, join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';

const here = fileURLToPath(new URL('.', import.meta.url));
const arg = (name, fallback) => {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
};

const root = resolve(here, '..', arg('dir', 'build/web'));
const runs = Number(arg('runs', '5'));
const chrome =
  process.env.CHROME_EXECUTABLE ??
  'C:/Program Files/Google/Chrome/Application/chrome.exe';

const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.json': 'application/json',
  '.css': 'text/css',
  '.wasm': 'application/wasm',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.svg': 'image/svg+xml',
  '.ttf': 'font/ttf',
  '.otf': 'font/otf',
  '.woff': 'font/woff',
  '.woff2': 'font/woff2',
  '.ico': 'image/x-icon',
  '.bin': 'application/octet-stream',
  '.symbols': 'application/octet-stream',
};

try {
  await stat(join(root, 'index.html'));
} catch {
  console.error(`no web build at ${root} — run: flutter build web --release --no-web-resources-cdn`);
  process.exit(1);
}

const server = createServer(async (req, res) => {
  const urlPath = decodeURIComponent(new URL(req.url, 'http://x').pathname);
  let file = join(root, urlPath === '/' ? 'index.html' : urlPath);
  try {
    const info = await stat(file);
    if (info.isDirectory()) file = join(file, 'index.html');
    res.writeHead(200, { 'content-type': MIME[extname(file)] ?? 'application/octet-stream' });
    createReadStream(file).pipe(res);
  } catch {
    res.writeHead(404).end('not found');
  }
});
await new Promise((ok) => server.listen(0, '127.0.0.1', ok));
const base = `http://127.0.0.1:${server.address().port}/`;

async function openPort() {
  const s = createServer();
  await new Promise((ok) => s.listen(0, '127.0.0.1', ok));
  const { port } = s.address();
  await new Promise((ok) => s.close(ok));
  return port;
}

async function getJson(url, method = 'GET') {
  const response = await fetch(url, { method });
  if (!response.ok) throw new Error(`${method} ${url}: ${response.status}`);
  return response.json();
}

const sleep = (ms) => new Promise((ok) => setTimeout(ok, ms));

async function cdp(wsUrl, onEvent) {
  const ws = new WebSocket(wsUrl);
  const pending = new Map();
  let next = 1;
  await new Promise((ok, fail) => {
    ws.onopen = ok;
    ws.onerror = fail;
  });
  ws.onmessage = (event) => {
    const message = JSON.parse(event.data);
    if (message.id && pending.has(message.id)) {
      pending.get(message.id)(message);
      pending.delete(message.id);
    } else if (onEvent) {
      onEvent(message);
    }
  };
  const send = (method, params = {}) =>
    new Promise((resolve) => {
      const id = next++;
      pending.set(id, resolve);
      ws.send(JSON.stringify({ id, method, params }));
    });
  return { send, close: () => ws.close() };
}

const samples = [];
for (let run = 0; run < runs; run++) {
  const profile = await mkdtemp(join(tmpdir(), 'mica-startup-'));
  const port = await openPort();
  const browser = spawn(
    chrome,
    [
      '--headless',
      '--disable-gpu',
      '--no-sandbox',
      '--no-first-run',
      '--no-default-browser-check',
      `--user-data-dir=${profile}`,
      `--remote-debugging-port=${port}`,
      'about:blank',
    ],
    { stdio: 'ignore' },
  );
  try {
    let version = null;
    for (let i = 0; i < 100 && version === null; i++) {
      try {
        version = await getJson(`http://127.0.0.1:${port}/json/version`);
      } catch {
        await sleep(100);
      }
    }
    if (version === null) throw new Error('chrome debug port never came up');
    const target = await getJson(`http://127.0.0.1:${port}/json/new?about:blank`, 'PUT');
    const client = await cdp(target.webSocketDebuggerUrl);
    await client.send('Page.enable');
    await client.send('Page.addScriptToEvaluateOnNewDocument', {
      source: `window.__micaFirstFrame = null;
        window.addEventListener('flutter-first-frame', () => {
          window.__micaFirstFrame = performance.now();
        });`,
    });
    await client.send('Page.navigate', { url: base });
    let ms = null;
    for (let i = 0; i < 1200 && ms === null; i++) {
      const result = await client.send('Runtime.evaluate', {
        expression: 'window.__micaFirstFrame',
        returnByValue: true,
      });
      const value = result.result?.result?.value;
      if (typeof value === 'number') ms = value;
      else await sleep(100);
    }
    client.close();
    if (ms === null) throw new Error('no flutter-first-frame within 120s');
    samples.push(ms);
    console.log(JSON.stringify({ platform: 'chrome', item: 'startup.first-frame', run, ms: Math.round(ms * 10) / 10 }));
  } finally {
    browser.kill();
    await rm(profile, { recursive: true, force: true }).catch(() => {});
    await sleep(500);
  }
}

server.close();
samples.sort((a, b) => a - b);
const median = samples[Math.floor(samples.length / 2)];
console.log(JSON.stringify({ platform: 'chrome', item: 'startup.first-frame', median_ms: Math.round(median * 10) / 10 }));
