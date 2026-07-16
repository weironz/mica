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
}
