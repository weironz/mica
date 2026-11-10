import '../src/rust/api/render.dart' as rust;
import 'preview_engine.dart' show MathSvg;

Future<String?> mermaidSvg(String source, rust.MermaidTheme theme) =>
    rust.renderMermaidSvg(source: source, theme: theme);

Future<MathSvg?> mathSvg(String source, bool display, String color) async {
  final result = await rust.renderMathSvg(
    source: source,
    display: display,
    color: color,
  );
  return result == null
      ? null
      : (
          svg: result.svg,
          width: result.width,
          height: result.height,
          baseline: result.baseline,
        );
}
