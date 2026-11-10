export 'preview_engine_io.dart'
    if (dart.library.js_interop) 'preview_engine_web.dart';

/// RaTeX dimensions are in em and the baseline is measured from the top.
typedef MathSvg = ({String svg, double width, double height, double baseline});
