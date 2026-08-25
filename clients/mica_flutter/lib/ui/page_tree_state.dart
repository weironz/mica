import 'package:flutter/material.dart';

import 'theme_tokens.dart';

/// Which of the four things the sidebar's page tree shows.
///
/// Pulled out of `_pageTree` so the choice can be tested without building the
/// sidebar, which lives inside a widget taking 97 required parameters. That
/// widget was long treated as the blocker for testing this; it is not. The
/// difference that matters here is one boolean, and it is the kind that goes
/// wrong silently.
enum PageTreeState {
  /// No workspace picked yet — nothing to say about pages.
  noWorkspace,

  /// The tree is still in flight and there is nothing to draw. Placeholder rows.
  skeleton,

  /// The tree arrived and the workspace really is empty.
  empty,

  /// There are pages. Draw them.
  tree,
}

/// The rule that `skeleton` exists to enforce:
///
/// "还没有页面" is a STATEMENT, and while the tree is still in flight it is a
/// FALSE one — the workspace may be full. An empty sidebar during loading tells
/// the user their notes are gone. Placeholder rows say the same thing an empty
/// list would ("nothing to read here yet") without asserting anything about the
/// workspace, and need no translation.
///
/// Order is the whole content of this function: `treePending` is checked BEFORE
/// the empty state, never after. Reversed, every slow tree load flashes
/// "no pages yet" first — which is exactly the bug, and it looks like a
/// rendering nicety in review.
PageTreeState pageTreeStateFor({
  required bool hasWorkspace,
  required bool viewsEmpty,
  required bool treePending,
}) {
  if (!hasWorkspace) return PageTreeState.noWorkspace;
  if (viewsEmpty && treePending) return PageTreeState.skeleton;
  if (viewsEmpty) return PageTreeState.empty;
  return PageTreeState.tree;
}

/// Placeholder rows shown while the page tree loads.
///
/// `IgnorePointer` on purpose: these are not rows, and a tap that appeared to
/// select one would be a lie about a page that does not exist yet.
class PageTreeSkeleton extends StatelessWidget {
  const PageTreeSkeleton({super.key});

  /// Varying widths so it reads as "rows of text", not as a broken layout.
  static const widths = [0.72, 0.55, 0.84, 0.48, 0.66, 0.60];

  @override
  Widget build(BuildContext context) {
    final tokens = MicaTheme.of(context);
    return IgnorePointer(
      child: ListView(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
        children: [
          for (final w in widths)
            Padding(
              padding: const EdgeInsets.only(bottom: 14),
              child: FractionallySizedBox(
                alignment: Alignment.centerLeft,
                widthFactor: w,
                child: Container(
                  height: 10,
                  decoration: BoxDecoration(
                    color: tokens.surface.hover,
                    borderRadius: BorderRadius.circular(4),
                  ),
                ),
              ),
            ),
        ],
      ),
    );
  }
}
