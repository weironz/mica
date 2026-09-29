// Browser Fetch supplies Content-Length itself. A presigned PUT must bind that
// real byte count to the value the API accepted, before RustFS stores bytes.
import { createHash } from 'node:crypto';
import { chromium } from '@playwright/test';

const arg = (name, fallback) => {
  const index = process.argv.indexOf(`--${name}`);
  return index >= 0 ? process.argv[index + 1] : fallback;
};
const api = arg('api', 'http://127.0.0.1:8080');
const site = arg('site', 'http://127.0.0.1:8090');
const email = arg('email', 'e2e@mica.test');
const password = arg('password', 'e2epassword123');

async function json(path, token, body, method = 'POST') {
  const response = await fetch(`${api}${path}`, {
    method,
    headers: {
      'content-type': 'application/json',
      ...(token ? { authorization: `Bearer ${token}` } : {}),
    },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });
  const value = response.status === 204 ? null : await response.json();
  if (!response.ok) throw new Error(`${method} ${path}: ${response.status} ${JSON.stringify(value)}`);
  return value;
}

async function launch() {
  try { return await chromium.launch(); }
  catch (bundled) {
    for (const channel of ['chrome', 'msedge']) {
      try { return await chromium.launch({ channel }); } catch { /* next */ }
    }
    throw bundled;
  }
}

const browser = await launch();
const page = await browser.newPage();
let workspace;
let token;
try {
  await page.goto(site, { waitUntil: 'load' });
  token = (await json('/api/auth/login', null, { email, password })).access_token;
  workspace = (await json('/api/workspaces', token, { name: 'Upload length E2E' })).workspace.id;

  async function presign(bytes, claimedSize) {
    const hash = createHash('sha256').update(bytes).digest('hex');
    return json(`/api/workspaces/${workspace}/files/presign`, token, {
      file_name: `${hash}.txt`, mime_type: 'text/plain',
      byte_size: claimedSize, content_hash: hash,
    });
  }
  async function put(presigned, bytes) {
    return page.evaluate(async ({ upload, data }) => {
      const response = await fetch(upload.upload_url, {
        method: 'PUT',
        headers: {
          'content-type': 'text/plain',
          'if-none-match': upload.if_none_match,
          'x-amz-checksum-sha256': upload.checksum_sha256,
        },
        body: Uint8Array.from(data),
      });
      return response.status;
    }, { upload: presigned.upload, data: [...bytes] });
  }

  const oversizedBytes = Buffer.from('longer');
  const oversized = await presign(oversizedBytes, 4);
  if (!oversized.upload) throw new Error('oversized probe unexpectedly deduplicated');
  const rejected = await put(oversized, oversizedBytes);
  if (rejected !== 403) throw new Error(`6-byte browser PUT signed for 4 bytes returned ${rejected}`);

  const validBytes = Buffer.from('okay');
  const valid = await presign(validBytes, validBytes.length);
  if (!valid.upload) throw new Error('valid probe unexpectedly deduplicated');
  const accepted = await put(valid, validBytes);
  if (accepted !== 200) throw new Error(`4-byte browser PUT signed for 4 bytes returned ${accepted}`);
  await json(`/api/workspaces/${workspace}/files/complete`, token, {
    object_key: valid.object_key,
    file_name: 'valid.txt', mime_type: 'text/plain', byte_size: validBytes.length,
  });
  console.log('browser upload length e2e: 6-byte body rejected; 4-byte body stored and completed');
} catch (error) {
  await page.screenshot({ path: 'upload-length-e2e-failure.png', fullPage: true }).catch(() => {});
  throw error;
} finally {
  if (workspace && token) {
    await json(`/api/workspaces/${workspace}`, token, undefined, 'DELETE')
      .catch(error => console.error(`cleanup failed: ${error}`));
  }
  await browser.close();
}
