// Operate only on copied bundles, private APPDATA and children spawned here.
const fs = require('node:fs');
const path = require('node:path');
const {spawn} = require('node:child_process');

const project = path.resolve(__dirname, '..');
const source = path.join(project, 'build/windows/x64/runner/Release');
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));

function observeChild(child) {
  const state = {done: false, code: null, signal: null, error: null};
  const completion = new Promise(resolve => {
    const finish = () => { state.done = true; resolve(); };
    child.once('error', error => { state.error = String(error); finish(); });
    child.once('exit', (code, signal) => { state.code = code; state.signal = signal; finish(); });
  });
  return {state, completion};
}

async function waitBounded(completion, milliseconds) {
  let timer;
  try {
    return await Promise.race([
      completion.then(() => true),
      new Promise(resolve => { timer = setTimeout(() => resolve(false), milliseconds); }),
    ]);
  } finally { clearTimeout(timer); }
}

async function stopOwnedChild(child, observed, timeout = 5000) {
  if (observed.state.done) return;
  if (!child.pid) {
    if (!await waitBounded(observed.completion, timeout)) {
      throw Error('Spawn did not produce a PID or an error within the deadline');
    }
    return;
  }
  child.kill();
  if (await waitBounded(observed.completion, timeout)) return;
  child.kill('SIGKILL');
  if (!await waitBounded(observed.completion, timeout)) {
    throw Error(`Owned child PID ${child.pid} did not exit; private APPDATA retained`);
  }
}

function summarize(reports, unavailable, root) {
  const samples = reports.map(report => report.init_ms).sort((a, b) => a - b);
  const bench = reports[0]?.bench;
  const benchPasses = Array.isArray(bench) && bench.length > 0 &&
    bench.every(row => row.passes === true);
  const initPasses = samples.length === 5 && samples.every(Number.isFinite) && samples[2] <= 30;
  return {reports, unavailable, initialization_p50_ms: samples[2] ?? null,
    initialization_budget_ms: 30, initialization_passes: initPasses,
    bench_passes: benchPasses,
    passes: initPasses && benchPasses && reports.every(report => report.success === true) &&
      unavailable?.success === true, artifacts: root};
}

async function run(root, index, unavailable) {
  const directory = path.join(root, `run-${index}`);
  const appdata = path.join(directory, 'isolated-appdata');
  const result = path.join(directory, 'result.json');
  let child, observed, report, failure;
  let stdout = '', stderr = '';
  const cleanupErrors = [];

  // One warmup launch per fresh copy, NOT timed: Windows Defender/SmartScreen
  // scans a newly copied binary on its first load, which costs seconds and
  // belongs to the copy, not to the engine. A fixed install directory sees
  // only the post-warmup state; that is what the 30 ms budget measures.
  // The warmup uses its OWN isolated APPDATA, never the user's data.
  async function launchOnce(extraEnv, resultPath, timeoutMs) {
    const warm = spawn(path.join(directory, 'mica_flutter.exe'), [], {
      cwd: directory,
      env: {...process.env, ...extraEnv, MICA_CODEC_PROBE_RESULT: resultPath,
        MICA_CODEC_PROBE_UNAVAILABLE: '0', MICA_CODEC_PROBE_BENCH: '0'},
      stdio: ['ignore', 'ignore', 'ignore'],
    });
    const warmObserved = observeChild(warm);
    const t0 = Date.now();
    while (!fs.existsSync(resultPath) && !warmObserved.state.done && Date.now() - t0 < timeoutMs) {
      await delay(200);
    }
    await stopOwnedChild(warm, warmObserved);
    if (!fs.existsSync(resultPath)) {
      throw Error(`Warmup launch produced no result: ${JSON.stringify(warmObserved.state)}`);
    }
    fs.rmSync(resultPath, {force: true});
  }

  try {
    fs.mkdirSync(directory);
    fs.cpSync(source, directory, {recursive: true});
    if (unavailable) {
      fs.renameSync(path.join(directory, 'rust_lib_mica_flutter.dll'),
        path.join(directory, 'native-library-withheld.dll'));
    } else {
      const warmAppdata = path.join(directory, 'warmup-appdata');
      fs.mkdirSync(warmAppdata);
      try {
        await launchOnce({APPDATA: warmAppdata, LOCALAPPDATA: warmAppdata},
          path.join(directory, 'warmup.json'), 90000);
      } finally {
        fs.rmSync(warmAppdata, {recursive: true, force: true});
      }
    }
    fs.mkdirSync(appdata);
    child = spawn(path.join(directory, 'mica_flutter.exe'), [], {
      cwd: directory,
      env: {...process.env, APPDATA: appdata, LOCALAPPDATA: appdata,
        MICA_CODEC_PROBE_RESULT: result,
        MICA_CODEC_PROBE_UNAVAILABLE: unavailable ? '1' : '0',
        MICA_CODEC_PROBE_BENCH: index === 0 ? '1' : '0'},
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    observed = observeChild(child); // Handle asynchronous spawn errors immediately.
    child.stdout?.on('data', data => stdout += data);
    child.stderr?.on('data', data => stderr += data);
    const deadline = Date.now() + 60000;
    while (!fs.existsSync(result) && !observed.state.done && Date.now() < deadline) {
      await delay(100);
    }
    if (!fs.existsSync(result)) {
      throw Error(`Probe ${index} produced no result: ${JSON.stringify(observed.state)}`);
    }
    report = JSON.parse(fs.readFileSync(result, 'utf8'));
    // A measured budget failure remains a report so all five cold samples and
    // the unavailable case finish; summarize() still returns a failing status.
  } catch (error) { failure = String(error.stack ?? error); }
  finally {
    if (child && observed) {
      try { await stopOwnedChild(child, observed); }
      catch (error) { cleanupErrors.push(String(error)); }
    }
    // Never remove data while our child could still hold its database open.
    if (!child || observed?.state.done) {
      try { fs.rmSync(appdata, {recursive: true, force: true}); }
      catch (error) { cleanupErrors.push(`APPDATA cleanup: ${error}`); }
    }
    try {
      fs.writeFileSync(path.join(directory, 'driver.json'), JSON.stringify({
        report, failure, cleanupErrors, child: observed?.state, stdout, stderr,
      }, null, 2));
    } catch (error) { cleanupErrors.push(`Receipt write: ${error}`); }
  }
  if (failure || cleanupErrors.length) {
    throw Error(JSON.stringify({index, failure, cleanupErrors, stdout, stderr}));
  }
  return {...report, stdout, stderr};
}

async function main() {
  let root, result;
  const reports = [];
  try {
    root = fs.mkdtempSync(path.join(project, 'build/wp-d-release-'));
    for (let i = 0; i < 5; i++) reports.push(await run(root, i, false));
    result = summarize(reports, await run(root, 5, true), root);
  } catch (error) {
    result = {passes: false, reports, artifacts: root, error: String(error.stack ?? error)};
  }
  try {
    fs.writeFileSync(path.join(project, 'build/wp-d-release-summary.json'), JSON.stringify(result, null, 2));
  } catch (error) {
    result.passes = false;
    result.summary_write_error = String(error);
  }
  console.log(JSON.stringify(result, null, 2));
  if (!result.passes) process.exitCode = 1;
}

module.exports = {observeChild, stopOwnedChild, summarize};
if (require.main === module) main().catch(error => {console.error(error); process.exitCode = 1;});
