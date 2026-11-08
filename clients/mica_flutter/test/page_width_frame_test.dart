import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/page_width_frame.dart';

// gh#164: drag the page column's edge to change the page width. The column is
// centred, so either edge dragged OUTWARD widens it by twice the distance;
// the result snaps to the Settings slider's 120 px steps and is committed
// once, on release.
void main() {
  Future<List<double>> pump(WidgetTester tester, double width) async {
    final commits = <double>[];
    tester.view.physicalSize = const Size(2400, 900);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: Center(
            child: PageWidthFrame(
              width: width,
              onCommit: commits.add,
              child: const SizedBox(height: 300, width: double.infinity),
            ),
          ),
        ),
      ),
    );
    return commits;
  }

  Offset edge(WidgetTester tester, {required bool right}) {
    final box = tester.getRect(find.byType(PageWidthFrame));
    return Offset(right ? box.right - 6 : box.left + 6, box.center.dy);
  }

  testWidgets('right edge outward: +60 px each side lands on the next step', (
    tester,
  ) async {
    final commits = await pump(tester, 800);
    final g = await tester.startGesture(edge(tester, right: true));
    await g.moveBy(const Offset(20, 0));
    await g.moveBy(const Offset(40, 0));
    await tester.pump();
    expect(find.text('920 px'), findsOneWidget, reason: 'live readout');
    expect(commits, isEmpty, reason: 'nothing saved mid-drag');
    await g.up();
    await tester.pump();
    expect(commits, [920]);
    expect(find.byKey(const ValueKey('page-width-readout')), findsNothing);
  });

  testWidgets('left edge outward widens too; clamped to the maximum', (
    tester,
  ) async {
    final commits = await pump(tester, 1760);
    final g = await tester.startGesture(edge(tester, right: false));
    await g.moveBy(const Offset(-20, 0));
    await g.moveBy(const Offset(-400, 0));
    await g.up();
    await tester.pump();
    expect(commits, [kPageWidthMax]);
  });

  testWidgets('a nudge smaller than half a step changes nothing', (
    tester,
  ) async {
    final commits = await pump(tester, 800);
    final g = await tester.startGesture(edge(tester, right: true));
    await g.moveBy(const Offset(20, 0));
    await g.moveBy(const Offset(5, 0));
    await g.up();
    await tester.pump();
    expect(commits, isEmpty);
  });

  test('snapping follows the Settings steps', () {
    expect(snapPageWidth(500), kPageWidthMin);
    expect(snapPageWidth(859), 800);
    expect(snapPageWidth(861), 920);
    expect(snapPageWidth(5000), kPageWidthMax);
  });
}
