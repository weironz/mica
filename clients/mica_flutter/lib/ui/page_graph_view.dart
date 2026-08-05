import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../api/models.dart';
import '../l10n/app_localizations.dart';
import 'theme_tokens.dart';

/// The workspace's page-link graph, drawn on one canvas.
///
/// Self-drawn rather than pulled from a graph package, matching the editor: the
/// whole thing is a layout pass plus a `CustomPainter`, and a package would
/// bring its own gesture, theming and layout opinions to override.
///
/// What it deliberately does NOT draw: pages that no link touches. They arrive
/// as [PageGraph.unlinked], a count, because on a real library most pages link
/// to nothing (a production snapshot: 798 documents, 136 with any link) and a
/// field of hundreds of disconnected dots hides the structure this view exists
/// to show. The count is stated on screen, so the omission is visible rather
/// than silent.
class PageGraphView extends StatefulWidget {
  const PageGraphView({
    required this.graph,
    required this.onOpen,
    this.currentViewId,
    super.key,
  });

  final PageGraph graph;

  /// Tapping a node opens that page.
  final void Function(String viewId) onOpen;

  /// Drawn as "you are here" when the graph is opened from a page.
  final String? currentViewId;

  @override
  State<PageGraphView> createState() => _PageGraphViewState();
}

class _PageGraphViewState extends State<PageGraphView> {
  /// Node id → position in the layout's own coordinate space (see [_layout]).
  Map<String, Offset> _positions = const {};
  String? _hovered;

  @override
  void initState() {
    super.initState();
    _positions = _layout(widget.graph);
  }

  @override
  void didUpdateWidget(PageGraphView old) {
    super.didUpdateWidget(old);
    if (!identical(old.graph, widget.graph)) {
      _positions = _layout(widget.graph);
    }
  }

  @override
  Widget build(BuildContext context) {
    final tokens = MicaTheme.of(context);
    final l10n = AppLocalizations.of(context);
    if (widget.graph.isEmpty) {
      return _EmptyGraph(
        unlinked: widget.graph.unlinked,
        tokens: tokens,
        l10n: l10n,
      );
    }
    return Stack(
      children: [
        Positioned.fill(
          child: InteractiveViewer(
            minScale: 0.2,
            maxScale: 4,
            boundaryMargin: const EdgeInsets.all(400),
            child: MouseRegion(
              onHover: (event) => _updateHover(event.localPosition),
              onExit: (_) => _updateHover(null),
              child: GestureDetector(
                behavior: HitTestBehavior.opaque,
                onTapUp: (details) {
                  final hit = _nearest(details.localPosition);
                  if (hit != null) widget.onOpen(hit);
                },
                child: CustomPaint(
                  size: const Size(_canvas, _canvas),
                  painter: _GraphPainter(
                    graph: widget.graph,
                    positions: _positions,
                    tokens: tokens,
                    hovered: _hovered,
                    current: widget.currentViewId,
                    textDirection: Directionality.of(context),
                  ),
                ),
              ),
            ),
          ),
        ),
        if (widget.graph.unlinked > 0)
          Positioned(
            left: 12,
            bottom: 12,
            child: Text(
              l10n.graphUnlinked(widget.graph.unlinked),
              style: TextStyle(fontSize: 12, color: tokens.text.faint),
            ),
          ),
      ],
    );
  }

  void _updateHover(Offset? at) {
    final next = at == null ? null : _nearest(at);
    if (next != _hovered) setState(() => _hovered = next);
  }

  /// The node under [at], or null. Nearest-within-radius rather than exact
  /// circle hit testing: nodes are small, and a tap landing two pixels off a dot
  /// should still open the page it was aimed at.
  String? _nearest(Offset at) {
    String? best;
    var bestDistance = double.infinity;
    for (final entry in _positions.entries) {
      final d = (entry.value - at).distance;
      if (d < bestDistance) {
        bestDistance = d;
        best = entry.key;
      }
    }
    return bestDistance <= _hitRadius ? best : null;
  }
}

