import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/status_kit.dart';

/// The import job reported `done`/`total` all along and the client parsed both,
/// then threw them away — a 96-page import showed one indeterminate spinner for
/// minutes, indistinguishable from a wedged job.
void main() {
  Future<void> pumpRow(
    WidgetTester tester, {
    required int done,
    required int total,
  }) {
    return tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: MicaProgressRow(
            label: '正在导入 $done / $total',
            done: done,
            total: total,
          ),
        ),
      ),
    );
  }

  LinearProgressIndicator bar(WidgetTester tester) => tester
      .widget<LinearProgressIndicator>(find.byType(LinearProgressIndicator));

  testWidgets('reports a real fraction, not an indeterminate spinner', (
    tester,
  ) async {
    await pumpRow(tester, done: 42, total: 96);

    // A non-null value is what makes the bar determinate; null renders the same
    // animation for "moving" and for "stuck".
    expect(bar(tester).value, closeTo(42 / 96, 1e-9));
    expect(find.text('正在导入 42 / 96'), findsOneWidget);
  });

  testWidgets('a just-started job reads as empty, not as complete', (
    tester,
  ) async {
    await pumpRow(tester, done: 0, total: 96);

    expect(bar(tester).value, 0.0);
  });

  testWidgets('overshoot clamps to full instead of overflowing the track', (
    tester,
  ) async {
    // Reachable: the planner counts pages while the worker can emit extra units.
    // A bar past its end reads as a rendering bug, not as progress.
    await pumpRow(tester, done: 120, total: 96);

    expect(bar(tester).value, 1.0);
  });

  test('a total of zero is refused rather than silently indeterminate', () {
    // Callers with no real total must render nothing at all; falling back to an
    // indeterminate bar would put back exactly the ambiguity this replaced.
    expect(
      () => MicaProgressRow(label: 'x', done: 0, total: 0),
      throwsA(isA<AssertionError>()),
    );
  });
}
