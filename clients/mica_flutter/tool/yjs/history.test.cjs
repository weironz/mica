const {test, before} = require('node:test');
const assert = require('node:assert/strict');
let y;
before(async () => { await import('./entry.js'); y = globalThis.micaYjs; });

function seed() {
  const doc = y.newDoc();
  const blocks = y.getMap(doc, 'blocks');
  const root = y.newMap();
  y.mapSet(root, 'ty', 'page');
  y.mapSet(root, 'children', y.newArray());
  y.mapSet(blocks, 'root', root);
  const block = y.newMap();
  y.mapSet(block, 'ty', 'paragraph');
  y.mapSet(block, 'text', y.newText(''));
  y.mapSet(block, 'props', y.newMap());
  y.mapSet(blocks, 'p', block);
  y.arrayInsertJson(y.mapGet(root, 'children'), 0, '["p"]');
  y.mapSet(y.getMap(doc, 'meta'), 'root', 'root');
  y.resetHistory(doc);
  return doc;
}
const text = doc => y.mapGet(y.mapGet(y.getMap(doc, 'blocks'), 'p'), 'text');
const value = doc => y.textToString(text(doc));
const sync = (a, b) => assert.equal(y.applyUpdate(b, y.encodeState(a)), true);
function batch(doc, fn) {
  y.beginHistoryBatch(doc);
  try { fn(); } finally { y.endHistoryBatch(doc); }
}

const children = (doc, id = 'root') =>
  y.mapGet(y.mapGet(y.getMap(doc, 'blocks'), id), 'children');
const childIds = (doc, id = 'root') => y.arrayToList(children(doc, id));
function moveSeed(nested = false) {
  const doc = seed();
  const blocks = y.getMap(doc, 'blocks');
  y.mapDelete(blocks, 'p');
  for (const id of ['a', 'b', 'c', ...(nested ? ['left', 'right'] : [])]) {
    const block = y.newMap();
    y.mapSet(block, 'ty', 'paragraph');
    y.mapSet(block, 'text', y.newText(id));
    y.mapSet(block, 'children', y.newArray());
    y.mapSet(blocks, id, block);
  }
  y.arrayDelete(children(doc), 0, 1);
  y.arrayInsert(children(doc), 0, nested ? ['left', 'right'] : ['a', 'b', 'c']);
  if (nested) y.arrayInsert(children(doc, 'left'), 0, ['a', 'b', 'c']);
  y.resetHistory(doc);
  return doc;
}
function move(doc, id, from, to, index) {
  const source = children(doc, from);
  y.arrayDelete(source, y.arrayToList(source).indexOf(id), 1);
  y.arrayInsert(children(doc, to), index, [id]);
}
function publishDiff(a, b) {
  const diff = y.encodeDiff(a, y.encodeStateVector(b));
  for (let replay = 0; replay < 2; replay++) {
    assert.equal(y.applyUpdate(b, diff), true);
    assert.deepEqual(
      JSON.parse(y.mapEntriesJson(y.getMap(a, 'blocks'))),
      JSON.parse(y.mapEntriesJson(y.getMap(b, 'blocks'))),
      'history diff converges, including replay',
    );
  }
}

test('undo and redo preserve the occurrence moved again remotely in the same parent', () => {
  const a = moveSeed(), b = y.docFromUpdate(y.encodeState(a));
  batch(a, () => move(a, 'a', 'root', 'root', 2));
  sync(a, b);
  batch(b, () => move(b, 'a', 'root', 'root', 1));
  sync(b, a);
  assert.deepEqual(childIds(a), ['b', 'a', 'c']);

  assert.equal(y.undo(a), true);
  assert.deepEqual(childIds(a), ['b', 'a', 'c']);
  assert.deepEqual(y.historyDepths(a), [0, 1], 'repair adds no history item');
  publishDiff(a, b);
  y.redo(a);
  assert.deepEqual(childIds(a), ['b', 'a', 'c']);
  publishDiff(a, b);
});

