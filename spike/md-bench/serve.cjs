// P0 static host: no COOP/COEP, no compression, no caching.
const http = require('node:http');
const fs = require('node:fs');
const path = require('node:path');
const root = path.resolve(__dirname, '../../clients/mica_flutter/build/web');
const mime = {'.wasm':'application/wasm', '.js':'application/javascript', '.html':'text/html', '.json':'application/json', '.svg':'image/svg+xml'};
http.createServer((req, res) => {
  const file = path.resolve(root, '.' + decodeURIComponent(new URL(req.url, 'http://localhost').pathname));
  if (!file.startsWith(root + path.sep) && file !== root) {res.writeHead(403).end();return;}
  const target = file === root || req.url === '/' ? path.join(root, 'index.html') : file;
  fs.readFile(target, (err, bytes) => {
    if (err) {res.writeHead(404).end(); return;}
    res.writeHead(200, {'Content-Type': mime[path.extname(target)] || 'application/octet-stream', 'Cache-Control':'no-store'});
    res.end(bytes);
  });
}).listen(8766, '127.0.0.1', () => console.log('P0 release host http://127.0.0.1:8766'));
