import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/autoscroll.dart';

// A long sidebar tree could not be reordered past what fit on screen: a drag
// that reached the bottom edge stalled, so you could never drop below the last
// visible row. Auto-scroll fixes that; this pins the velocity RULES — direction
// and ramp — because a sign flip (scrolls the wrong way) is the classic bug.
void main() {
  const height = 600.0;
  const zone = 60.0;
  const maxStep = 14.0;

  test('the slack middle does not scroll', () {
    expect(edgeAutoScrollVelocity(300, height, zone: zone, maxStep: maxStep), 0);
    expect(edgeAutoScrollVelocity(zone + 1, height, zone: zone, maxStep: maxStep), 0);
    expect(edgeAutoScrollVelocity(height - zone - 1, height, zone: zone, maxStep: maxStep), 0);
  });

  test('near the top scrolls UP (negative), near the bottom DOWN (positive)', () {
    expect(edgeAutoScrollVelocity(5, height, zone: zone, maxStep: maxStep), lessThan(0));
    expect(
      edgeAutoScrollVelocity(height - 5, height, zone: zone, maxStep: maxStep),
      greaterThan(0),
    );
  });

  test('speed ramps with depth into the edge and never exceeds maxStep', () {
    final shallow = edgeAutoScrollVelocity(zone - 10, height, zone: zone, maxStep: maxStep);
    final deep = edgeAutoScrollVelocity(2, height, zone: zone, maxStep: maxStep);
    expect(deep.abs(), greaterThan(shallow.abs()), reason: 'deeper = faster');

    // At the very edge it reaches full speed; past it, clamped, never more.
    expect(edgeAutoScrollVelocity(0, height, zone: zone, maxStep: maxStep), -maxStep);
    expect(edgeAutoScrollVelocity(height, height, zone: zone, maxStep: maxStep), maxStep);
    expect(edgeAutoScrollVelocity(-50, height, zone: zone, maxStep: maxStep), -maxStep);
  });

  test('a viewport shorter than two zones keeps a neutral midpoint', () {
    // height 80, zone 60 would overlap the bands; the midpoint must still be 0
    // rather than pulling in both directions.
    expect(edgeAutoScrollVelocity(40, 80, zone: zone, maxStep: maxStep), 0);
    // and the extremes still resolve to a single, correct direction.
    expect(edgeAutoScrollVelocity(2, 80, zone: zone, maxStep: maxStep), lessThan(0));
    expect(edgeAutoScrollVelocity(78, 80, zone: zone, maxStep: maxStep), greaterThan(0));
  });

  /// The rules above were right and the sidebar still ran away, because the
  /// thing feeding them stopped talking.
  ///
  /// `Draggable.onDragUpdate` is mounted-guarded (Flutter's own doc: "will only
  /// be called while this widget is still mounted"). Auto-scrolling the list
  /// carries the GRABBED row out of the viewport, the list recycles it, the
  /// callback goes silent — and the scroll kept running on the last position it
  /// had seen, ignoring the pointer until the button came up.
  ///
  /// So the assertion that matters is not "does it scroll" but "does it STOP
  /// once the row that started the drag is gone". The earlier fix could not be
  /// tested through the callback, was shipped unverified, and did not work.
  group('DragAutoScrollRegion keeps following the pointer after the dragged '
      'row unmounts', () {
    const rowHeight = 60.0;
    const viewportHeight = 600.0;

    /// A list tall enough that auto-scrolling it recycles the first row.
    Widget harness(ScrollController controller) {
      var dragging = false;
      return MaterialApp(
        home: Scaffold(
          body: Center(
            child: SizedBox(
              height: viewportHeight,
              width: 300,
              child: StatefulBuilder(
                builder: (context, setState) => DragAutoScrollRegion(
                  enabled: dragging,
                  controller: controller,
                  child: ListView.builder(
                    controller: controller,
                    itemCount: 100,
                    itemExtent: rowHeight,
                    itemBuilder: (context, i) => Draggable<int>(
                      data: i,
                      dragAnchorStrategy: pointerDragAnchorStrategy,
                      onDragStarted: () => setState(() => dragging = true),
                      onDragEnd: (_) => setState(() => dragging = false),
                      onDraggableCanceled: (_, _) =>
                          setState(() => dragging = false),
                      feedback: const SizedBox(width: 100, height: 20),
                      child: SizedBox(
                        key: ValueKey('row$i'),
                        height: rowHeight,
                        child: Text('row $i'),
                      ),
                    ),
                  ),
                ),
              ),
            ),
          ),
        ),
      );
    }

    testWidgets('the pointer leaving the edge zone stops it', (tester) async {
      final controller = ScrollController();
      addTearDown(controller.dispose);
      await tester.pumpWidget(harness(controller));

      // Grab the first row and drag to the bottom edge.
      final start = tester.getCenter(find.byKey(const ValueKey('row0')));
      final gesture = await tester.startGesture(start);
      final viewport = tester.getRect(find.byType(DragAutoScrollRegion));
      await gesture.moveTo(Offset(start.dx, viewport.bottom - 5));
      await tester.pump();

      for (var i = 0; i < 40; i++) {
        await tester.pump(const Duration(milliseconds: 16));
      }
      final afterEdge = controller.offset;
      expect(afterEdge, greaterThan(0), reason: 'the edge must scroll at all');

      // THE PRECONDITION: the row that started the drag is off-screen and has
      // been recycled. Without this the test would pass on the broken code too,
      // because the callback only goes silent once the row is gone.
      expect(
        find.byKey(const ValueKey('row0')),
        findsNothing,
        reason: 'the dragged row must have unmounted for this to be the bug',
      );

      // Now move back to the middle. This is the gesture the user reported as
      // having no effect.
      await gesture.moveTo(viewport.center);
      await tester.pump();
      for (var i = 0; i < 40; i++) {
        await tester.pump(const Duration(milliseconds: 16));
      }
      expect(
        controller.offset,
        afterEdge,
        reason: 'resting in the slack middle must hold the list still',
      );

      await gesture.up();
      await tester.pumpAndSettle();
    });

    testWidgets('coming back to the edge scrolls again', (tester) async {
      // The stop must not be a latch: after resting in the middle, pushing back
      // into the zone has to resume, or "it stopped" would just be a new way of
      // being stuck.
      final controller = ScrollController();
      addTearDown(controller.dispose);
      await tester.pumpWidget(harness(controller));

      final start = tester.getCenter(find.byKey(const ValueKey('row0')));
      final gesture = await tester.startGesture(start);
      final viewport = tester.getRect(find.byType(DragAutoScrollRegion));
      await gesture.moveTo(Offset(start.dx, viewport.bottom - 5));
      await tester.pump();
      for (var i = 0; i < 20; i++) {
        await tester.pump(const Duration(milliseconds: 16));
      }

      await gesture.moveTo(viewport.center);
      await tester.pump();
      for (var i = 0; i < 10; i++) {
        await tester.pump(const Duration(milliseconds: 16));
      }
      final parked = controller.offset;

      await gesture.moveTo(Offset(start.dx, viewport.bottom - 5));
      await tester.pump();
      for (var i = 0; i < 20; i++) {
        await tester.pump(const Duration(milliseconds: 16));
      }
      expect(controller.offset, greaterThan(parked));

      await gesture.up();
      await tester.pumpAndSettle();
    });

    testWidgets('a press-and-move with no drag never scrolls', (tester) async {
      // `enabled` is the only thing separating this from a scrollbar drag or a
      // stray press: pointer moves over the region must be inert until a drag
      // actually starts.
      final controller = ScrollController();
      addTearDown(controller.dispose);
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: SizedBox(
              height: viewportHeight,
              child: DragAutoScrollRegion(
                enabled: false,
                controller: controller,
                child: ListView.builder(
                  controller: controller,
                  itemCount: 100,
                  itemExtent: rowHeight,
                  itemBuilder: (context, i) => SizedBox(height: rowHeight),
                ),
              ),
            ),
          ),
        ),
      );
      final viewport = tester.getRect(find.byType(DragAutoScrollRegion));
      final gesture = await tester.startGesture(viewport.center);
      await gesture.moveTo(Offset(viewport.center.dx, viewport.bottom - 5));
      for (var i = 0; i < 40; i++) {
        await tester.pump(const Duration(milliseconds: 16));
      }
      expect(controller.offset, 0);
      await gesture.up();
      await tester.pumpAndSettle();
    });
  });
}
