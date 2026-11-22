import 'package:flutter/material.dart';

import '../l10n/locale_controller.dart';
import '../ui/theme_tokens.dart';
import '../ui/typography.dart';

/// Eight drag handles around an embedded `mica-board` block in the document body.
enum BoardResizeHandle { nw, n, ne, e, se, s, sw, w }

/// Active inline resize drag state for an embedded `mica-board` block.
class BoardResizeSession {
  BoardResizeSession({
    required this.node,
    required this.handle,
    required this.startLocal,
    required Rect rect,
  }) : startWidth = rect.width,
       aspect = rect.height / rect.width.clamp(1.0, double.infinity),
       width = rect.width;

  final int node;
  final BoardResizeHandle handle;
  final Offset startLocal;
  final double startWidth;
  final double aspect;
  double width;

  double update(Offset currentLocal, double maxWidth) {
    width = computeResizedBoardWidth(
      handle: handle,
      startLocal: startLocal,
      currentLocal: currentLocal,
      startWidth: startWidth,
      aspect: aspect,
      maxWidth: maxWidth,
    );
    return width;
  }
}

const double kBoardMinWidth = 160.0;
const double kBoardFramePadV = 8.0;
const double kBoardFramePadRight = 8.0;
const double kBoardHandleRadius = 4.5;
const double kBoardHandleHitHalf = 9.0;

final Map<String, double> _boardNaturalWidths = {};

/// Cache the SVG's natural logical width for a `mica-board` source string so a
/// compact board renders at crisp 1:1 size instead of stretching to 100% width,
/// leaving room to drag-enlarge as well as shrink.
void rememberBoardNaturalWidth(String source, double width) {
  if (!width.isFinite || width <= 0) return;
  if (_boardNaturalWidths.length >= 200) {
    _boardNaturalWidths.remove(_boardNaturalWidths.keys.first);
  }
  _boardNaturalWidths[source] = width;
}

double? boardNaturalWidthFor(String source) => _boardNaturalWidths[source];

/// Maximum display width of an embedded board inside a document of [maxWidth].
double boardMaxWidthFor(double maxWidth, double gutter) =>
    (maxWidth - gutter - kBoardFramePadRight).clamp(
      kBoardMinWidth,
      double.infinity,
    );

/// Compute the embedded board frame rect, total block height, and top-right
/// `[preview | code]` view-switch rects (kept clear of the resize handles).
({Rect rect, double boxHeight, Rect viewPreviewTab, Rect viewCodeTab})
computeBoardFrame({
  required double maxWidth,
  required double gutter,
  required Size pixels,
  required double y,
  double? requestedWidth,
  double? naturalWidth,
}) {
  final maxW = boardMaxWidthFor(maxWidth, gutter);
  final aspect = (pixels.height / pixels.width.clamp(1.0, 1e9)).clamp(
    0.15,
    6.0,
  );
  final defaultW = (naturalWidth ?? maxW).clamp(240.0, maxW);
  final w = (requestedWidth ?? defaultW).clamp(kBoardMinWidth, maxW);
  final h = (w * aspect).clamp(64.0, 1400.0);
  final rect = Rect.fromLTWH(gutter, y + kBoardFramePadV, w, h);
  final tabTop = rect.top + 8;
  final codeLeft = (rect.right - 12 - 44).clamp(rect.left + 8, rect.right);
  final prevLeft = (codeLeft - 2 - 56).clamp(rect.left + 8, codeLeft);
  return (
    rect: rect,
    boxHeight: h + kBoardFramePadV * 2,
    viewPreviewTab: Rect.fromLTWH(prevLeft, tabTop, 56, 20),
    viewCodeTab: Rect.fromLTWH(codeLeft, tabTop, 44, 20),
  );
}

/// Center point of each of the 8 resize handles (corners first for hit priority).
List<(BoardResizeHandle, Offset)> boardHandleCenters(Rect rect) => [
  (BoardResizeHandle.se, rect.bottomRight),
  (BoardResizeHandle.sw, rect.bottomLeft),
  (BoardResizeHandle.ne, rect.topRight),
  (BoardResizeHandle.nw, rect.topLeft),
  (BoardResizeHandle.e, rect.centerRight),
  (BoardResizeHandle.w, rect.centerLeft),
  (BoardResizeHandle.s, rect.bottomCenter),
  (BoardResizeHandle.n, rect.topCenter),
];