/// The layout canvas is a fixed square; [InteractiveViewer] does the panning and
/// zooming, so the layout never has to know the widget's size.
const double _canvas = 2000;
const double _hitRadius = 26;

/// Force-directed placement, run ONCE to a fixed iteration count rather than
/// animated continuously.
///
/// A live simulation looks impressive and costs a frame budget forever; this is
/// a view you open, read, and leave. Running to a fixed count also makes the
/// layout DETERMINISTIC — the same workspace draws the same picture every time,
/// so a page you found once is still where you left it. That is why the seed
/// positions come from the node index on a circle rather than from a random
/// number generator.
///
/// Fruchterman–Reingold: every pair repels, every edge attracts, and a
/// per-iteration temperature caps how far a node may move, so the graph cools
/// into place instead of oscillating.
@visibleForTesting
Map<String, Offset> layoutPageGraph(PageGraph graph) => _layout(graph);

Map<String, Offset> _layout(PageGraph graph) {
  final nodes = graph.nodes;
  if (nodes.isEmpty) return const {};

  final index = {for (var i = 0; i < nodes.length; i++) nodes[i].viewId: i};
  const centre = Offset(_canvas / 2, _canvas / 2);
  // Deterministic seeding: evenly spaced on a circle, in the order the data
  // arrived (degree first), so hubs start apart rather than stacked.
  final pos = <Offset>[
    for (var i = 0; i < nodes.length; i++)
      centre +
          Offset(
                math.cos(2 * math.pi * i / nodes.length),
                math.sin(2 * math.pi * i / nodes.length),
              ) *
              (_canvas * 0.35),
  ];

  // The ideal edge length for this many nodes — the classic k = C·sqrt(area/n).
  final k = 0.9 * math.sqrt(_canvas * _canvas / nodes.length);
  // Fewer passes on a big graph: cost is O(iterations · n²), and past a few
  // hundred nodes the picture is about clusters, not exact positions.
  final iterations = nodes.length > 300 ? 120 : 300;
  var temperature = _canvas / 10;
  final cooling = temperature / (iterations + 1);

  final disp = List<Offset>.filled(nodes.length, Offset.zero);
  for (var pass = 0; pass < iterations; pass++) {
    for (var i = 0; i < disp.length; i++) {
      disp[i] = Offset.zero;
    }
    // Repulsion: every pair pushes apart.
    for (var i = 0; i < nodes.length; i++) {
      for (var j = i + 1; j < nodes.length; j++) {
        var delta = pos[i] - pos[j];
        var distance = delta.distance;
        if (distance < 0.01) {
          // Two nodes exactly on top of each other have no direction to
          // separate along. Nudge by index, so the result stays reproducible.
          delta = Offset(0.01 * (i + 1), 0.01 * (j + 1));
          distance = delta.distance;
        }
        final push = delta / distance * ((k * k) / distance);
        disp[i] += push;
        disp[j] -= push;
      }
    }
    // Attraction: linked pages pull together.
    for (final edge in graph.edges) {
      final a = index[edge.source];
      final b = index[edge.target];
      if (a == null || b == null || a == b) continue;
      final delta = pos[a] - pos[b];
      final distance = delta.distance;
      if (distance < 0.01) continue;
      final pull = delta / distance * ((distance * distance) / k);
      disp[a] -= pull;
      disp[b] += pull;
    }
    // Move, capped by the temperature, and keep everything on the canvas.
    for (var i = 0; i < nodes.length; i++) {
      final d = disp[i];
      final distance = d.distance;
      if (distance < 0.01) continue;
      final move = d / distance * math.min(distance, temperature);
      pos[i] = Offset(
        (pos[i].dx + move.dx).clamp(40.0, _canvas - 40),
        (pos[i].dy + move.dy).clamp(40.0, _canvas - 40),
      );
    }
    temperature -= cooling;
  }

  return {for (var i = 0; i < nodes.length; i++) nodes[i].viewId: pos[i]};
}

