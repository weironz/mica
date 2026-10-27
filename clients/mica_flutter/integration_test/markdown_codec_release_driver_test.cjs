const {test} = require('node:test');
const assert = require('node:assert/strict');
const {EventEmitter} = require('node:events');
const {spawn} = require('node:child_process');
const {observeChild, stopOwnedChild, summarize} = require('./run_markdown_codec_release_probe.cjs');

test('good init cannot hide a failed performance row', () => {
  const reports = Array.from({length: 5}, () => ({init_ms: 2, success: true}));
  reports[0].bench = [{item: 'parse.87', passes: false}];
  assert.equal(summarize(reports, {success: true}, 'test').passes, false);
  reports[0].bench[0].passes = true;
  assert.equal(summarize(reports, {success: true}, 'test').passes, true);
  reports[3].init_ms = NaN;
  assert.equal(summarize(reports, {success: true}, 'test').passes, false);
});

test('spawn failure is observed without an unhandled rejection', async () => {
  const child = spawn('mica-probe-executable-does-not-exist', [], {stdio: 'ignore'});
  const observed = observeChild(child);
  await stopOwnedChild(child, observed, 500);
  assert.ok(observed.state.error);
});

test('a child that never exits fails within a bounded deadline', async () => {
  const child = new EventEmitter();
  child.pid = 123;
  const signals = [];
  child.kill = signal => { signals.push(signal ?? 'SIGTERM'); };
  const observed = observeChild(child);
  await assert.rejects(stopOwnedChild(child, observed, 10), /did not exit/);
  assert.deepEqual(signals, ['SIGTERM', 'SIGKILL']);
});