test('undo keeps a remote cross-parent move while undoing text in the same batch', () => {
  const a = moveSeed(true), b = y.docFromUpdate(y.encodeState(a));
  const aText = doc => y.mapGet(y.mapGet(y.getMap(doc, 'blocks'), 'a'), 'text');
  batch(a, () => {
    move(a, 'a', 'left', 'left', 2);
    y.textInsert(aText(a), 1, ' local');
  });
  sync(a, b);
  batch(b, () => {
    move(b, 'a', 'left', 'right', 0);
    y.textInsert(aText(b), 7, ' remote');
  });
  sync(b, a);

  assert.equal(y.undo(a), true);
  assert.deepEqual(childIds(a, 'left'), ['b', 'c']);
  assert.deepEqual(childIds(a, 'right'), ['a']);
  assert.equal(y.textToString(aText(a)), 'a remote');
  publishDiff(a, b);
  assert.equal(y.redo(a), true);
  assert.deepEqual(childIds(a, 'left'), ['b', 'c']);
  assert.deepEqual(childIds(a, 'right'), ['a']);
  assert.equal(y.textToString(aText(a)), 'a local remote');
  publishDiff(a, b);
});

test('undo keeps the remote position after a cross-parent move back', () => {
  const a = moveSeed(true), b = y.docFromUpdate(y.encodeState(a));
  batch(a, () => move(a, 'a', 'left', 'right', 0));
  sync(a, b);
  batch(b, () => move(b, 'a', 'right', 'left', 1));
  sync(b, a);

  assert.equal(y.undo(a), true);
  assert.deepEqual(childIds(a, 'left'), ['b', 'a', 'c']);
  assert.deepEqual(childIds(a, 'right'), []);
  publishDiff(a, b);
  y.redo(a);
  assert.deepEqual(childIds(a, 'left'), ['b', 'a', 'c']);
  assert.deepEqual(childIds(a, 'right'), []);
  publishDiff(a, b);
});

test('redo preserves a remote occurrence created after undo', () => {
  const a = moveSeed(), b = y.docFromUpdate(y.encodeState(a));
  batch(a, () => move(a, 'a', 'root', 'root', 2));
  sync(a, b);
  assert.equal(y.undo(a), true);
  assert.deepEqual(childIds(a), ['a', 'b', 'c']);
  publishDiff(a, b);
  batch(b, () => move(b, 'a', 'root', 'root', 1));
  sync(b, a);

  assert.equal(y.redo(a), true);
  assert.deepEqual(childIds(a), ['b', 'a', 'c']);
  publishDiff(a, b);
});

test('local origins undo only local characters and publish a convergent diff', () => {
  const a = seed(), b = y.docFromUpdate(y.encodeState(a));
  batch(a, () => y.textInsert(text(a), 0, 'abc'));
  sync(a, b);
  batch(b, () => y.textInsert(text(b), 3, 'xyz'));
  sync(b, a);
  const sv = y.encodeStateVector(b);
  assert.equal(value(a), 'abcxyz');
  assert.equal(y.undo(a), true);
  assert.equal(value(a), 'xyz');
  y.applyUpdate(b, y.encodeDiff(a, sv));
  assert.equal(value(b), 'xyz');
  assert.equal(y.redo(a), true);
  assert.equal(value(a), 'abcxyz');
});

test('identical UTF-16 text retains remote identity', () => {
  const a = seed(), b = y.docFromUpdate(y.encodeState(a));
  batch(a, () => y.textInsert(text(a), 0, '😀aaa'));
  sync(a, b);
  batch(b, () => y.textInsert(text(b), 2, '😀aaa'));
  sync(b, a);
  assert.equal(y.undo(a), true);
  assert.equal(value(a), '😀aaa');
  assert.equal(y.redo(a), true);
  assert.equal(value(a), '😀😀aaaaaa');
});

test('redo does not resurrect a character deleted remotely', () => {
  const a = seed(), b = y.docFromUpdate(y.encodeState(a));
  batch(a, () => y.textInsert(text(a), 0, 'abc'));
  sync(a, b);
  batch(b, () => y.textDelete(text(b), 0, 1));
  sync(b, a);
  y.undo(a);
  assert.equal(value(a), '');
  y.redo(a);
  assert.equal(value(a), 'bc');
});

test('a remote update between undo and redo keeps the redo stack', () => {
  const a = seed(), b = y.docFromUpdate(y.encodeState(a));
  batch(a, () => y.textInsert(text(a), 0, 'mine'));
  sync(a, b);
  y.undo(a);
  sync(a, b);
  batch(b, () => y.textInsert(text(b), 0, 'theirs'));
  sync(b, a);
  assert.equal(y.canRedo(a), true);
  y.redo(a);
  assert.ok(value(a).includes('mine'));
  assert.ok(value(a).includes('theirs'));
});

