// High-performance two-layer RepaintBoundary renderer for Mica's whiteboard:
// - Layer 1 (StaticBoardPainter): vector ui.Picture cache (0 repaints during drag!)
// - Layer 2 (ActiveBoardPainter): dragged/resized nodes, live edges, 8 resize handles,
//   4 connector anchor ports, smart alignment guides, marquee, active ink, collapse badges
import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/material.dart';

import 'board_controller.dart';
import 'board_icons.dart';
import 'board_models.dart';

class StaticBoardPainter extends CustomPainter {
  StaticBoardPainter({
    required this.controller,
    required this.pictureProvider,
    required this.graphRev,
    required this.viewportRev,
    required this.dark,
  }) : super(repaint: controller.staticPaintSignal);

  final BoardController controller;
  final ui.Picture Function() pictureProvider;
  final int graphRev;
  final int viewportRev;
  final bool dark;

  @override
  void paint(Canvas canvas, Size size) {
    controller.staticPaintCount++;
    _paintDotGrid(canvas, size, controller.zoom, controller.pan, dark);
    final picture = pictureProvider();
    canvas.save();
    canvas.translate(controller.pan.dx, controller.pan.dy);
    canvas.scale(controller.zoom, controller.zoom);
    canvas.drawPicture(picture);
    canvas.restore();
  }

  @override
  bool shouldRepaint(covariant StaticBoardPainter oldDelegate) =>
      oldDelegate.graphRev != graphRev ||
      oldDelegate.viewportRev != viewportRev ||
      oldDelegate.dark != dark;
}

class ActiveBoardPainter extends CustomPainter {
  ActiveBoardPainter({
    required this.controller,
    required this.activeRev,
    required this.dark,
  }) : super(repaint: controller.activePaintSignal);

  final BoardController controller;
  final int activeRev;
  final bool dark;

