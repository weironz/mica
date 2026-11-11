export 'preview_engine_io.dart'
    if (dart.library.js_interop) 'preview_engine_web.dart';

/// RaTeX dimensions are in em and the baseline is measured from the top.
typedef MathSvg = ({String svg, double width, double height, double baseline});

/// The renderer could not run at all — its code failed to load, or another
/// source crashed it while this one was in flight. Unlike a null result (the
/// source itself cannot be rendered), this is worth retrying later.
class PreviewUnavailable implements Exception {
  const PreviewUnavailable();

  @override
  String toString() => 'PreviewUnavailable';
}
