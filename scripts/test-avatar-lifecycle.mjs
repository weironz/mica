// Real-API avatar lifecycle regressions (gh#152, gh#153). Run with
// scripts/test-avatar-lifecycle.ps1 against an isolated database and the
// hold-capable loopback store in scripts/test-avatar-s3.mjs.
import { test, expect } from 'bun:test';
import { SQL } from 'bun';
import { createHash } from 'node:crypto';

const api = process.env.AVATAR_TEST_API;
if (!api) throw new Error('Run scripts/test-avatar-lifecycle.ps1 against an isolated database');
const db = new SQL(process.env.DATABASE_URL);
const store = `http://127.0.0.1:${process.env.AVATAR_S3_PORT}`;

async function call(method, path, body, token, rawBytes) {
  return fetch(`${api}/api${path}`, {
    method,
    headers: {
      ...(body !== undefined ? { 'content-type': rawBytes ? 'image/png' : 'application/json' } : {}),
      ...(token ? { authorization: `Bearer ${token}` } : {}),
    },
    body: body === undefined ? undefined : rawBytes ? body : JSON.stringify(body),
  });
}

async function storeEvents() {
  return (await (await fetch(`${store}/_events`)).json());
}
async function hold(method, key) {
  const query = new URLSearchParams({ method, key });
  await fetch(`${store}/_control/hold?${query}`, { method: 'POST' });
}
async function releaseHold() {
  await fetch(`${store}/_control/release`, { method: 'POST' });
}
async function storedObject(key) {
  return fetch(`${store}/_object?key=${encodeURIComponent(key)}`);
}

async function until(check, what, ms = 10000) {
  const deadline = Date.now() + ms;
  while (Date.now() < deadline) {
    if (await check()) return;
    await Bun.sleep(20);
  }
  throw new Error(`timed out waiting for: ${what}`);
}

// A login-able account per test: the seed's verified hash, a fresh address.
async function fixture() {
  const email = `avatar-${crypto.randomUUID()}@mica.test`;
  await db`INSERT INTO users(email,display_name,password_hash,email_verified_at)
           SELECT ${email},'Avatar',password_hash,now() FROM users WHERE email='avatar-seed@mica.test'`;
  const auth = await call('POST', '/auth/login', { email, password: 'avatar-password-123' });
  expect(auth.status).toBe(200);
  const body = await auth.json();
  return { id: body.user.id, token: body.access_token };
}

// PNG magic followed by filler: enough for the magic-number sniff, different
// bytes per picture so the content-addressed keys differ.
const pngA = Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), Buffer.alloc(64, 1)]);
const pngB = Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), Buffer.alloc(64, 2)]);
const keyOf = (userId, bytes) => `avatars/${userId}/${createHash('sha256').update(bytes).digest('hex')}.png`;
const lockKey = (userId) => `avatar:${userId}`;

async function currentKey(userId) {
  return (await db`SELECT avatar_key FROM users WHERE id=${userId}`)[0]?.avatar_key;
}

// gh#152: two overlapping avatar PUTs on one account. R1 replaces A with B
// and its cleanup DELETE of A is parked at the store; R2 puts the SAME picture
// A back and is told it succeeded. The stale cleanup must not delete the
// picture the database now points at.
test('a parked cleanup cannot delete a re-selected avatar', async () => {
  const f = await fixture();
  const keyA = keyOf(f.id, pngA);
  const keyB = keyOf(f.id, pngB);
  expect((await call('PUT', '/auth/me/avatar', pngA, f.token, true)).status).toBe(200);

  await hold('DELETE', keyA);
  const r1 = call('PUT', '/auth/me/avatar', pngB, f.token, true);
  await until(async () => (await currentKey(f.id)) === keyB, 'the swap to B committed');
  await until(async () => (await storeEvents()).some((e) => e.method === 'DELETE-HELD' && e.key === keyA),
    'the cleanup of A parked at the store');
  const r2 = call('PUT', '/auth/me/avatar', pngA, f.token, true);
  // Whether R2 finished inside this window is exactly what the fix decides;
  // both orders must end with the same picture actually fetchable.
  await Bun.sleep(400);
  await releaseHold();
  expect((await r1).status).toBe(200);
  expect((await r2).status).toBe(200);
  expect(await currentKey(f.id)).toBe(keyA);

  const avatar = await fetch(`${api}/api/users/${f.id}/avatar`, { redirect: 'manual' });
  expect(avatar.status).toBe(307);
  const object = await fetch(avatar.headers.get('location'));
  expect(object.status).toBe(200);
  expect(Buffer.from(await object.arrayBuffer())).toEqual(pngA);
}, 30000);