  @override
  void paint(Canvas canvas, Size size) {
    controller.activePaintCount++;
    final hidden = collapsedDescendants(controller.snapshot);
    final dynamicIds = _dynamicNodeIds(controller);

    canvas.save();
    canvas.translate(controller.pan.dx, controller.pan.dy);
    canvas.scale(controller.zoom, controller.zoom);

    // 1. Live incident edges for dynamic (dragged / resized / edited) nodes.
    if (dynamicIds.isNotEmpty) {
      for (final edge in controller.snapshot.edges) {
        if (hidden.contains(edge.fromId) || hidden.contains(edge.toId)) continue;
        if (!dynamicIds.contains(edge.fromId) && !dynamicIds.contains(edge.toId)) {
          continue;
        }
        paintBoardEdge(
          canvas,
          edge,
          controller,
          selected: edge.id == controller.selectedEdgeId,
          dark: dark,
        );
      }
      // Paint dragged/resized nodes (editing node box is rendered by overlay widget).
      for (final id in dynamicIds) {
        if (id == controller.editingNodeId) continue;
        final n = controller.nodeById[id];
        if (n != null && !hidden.contains(id)) {
          paintBoardNode(canvas, n, controller, dark: dark);
        }
      }
    }

    // 2. Highlighted selected edge (when not already drawn above).
    if (controller.selectedEdgeId != null) {
      final edge = controller.snapshot.edges
          .where((e) => e.id == controller.selectedEdgeId)
          .firstOrNull;
      if (edge != null &&
          !dynamicIds.contains(edge.fromId) &&
          !dynamicIds.contains(edge.toId)) {
        paintBoardEdge(canvas, edge, controller, selected: true, dark: dark);
      }
    }

    // 3. Mind map collapse/expand badges on parent nodes.
    _paintCollapseBadges(canvas, controller, hidden, dark);

    // 4. Selection outline, 8 resize handles, and 4 connector ports.
    final selPaint = Paint()
      ..color = BoardColors.brand
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.8 / controller.zoom;
    for (final id in controller.selectedNodeIds) {
      if (id == controller.editingNodeId || hidden.contains(id)) continue;
      final n = controller.nodeById[id];
      if (n == null) continue;
      canvas.drawRect(n.rect.inflate(2 / controller.zoom), selPaint);
    }

    if (controller.selectedNodeIds.length == 1 &&
        controller.editingNodeId == null) {
      final n = controller.nodeById[controller.selectedNodeIds.first];
      if (n != null && !hidden.contains(n.id)) {
        _paintResizeHandles(canvas, n, controller.zoom);
      }
    }

    // Connector ports on selected or hovered node.
    final portTargets = <String>{
      if (controller.selectedNodeIds.length == 1) controller.selectedNodeIds.first,
      if (controller.hoveredNodeId != null) controller.hoveredNodeId!,
    };
    for (final id in portTargets) {
      if (id == controller.editingNodeId || hidden.contains(id)) continue;
      final n = controller.nodeById[id];
      if (n != null && n.kind != 'frame') {
        _paintAnchorPorts(canvas, n, controller.zoom);
      }
    }

    // 5. Pending connector rubber-band line.
    final pc = controller.pendingConnector;
    if (pc != null) {
      final p = Paint()
        ..color = BoardColors.brand
        ..style = PaintingStyle.stroke
        ..strokeWidth = 2.0 / controller.zoom;
      canvas.drawLine(pc.fromWorld, pc.currentWorld, p);
      _paintArrowhead(
        canvas,
        pc.currentWorld,
        pc.currentWorld - pc.fromWorld,
        BoardColors.brand,
      );
    }

    // 6. Smart alignment guides (subtle soft blue dashed hairline).
    if (controller.activeGuides.isNotEmpty) {
      final gp = Paint()
        ..color = BoardColors.brand.withValues(alpha: 0.65)
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.0 / controller.zoom;
      final dash = 4.5 / controller.zoom;
      final gap = 3.5 / controller.zoom;
      for (final g in controller.activeGuides) {
        final path = Path();
        if (g.vertical) {
          path
            ..moveTo(g.coord, g.start)
            ..lineTo(g.coord, g.end);
        } else {
          path
            ..moveTo(g.start, g.coord)
            ..lineTo(g.end, g.coord);
        }
        _drawDashedPath(canvas, path, gp, dash, gap);
      }
    }

    // 7. Rubber-band marquee selection rectangle.
    final m = controller.marqueeWorldRect;
    if (m != null) {
      canvas.drawRect(
        m,
        Paint()..color = BoardColors.brandMarquee,
      );
      canvas.drawRect(
        m,
        Paint()
          ..color = BoardColors.brand
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1.2 / controller.zoom,
      );
    }

    // 8. Active freehand pen stroke.
    final st = controller.activeStroke;
    if (st != null && st.points.length >= 2) {
      paintBoardStroke(canvas, st);
    }

    canvas.restore();
  }

  @override
  bool shouldRepaint(covariant ActiveBoardPainter oldDelegate) =>
      oldDelegate.activeRev != activeRev || oldDelegate.dark != dark;
}

/// Record the static layer vector display list (`ui.Picture`).
ui.Picture recordStaticBoardPicture(
  BoardController controller, {
  required bool dark,
}) {
  final recorder = ui.PictureRecorder();
  final canvas = Canvas(recorder);
  final hidden = collapsedDescendants(controller.snapshot);
  final dynamicIds = _dynamicNodeIds(controller);

  final visibleNodes = controller.snapshot.nodes
      .where((n) => !hidden.contains(n.id) && !dynamicIds.contains(n.id))
      .toList()
    ..sort((a, b) {
      final af = a.kind == 'frame' ? 0 : 1;
      final bf = b.kind == 'frame' ? 0 : 1;
      if (af != bf) return af.compareTo(bf);
      return a.z.compareTo(b.z);
    });

  // 1. Frames
  for (final n in visibleNodes.where((n) => n.kind == 'frame')) {
    paintBoardNode(canvas, n, controller, dark: dark);
  }

  // 2. Static edges (skip edges attached to dynamic nodes)
  for (final edge in controller.snapshot.edges) {
    if (hidden.contains(edge.fromId) || hidden.contains(edge.toId)) continue;
    if (dynamicIds.contains(edge.fromId) || dynamicIds.contains(edge.toId)) {
      continue;
    }
    paintBoardEdge(canvas, edge, controller, selected: false, dark: dark);
  }

  // 3. Static nodes
  for (final n in visibleNodes.where((n) => n.kind != 'frame')) {
    paintBoardNode(canvas, n, controller, dark: dark);
  }

  // 4. Committed freehand strokes
  for (final st in controller.snapshot.strokes) {
    paintBoardStroke(canvas, st);
  }

  controller.pictureRebuildCount++;
  return recorder.endRecording();
}

