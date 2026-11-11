// A stand-in for mica_preview_worker.js that speaks the same reply protocol
// without loading Rust, so the host's handling of each reply can be pinned:
//   'crash'  -> [id, null, 'crashed'] and the worker closes
//   'away'   -> [id, null, 'unavailable'] and the worker closes
//   'hold'   -> no reply (stays in flight)
//   'bad'    -> [id, null]
//   other    -> [id, 'ok:' + source]
self.onmessage = ({data: [id, , source]}) => {
  if (source === 'crash') {
    self.postMessage([id, null, 'crashed']);
    self.close();
  } else if (source === 'away') {
    self.postMessage([id, null, 'unavailable']);
    self.close();
  } else if (source === 'bad') {
    self.postMessage([id, null]);
  } else if (source !== 'hold') {
    self.postMessage([id, 'ok:' + source]);
  }
};