class _GraphPainter extends CustomPainter {
  _GraphPainter({
    required this.graph,
    required this.positions,
    required this.tokens,
    required this.hovered,
    required this.current,
    required this.textDirection,
  });

  final PageGraph graph;
  final Map<String, Offset> positions;
  final MicaTokens tokens;
  final String? hovered;
  final String? current;
  final TextDirection textDirection;

  /// How many labels to draw at most. Labelling every node turns a dense graph
  /// into a wall of overlapping text; the nodes arrive sorted by degree, so the
  /// first N are the ones worth naming.
  static const int _maxLabels = 30;

  @override
  void paint(Canvas canvas, Size size) {
    final edgePaint = Paint()
      ..color = tokens.border.subtle
      ..strokeWidth = 1
      ..style = PaintingStyle.stroke;
    final litPaint = Paint()
      ..color = tokens.accent.primary
      ..strokeWidth = 1.6
      ..style = PaintingStyle.stroke;

    for (final edge in graph.edges) {
      final a = positions[edge.source];
      final b = positions[edge.target];
      if (a == null || b == null) continue;
      final lit =
          hovered != null && (edge.source == hovered || edge.target == hovered);
      canvas.drawLine(a, b, lit ? litPaint : edgePaint);
    }

    for (var i = 0; i < graph.nodes.length; i++) {
      final node = graph.nodes[i];
      final at = positions[node.viewId];
      if (at == null) continue;
      // Radius grows with sqrt(degree) so AREA tracks degree: doubling the
      // radius quadruples the ink, which reads as far more than "twice as
      // linked".
      final radius = math.min(4 + math.sqrt(node.degree) * 3.0, 22.0);
      final isCurrent = node.viewId == current;
      final isHovered = node.viewId == hovered;
      canvas.drawCircle(
        at,
        radius,
        Paint()
          ..color = isCurrent || isHovered
              ? tokens.accent.primary
              : tokens.text.muted,
      );
      if (isCurrent) {
        canvas.drawCircle(
          at,
          radius + 4,
          Paint()
            ..color = tokens.accent.primary
            ..strokeWidth = 1.5
            ..style = PaintingStyle.stroke,
        );
      }
      if (i < _maxLabels || isHovered || isCurrent) {
        _label(canvas, node.name, at, radius);
      }
    }
  }

  void _label(Canvas canvas, String name, Offset at, double radius) {
    if (name.isEmpty) return;
    final painter = TextPainter(
      text: TextSpan(
        text: name,
        style: TextStyle(fontSize: 12, color: tokens.text.primary),
      ),
      textDirection: textDirection,
      maxLines: 1,
      ellipsis: '…',
    )..layout(maxWidth: 140);
    painter.paint(canvas, Offset(at.dx - painter.width / 2, at.dy + radius + 4));
  }

  @override
  bool shouldRepaint(_GraphPainter old) =>
      old.graph != graph ||
      old.positions != positions ||
      old.hovered != hovered ||
      old.current != current ||
      old.tokens != tokens;
}

/// Nothing to draw. Says WHY rather than showing a blank canvas: a workspace
/// with no links yet is the normal starting state, not a failure, and `[[` is
/// the one next step worth naming.
class _EmptyGraph extends StatelessWidget {
  const _EmptyGraph({
    required this.unlinked,
    required this.tokens,
    required this.l10n,
  });

  final int unlinked;
  final MicaTokens tokens;
  final AppLocalizations l10n;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(32),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(Icons.hub_outlined, size: 40, color: tokens.text.faint),
            const SizedBox(height: 12),
            Text(
              unlinked > 0
                  ? l10n.graphEmptyNoLinks(unlinked)
                  : l10n.graphEmptyNoPages,
              style: TextStyle(color: tokens.text.muted),
              textAlign: TextAlign.center,
            ),
            const SizedBox(height: 6),
            Text(
              l10n.graphEmptyHint,
              style: TextStyle(color: tokens.text.faint, fontSize: 12),
              textAlign: TextAlign.center,
            ),
          ],
        ),
      ),
    );
  }
}
