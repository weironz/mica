import 'dart:ui' as ui;

/// Non-web: no mermaid engine — returning null keeps ```mermaid blocks on
/// their highlighted source form (the dialect principle's degradation).
Future<ui.Image?> renderMermaid(String source, double targetWidth) async => null;

/// Whether this platform can render mermaid at all. The previewer opts out
/// entirely when false, so the pipeline never even tracks the source.
const bool mermaidAvailable = false;
