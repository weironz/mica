#!/usr/bin/env node
/**
 * Minimal zero-dependency local dev/perf origin server:
 *   - Serves static WebAssembly bundle from `deploy/web` (with proper MIME + Brotli support)
 *   - Reverse-proxies `/api/*`, `/ws/*`, `/metrics` (HTTP + WebSocket upgrade) to `mica-api-server`
 *
 * Used by `benchmarks/web_perf.mjs` so browser HttpOnly cookies and `/ws/` live on one origin.
 */
import http from 'node:http';
import net from 'node:net';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, '..');

function parseArgs(argv) {
  const out = {
    port: 18090,
    apiHost: '127.0.0.1',
    apiPort: 18088,
    webDir: path.join(repoRoot, 'deploy/web'),
  };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a === '--port') out.port = Number(argv[++i]);
    else if (a === '--api-port') out.apiPort = Number(argv[++i]);
    else if (a === '--web-dir') out.webDir = path.resolve(argv[++i]);
  }
  return out;
}

const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'application/javascript; charset=utf-8',
  '.mjs': 'application/javascript; charset=utf-8',
  '.wasm': 'application/wasm',
  '.css': 'text/css; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.ico': 'image/x-icon',
  '.json': 'application/json; charset=utf-8',
  '.woff2': 'font/woff2',
};

const opts = parseArgs(process.argv.slice(2));

const server = http.createServer((req, res) => {
  const urlPath = (req.url || '/').split('?')[0];
  if (urlPath.startsWith('/api/') || urlPath === '/metrics' || urlPath.startsWith('/ws/')) {
    const proxyReq = http.request(
      {
        hostname: opts.apiHost,
        port: opts.apiPort,
        path: req.url,
        method: req.method,
        headers: { ...req.headers, host: `${opts.apiHost}:${opts.apiPort}` },
      },
      (proxyRes) => {
        res.writeHead(proxyRes.statusCode || 502, proxyRes.headers);
        proxyRes.pipe(res);
      },
    );
    proxyReq.on('error', (err) => {
      res.writeHead(502, { 'Content-Type': 'text/plain' });
      res.end(`proxy error: ${err.message}`);
    });
    req.pipe(proxyReq);
    return;
  }

  let rel = decodeURIComponent(urlPath).replace(/^\/+/, '');
  if (!rel) rel = 'index.html';
  let filePath = path.join(opts.webDir, rel);
  if (!filePath.startsWith(opts.webDir) || !fs.existsSync(filePath) || fs.statSync(filePath).isDirectory()) {
    filePath = path.join(opts.webDir, 'index.html');
  }
  const ext = path.extname(filePath).toLowerCase();
  const contentType = MIME[ext] || 'application/octet-stream';
  const acceptBr = String(req.headers['accept-encoding'] || '').includes('br');
  const brPath = `${filePath}.br`;
  if (acceptBr && fs.existsSync(brPath)) {
    res.writeHead(200, {
      'Content-Type': contentType,
      'Content-Encoding': 'br',
      'Cache-Control': ext === '.html' ? 'no-cache' : 'public, max-age=3600',
    });
    fs.createReadStream(brPath).pipe(res);
  } else {
    res.writeHead(200, {
      'Content-Type': contentType,
      'Cache-Control': ext === '.html' ? 'no-cache' : 'public, max-age=3600',
    });
    fs.createReadStream(filePath).pipe(res);
  }
});

// Tunnel WebSocket upgrade requests directly to mica-api-server.
server.on('upgrade', (req, clientSocket, head) => {
  const upstream = net.connect(opts.apiPort, opts.apiHost, () => {
    const lines = [`${req.method} ${req.url} HTTP/${req.httpVersion}`];
    for (let i = 0; i < req.rawHeaders.length; i += 2) {
      const k = req.rawHeaders[i];
      const v = req.rawHeaders[i + 1];
      if (k.toLowerCase() === 'host') {
        lines.push(`Host: ${opts.apiHost}:${opts.apiPort}`);
      } else {
        lines.push(`${k}: ${v}`);
      }
    }
    upstream.write(`${lines.join('\r\n')}\r\n\r\n`);
    if (head && head.length) upstream.write(head);
    upstream.pipe(clientSocket);
    clientSocket.pipe(upstream);
  });
  upstream.on('error', () => clientSocket.destroy());
  clientSocket.on('error', () => upstream.destroy());
});

server.listen(opts.port, '127.0.0.1', () => {
  console.log(`mica-web-proxy listening on http://127.0.0.1:${opts.port} -> api :${opts.apiPort}`);
});