Set<String> _dynamicNodeIds(BoardController c) {
  final set = <String>{};
  if (c.isDraggingNodes) set.addAll(c.selectedNodeIds);
  if (c.resizingNodeId != null) set.add(c.resizingNodeId!);
  if (c.editingNodeId != null) set.add(c.editingNodeId!);
  return set;
}

void paintBoardNode(
  Canvas canvas,
  BoardNode n,
  BoardController controller, {
  required bool dark,
}) {
  final fillHex = (dark && n.style.fill.toUpperCase() == '#FFFFFF')
      ? '#252830'
      : n.style.fill;
  final fillColor = parseBoardColor(fillHex, fallback: Colors.white);
  final strokeColor = parseBoardColor(n.style.stroke, fallback: BoardColors.defaultStroke);
  final textHex = (dark &&
          n.style.textColor.toUpperCase() == '#1F2329' &&
          (fillHex == '#252830' || fillHex == 'transparent'))
      ? '#E6E9EF'
      : n.style.textColor;
  final textColor = parseBoardColor(textHex, fallback: BoardColors.inkLight);

  final fillPaint = Paint()..color = fillColor;
  final strokePaint = Paint()
    ..color = strokeColor
    ..style = PaintingStyle.stroke
    ..strokeWidth = n.style.strokeWidth;

  final r = n.rect;
  final cx = r.center.dx;
  final cy = r.center.dy;

  void strokePath(Path path) {
    if (strokeColor.a == 0 || n.style.strokeWidth <= 0) return;
    if (n.style.dashed) {
      _drawDashedPath(canvas, path, strokePaint, 6, 4);
    } else {
      canvas.drawPath(path, strokePaint);
    }
  }

  switch (n.kind) {
    case 'rect':
      final path = Path()..addRect(r);
      if (fillColor.a > 0) canvas.drawPath(path, fillPaint);
      strokePath(path);
    case 'pill':
      final rr = RRect.fromRectAndRadius(r, Radius.circular(math.min(r.width, r.height) / 2));
      final path = Path()..addRRect(rr);
      if (fillColor.a > 0) canvas.drawPath(path, fillPaint);
      strokePath(path);
    case 'ellipse':
      final path = Path()..addOval(r);
      if (fillColor.a > 0) canvas.drawPath(path, fillPaint);
      strokePath(path);
    case 'diamond':
      final path = Path()
        ..moveTo(cx, r.top)
        ..lineTo(r.right, cy)
        ..lineTo(cx, r.bottom)
        ..lineTo(r.left, cy)
        ..close();
      if (fillColor.a > 0) canvas.drawPath(path, fillPaint);
      strokePath(path);
    case 'parallelogram':
      final sk = math.min(24.0, r.width * 0.14);
      final path = Path()
        ..moveTo(r.left + sk, r.top)
        ..lineTo(r.right, r.top)
        ..lineTo(r.right - sk, r.bottom)
        ..lineTo(r.left, r.bottom)
        ..close();
      if (fillColor.a > 0) canvas.drawPath(path, fillPaint);
      strokePath(path);
    case 'hexagon':
      final sk = math.min(24.0, r.width * 0.16);
      final path = Path()
        ..moveTo(r.left + sk, r.top)
        ..lineTo(r.right - sk, r.top)
        ..lineTo(r.right, cy)
        ..lineTo(r.right - sk, r.bottom)
        ..lineTo(r.left + sk, r.bottom)
        ..lineTo(r.left, cy)
        ..close();
      if (fillColor.a > 0) canvas.drawPath(path, fillPaint);
      strokePath(path);
    case 'triangle':
      final path = Path()
        ..moveTo(cx, r.top)
        ..lineTo(r.right, r.bottom)
        ..lineTo(r.left, r.bottom)
        ..close();
      if (fillColor.a > 0) canvas.drawPath(path, fillPaint);
      strokePath(path);
    case 'cylinder':
      final ry = (r.height * 0.16).clamp(6.0, 14.0);
      final topRect = Rect.fromLTWH(r.left, r.top, r.width, ry * 2);
      final botRect = Rect.fromLTWH(r.left, r.bottom - ry * 2, r.width, ry * 2);
      final body = Path()
        ..moveTo(r.left, r.top + ry)
        ..arcTo(topRect, math.pi, math.pi, false)
        ..lineTo(r.right, r.bottom - ry)
        ..arcTo(botRect, 0, math.pi, false)
        ..close();
      if (fillColor.a > 0) canvas.drawPath(body, fillPaint);
      strokePath(body);
      canvas.drawOval(topRect, fillPaint);
      if (strokeColor.a > 0) canvas.drawOval(topRect, strokePaint);
    case 'document':
      final wave = (r.height * 0.14).clamp(6.0, 14.0);
      final by = r.bottom - wave;
      final path = Path()
        ..moveTo(r.left, r.top)
        ..lineTo(r.right, r.top)
        ..lineTo(r.right, by)
        ..quadraticBezierTo(r.left + r.width * 0.75, by - wave, cx, by)
        ..quadraticBezierTo(r.left + r.width * 0.25, by + wave, r.left, by)
        ..close();
      if (fillColor.a > 0) canvas.drawPath(path, fillPaint);
      strokePath(path);
    case 'cloud':
      final path = Path()
        ..moveTo(r.left + r.width * 0.25, r.top + r.height * 0.82)
        ..cubicTo(
          r.left + r.width * 0.04,
          r.top + r.height * 0.82,
          r.left + r.width * 0.04,
          r.top + r.height * 0.45,
          r.left + r.width * 0.22,
          r.top + r.height * 0.42,
        )
        ..cubicTo(
          r.left + r.width * 0.24,
          r.top + r.height * 0.12,
          r.left + r.width * 0.58,
          r.top + r.height * 0.10,
          r.left + r.width * 0.68,
          r.top + r.height * 0.30,
        )
        ..cubicTo(
          r.left + r.width * 0.96,
          r.top + r.height * 0.30,
          r.left + r.width * 0.98,
          r.top + r.height * 0.82,
          r.left + r.width * 0.78,
          r.top + r.height * 0.82,
        )
        ..close();
      if (fillColor.a > 0) canvas.drawPath(path, fillPaint);
      strokePath(path);
    case 'sticky':
      const fold = 14.0;
      final path = Path()
        ..moveTo(r.left, r.top)
        ..lineTo(r.right, r.top)
        ..lineTo(r.right, r.bottom - fold)
        ..lineTo(r.right - fold, r.bottom)
        ..lineTo(r.left, r.bottom)
        ..close();
      if (fillColor.a > 0) canvas.drawPath(path, fillPaint);
      strokePath(path);
      final foldPath = Path()
        ..moveTo(r.right - fold, r.bottom)
        ..lineTo(r.right - fold, r.bottom - fold)
        ..lineTo(r.right, r.bottom - fold);
      canvas.drawPath(foldPath, strokePaint..strokeWidth = 1.0);
    case 'text':
      if (fillColor.a > 0 || strokeColor.a > 0) {
        final rr = RRect.fromRectAndRadius(r, const Radius.circular(4));
        if (fillColor.a > 0) canvas.drawRRect(rr, fillPaint);
        if (strokeColor.a > 0) strokePath(Path()..addRRect(rr));
      }
    case 'frame':
      final rr = RRect.fromRectAndRadius(r, const Radius.circular(10));
      if (fillColor.a > 0) {
        canvas.drawRRect(
          rr,
          Paint()..color = fillColor.withValues(alpha: dark ? 0.18 : 0.7),
        );
      }
      strokePath(Path()..addRRect(rr));
      final tp = controller.getCachedTextPainter(n, overrideColor: textColor);
      tp.paint(canvas, Offset(r.left + 12, r.top + 8));
      return;
    default:
      final rad = n.kind == 'mind_root' ? 10.0 : 8.0;
      final rr = RRect.fromRectAndRadius(r, Radius.circular(rad));
      final path = Path()..addRRect(rr);
      if (fillColor.a > 0) canvas.drawPath(path, fillPaint);
      strokePath(path);
  }

  // Optional tech icon glyph.
  var textTopShift = 0.0;
  if (n.icon != null && n.icon!.isNotEmpty) {
    final iconRect = Rect.fromCenter(
      center: Offset(cx, r.top + 24),
      width: 26,
      height: 26,
    );
    paintBoardTechIcon(
      canvas,
      n.icon!,
      iconRect,
      strokeColor.a > 0 ? strokeColor : BoardColors.brand,
    );
    textTopShift = 15.0;
  }

  if (n.text.isNotEmpty) {
    final tp = controller.getCachedTextPainter(n, overrideColor: textColor);
    final tx = switch (n.style.align) {
      'left' => r.left + BoardController.nodePaddingH,
      'right' => r.right - BoardController.nodePaddingH - tp.width,
      _ => cx - tp.width / 2,
    };
    final ty = (cy + textTopShift - tp.height / 2).clamp(
      r.top + BoardController.nodePaddingV,
      r.bottom - BoardController.nodePaddingV,
    );
    tp.paint(canvas, Offset(tx, ty));
  }
}

