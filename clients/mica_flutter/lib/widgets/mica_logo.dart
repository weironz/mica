import 'package:flutter/material.dart';

/// The Mica mark: an isometric cube drawn as a wireframe, with graph nodes on
/// its vertices — the knowledge graph held inside a crystal.
///
/// Replaces the three stacked rhombus sheets (2026-08-28). The mineral it was
/// named for is still the idea; what changed is that the mark now also says
/// what the product DOES — nodes and edges, not just layers.
///
/// Drawn in-house, no asset: the geometry is a regular hexagon (the cube's
/// silhouette in isometric projection) plus three spokes from the centre to
/// alternating vertices (the three edges facing the viewer). That is the whole
/// figure — which is why it survives being 16px in a system tray, where the
/// glowing-crystal artwork this simplifies would smear into a blue blob.
///
/// MIRRORED by `web/favicon.svg` and by the app/tray `.ico`s (see
/// `scripts/gen-icons.py`). Change the geometry here and those must follow —
/// the release gate refuses when the two icons drift apart.
class MicaLogo extends StatelessWidget {
  const MicaLogo({this.size = 24, this.color, super.key});

  final double size;

  /// Overrides the gradient with a flat colour — for the tray/monochrome
  /// contexts where a gradient reads as mud.
  final Color? color;

  @override
  Widget build(BuildContext context) {
    return SizedBox.square(
      dimension: size,
      child: CustomPaint(painter: MicaLogoPainter(flat: color)),
    );
  }
}

/// Public so the icon generator and golden tests can rasterise the same
/// painter the app uses, instead of a second copy of the geometry.
class MicaLogoPainter extends CustomPainter {
  const MicaLogoPainter({this.flat});

  final Color? flat;

  /// Deep blue → violet, the A1 artwork's axis.
  static const _from = Color(0xFF2563EB);
  static const _to = Color(0xFF7C3AED);

  /// The six silhouette vertices, at radius 1 around the origin, starting at
  /// the top and going clockwise. Pointy-top: vertex 0 is straight up, which
  /// is what makes the three spokes below land on the cube's near edges.
  static const _hex = <Offset>[
    Offset(0, -1),
    Offset(0.866, -0.5),
    Offset(0.866, 0.5),
    Offset(0, 1),
    Offset(-0.866, 0.5),
    Offset(-0.866, -0.5),
  ];

  /// Alternating vertices — the three cube edges that meet at the near corner.
  static const _spokes = <int>[0, 2, 4];

  @override
  void paint(Canvas canvas, Size size) {
    final s = size.shortestSide;
    final big = s >= 22;
    // Stroke and node dots stick OUT past the silhouette, so the radius is what
    // is left after both — the hexagon is pointy-top, which makes the vertical
    // the binding dimension (extent = 2r, versus 1.73r across).
    //
    // First cut used a flat 0.42 and clipped the bottom vertex off every size
    // that draws corner nodes: 0.42 + 0.0375 (half stroke) + 0.085 (node) =
    // 0.5425 of a half-box that only has 0.5. Only the small variant survived,
    // because it has no corner nodes — which is exactly why it has to be
    // computed per variant rather than guessed once.
    final r = s * (big ? 0.375 : 0.44);
    final c = Offset(size.width / 2, size.height / 2);
    Offset at(Offset u) => c + Offset(u.dx * r, u.dy * r);

    final stroke = Paint()
      ..style = PaintingStyle.stroke
      // Relatively heavier when small: a hairline that is optically correct at
      // 128px disappears into the antialiasing at 16px.
      ..strokeWidth = s * (big ? 0.075 : 0.095)
      ..strokeJoin = StrokeJoin.round
      ..strokeCap = StrokeCap.round
      ..isAntiAlias = true;
    final fill = Paint()..isAntiAlias = true;
    if (flat != null) {
      stroke.color = flat!;
      fill.color = flat!;
    } else {
      final shader = const LinearGradient(
        begin: Alignment.topLeft,
        end: Alignment.bottomRight,
        colors: [_from, _to],
      ).createShader(Offset.zero & size);
      stroke.shader = shader;
      fill.shader = shader;
    }

    final outline = Path()..addPolygon([for (final v in _hex) at(v)], true);
    canvas.drawPath(outline, stroke);
    for (final i in _spokes) {
      canvas.drawLine(c, at(_hex[i]), stroke);
    }

    // Nodes: the six corners plus the near one — but only while there is room
    // for them. An OPTICAL SIZE, not a scale: six dots sized proportionally sit
    // on a 13px-wide hexagon at 16px and close the figure into a blob (seen in
    // the first render of this mark). Below the threshold the cube keeps its
    // wireframe and only the centre node survives, which is the part that says
    // "graph" — and stays legible in a 16px tray slot on either theme.
    if (big) {
      final node = s * 0.085;
      for (final v in _hex) {
        canvas.drawCircle(at(v), node, fill);
      }
      canvas.drawCircle(c, node * 1.15, fill);
    } else {
      canvas.drawCircle(c, s * 0.1, fill);
    }
  }

  @override
  bool shouldRepaint(covariant MicaLogoPainter old) => old.flat != flat;
}
