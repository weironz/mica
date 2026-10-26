// Real API regression tests. Run with scripts/test-files-quota.ps1.
import { test, expect } from 'bun:test';
import { SQL } from 'bun';
import { createHash } from 'node:crypto';
const api = process.env.FILES_TEST_API;
if (!api) throw new Error('Run scripts/test-files-quota.ps1 against an isolated database');
const db = new SQL(process.env.DATABASE_URL);
async function call(method, path, body, token) {
  const r = await fetch(`${api}/api${path}`, {method, headers: {'content-type':'application/json', ...(token ? {authorization:`Bearer ${token}`} : {})}, body: body === undefined ? undefined : JSON.stringify(body)});
  const text=await r.text();
  return {status:r.status, body:text?JSON.parse(text):null};
}
async function fixture() {
  const email = `quota-${crypto.randomUUID()}@mica.test`;
  await db`INSERT INTO users(email,display_name,password_hash,email_verified_at) SELECT ${email},'Quota',password_hash,now() FROM users WHERE email='quota-seed@mica.test'`;
  const auth = await call('POST','/auth/login',{email,password:'quota-password-123'});
  expect(auth.status).toBe(200);
  const token = auth.body.access_token;
  async function workspace() {
    const r = await call('POST','/workspaces',{name:'Quota regression'},token);
    expect([200,201]).toContain(r.status);
    return r.body.workspace.id;
  }
  return {token, workspace};
}
function payload(byte=1) {
  const bytes = Buffer.alloc(600,byte);
  return {bytes, body:{file_name:'quota.png',mime_type:'image/png',byte_size:600,content_hash:createHash('sha256').update(bytes).digest('hex')}};
}
async function upload(ws,token,p=payload()) {
  const signed = await call('POST',`/workspaces/${ws}/files/presign`,p.body,token);
  expect(signed.status).toBe(200);
  const u = signed.body.upload;
  expect((await fetch(u.upload_url,{method:'PUT',headers:{'content-type':u.content_type,'if-none-match':u.if_none_match,'x-amz-checksum-sha256':u.checksum_sha256},body:p.bytes})).status).toBe(200);
  return {key:signed.body.object_key, body:{object_key:signed.body.object_key,file_name:p.body.file_name,mime_type:p.body.mime_type,byte_size:p.body.byte_size}};
}
test('owner admission serializes distinct keys across owned workspaces', async () => {
  const f=await fixture(), a=await f.workspace(), b=await f.workspace();
  const p=payload(1), q=payload(2);
  const keys=[`workspaces/${a}/${p.body.content_hash}.png`,`workspaces/${b}/${q.body.content_hash}.png`];
  const barrier=await db.reserve();
  await barrier`BEGIN`;
  try {
    for (const key of keys) await barrier`SELECT pg_advisory_xact_lock(1296646977,hashtext(${key}))`;
    const pending=Promise.all([call('POST',`/workspaces/${a}/files/presign`,p.body,f.token),call('POST',`/workspaces/${b}/files/presign`,q.body,f.token)]);
    const deadline=Date.now()+10000;
    let blocked=false;
    while(Date.now()<deadline) {
      const [row]=await db`SELECT count(*)::int AS n FROM pg_stat_activity WHERE datname=current_database() AND wait_event='advisory'`;
      if(row.n>=2) {blocked=true;break;}
      await Bun.sleep(20);
    }
    expect(blocked).toBe(true);
    await barrier`COMMIT`;
    const results=await pending;
    expect(results.map(r=>r.status).sort()).toEqual([200,400]);
    expect(results.find(r=>r.status===400).body.code).toBe('owner_quota_exceeded');
  } finally {await barrier`ROLLBACK`;barrier.release();}
},20000);