test('remote map overwrite is preserved and untracked writes are excluded', () => {
  const a = seed(), b = y.docFromUpdate(y.encodeState(a));
  const props = d => y.mapGet(y.mapGet(y.getMap(d, 'blocks'), 'p'), 'props');
  batch(a, () => y.mapSetJson(props(a), 'level', '2'));
  sync(a, b);
  batch(b, () => y.mapSetJson(props(b), 'level', '3'));
  sync(b, a);
  y.undo(a);
  assert.equal(y.mapGet(props(a), 'level'), 3);
  y.redo(a);
  assert.equal(y.mapGet(props(a), 'level'), 3);
  y.resetHistory(a);
  y.textInsert(text(a), 0, 'system');
  assert.equal(y.canUndo(a), false);
});

test('separate batches stay separate while a composite batch is one step', () => {
  const a = seed();
  batch(a, () => {
    y.textInsert(text(a), 0, 'a');
    y.textInsert(text(a), 1, 'b');
    y.mapSetJson(y.mapGet(y.getMap(a, 'blocks'), 'p'), 'ty', '"heading"');
  });
  batch(a, () => y.textInsert(text(a), 2, 'c'));
  y.undo(a);
  assert.equal(value(a), 'ab');
  y.undo(a);
  assert.equal(value(a), '');
  assert.equal(y.mapGet(y.mapGet(y.getMap(a, 'blocks'), 'p'), 'ty'), 'paragraph');
  assert.equal(y.canUndo(a), false);
  y.redo(a);
  assert.equal(value(a), 'ab');
});

test('nested batches retain the outer boundary across remote application', () => {
  const a = seed(), b = y.docFromUpdate(y.encodeState(a));
  y.beginHistoryBatch(a);
  y.textInsert(text(a), 0, 'a');
  batch(a, () => y.textInsert(text(a), 1, 'b'));
  assert.equal(y.undo(a), false);
  sync(a, b);
  batch(b, () => y.textInsert(text(b), 2, 'R'));
  sync(b, a);
  y.textInsert(text(a), 3, 'c');
  y.endHistoryBatch(a);
  assert.deepEqual(y.historyDepths(a), [1, 0]);
  assert.equal(y.undo(a), true);
  assert.equal(value(a), 'R');
  assert.equal(y.redo(a), true);
  assert.equal(value(a), 'abRc');
});

test('equal-value retries and maintenance writes do not add undo items', () => {
  const a = seed();
  const block = y.mapGet(y.getMap(a, 'blocks'), 'p');
  const props = y.mapGet(block, 'props');
  batch(a, () => {
    y.textInsert(text(a), 0, 'typed');
    y.mapSet(block, 'ty', 'heading');
    y.mapSetJson(props, 'settings', '{"level":1,"color":"red"}');
  });
  batch(a, () => {
    y.mapSet(block, 'ty', 'heading');
    y.mapSetJson(props, 'settings', '{"color":"red","level":1}');
  });
  y.mapSetJson(y.getMap(a, 'meta'), 'maintenance', 'true');
  y.mapSetJson(props, 'file_id', '"uploaded"');
  assert.deepEqual(y.historyDepths(a), [1, 0]);
  y.undo(a);
  assert.equal(value(a), '');
  assert.equal(y.mapGet(props, 'file_id'), 'uploaded');
});

test('stack depths report skipped overwritten items', () => {
  const a = seed(), b = y.docFromUpdate(y.encodeState(a));
  batch(a, () => y.textInsert(text(a), 0, 'first'));
  batch(a, () => y.textInsert(text(a), 5, ' second'));
  const block = d => y.mapGet(y.getMap(d, 'blocks'), 'p');
  batch(a, () => y.mapSetJson(block(a), 'ty', '"heading"'));
  sync(a, b);
  batch(b, () => y.mapSetJson(block(b), 'ty', '"quote"'));
  sync(b, a);
  assert.deepEqual(y.historyDepths(a), [3, 0]);
  y.undo(a);
  assert.equal(value(a), 'first');
  assert.equal(y.mapGet(block(a), 'ty'), 'quote');
  assert.deepEqual(y.historyDepths(a), [1, 1]);
});
