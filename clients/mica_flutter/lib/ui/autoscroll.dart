/// Edge auto-scroll velocity for a drag inside a scrollable.
///
/// Given the pointer's vertical position [dy] within a viewport of
/// [viewportHeight], returns pixels-per-tick to scroll: negative near the top
/// (scroll up, reveal earlier content), positive near the bottom, zero in the
/// slack middle. Speed ramps linearly with how far into the [zone] the pointer
/// has pushed, so a small overshoot creeps and a hard push races.
///
/// Pure and framework-free so the direction/zone/clamp rules can be tested
/// without a live drag; the widget wires it to a ticking timer + ScrollController.
double edgeAutoScrollVelocity(
  double dy,
  double viewportHeight, {
  double zone = 60,
  double maxStep = 14,
}) {
  // A viewport shorter than two zones would make the top and bottom bands
  // overlap; clamp the effective zone so the midpoint stays neutral.
  final effectiveZone = zone.clamp(0.0, viewportHeight / 2);
  if (effectiveZone <= 0) {
    return 0;
  }
  if (dy < effectiveZone) {
    final depth = ((effectiveZone - dy) / effectiveZone).clamp(0.0, 1.0);
    return -maxStep * depth;
  }
  final bottomEdge = viewportHeight - effectiveZone;
  if (dy > bottomEdge) {
    final depth = ((dy - bottomEdge) / effectiveZone).clamp(0.0, 1.0);
    return maxStep * depth;
  }
  return 0;
}