void paintBoardEdge(
  Canvas canvas,
  BoardEdge e,
  BoardController controller, {
  required bool selected,
  required bool dark,
}) {
  final from = controller.nodeById[e.fromId];
  final to = controller.nodeById[e.toId];
  if (from == null || to == null) return;

  final color = selected
      ? BoardColors.brand
      : parseBoardColor(e.style.stroke, fallback: BoardColors.mutedLight);
  final paint = Paint()
    ..color = color
    ..style = PaintingStyle.stroke
    ..strokeWidth = selected ? e.style.strokeWidth + 0.8 : e.style.strokeWidth
    ..strokeCap = StrokeCap.round
    ..strokeJoin = StrokeJoin.round;

  final (start, nf, end, nt) = resolveEdgeEndpoints(e, from, to);
  final path = Path();
  Offset mid;
  Offset endDir;
  Offset startDir;

  if (e.routing == 'curved' && e.waypoints.isEmpty) {
    final dist = ((end - start).distance * 0.42).clamp(28.0, 160.0);
    final c1 = start + nf * dist;
    final c2 = end + nt * dist;
    path
      ..moveTo(start.dx, start.dy)
      ..cubicTo(c1.dx, c1.dy, c2.dx, c2.dy, end.dx, end.dy);
    mid = start * 0.125 + c1 * 0.375 + c2 * 0.375 + end * 0.125;
    endDir = end - c2;
    startDir = start - c1;
  } else {
    final pts = controller.edgeDisplayPoints(e);
    if (pts.length < 2) return;
    path.moveTo(pts.first.dx, pts.first.dy);
    for (var i = 1; i < pts.length; i++) {
      path.lineTo(pts[i].dx, pts[i].dy);
    }
    mid = _polylineMidpoint(pts);
    endDir = pts.last - pts[pts.length - 2];
    startDir = pts.first - pts[1];
  }

  if (e.style.dashed) {
    _drawDashedPath(canvas, path, paint, 6, 4);
  } else {
    canvas.drawPath(path, paint);
  }

  if (e.arrowEnd == 'arrow') {
    _paintArrowhead(canvas, end, endDir, color);
  }
  if (e.arrowStart == 'arrow') {
    _paintArrowhead(canvas, start, startDir, color);
  }

  if (e.label.trim().isNotEmpty) {
    final tp = TextPainter(
      text: TextSpan(
        text: e.label.trim(),
        style: TextStyle(
          fontSize: 12,
          color: dark ? BoardColors.inkDark : BoardColors.inkLight,
          fontFamilyFallback: const ['Microsoft YaHei', 'PingFang SC'],
        ),
      ),
      textDirection: TextDirection.ltr,
    )..layout();
    final bg = Rect.fromCenter(
      center: mid,
      width: tp.width + 12,
      height: tp.height + 6,
    );
    final rr = RRect.fromRectAndRadius(bg, const Radius.circular(4));
    canvas.drawRRect(
      rr,
      Paint()..color = dark ? BoardColors.surfaceDark : Colors.white,
    );
    canvas.drawRRect(
      rr,
      Paint()
        ..color = color.withValues(alpha: 0.6)
        ..style = PaintingStyle.stroke
        ..strokeWidth = 0.8,
    );
    tp.paint(canvas, Offset(mid.dx - tp.width / 2, mid.dy - tp.height / 2));
  }
}

