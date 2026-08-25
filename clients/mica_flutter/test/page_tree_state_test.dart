import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/page_tree_state.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

/// The sidebar's "false empty state" bug, as a test.
///
/// Until now this was covered by hand only — `docker pause` the API, look at
/// the sidebar, see placeholder rows. That check is real, but it runs when
/// somebody remembers to run it, which for a rendering detail is never twice.
///
/// The reason it stayed manual was believed to be the sidebar widget: it takes
/// 97 required parameters, so `_pageTree` cannot be built in a test. That was
/// true and beside the point. What is worth protecting is not the pixels, it is
/// one ordering decision over three booleans — and that lifts out on its own.
void main() {
  group('pageTreeStateFor', () {
    test('a loading tree shows the skeleton, NOT "no pages yet"', () {
      // THE regression. "还没有页面" is a statement about the workspace, and
      // while the tree is in flight it is a false one — an empty sidebar during
      // loading tells someone their notes are gone.
      expect(
        pageTreeStateFor(
          hasWorkspace: true,
          viewsEmpty: true,
          treePending: true,
        ),
        PageTreeState.skeleton,
      );
    });

    test('pending is checked BEFORE empty, not after', () {
      // The same rule stated as the ordering it rests on: if the empty check
      // ran first, a loading tree would be `empty` and every slow load would
      // flash "no pages yet". Swapping those two ifs reads as a tidy-up in
      // review, which is exactly why the order needs a test of its own.
      final loading = pageTreeStateFor(
        hasWorkspace: true,
        viewsEmpty: true,
        treePending: true,
      );
      final settled = pageTreeStateFor(
        hasWorkspace: true,
        viewsEmpty: true,
        treePending: false,
      );
      expect(loading, isNot(settled));
      expect(loading, PageTreeState.skeleton);
      expect(settled, PageTreeState.empty);
    });

    test('pages beat a still-pending refresh — never hide what we have', () {
      // The other half of the same rule, and the easier half to break:
      // `treePending` is set on every reload, not only the first, so a
      // background refresh must not blank out a tree already on screen.
      expect(
        pageTreeStateFor(
          hasWorkspace: true,
          viewsEmpty: false,
          treePending: true,
        ),
        PageTreeState.tree,
      );
    });

    test('no workspace wins over everything else', () {
      for (final viewsEmpty in [true, false]) {
        for (final pending in [true, false]) {
          expect(
            pageTreeStateFor(
              hasWorkspace: false,
              viewsEmpty: viewsEmpty,
              treePending: pending,
            ),
            PageTreeState.noWorkspace,
            reason: 'viewsEmpty=$viewsEmpty pending=$pending',
          );
        }
      }
    });
  });

  group('PageTreeSkeleton', () {
    Widget host(Widget child) => MaterialApp(
      home: MicaTheme(
        tokens: MicaTokens.light,
        child: Scaffold(body: SizedBox(width: 260, height: 400, child: child)),
      ),
    );

    testWidgets('draws placeholder rows', (tester) async {
      await tester.pumpWidget(host(const PageTreeSkeleton()));
      // One per declared width — the count is what makes it read as a list
      // rather than as a single stuck bar.
      expect(
        find.byType(FractionallySizedBox),
        findsNWidgets(PageTreeSkeleton.widths.length),
      );
    });

    testWidgets('is not interactive — a tap falls through it', (tester) async {
      // A tap that appeared to select a placeholder would be a lie about a page
      // that does not exist yet.
      //
      // Asserted behaviourally, by putting a target underneath and checking the
      // tap reaches it. The first version of this counted `IgnorePointer`
      // widgets and found five: the ListView's own Scrollable contributes four
      // more, all `ignoring: false`. Counting widget types was measuring the
      // implementation; without the IgnorePointer the ListView would swallow
      // this tap, so this version measures the property that matters.
      var tappedBehind = false;
      await tester.pumpWidget(
        host(
          Stack(
            children: [
              GestureDetector(
                behavior: HitTestBehavior.opaque,
                onTap: () => tappedBehind = true,
                child: const SizedBox.expand(),
              ),
              const PageTreeSkeleton(),
            ],
          ),
        ),
      );
      // `warnIfMissed: false` because MISSING IS THE POINT: flutter_test warns
      // when a tap does not land on the widget you targeted, and here that
      // warning is the behaviour under test. Left on, it prints a stack trace
      // on every green run, and warnings nobody can act on get skimmed past.
      await tester.tap(find.byType(PageTreeSkeleton), warnIfMissed: false);
      expect(
        tappedBehind,
        isTrue,
        reason: 'the skeleton absorbed a tap it should have ignored',
      );
    });
  });
}
