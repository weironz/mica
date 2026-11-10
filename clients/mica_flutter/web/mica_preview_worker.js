// Loaded only when a preview is requested. Each renderer is fetched on its
// first use; rendering untrusted document content stays off the UI thread.
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
  let result = null;
  try {
    const engine = await load(kind);
    result = kind === 'math'
      ? engine.render_math(source, display, theme)
      : engine.render_mermaid(source, theme);
  } catch (error) {
    // A panic can leave Rust's instance unusable. Retire this worker and let
    // the host release any other requests waiting on it.
    if (error instanceof WebAssembly.RuntimeError) {
      self.postMessage([null, null]);
      self.close();
      return;
    }
  }
  self.postMessage([id, result ?? null]);
};