void paintBoardStroke(Canvas canvas, BoardStroke st) {
  if (st.points.length < 2) return;
  final path = Path()..moveTo(st.points.first.dx, st.points.first.dy);
  for (var i = 1; i < st.points.length - 1; i++) {
    final p0 = st.points[i];
    final p1 = st.points[i + 1];
    final mid = Offset((p0.dx + p1.dx) / 2, (p0.dy + p1.dy) / 2);
    path.quadraticBezierTo(p0.dx, p0.dy, mid.dx, mid.dy);
  }
  path.lineTo(st.points.last.dx, st.points.last.dy);
  canvas.drawPath(
    path,
    Paint()
      ..color = parseBoardColor(st.color, fallback: BoardColors.brand)
      ..style = PaintingStyle.stroke
      ..strokeWidth = st.width
      ..strokeCap = StrokeCap.round
      ..strokeJoin = StrokeJoin.round,
  );
}

void _paintCollapseBadges(
  Canvas canvas,
  BoardController c,
  Set<String> hidden,
  bool dark,
) {
  final childCount = <String, int>{};
  for (final n in c.snapshot.nodes) {
    final pid = n.parentId;
    if (pid != null) childCount[pid] = (childCount[pid] ?? 0) + 1;
  }
  for (final n in c.snapshot.nodes) {
    if (hidden.contains(n.id)) continue;
    final count = childCount[n.id] ?? 0;
    if (count == 0 || (n.kind != 'mind_root' && n.kind != 'mind_node')) continue;
    final center = Offset(n.x + n.w + 10, n.y + n.h / 2);
    canvas.drawCircle(
      center,
      8,
      Paint()..color = n.collapsed ? BoardColors.brand : (dark ? BoardColors.chipDark : Colors.white),
    );
    canvas.drawCircle(
      center,
      8,
      Paint()
        ..color = BoardColors.brand
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.2,
    );
    final label = n.collapsed ? '$count' : '-';
    final tp = TextPainter(
      text: TextSpan(
        text: label,
        style: TextStyle(
          fontSize: 11,
          fontWeight: FontWeight.w600,
          color: n.collapsed ? Colors.white : BoardColors.brand,
        ),
      ),
      textDirection: TextDirection.ltr,
    )..layout();
    tp.paint(canvas, Offset(center.dx - tp.width / 2, center.dy - tp.height / 2));
  }
}

