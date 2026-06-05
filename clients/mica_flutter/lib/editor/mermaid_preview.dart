// Mermaid diagram rendering for ```mermaid fenced blocks.
//
// `renderMermaid(source)` resolves to a rasterized ui.Image of the diagram,
// or null when rendering is unavailable (non-web platforms) or fails (bad
// syntax) — the preview pipeline then leaves the block on its highlighted
// source form. Web drives the vendored mermaid.min.js through JS interop;
// the stub keeps every other platform on the graceful-degradation path.
export 'mermaid_preview_stub.dart'
    if (dart.library.html) 'mermaid_preview_web.dart';
