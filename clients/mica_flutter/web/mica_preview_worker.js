// Loaded only when a preview is requested. Each renderer is fetched on its
// first use; rendering untrusted document content stays off the UI thread.
//
// Replies are [id, svg] — svg null when the source cannot be rendered — or
// [id, null, status] when the renderer itself could not run:
//   'unavailable'  the renderer failed to load (network, missing file); the
//                  worker closes — a browser remembers a failed module import
//                  for the worker's lifetime, so only a fresh worker can load
//                  it again — and the host retries later
//   'crashed'      this source crashed the renderer; the worker closes, and
//                  the host gives up on this source only
const engines = new Map();

function load(kind) {
  if (!engines.has(kind)) {
    const path = kind === 'math' ? './mica_preview/math.js' : './mica_preview/mermaid.js';
    engines.set(kind, import(path).then(async (module) => {
      await module.default();
      return module;
    }).catch((error) => {
      engines.delete(kind);
      throw error;
    }));
  }
  return engines.get(kind);
}

self.onmessage = async ({data: [id, kind, source, theme, display]}) => {
  let engine;
  try {
    engine = await load(kind);
  } catch (_) {
    self.postMessage([id, null, 'unavailable']);
    self.close();
    return;
  }
  let result = null;
  try {
    result = kind === 'math'
      ? engine.render_math(source, display, theme)
      : engine.render_mermaid(source, theme);
  } catch (error) {
    // A panic can leave Rust's instance unusable: retire this worker. Only
    // this source is to blame; the host retries everything else in flight.
    if (error instanceof WebAssembly.RuntimeError) {
      self.postMessage([id, null, 'crashed']);
      self.close();
      return;
    }
  }
  self.postMessage([id, result ?? null]);
};
