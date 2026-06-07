// Mermaid diagram rendering for ```mermaid fenced blocks.
//
// `renderMermaid(source)` resolves to a rasterized ui.Image of the diagram,
// or null when rendering fails (bad syntax, engine load) — the preview pipeline
// then leaves the block on its highlighted source form. Web drives the vendored
// mermaid.min.js through JS interop; the non-web (_stub) variant drives the
// headless pure-Rust `merman` engine via FFI, rendering OFFLINE. Both platforms
// support mermaid (`mermaidAvailable == true`); only the backend differs.
export 'mermaid_preview_stub.dart'
    if (dart.library.html) 'mermaid_preview_web.dart';
