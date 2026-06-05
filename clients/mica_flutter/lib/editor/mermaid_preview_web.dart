import 'dart:async';
import 'dart:html' as html;
import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:typed_data';
import 'dart:ui' as ui;

const bool mermaidAvailable = true;

/// Lazy one-shot loader for the vendored mermaid.min.js (~2.5MB) — injected
/// only when a document actually contains a mermaid block, so ordinary pages
/// never pay for it.
Future<void>? _loading;

Future<void> _ensureLoaded() {
  if (globalContext.hasProperty('mermaid'.toJS).toDart) return Future.value();
  return _loading ??= () async {
    final script = html.ScriptElement()..src = 'mermaid.min.js';
    final done = Completer<void>();
    script.onLoad.first.then((_) => done.complete());
    script.onError.first.then((_) =>
        done.completeError(StateError('mermaid.min.js failed to load')));
    html.document.head!.append(script);
    await done.future;
    final mermaid = globalContext.getProperty('mermaid'.toJS) as JSObject;
    mermaid.callMethod(
      'initialize'.toJS,
      {
        'startOnLoad': false,
        'theme': 'neutral',
        // No foreignObject labels: an SVG containing them TAINTS the canvas
        // and toBlob throws. v11 honors the TOP-LEVEL htmlLabels key (the
        // flowchart-scoped one alone is ignored); useMaxWidth:false gives the
        // SVG an explicit width so the <img> decode size is trustworthy.
        'htmlLabels': false,
        'flowchart': {'htmlLabels': false, 'useMaxWidth': false},
      }.jsify(),
    );
  }();
}

int _renderSeq = 0;

/// Render mermaid [source] to a rasterized image: mermaid.js → SVG → blob
/// `<img>` → 2x offscreen canvas → PNG bytes → ui.Image. Any failure (load,
/// syntax, raster) resolves to null; the preview pipeline records it and the
/// block stays on its highlighted source form.
Future<ui.Image?> renderMermaid(String source) async {
  try {
    await _ensureLoaded();
    final mermaid = globalContext.getProperty('mermaid'.toJS) as JSObject;
    final promise = mermaid.callMethod(
      'render'.toJS,
      'micaMermaid${_renderSeq++}'.toJS,
      source.toJS,
    ) as JSPromise<JSObject>;
    final result = await promise.toDart;
    final svg = (result.getProperty('svg'.toJS) as JSString).toDart;

    // SVG → <img>. A blob URL keeps it same-origin so the canvas isn't
    // tainted (mermaid inlines its styles; nothing external is referenced).
    final blob = html.Blob([svg], 'image/svg+xml');
    final url = html.Url.createObjectUrlFromBlob(blob);
    try {
      final img = html.ImageElement();
      final loaded = Completer<void>();
      img.onLoad.first.then((_) => loaded.complete());
      img.onError.first.then(
          (_) => loaded.completeError(StateError('svg decode failed')));
      img.src = url;
      await loaded.future;

      // Draw at 2x for crisp text; the renderer draws back at half size.
      final w = ((img.naturalWidth == 0 ? 600 : img.naturalWidth) * 2);
      final h = ((img.naturalHeight == 0 ? 400 : img.naturalHeight) * 2);
      final canvas = html.CanvasElement(width: w, height: h);
      canvas.context2D.drawImageScaled(img, 0, 0, w, h);
      final png = await canvas.toBlob('image/png');
      final reader = html.FileReader()..readAsArrayBuffer(png);
      await reader.onLoad.first;
      final raw = reader.result;
      final bytes =
          raw is Uint8List ? raw : (raw as ByteBuffer).asUint8List();
      final codec = await ui.instantiateImageCodec(bytes);
      final frame = await codec.getNextFrame();
      return frame.image;
    } finally {
      html.Url.revokeObjectUrl(url);
    }
  } catch (_) {
    return null;
  }
}