test('exact presign and complete retries reuse the registered file at quota', async () => {
  const f=await fixture(), ws=await f.workspace();
  const uploaded=await upload(ws,f.token);
  const first=await call('POST',`/workspaces/${ws}/files/complete`,uploaded.body,f.token);
  expect(first.status).toBe(200);
  const retry=await call('POST',`/workspaces/${ws}/files/presign`,payload().body,f.token);
  expect(retry.status).toBe(200);
  expect(retry.body.existing.file.id).toBe(first.body.file.id);
  const completed=await call('POST',`/workspaces/${ws}/files/complete`,uploaded.body,f.token);
  expect(completed.status).toBe(200);
  expect(completed.body.file.id).toBe(first.body.file.id);
  const usage=await call('GET',`/workspaces/${ws}/usage`,undefined,f.token);
  expect(usage.body.bytes_used).toBe(600);
});

test('import-url reserves bytes before server PUT and deduplicates retries', async () => {
  const f=await fixture(), a=await f.workspace(), b=await f.workspace();
  const p={url:'http://example.com/fixture-image.png?byte=3'};
  const results=await Promise.all([call('POST',`/workspaces/${a}/files/import-url`,p,f.token),call('POST',`/workspaces/${b}/files/import-url`,{url:'http://example.com/fixture-image.png?byte=4'},f.token)]);
  expect(results.map(r=>r.status).sort()).toEqual([200,400]);
  const winner=results[0].status===200?a:b;
  const retry=await call('POST',`/workspaces/${winner}/files/import-url`,winner===a?p:{url:'http://example.com/fixture-image.png?byte=4'},f.token);
  expect(retry.status).toBe(200);
  expect(retry.body.file.id).toBe(results.find(r=>r.status===200).body.file.id);
},20000);

test('workspace deletion retains pending upload owner quota', async () => {
  const f=await fixture(), a=await f.workspace(), b=await f.workspace();
  await upload(a,f.token);
  expect((await call('DELETE',`/workspaces/${a}`,undefined,f.token)).status).toBe(204);
  const next=await call('POST',`/workspaces/${b}/files/presign`,payload(2).body,f.token);
  expect(next.status).toBe(Number(process.env.MICA_OWNER_QUOTA_BYTES)>0?400:200);
  if(Number(process.env.MICA_OWNER_QUOTA_BYTES)>0) expect(next.body.code).toBe('owner_quota_exceeded');
});

async function sweep() {
  const run=Bun.spawn([process.env.FILES_TEST_GC],{env:process.env,stdout:'pipe',stderr:'pipe'});
  const text=await new Response(run.stdout).text();
  const err=await new Response(run.stderr).text();
  expect(await run.exited,err).toBe(0);
  return text;
}
test('file DELETE remains collectible after its registered pending ledger was pruned',async()=>{
  const f=await fixture(), ws=await f.workspace();
  const u=await upload(ws,f.token);
  const completed=await call('POST',`/workspaces/${ws}/files/complete`,u.body,f.token);
  expect(completed.status).toBe(200);
  await db`UPDATE pending_file_uploads SET created_at=now()-interval '35 days',expires_at=now()-interval '2 days' WHERE workspace_id=${ws}::uuid`;
  await sweep();
  expect((await fetch(`${process.env.S3_ENDPOINT}/mica/${u.key}`,{method:'HEAD'})).status).toBe(200);
  const control=await fixture(), cw=await control.workspace();
  const cu=await upload(cw,control.token,payload(8));
  const cf=await call('POST',`/workspaces/${cw}/files/complete`,cu.body,control.token);
  expect(cf.status).toBe(200);
  await db`UPDATE files SET created_at=now()-interval '8 days',unreferenced_since=now()-interval '31 days' WHERE workspace_id=${cw}::uuid`;
  expect((await call('DELETE',`/workspaces/${ws}/files/${completed.body.file.id}`,undefined,f.token)).status).toBe(200);
  expect((await call('GET',`/workspaces/${ws}/files/${completed.body.file.id}`,undefined,f.token)).status).toBe(404);
  expect((await call('GET',`/workspaces/${ws}/usage`,undefined,f.token)).body.bytes_used).toBe(600);
  // Advance the existing collector grace in this test's private database.
  await db`UPDATE pending_file_uploads SET created_at=now()-interval '35 days',expires_at=now()-interval '2 days' WHERE workspace_id=${ws}::uuid`;
  await sweep();
  expect((await fetch(`${process.env.S3_ENDPOINT}/mica/${cu.key}`,{method:'HEAD'})).status).toBe(404);
  expect((await fetch(`${process.env.S3_ENDPOINT}/mica/${u.key}`,{method:'HEAD'})).status).toBe(404);
  expect((await call('GET',`/workspaces/${ws}/usage`,undefined,f.token)).body.bytes_used).toBe(0);
},120000);