/// Return whichever resize handle contains [local], or null.
BoardResizeHandle? hitTestBoardHandle(Rect rect, Offset local) {
  for (final (handle, center) in boardHandleCenters(rect)) {
    if ((local.dx - center.dx).abs() <= kBoardHandleHitHalf &&
        (local.dy - center.dy).abs() <= kBoardHandleHitHalf) {
      return handle;
    }
  }
  return null;
}

/// Directional resize cursor matching each handle.
MouseCursor boardResizeCursor(BoardResizeHandle handle) => switch (handle) {
  BoardResizeHandle.nw ||
  BoardResizeHandle.se => SystemMouseCursors.resizeUpLeftDownRight,
  BoardResizeHandle.ne ||
  BoardResizeHandle.sw => SystemMouseCursors.resizeUpRightDownLeft,
  BoardResizeHandle.e ||
  BoardResizeHandle.w => SystemMouseCursors.resizeLeftRight,
  BoardResizeHandle.n ||
  BoardResizeHandle.s => SystemMouseCursors.resizeUpDown,
};

/// Compute the new board width while dragging [handle] from [startLocal] to
/// [currentLocal], keeping aspect ratio locked and tracking the pointer 1:1.
double computeResizedBoardWidth({
  required BoardResizeHandle handle,
  required Offset startLocal,
  required Offset currentLocal,
  required double startWidth,
  required double aspect,
  required double maxWidth,
}) {
  final dx = currentLocal.dx - startLocal.dx;
  final dy = currentLocal.dy - startLocal.dy;
  final safeAspect = aspect.clamp(0.15, 6.0);
  final dyAsW = dy / safeAspect;

  double dominant(double sx, double sy) {
    final ax = sx * dx;
    final ay = sy * dyAsW;
    return ax.abs() >= ay.abs() ? ax : ay;
  }

  final delta = switch (handle) {
    BoardResizeHandle.e => dx,
    BoardResizeHandle.w => -dx,
    BoardResizeHandle.s => dyAsW,
    BoardResizeHandle.n => -dyAsW,
    BoardResizeHandle.se => dominant(1, 1),
    BoardResizeHandle.ne => dominant(1, -1),
    BoardResizeHandle.sw => dominant(-1, 1),
    BoardResizeHandle.nw => dominant(-1, -1),
  };
  final raw = (startWidth + delta).clamp(kBoardMinWidth, maxWidth);
  return (raw * 10).round() / 10.0;
}

/// Subtle resting card border around an embedded board.
void paintBoardFrame(Canvas canvas, Rect rect, MicaTokens tokens) {
  canvas.drawRRect(
    RRect.fromRectAndRadius(rect, const Radius.circular(6)),
    Paint()
      ..color = tokens.border.subtle
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.0,
  );
}

/// Hover / active-resize chrome: crisp blue border + 8 circular handles +
/// live pixel readout pill while dragging.
void paintBoardResizeChrome(
  Canvas canvas,
  Rect rect,
  MicaTokens tokens, {
  bool resizing = false,
  BoardResizeHandle? activeHandle,
}) {
  final strokeColor = tokens.editor.caret;
  canvas.drawRect(
    rect,
    Paint()
      ..color = strokeColor
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.5,
  );
  final fillPaint = Paint()
    ..color = tokens.surface.base
    ..style = PaintingStyle.fill;
  final activeFillPaint = Paint()
    ..color = strokeColor
    ..style = PaintingStyle.fill;
  final ringPaint = Paint()
    ..color = strokeColor
    ..style = PaintingStyle.stroke
    ..strokeWidth = 1.5;

  for (final (handle, center) in boardHandleCenters(rect)) {
    final active = resizing && handle == activeHandle;
    final r = active ? kBoardHandleRadius + 0.5 : kBoardHandleRadius;
    canvas.drawCircle(center, r, active ? activeFillPaint : fillPaint);
    canvas.drawCircle(center, r, ringPaint);
  }

  if (!resizing) return;
  final label = '${rect.width.round()} × ${rect.height.round()}';
  final tp = TextPainter(
    text: TextSpan(
      text: label,
      style: MicaTypography.canvasMicro.copyWith(color: tokens.surface.base),
    ),
    textDirection: TextDirection.ltr,
  )..layout();
  const padH = 6.0, padV = 3.0;
  final pillW = tp.width + padH * 2;
  final pillH = tp.height + padV * 2;
  final pill = Rect.fromLTWH(
    (rect.right - pillW - 10).clamp(rect.left + 6, rect.right - pillW),
    (rect.bottom - pillH - 10).clamp(rect.top + 6, rect.bottom - pillH),
    pillW,
    pillH,
  );
  canvas.drawRRect(
    RRect.fromRectAndRadius(pill, const Radius.circular(4)),
    Paint()..color = tokens.text.primary.withValues(alpha: 0.78),
  );
  tp.paint(canvas, Offset(pill.left + padH, pill.top + padV));
  tp.dispose();
}