void _paintResizeHandles(Canvas canvas, BoardNode n, double zoom) {
  final r = n.rect;
  final s = 6.5 / zoom;
  final fill = Paint()..color = Colors.white;
  final stroke = Paint()
    ..color = BoardColors.brand
    ..style = PaintingStyle.stroke
    ..strokeWidth = 1.4 / zoom;
  for (final pt in [
    r.topLeft,
    r.topCenter,
    r.topRight,
    r.centerRight,
    r.bottomRight,
    r.bottomCenter,
    r.bottomLeft,
    r.centerLeft,
  ]) {
    final box = Rect.fromCenter(center: pt, width: s, height: s);
    canvas.drawRect(box, fill);
    canvas.drawRect(box, stroke);
  }
}

void _paintAnchorPorts(Canvas canvas, BoardNode n, double zoom) {
  final rad = 4.2 / zoom;
  final fill = Paint()..color = BoardColors.brand;
  final ring = Paint()
    ..color = Colors.white
    ..style = PaintingStyle.stroke
    ..strokeWidth = 1.5 / zoom;
  for (final side in const ['t', 'r', 'b', 'l']) {
    final pt = nodeAnchorOffset(n, side);
    canvas.drawCircle(pt, rad, fill);
    canvas.drawCircle(pt, rad, ring);
  }
}