test('in-flight import PUT reserves owner capacity without holding a DB transaction',async()=>{
  const f=await fixture(), a=await f.workspace(), b=await f.workspace();
  await fetch(`${process.env.S3_ENDPOINT}/_hold-put`);
  const importing=call('POST',`/workspaces/${a}/files/import-url`,{url:'http://example.com/fixture-image.png?byte=5'},f.token);
  try {
    const deadline=Date.now()+10000;
    let key;
    while(Date.now()<deadline) {
      key=(await (await fetch(`${process.env.S3_ENDPOINT}/_held`)).json()).key;
      if(key) break;
      await Bun.sleep(20);
    }
    expect(key).toBeTruthy();
    const signed=await call('POST',`/workspaces/${b}/files/presign`,payload(9).body,f.token);
    expect(signed.status).toBe(400);
    expect(signed.body.code).toBe('owner_quota_exceeded');
    const usage=await call('GET',`/workspaces/${a}/usage`,undefined,f.token);
    expect(usage.body.bytes_used).toBe(600);
  } finally {
    await fetch(`${process.env.S3_ENDPOINT}/_release-put`);
    expect((await importing).status).toBe(200);
  }
},20000);

test('exact complete retry succeeds when registered bytes fill most quota',async()=>{
  const f=await fixture(), ws=await f.workspace();
  const uploaded=await upload(ws,f.token);
  const first=await call('POST',`/workspaces/${ws}/files/complete`,uploaded.body,f.token);
  expect(first.status).toBe(200);
  const retry=await call('POST',`/workspaces/${ws}/files/complete`,uploaded.body,f.token);
  expect(retry.status).toBe(200);
  expect(retry.body.file.id).toBe(first.body.file.id);
});

test('an unfinished presign retry reserves its object only once',async()=>{
  const f=await fixture(), ws=await f.workspace();
  const first=await call('POST',`/workspaces/${ws}/files/presign`,payload().body,f.token);
  expect(first.status).toBe(200);
  const retry=await call('POST',`/workspaces/${ws}/files/presign`,payload().body,f.token);
  expect(retry.status).toBe(200);
  expect(retry.body.object_key).toBe(first.body.object_key);
  expect((await call('GET',`/workspaces/${ws}/usage`,undefined,f.token)).body.bytes_used).toBe(600);
});

test('concurrent distinct uploads respect the workspace quota',async()=>{
  const f=await fixture(), ws=await f.workspace();
  const p=payload(6),q=payload(7);
  const barrier=await db.reserve();
  await barrier`BEGIN`;
  try {
    for(const hash of [p.body.content_hash,q.body.content_hash]) await barrier`SELECT pg_advisory_xact_lock(1296646977,hashtext(${`workspaces/${ws}/${hash}.png`}))`;
    const pending=Promise.all([p,q].map(p=>call('POST',`/workspaces/${ws}/files/presign`,p.body,f.token)));
    const deadline=Date.now()+10000;
    let blocked=false;
    while(Date.now()<deadline) {
      const [r]=await db`SELECT count(*)::int AS n FROM pg_stat_activity WHERE datname=current_database() AND wait_event='advisory'`;
      if(r.n>=2) {blocked=true;break;}
      await Bun.sleep(20);
    }
    expect(blocked).toBe(true);
    await barrier`COMMIT`;
    const results=await pending;
    expect(results.map(r=>r.status).sort()).toEqual([200,400]);
    expect((await call('GET',`/workspaces/${ws}/usage`,undefined,f.token)).body.bytes_used).toBe(600);
  } finally {await barrier`ROLLBACK`;barrier.release();}
},20000);
