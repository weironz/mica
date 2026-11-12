import 'package:flutter/material.dart';
import '../cjk_fonts.dart';

/// The UI size ladder also pins line heights, so overriding fontSize cannot
/// accidentally inherit Material's unrelated leading from a different size.
abstract final class MicaTypography {
  static const micro = TextStyle(
    fontSize: 11,
    height: 14 / 11,
    letterSpacing: 0,
  );
  static const caption = TextStyle(
    fontSize: 12,
    height: 16 / 12,
    letterSpacing: 0,
  );
  static const small = TextStyle(
    fontSize: 13,
    height: 18 / 13,
    letterSpacing: 0,
  );
  static const body = TextStyle(
    fontSize: 14,
    height: 20 / 14,
    letterSpacing: 0,
  );
  static const large = TextStyle(
    fontSize: 16,
    height: 24 / 16,
    letterSpacing: 0,
  );
  static const heading3 = TextStyle(
    fontSize: 20,
    height: 28 / 20,
    letterSpacing: 0,
  );
  static const heading2 = TextStyle(
    fontSize: 24,
    height: 32 / 24,
    letterSpacing: 0,
  );
  static const heading1 = TextStyle(
    fontSize: 30,
    height: 39 / 30,
    letterSpacing: 0,
  );
  static const loginBrand = TextStyle(
    fontSize: 26,
    height: 34 / 26,
    letterSpacing: 0,
  );
  static const loginTitle = TextStyle(
    fontSize: 34,
    height: 44 / 34,
    letterSpacing: 0,
  );

  static const documentParagraph = TextStyle(fontSize: 16, height: 26 / 16);
  static const documentFootnote = TextStyle(fontSize: 13, height: 20 / 13);
  static const documentCode = TextStyle(fontSize: 14, height: 22 / 14);
  static const documentHeadings = [
    TextStyle(fontSize: 30, height: 39 / 30, fontWeight: FontWeight.w600),
    TextStyle(fontSize: 24, height: 32 / 24, fontWeight: FontWeight.w600),
    TextStyle(fontSize: 20, height: 28 / 20, fontWeight: FontWeight.w600),
    TextStyle(fontSize: 18, height: 26 / 18, fontWeight: FontWeight.w600),
    TextStyle(fontSize: 16, height: 24 / 16, fontWeight: FontWeight.w600),
    TextStyle(fontSize: 14, height: 22 / 14, fontWeight: FontWeight.w500),
  ];
  static TextStyle get canvasMicro => withUiFont(micro);
  static TextStyle get canvasCaption => withUiFont(caption);
  static TextStyle get canvasSmall => withUiFont(small);
  static TextStyle get canvasBody => withUiFont(body);
  static TextStyle get canvasLarge => withUiFont(large);

  /// Painted labels have no DefaultTextStyle to inherit the shared families.
  static TextStyle withUiFont(TextStyle style) => style.copyWith(
    fontFamily: style.fontFamily ?? uiFontFamily,
    fontFamilyFallback: style.fontFamilyFallback ?? cjkFontFallback,
  );

  static TextStyle _merge(TextStyle? old, TextStyle size, FontWeight weight) =>
      (old ?? const TextStyle()).merge(size).copyWith(fontWeight: weight);

  static TextTheme textTheme(TextTheme base) => base.copyWith(
    displayLarge: _merge(base.displayLarge, heading1, FontWeight.w600),
    displayMedium: _merge(base.displayMedium, heading1, FontWeight.w600),
    displaySmall: _merge(base.displaySmall, heading1, FontWeight.w600),
    headlineLarge: _merge(base.headlineLarge, heading1, FontWeight.w600),
    headlineMedium: _merge(base.headlineMedium, heading2, FontWeight.w600),
    headlineSmall: _merge(base.headlineSmall, heading3, FontWeight.w600),
    titleLarge: _merge(base.titleLarge, large, FontWeight.w600),
    titleMedium: _merge(base.titleMedium, large, FontWeight.w600),
    titleSmall: _merge(base.titleSmall, body, FontWeight.w500),
    bodyLarge: _merge(base.bodyLarge, body, FontWeight.w400),
    bodyMedium: _merge(base.bodyMedium, body, FontWeight.w400),
    bodySmall: _merge(base.bodySmall, caption, FontWeight.w400),
    labelLarge: _merge(base.labelLarge, body, FontWeight.w500),
    labelMedium: _merge(base.labelMedium, caption, FontWeight.w400),
    labelSmall: _merge(base.labelSmall, micro, FontWeight.w400),
  );
}

/// Document font scaling happens before the pixel-height quantization.
TextStyle scaleDocumentTextStyle(
  TextStyle base,
  double scale, {
  String? family,
}) {
  final size = (base.fontSize ?? 16) * scale;
  return base.copyWith(
    fontSize: size,
    fontWeight:
        size < 16 &&
            (base.fontWeight?.value ?? FontWeight.w400.value) >=
                FontWeight.w600.value
        ? FontWeight.w500
        : base.fontWeight,
    height: base.height == null ? null : (size * base.height!).round() / size,
    fontFamily: family ?? base.fontFamily ?? uiFontFamily,
    fontFamilyFallback: cjkFontFallback,
  );
}