/// Paint the hover `[preview | code]` view-switch tabs for Mermaid / board blocks.
void paintDiagramViewTabs({
  required Canvas canvas,
  required Offset offset,
  required MicaTokens tokens,
  required Rect? previewRect,
  required Rect? codeRect,
  required String previewLabel,
  required String codeLabel,
  required String active,
  required bool hoverPreview,
  required bool hoverCode,
}) {
  void tab(Rect? r0, String text, bool isActive, bool isHovered) {
    if (r0 == null) return;
    final r = r0.shift(offset);
    canvas.drawRRect(
      RRect.fromRectAndRadius(r, const Radius.circular(6)),
      Paint()
        ..color = isActive || isHovered
            ? tokens.surface.hover
            : tokens.border.normal,
    );
    final tp = TextPainter(
      text: TextSpan(
        text: text,
        style: MicaTypography.canvasMicro.copyWith(
          color: isActive ? tokens.text.primary : tokens.text.muted,
        ),
      ),
      textDirection: TextDirection.ltr,
    )..layout();
    tp.paint(canvas, r.center - Offset(tp.width / 2, tp.height / 2));
    tp.dispose();
  }

  tab(previewRect, previewLabel, active == 'preview', hoverPreview);
  tab(codeRect, codeLabel, active == 'code', hoverCode);
}

/// Hostname display helper for broken/external image badges.
String hostOfImageUrl(String url) {
  final h = Uri.tryParse(url)?.host ?? '';
  return h.isEmpty ? url : h;
}

/// Paint the "外链" chip in an external image's top-left corner.
void paintExternalImageBadge(Canvas canvas, Rect r) {
  final label = TextPainter(
    text: TextSpan(
      text: l10nNoContext.imageExternalBadge,
      style: MicaTypography.canvasMicro.copyWith(
        color: MicaTokens.light.accent.onPrimary,
      ),
    ),
    textDirection: TextDirection.ltr,
  )..layout();
  const padH = 5.0, padV = 2.0;
  final chip = Rect.fromLTWH(
    r.left + 6,
    r.top + 6,
    label.width + padH * 2,
    label.height + padV * 2,
  );
  if (chip.width > r.width - 12 || chip.height > r.height - 12) {
    label.dispose();
    return;
  }
  canvas.drawRRect(
    RRect.fromRectAndRadius(chip, const Radius.circular(4)),
    Paint()..color = MicaTokens.dark_.surface.hover.withValues(alpha: 0.7),
  );
  label.paint(canvas, Offset(chip.left + padH, chip.top + padV));
  label.dispose();
}

/// Paint the placeholder box (icon + optional broken-host caption) for an
/// image whose bitmap has not loaded or failed.
void paintImagePlaceholder(
  Canvas canvas,
  Rect r,
  RRect rr,
  MicaTokens tokens, {
  required bool isError,
  required bool external,
  required String? url,
}) {
  canvas.drawRRect(rr, Paint()..color = tokens.editor.codeBg);
  final icon = isError ? Icons.broken_image_outlined : Icons.image_outlined;
  final glyph = TextPainter(
    text: TextSpan(
      text: String.fromCharCode(icon.codePoint),
      style: TextStyle(
        fontFamily: icon.fontFamily,
        package: icon.fontPackage,
        fontSize: 30,
        color: tokens.text.faint,
      ),
    ),
    textDirection: TextDirection.ltr,
  )..layout();
  final caption = isError && external ? hostOfImageUrl(url ?? '') : null;
  final capPainter = caption == null
      ? null
      : (TextPainter(
          text: TextSpan(
            text: l10nNoContext.imageBrokenCaption(caption),
            style: MicaTypography.canvasMicro.copyWith(color: tokens.text.faint),
          ),
          textDirection: TextDirection.ltr,
          maxLines: 1,
          ellipsis: '…',
        )..layout(maxWidth: r.width - 16));
  final blockH =
      glyph.height + (capPainter == null ? 0 : capPainter.height + 4);
  final top = r.center.dy - blockH / 2;
  glyph.paint(canvas, Offset(r.center.dx - glyph.width / 2, top));
  capPainter?.paint(
    canvas,
    Offset(r.center.dx - capPainter.width / 2, top + glyph.height + 4),
  );
  glyph.dispose();
  capPainter?.dispose();
  if (external) paintExternalImageBadge(canvas, r);
}
