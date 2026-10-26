// Loopback object store for the isolated avatar lifecycle tests
// (scripts/test-avatar-lifecycle.ps1). Answers the presigned requests the API
// sends and can HOLD one chosen request until /_control/release says otherwise
// — the scheduling knob the gh#152 race regression needs. Signatures are not
// checked: this is a local stand-in, not an S3 emulator.
const PORT = Number(process.env.AVATAR_S3_PORT);
const BUCKET = 'mica';
const objects = new Map(); // key -> { bytes, type }
const events = [];

// One armed hold at a time: the next request with this method + exact key is
// parked (and recorded as `<METHOD>-HELD`) until released.
let hold = null;

function armHold(method, key) {
  let release = null;
  const gate = new Promise((resolve) => { release = resolve; });
  hold = { method, key, gate, release, held: false };
}

Bun.serve({ hostname: '127.0.0.1', port: PORT, async fetch(req) {
  const url = new URL(req.url);
  if (url.pathname === '/_events') return Response.json(events);
  if (url.pathname === '/_object') {
    const key = url.searchParams.get('key') ?? '';
    return objects.has(key)
      ? Response.json({ exists: true, bytes: objects.get(key).bytes.length })
      : Response.json({ exists: false }, { status: 404 });
  }
  if (url.pathname === '/_control/hold' && req.method === 'POST') {
    const method = (url.searchParams.get('method') ?? 'DELETE').toUpperCase();
    const key = url.searchParams.get('key') ?? '';
    armHold(method, key);
    return Response.json({ ok: true, method, key });
  }
  if (url.pathname === '/_control/release' && req.method === 'POST') {
    const released = hold;
    hold = null;
    released?.release();
    return Response.json({ ok: true, method: released?.method ?? null, key: released?.key ?? null });
  }
  // The startup bucket probe (crates/api-server/src/bucket.rs): 2xx = exists.
  if (url.pathname === `/${BUCKET}` || url.pathname === `/${BUCKET}/`) {
    return new Response(null, { status: 200 });
  }
  const key = decodeURIComponent(url.pathname.replace(/^\/mica\//, ''));
  if (hold && !hold.held && hold.method === req.method && hold.key === key) {
    // Visible the moment the request arrives, so the test can proceed while
    // the API is parked mid-operation — not only once it finishes.
    hold.held = true;
    events.push({ method: `${req.method}-HELD`, key });
    await hold.gate;
  }
  if (req.method === 'PUT') {
    const bytes = Buffer.from(await req.arrayBuffer());
    objects.set(key, { bytes, type: req.headers.get('content-type') ?? 'application/octet-stream' });
    events.push({ method: 'PUT', key });
    return new Response(null, { status: 200 });
  }
  if (req.method === 'DELETE') {
    objects.delete(key);
    events.push({ method: 'DELETE', key });
    return new Response(null, { status: 204 });
  }
  const object = objects.get(key);
  if (!object) return new Response(null, { status: 404 });
  events.push({ method: req.method, key });
  return new Response(req.method === 'HEAD' ? null : object.bytes, {
    headers: { 'content-type': object.type, 'content-length': String(object.bytes.length) },
  });
}});
