// Run a prebuilt release probe in five cold processes, then a sixth with the
// native DLL withheld to exercise actual app.main unavailable initialization.
// Only the child process spawned here is stopped; user Mica is never touched.
const fs = require('node:fs');
const path = require('node:path');
const {spawn} = require('node:child_process');
const {once} = require('node:events');

const project = path.resolve(__dirname, '..');
const source = path.join(project, 'build/windows/x64/runner/Release');
const root = fs.mkdtempSync(path.join(project, 'build/wp-d-release-'));

async function run(index, unavailable) {
  const directory = path.join(root, `run-${index}`);
  fs.cpSync(source, directory, {recursive: true});
  const appdata = path.join(directory, 'isolated-appdata');
  fs.mkdirSync(appdata);
  const result = path.join(directory, 'result.json');
  if (unavailable) {
    fs.renameSync(path.join(directory, 'rust_lib_mica_flutter.dll'),
      path.join(directory, 'native-library-withheld.dll'));
  }
  const child = spawn(path.join(directory, 'mica_flutter.exe'), [], {
    cwd: directory,
    env: {...process.env, APPDATA: appdata, LOCALAPPDATA: appdata,
      MICA_CODEC_PROBE_RESULT: result,
      MICA_CODEC_PROBE_UNAVAILABLE: unavailable ? '1' : '0',
      MICA_CODEC_PROBE_BENCH: index === 0 ? '1' : '0'},
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let stdout = '', stderr = '';
  child.stdout.on('data', data => stdout += data);
  child.stderr.on('data', data => stderr += data);
  const exited = once(child, 'exit');
  try {
    const deadline = Date.now() + 60000;
    while (!fs.existsSync(result) && child.exitCode === null && Date.now() < deadline) {
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    if (!fs.existsSync(result)) throw Error(`probe ${index} produced no first-frame result; exit=${child.exitCode}`);
    const report = JSON.parse(fs.readFileSync(result, 'utf8'));
    if (!report.success) throw Error(`probe ${index} failed: ${JSON.stringify(report)}`);
    return {...report, stdout, stderr};
  } finally {
    if (child.exitCode === null) child.kill();
    await exited;
    fs.writeFileSync(path.join(directory, 'console.json'), JSON.stringify({stdout, stderr}));
    // The directory was created by this invocation; keep receipts, remove only
    // its private runtime preferences/database after our child has exited.
    fs.rmSync(appdata, {recursive: true, force: true});
  }
}

(async () => {
  const reports = [];
  for (let i = 0; i < 5; i++) reports.push(await run(i, false));
  const unavailable = await run(5, true);
  const samples = reports.map(report => report.init_ms).sort((a, b) => a - b);
  const result = {reports, unavailable, initialization_p50_ms: samples[2],
    initialization_budget_ms: 30, passes: samples[2] <= 30, artifacts: root};
  fs.writeFileSync(path.join(project, 'build/wp-d-release-summary.json'), JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result, null, 2));
  if (!result.passes) process.exitCode = 1;
})().catch(error => {console.error(error); process.exitCode = 1;});
