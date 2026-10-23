// A move deletes one children occurrence and inserts another. UndoManager can
// restore the deleted occurrence even after a peer moved the block again. Keep
// only a transient set of CRDT item IDs around each undo/redo, never snapshots
// attached to history entries.
import * as Y from 'yjs';

function placements(doc) {
  const result = [];
  for (const block of doc.getMap('blocks').values()) {
    if (!(block instanceof Y.Map)) continue;
    const children = block.get('children');
    if (!(children instanceof Y.Array)) continue;
    children.toArray().forEach((blockId, index) => {
      if (typeof blockId !== 'string') return;
      const item = Y.createRelativePositionFromTypeIndex(children, index).item;
      if (item) result.push({blockId, children, index, item});
    });
  }
  return result;
}

export function applyHistory(doc, manager, redo) {
  const before = new Map();
  for (const {item} of placements(doc)) {
    let clocks = before.get(item.client);
    if (!clocks) before.set(item.client, clocks = new Set());
    clocks.add(item.clock);
  }
  const existed = item => before.get(item.client)?.has(item.clock) === true;
  const changed = !!(redo ? manager.redo() : manager.undo());
  if (!changed) return false;

  doc.transact(() => {
    const after = placements(doc);
    const surviving = new Set(after.filter(p => existed(p.item)).map(p => p.blockId));
    // Reverse the scan so removals do not shift the remaining array indexes.
    // A surviving item may belong to a different parent than the restored one.
    for (let i = after.length - 1; i >= 0; i--) {
      const {blockId, children, index, item} = after[i];
      if (surviving.has(blockId) && !existed(item)) children.delete(index, 1);
    }
  }); // Ordinary origin: repair must neither become history nor clear redo.
  return true;
}
