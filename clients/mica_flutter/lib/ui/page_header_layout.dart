// How the page header row splits its width between the path, the optional
// format bar and the fixed tool icons. Pure arithmetic, kept beside nothing
// else so it can be reasoned about (and tested) without building the pane.

import 'dart:math' as math;

/// Narrowest share of the header the format bar may be squeezed to.
///
/// MEASURED, not chosen: the full strip is 19 buttons over 606px. 360 keeps
/// roughly the first eleven — undo/redo, the three heading levels, bold /
/// italic / strike, code, link — visible without scrolling, which is the set
/// you reach for without looking. Past that the bar scrolls, which it has
/// always been able to do (it is a horizontal SingleChildScrollView).
const double kFormatBarFloor = 360;

/// The right-hand cluster (side-panel toggle, comments, properties, page
/// menu) shares this row too, and is not flexible. Reserved alongside the
/// bar's floor so the floor is a floor on what the BAR gets, not on what is
/// left over before the icons take their cut.
const double kPageHeaderIconsWidth = 160;

/// Whether the format bar sits in the header row. Below 720 it moves under
/// the header instead, so the row never has to share a narrow pane.
bool pageFormatBarInHeader({
  required bool enabled,
  required bool canEdit,
  required double width,
}) => enabled && canEdit && width >= 720;

/// How wide the breadcrumb may get while it shares the row with the format
/// bar. (With the bar off it simply takes the whole row.)
///
/// "Path first" — it takes what it needs — under two
/// bounds, and each one is the answer to a question that was actually asked:
///
///  * A ceiling at 70% of the header, so a pathological title cannot span the
///    whole thing. Real names and real nesting stay well under it; the one
///    that reached it was a hand-typed `asdfsadfasfdasfds…`.
///  * A reserve for the bar: the path may not eat the last [kFormatBarFloor]
///    + [kPageHeaderIconsWidth] of the row. That is a bound on what the PATH
///    may take — not a promise about the bar's rendered width, which
///    `_formatBar` narrows again with its own `Center(maxWidth: pageWidth)`.
///    Measured on a 1050-wide window the bar ends up around 317px and shows
///    ten buttons before it scrolls.
///
/// The 200 floor at the end is for a genuinely narrow pane, where the two
/// bounds cross and something has to give; there the path keeps enough to
/// show the page name and the bar scrolls from the start.
///
/// Measured, bar on, this page's path needing 221px:
///
///   window 1600 -> path 221, bar 606 (everything fits, no scroll)
///   window 1400 -> path 221, bar 562
///   window 1280 -> path 221, bar 442
///   window 1050 -> path yields, bar 317
///
/// The path holding its natural 221 rather than its whole budget is the point
/// of `MainAxisSize.min` on PageBreadcrumb's Row — a cap is not a demand.
double pagePathBudget(double header) {
  return math.max(
    math.min(header * 0.7, header - kFormatBarFloor - kPageHeaderIconsWidth),
    200.0,
  );
}
