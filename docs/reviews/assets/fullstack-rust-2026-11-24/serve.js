// Usage: node serve.js <web-dir> [port]  (web-dir = output of scripts/build-web-rust.sh plus .br files)
// Static server mirroring deploy/nginx.conf: precompressed .br for js/wasm when the
// client accepts br, application/wasm content type, Cache-Control: no-cache.
const http = require('http'), fs = require('fs'), path = require('path');
const root = process.argv[2], port = +process.argv[3] || 8099;
const types = { '.html': 'text/html; charset=utf-8', '.js': 'application/javascript', '.wasm': 'application/wasm', '.svg': 'image/svg+xml' };
http.createServer((req, res) => {
  let p = decodeURIComponent(req.url.split('?')[0]);
  if (p === '/') p = '/index.html';
  const file = path.join(root, p);
  if (!file.startsWith(root) || !fs.existsSync(file)) {
    // Cloud API calls from the app: answer like an unreachable backend quickly.
    res.writeHead(p.startsWith('/api/') ? 401 : 404, { 'content-type': 'application/json' });
    return res.end('{}');
  }
  const ext = path.extname(file);
  const headers = { 'content-type': types[ext] || 'application/octet-stream', 'cache-control': 'no-cache' };
  let body = file;
  if ((ext === '.js' || ext === '.wasm') && /\bbr\b/.test(req.headers['accept-encoding'] || '') && fs.existsSync(file + '.br')) {
    body = file + '.br'; headers['content-encoding'] = 'br'; headers['vary'] = 'Accept-Encoding';
  }
  const st = fs.statSync(body);
  const etag = '"' + st.mtimeMs.toString(16) + '-' + st.size.toString(16) + '"';
  headers['etag'] = etag; headers['last-modified'] = st.mtime.toUTCString();
  if (req.headers['if-none-match'] === etag) { res.writeHead(304, { etag, 'cache-control': 'no-cache' }); return res.end(); }
  headers['content-length'] = st.size;
  res.writeHead(200, headers);
  fs.createReadStream(body).pipe(res);
}).listen(port, () => console.log('listening', port));