// gh#152, the fail-closed half: if the cleanup cannot READ the current avatar
// key, it must not delete the object it replaced — being unable to confirm
// "nobody points at this" is not the same as confirming it. The database fault
// is injected deterministically: the test queues for the account's advisory
// lock BEHIND the running PUT's transaction, so it is handed the lock between
// that PUT's commit and its cleanup, then breaks the `users` table for the
// cleanup's re-read.
test('a failed re-check keeps the replaced object', async () => {
  const f = await fixture();
  const keyA = keyOf(f.id, pngA);
  const keyB = keyOf(f.id, pngB);
  expect((await call('PUT', '/auth/me/avatar', pngA, f.token, true)).status).toBe(200);

  await hold('PUT', keyB);
  const put = call('PUT', '/auth/me/avatar', pngB, f.token, true);
  await until(async () => (await storeEvents()).some((e) => e.method === 'PUT-HELD' && e.key === keyB),
    'the upload of B parked at the store');

  const conn = await db.reserve();
  try {
    // Queue behind the PUT's transaction, which is parked mid-upload holding
    // the account lock. Advisory waiters are granted in arrival order, so this
    // lands BEFORE the cleanup's own request for the same lock.
    const lockWait = conn`SELECT pg_advisory_lock(hashtextextended(${lockKey(f.id)}, 0))`;
    await Bun.sleep(50); // let the waiter reach Postgres before we unblock
    await releaseHold();
    await lockWait;
    // The cleanup's re-read now hits a database that cannot answer.
    await db`ALTER TABLE users RENAME TO users_e2_hidden`;
    await conn`SELECT pg_advisory_unlock(hashtextextended(${lockKey(f.id)}, 0))`;

    expect((await put).status).toBe(200); // the swap itself had already succeeded
    await db`ALTER TABLE users_e2_hidden RENAME TO users`;
  } finally {
    await db`ALTER TABLE IF EXISTS users_e2_hidden RENAME TO users`.catch(() => {});
    conn.release();
  }

  expect(await currentKey(f.id)).toBe(keyB);
  // The replaced object is still there: the delete was refused, not guessed.
  expect((await storedObject(keyA)).status).toBe(200);
  expect((await storeEvents()).some((e) => e.method === 'DELETE' && e.key === keyA)).toBe(false);
}, 30000);

// gh#153: an access JWT outlives its account until it expires, so a deleted
// account's token must not be able to write a new public avatar onto the
// tombstone, and the public GET must not serve whatever the tombstone says.
test('a deleted account cannot resurrect its avatar with a stale token', async () => {
  const f = await fixture();
  expect((await call('PUT', '/auth/me/avatar', pngA, f.token, true)).status).toBe(200);
  const live = await fetch(`${api}/api/users/${f.id}/avatar`, { redirect: 'manual' });
  expect(live.status).toBe(307); // the live account's picture still redirects

  expect((await call('DELETE', '/auth/me', { password: 'avatar-password-123' }, f.token)).status).toBe(204);

  const putAfter = await call('PUT', '/auth/me/avatar', pngB, f.token, true);
  expect(putAfter.status).toBe(401);
  const puts = (await storeEvents()).filter((e) => e.method === 'PUT' && e.key.startsWith(`avatars/${f.id}/`));
  expect(puts.length).toBe(1); // only the pre-deletion upload: nothing written for the dead account

  const after = await fetch(`${api}/api/users/${f.id}/avatar`, { redirect: 'manual' });
  expect(after.status).toBe(404);
}, 30000);
