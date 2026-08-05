// The graph view's two claims that can rot silently: the layout is
// DETERMINISTIC (a page you found once is still where you left it), and pages
// nobody links to are COUNTED rather than drawn (otherwise a real workspace
// renders as a field of disconnected dots and the structure disappears).
//
// Neither shows up as a crash. Both show up as "the graph looks wrong", which
// is exactly the kind of thing nobody files a bug about.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/ui/page_graph_view.dart';

PageGraph _graph({
  required List<(String, int)> nodes,
  required List<(String, String)> edges,
  int unlinked = 0,
}) {
  return PageGraph(
    nodes: [
      for (final (id, degree) in nodes)
        GraphNode(viewId: id, name: 'page $id', degree: degree),
    ],
    edges: [for (final (a, b) in edges) GraphEdge(source: a, target: b)],
    unlinked: unlinked,
  );
}

Widget _host(Widget child) => MaterialApp(
  localizationsDelegates: AppLocalizations.localizationsDelegates,
  supportedLocales: AppLocalizations.supportedLocales,
  home: Scaffold(body: child),
);

void main() {
  group('layout', () {
    test('the same graph lays out identically every time', () {
      final g = _graph(
        nodes: [('a', 2), ('b', 1), ('c', 1)],
        edges: [('a', 'b'), ('a', 'c')],
      );
      final first = layoutPageGraph(g);
      final second = layoutPageGraph(g);
      expect(first.keys.toSet(), second.keys.toSet());
      for (final id in first.keys) {
        expect(first[id], second[id], reason: 'node $id moved between runs');
      }
    });

    test('every node lands on the canvas', () {
      final g = _graph(
        nodes: [for (var i = 0; i < 40; i++) ('n$i', 1)],
        edges: [for (var i = 1; i < 40; i++) ('n0', 'n$i')],
      );
      for (final at in layoutPageGraph(g).values) {
        expect(at.dx, inInclusiveRange(0, 2000));
        expect(at.dy, inInclusiveRange(0, 2000));
      }
    });

    test('linked pages end up closer than unlinked ones', () {
      // Two tight triangles with no edge between them. If the forces were wired
      // backwards this still produces a picture — just a meaningless one.
      final g = _graph(
        nodes: [
          ('a1', 2),
          ('a2', 2),
          ('a3', 2),
          ('b1', 2),
          ('b2', 2),
          ('b3', 2),
        ],
        edges: [
          ('a1', 'a2'),
          ('a2', 'a3'),
          ('a3', 'a1'),
          ('b1', 'b2'),
          ('b2', 'b3'),
          ('b3', 'b1'),
        ],
      );
      final pos = layoutPageGraph(g);
      final withinA = (pos['a1']! - pos['a2']!).distance;
      final across = (pos['a1']! - pos['b1']!).distance;
      expect(
        withinA,
        lessThan(across),
        reason: 'a linked pair must sit closer than an unlinked one',
      );
    });

    test('an empty graph lays out to nothing rather than throwing', () {
      expect(layoutPageGraph(PageGraph.empty), isEmpty);
    });
  });

  group('what it draws', () {
    testWidgets('an empty graph explains itself instead of showing a blank', (
      tester,
    ) async {
      await tester.pumpWidget(
        _host(
          PageGraphView(
            graph: const PageGraph(nodes: [], edges: [], unlinked: 12),
            onOpen: (_) {},
          ),
        ),
      );
      // States the count AND the next step — a blank canvas would read as a
      // broken view rather than as "you have not linked anything yet".
      expect(find.textContaining('12'), findsOneWidget);
      expect(find.textContaining('[['), findsOneWidget);
    });

    testWidgets('unlinked pages are reported, not drawn', (tester) async {
      await tester.pumpWidget(
        _host(
          PageGraphView(
            graph: _graph(
              nodes: [('a', 1), ('b', 1)],
              edges: [('a', 'b')],
              unlinked: 662,
            ),
            onOpen: (_) {},
          ),
        ),
      );
      // The number is on screen: the omission has to be visible, or the view
      // quietly misrepresents how much of the workspace it is showing.
      expect(find.textContaining('662'), findsOneWidget);
    });

    testWidgets('tapping a node opens that page', (tester) async {
      // The layout canvas is 2000² and nodes settle near its middle, so the
      // default 800×600 test viewport puts every node off-screen and the tap
      // lands on nothing — which looks exactly like "hit testing is broken".
      tester.view.physicalSize = const Size(2100, 2100);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);

      final opened = <String>[];
      final graph = _graph(nodes: [('a', 1), ('b', 1)], edges: [('a', 'b')]);
      await tester.pumpWidget(
        _host(PageGraphView(graph: graph, onOpen: opened.add)),
      );

      // Tap where the layout actually put node 'a', translated into the painted
      // canvas's own coordinate space.
      final at = layoutPageGraph(graph)['a']!;
      final canvas = tester.getTopLeft(find.byType(CustomPaint).first);
      await tester.tapAt(canvas + at);
      await tester.pump();

      expect(opened, ['a']);
    });
  });
}
