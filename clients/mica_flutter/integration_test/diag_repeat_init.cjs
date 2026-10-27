// Diagnostic only: launch the SAME copied bundle repeatedly to separate
// cold-copy directory cost from codec init cost. Operates solely on this
// probe directory and its own child processes.
const fs = require('node:fs');
const path = require('node:path');
const {spawn} = require('node:child_process');

const dir = path.resolve('clients/mica_flutter/build/wp-d-release-rRIrKz/run-0');
const out = [];
(async () => {
  for (let n = 1; n <= 4; n++) {
    const appdata = path.join(dir, `diag-${n}-appdata`);
    const result = path.join(dir, `diag-${n}.json`);
    try { fs.rmSync(appdata, {recursive: true, force: true}); } catch {}
    fs.mkdirSync(appdata);
    const child = spawn(path.join(dir, 'mica_flutter.exe'), [], {
      cwd: dir,
      env: {...process.env, APPDATA: appdata, LOCALAPPDATA: appdata,
        MICA_CODEC_PROBE_RESULT: result, MICA_CODEC_PROBE_UNAVAILABLE: '0',
        MICA_CODEC_PROBE_BENCH: '0'},
      stdio: ['ignore', 'ignore', 'ignore'],
    });
    const t0 = Date.now();
    let done = false;
    child.once('exit', () => { done = true; });
    while (!fs.existsSync(result) && !done && Date.now() - t0 < 60000) {
      await new Promise(r => setTimeout(r, 200));
    }
    if (child.exitCode === null) child.kill();
    await new Promise(r => child.once('exit', r));
    out.push({run: n, exists: fs.existsSync(result),
      init_ms: fs.existsSync(result) ? JSON.parse(fs.readFileSync(result)).init_ms : null});
    try { fs.rmSync(appdata, {recursive: true, force: true}); } catch {}
  }
  fs.writeFileSync(path.join(dir, 'diag-repeat.json'), JSON.stringify(out, null, 2));
  console.log(JSON.stringify(out));
})().catch(e => { console.error(e); process.exitCode = 1; });