void _paintArrowhead(Canvas canvas, Offset tip, Offset dir, Color color) {
  final len = dir.distance;
  if (len < 1e-3) return;
  final u = dir / len;
  const size = 9.0;
  const half = 4.5;
  final base = tip - u * size;
  final perp = Offset(-u.dy, u.dx) * half;
  final path = Path()
    ..moveTo(tip.dx, tip.dy)
    ..lineTo(base.dx + perp.dx, base.dy + perp.dy)
    ..lineTo(base.dx - perp.dx, base.dy - perp.dy)
    ..close();
  canvas.drawPath(path, Paint()..color = color);
}

Offset _polylineMidpoint(List<Offset> pts) {
  var total = 0.0;
  for (var i = 0; i < pts.length - 1; i++) {
    total += (pts[i + 1] - pts[i]).distance;
  }
  if (total < 1e-3) return pts.first;
  final half = total / 2;
  var acc = 0.0;
  for (var i = 0; i < pts.length - 1; i++) {
    final seg = (pts[i + 1] - pts[i]).distance;
    if (acc + seg >= half && seg > 1e-3) {
      return Offset.lerp(pts[i], pts[i + 1], (half - acc) / seg)!;
    }
    acc += seg;
  }
  return pts[pts.length ~/ 2];
}

void _paintDotGrid(
  Canvas canvas,
  Size size,
  double zoom,
  Offset pan,
  bool dark,
) {
  final step = 28.0 * zoom;
  if (step < 10) return;
  final startX = pan.dx % step;
  final startY = pan.dy % step;
  final pts = <Offset>[];
  for (var x = startX; x < size.width; x += step) {
    for (var y = startY; y < size.height; y += step) {
      pts.add(Offset(x, y));
    }
  }
  final paint = Paint()
    ..color = dark ? BoardColors.dotDark : BoardColors.dotLight
    ..strokeWidth = (1.5 * zoom).clamp(1.0, 2.4)
    ..strokeCap = StrokeCap.round;
  canvas.drawPoints(ui.PointMode.points, pts, paint);
}

void _drawDashedPath(
  Canvas canvas,
  Path path,
  Paint paint,
  double dashLen,
  double gapLen,
) {
  for (final metric in path.computeMetrics()) {
    var d = 0.0;
    while (d < metric.length) {
      final end = math.min(d + dashLen, metric.length);
      canvas.drawPath(metric.extractPath(d, end), paint);
      d += dashLen + gapLen;
    }
  }
}
