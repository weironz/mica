// The two rules that keep re-tapping a sidebar row from blanking the page.
//
// Reported as "double-clicking the title empties the page; switching away and
// back brings it back". Double-click is deliberately unbound in the sidebar
// (see the F2 comment in main.dart), so a double-click is just two taps on the
// row that is already open — and each tap re-ran the whole bootstrap. Two in
// flight at once meant the loser hit a disposed session, fell through to the
// on-device mirror, found nothing (a doc opened online has no mirror) and
// assigned null over the content on screen.
//
// Nothing was ever lost on the server, which is why switching back "fixed" it —
// and why this looked like a rename bug rather than a navigation one.
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';

void main() {
  group('needsBootstrapOnSelect', () {
    test('re-tapping the page already on screen loads nothing', () {
      expect(
        needsBootstrapOnSelect(
          openViewId: 'v1',
          openViewHasContent: true,
          targetViewId: 'v1',
        ),
        isFalse,
        reason: 'this is the tap that used to race itself and blank the page',
      );
    });

    test('a different page still loads', () {
      expect(
        needsBootstrapOnSelect(
          openViewId: 'v1',
          openViewHasContent: true,
          targetViewId: 'v2',
        ),
        isTrue,
      );
    });

    test('the same id with an EMPTY pane still loads', () {
      // The first open failed (offline, no mirror). Tapping again is the user
      // retrying, and must not be swallowed by the guard.
      expect(
        needsBootstrapOnSelect(
          openViewId: 'v1',
          openViewHasContent: false,
          targetViewId: 'v1',
        ),
        isTrue,
      );
    });

    test('nothing open yet always loads', () {
      expect(
        needsBootstrapOnSelect(
          openViewId: null,
          openViewHasContent: false,
          targetViewId: 'v1',
        ),
        isTrue,
      );
    });
  });

  group('mayReplaceBootstrap', () {
    test('an empty result may NOT blank the page it is already showing', () {
      expect(
        mayReplaceBootstrap(haveNewBootstrap: false, wasShowingSameView: true),
        isFalse,
        reason: 'stale-by-a-moment content beats an empty pane',
      );
    });

    test('an empty result is fine when switching to a different page', () {
      // Nothing to lose: that view was not on screen.
      expect(
        mayReplaceBootstrap(haveNewBootstrap: false, wasShowingSameView: false),
        isTrue,
      );
    });

    test('a real result always wins', () {
      expect(
        mayReplaceBootstrap(haveNewBootstrap: true, wasShowingSameView: true),
        isTrue,
      );
      expect(
        mayReplaceBootstrap(haveNewBootstrap: true, wasShowingSameView: false),
        isTrue,
      );
    });
  });
}
