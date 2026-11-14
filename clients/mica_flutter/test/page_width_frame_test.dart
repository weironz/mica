import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
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

  // Reported: on an ordinary window the slider was already full width at
  // 1520, and the last stops all drew the same column. The pane, not the
  // setting, decides how wide the column can be — and the sidebars decide
  // the pane.
  group('a pane narrower than the stored width', () {
    Future<List<double>> pumpIn(
      WidgetTester tester, {
      required double stored,
      required double pane,
    }) async {
      final commits = <double>[];
      tester.view.physicalSize = const Size(2400, 900);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(
        MaterialApp(
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          locale: const Locale('zh'),
          home: Scaffold(
            body: Center(
              child: SizedBox(
                width: pane,
                child: Center(
                  child: PageWidthFrame(
                    width: stored,
                    onCommit: commits.add,
                    child: const SizedBox(height: 300, width: double.infinity),
                  ),
                ),
              ),
            ),
          ),
        ),
      );
      await tester.pump();
      return commits;
    }

    double drawn(WidgetTester tester) =>
        tester.getSize(find.byType(PageWidthFrame)).width -
        2 * PageWidthFrame.grip;

    testWidgets('an inward drag takes effect at once, from what is drawn', (
      tester,
    ) async {
      final commits = await pumpIn(tester, stored: 1520, pane: 1000);
      expect(drawn(tester), 976);
      expect(PageWidthFrame.room.value, 976, reason: 'room is published');

      final g = await tester.startGesture(edge(tester, right: true));
      await g.moveBy(const Offset(-20, 0));
      await g.moveBy(const Offset(-40, 0));
      await tester.pump();
      // 976 - 2 × 60 = 856, nearest stop 800. Starting from the stored 1520
      // it used to land on 1400 — still wider than the pane, so nothing moved.
      expect(find.text('800 px'), findsOneWidget);
      expect(drawn(tester), 800);
      await g.up();
      await tester.pump();
      expect(commits, [800]);
    });

    testWidgets('dragging out past the pane means full width', (tester) async {
      final commits = await pumpIn(tester, stored: 800, pane: 1000);
      final g = await tester.startGesture(edge(tester, right: true));
      await g.moveBy(const Offset(20, 0));
      await g.moveBy(const Offset(100, 0));
      await tester.pump();
      expect(find.text('全宽 · 976 px'), findsOneWidget);
      await g.up();
      await tester.pump();
      expect(commits, [kPageWidthMax]);
    });

    testWidgets('full width follows the pane as the sidebars open and close', (
      tester,
    ) async {
      await pumpIn(tester, stored: kPageWidthMax, pane: 1000);
      expect(drawn(tester), 976);
      // Sidebars collapsed: the pane grows past every pixel stop.
      await pumpIn(tester, stored: kPageWidthMax, pane: 2300);
      expect(drawn(tester), 2276);
      expect(PageWidthFrame.room.value, 2276);
      // A pixel stop does not follow; it is a cap.
      await pumpIn(tester, stored: 1520, pane: 2300);
      expect(drawn(tester), 1520);
    });

    testWidgets('closing the page clears the published room', (tester) async {
      await pumpIn(tester, stored: 800, pane: 1000);
      expect(PageWidthFrame.room.value, 976);
      await tester.pumpWidget(const SizedBox());
      await tester.pump();
      expect(PageWidthFrame.room.value, isNull);
    });
  });

  testWidgets('the Settings readout says what is drawn when it differs', (
    tester,
  ) async {
    final labels = <String>[];
    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        locale: const Locale('zh'),
        home: Builder(
          builder: (context) {
            labels
              ..clear()
              ..addAll([
                pageWidthLabel(context, 1520),
                pageWidthLabel(context, kPageWidthMax),
                '${pageWidthShownNote(context, 1520, 1378)}',
                '${pageWidthShownNote(context, kPageWidthMax, 1378)}',
                '${pageWidthShownNote(context, 800, 1378)}',
                '${pageWidthShownNote(context, 1520, null)}',
              ]);
            return const SizedBox();
          },
        ),
      ),
    );
    expect(labels, [
      '1520 px',
      '全宽',
      '当前显示 1378 px',
      '当前显示 1378 px',
      'null',
      'null',
    ]);
  });

  test('full width is the stored top stop and exports as the drawn width', () {
    expect(isFullPageWidth(kPageWidthMax), isTrue);
    expect(isFullPageWidth(1760), isFalse);
    expect(pageColumnCap(kPageWidthMax), double.infinity);
    expect(pageColumnCap(1040), 1040);
    expect(pageWidthForExport(1040), 1040);
    PageWidthFrame.room.value = 1378;
    addTearDown(() => PageWidthFrame.room.value = null);
    expect(pageWidthForExport(kPageWidthMax), 1378);
  });

  test('snapping follows the Settings steps', () {
    expect(snapPageWidth(500), kPageWidthMin);
    expect(snapPageWidth(859), 800);
    expect(snapPageWidth(861), 920);
    expect(snapPageWidth(5000), kPageWidthMax);
  });
}
