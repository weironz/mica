// Tabs: the close-index arithmetic, and what the strip does and does not draw.
//
// The arithmetic is tested apart from the widget because it is the only part
// that can be wrong in a way the user notices — close a tab, land on the wrong
// page — and a widget test would only reach it through a pile of pumping.

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/api/sync_client.dart';
import 'package:mica_flutter/doc_tab.dart';
import 'package:mica_flutter/ui/doc_tab_strip.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

DocumentView _view(String id, String name) => DocumentView(
  id: id,
  parentViewId: null,
  objectId: 'obj-$id',
  objectType: 'document',
  name: name,
  position: '10',
);

Widget _host(Widget child) => MaterialApp(
  home: MicaTheme(
    tokens: MicaTokens.light,
    child: Scaffold(body: child),
  ),
);

/// A tab holding a socket. `DocumentSyncClient`'s constructor is inert —
/// `connect()` is a separate call — so this opens nothing.
DocTab _liveTab(int tick) => DocTab()
  ..lastActivated = tick
  ..sync = DocumentSyncClient(
    documentId: 'doc-$tick',
    uri: Uri.parse('ws://localhost/ws'),
    selfName: 'tester',
    onRemoteSeq: (_, _) {},
    onPresence: (_) {},
  );

void main() {
  group('tabsToPark', () {
    test('parks nothing while the live tabs fit under the cap', () {
      final active = _liveTab(3);
      final tabs = [_liveTab(1), _liveTab(2), active];
      expect(tabsToPark(tabs, active, max: 3), isEmpty);
    });

    test('parks the least-recently-activated once the cap is exceeded', () {
      final oldest = _liveTab(1);
      final active = _liveTab(4);
      final tabs = [oldest, _liveTab(2), _liveTab(3), active];
      expect(tabsToPark(tabs, active, max: 3), [oldest]);
    });

    test('parks several at once, oldest first', () {
      final a = _liveTab(1);
      final b = _liveTab(2);
      final active = _liveTab(5);
      final tabs = [a, b, _liveTab(3), _liveTab(4), active];
      // Only the active tab plus the two newest survive a cap of 3.
      expect(tabsToPark(tabs, active, max: 3), [b, a]);
    });

    test('never parks the active tab, even when it is the oldest', () {
      // The regression this guards: the active tab is stamped on activation, so
      // it should sort first — but a tab restored or opened without a stamp
      // would sort last and get its socket pulled while the user types in it.
      final active = _liveTab(0);
      final tabs = [active, _liveTab(7), _liveTab(8), _liveTab(9)];
      expect(tabsToPark(tabs, active, max: 3), isNot(contains(active)));
    });

    test('the active tab occupies one of the slots', () {
      // Three live BACKGROUND tabs plus the active one is four connections; a
      // cap of 3 has to park one. Counting only the background tabs against the
      // cap would admit max + 1 sockets.
      final active = _liveTab(9);
      final oldest = _liveTab(1);
      final tabs = [oldest, _liveTab(2), _liveTab(3), active];
      expect(tabsToPark(tabs, active, max: 3).length, 1);
      expect(tabsToPark(tabs, active, max: 3), [oldest]);
    });

    test('ignores tabs that are already parked', () {
      // A parked tab has no socket to give up. Returning it again would make
      // the caller drain-and-dispose a null session on every reconcile.
      final active = _liveTab(4);
      final parked = DocTab()..lastActivated = 1;
      final tabs = [parked, _liveTab(2), _liveTab(3), active];
      expect(tabsToPark(tabs, active, max: 3), isEmpty);
    });
  });

  group('activeIndexAfterClose', () {
    test('closing a tab to the RIGHT of the active one leaves it alone', () {
      expect(activeIndexAfterClose(closing: 2, active: 0, count: 3), 0);
      expect(activeIndexAfterClose(closing: 2, active: 1, count: 3), 1);
    });

    test('closing a tab to the LEFT shifts the active index down', () {
      // The regression this exists for: without the shift, closing tab 0 keeps
      // active=2 while the row renumbers under it — the user lands on a
      // different page than the one they were reading.
      expect(activeIndexAfterClose(closing: 0, active: 2, count: 3), 1);
      expect(activeIndexAfterClose(closing: 1, active: 2, count: 3), 1);
    });

    test('closing the ACTIVE tab lands on the one that slides into it', () {
      expect(activeIndexAfterClose(closing: 0, active: 0, count: 3), 0);
      expect(activeIndexAfterClose(closing: 1, active: 1, count: 3), 1);
    });

    test('closing the active LAST tab falls back to the new last tab', () {
      expect(activeIndexAfterClose(closing: 2, active: 2, count: 3), 1);
      expect(activeIndexAfterClose(closing: 1, active: 1, count: 2), 0);
    });

    test('never returns an index past the end of the shortened list', () {
      for (var count = 2; count <= 6; count++) {
        for (var active = 0; active < count; active++) {
          for (var closing = 0; closing < count; closing++) {
            final next = activeIndexAfterClose(
              closing: closing,
              active: active,
              count: count,
            );
            expect(next, greaterThanOrEqualTo(0));
            expect(next, lessThan(count - 1));
          }
        }
      }
    });
  });

  group('DocTabStrip', () {
    testWidgets('is resident: a single tab still draws the strip', (
      tester,
    ) async {
      // It used to hide here, matching AppFlowy. It must not any more: hiding
      // the strip hides the `+`, which is the only visible way to open a
      // second tab.
      await tester.pumpWidget(
        _host(
          DocTabStrip(
            tabs: [DocTab(view: _view('a', 'Alpha'))],
            activeIndex: 0,
            onSelect: (_) {},
            onClose: (_) {},
            untitledLabel: 'Untitled',
            onNewTab: (_) {},
          ),
        ),
      );
      expect(find.text('Alpha'), findsOneWidget);
      expect(find.byIcon(Icons.add), findsOneWidget);
    });

    testWidgets('the only tab has no close button', (tester) async {
      // The host refuses to close the last tab, so an X there would be an
      // affordance that does nothing.
      await tester.pumpWidget(
        _host(
          DocTabStrip(
            tabs: [DocTab(view: _view('a', 'Alpha'))],
            activeIndex: 0,
            onSelect: (_) {},
            onClose: (_) {},
            untitledLabel: 'Untitled',
            onNewTab: (_) {},
          ),
        ),
      );
      expect(find.byIcon(Icons.close), findsNothing);
    });

    testWidgets('the close button comes back with a second tab', (
      tester,
    ) async {
      await tester.pumpWidget(
        _host(
          DocTabStrip(
            tabs: [
              DocTab(view: _view('a', 'Alpha')),
              DocTab(view: _view('b', 'Beta')),
            ],
            activeIndex: 0,
            onSelect: (_) {},
            onClose: (_) {},
            untitledLabel: 'Untitled',
          ),
        ),
      );
      expect(find.byIcon(Icons.close), findsOneWidget);
    });

    testWidgets('shows every tab title once a second one opens', (
      tester,
    ) async {
      await tester.pumpWidget(
        _host(
          DocTabStrip(
            tabs: [
              DocTab(view: _view('a', 'Alpha')),
              DocTab(view: _view('b', 'Beta')),
            ],
            activeIndex: 0,
            onSelect: (_) {},
            onClose: (_) {},
            untitledLabel: 'Untitled',
          ),
        ),
      );
      expect(find.text('Alpha'), findsOneWidget);
      expect(find.text('Beta'), findsOneWidget);
    });

    testWidgets('a tab still loading shows the untitled label, not blank', (
      tester,
    ) async {
      // view == null is the loading state (bootstrap has not arrived). The
      // strip has to stay readable through it — a blank tab tells the user
      // nothing about what they would be switching to.
      await tester.pumpWidget(
        _host(
          DocTabStrip(
            tabs: [DocTab(view: _view('a', 'Alpha')), DocTab()],
            activeIndex: 0,
            onSelect: (_) {},
            onClose: (_) {},
            untitledLabel: 'Untitled',
          ),
        ),
      );
      expect(find.text('Untitled'), findsOneWidget);
    });

    testWidgets('tapping a tab reports its index', (tester) async {
      var selected = -1;
      await tester.pumpWidget(
        _host(
          DocTabStrip(
            tabs: [
              DocTab(view: _view('a', 'Alpha')),
              DocTab(view: _view('b', 'Beta')),
            ],
            activeIndex: 0,
            onSelect: (i) => selected = i,
            onClose: (_) {},
            untitledLabel: 'Untitled',
          ),
        ),
      );
      await tester.tap(find.text('Beta'));
      expect(selected, 1);
    });

    testWidgets('no + button without a handler', (tester) async {
      // The local world passes none — it has no tab model to add to.
      await tester.pumpWidget(
        _host(
          DocTabStrip(
            tabs: [
              DocTab(view: _view('a', 'Alpha')),
              DocTab(view: _view('b', 'Beta')),
            ],
            activeIndex: 0,
            onSelect: (_) {},
            onClose: (_) {},
            untitledLabel: 'Untitled',
          ),
        ),
      );
      expect(find.byIcon(Icons.add), findsNothing);
    });

    testWidgets('the + reports where it was tapped, for anchoring a menu', (
      tester,
    ) async {
      Offset? at;
      await tester.pumpWidget(
        _host(
          DocTabStrip(
            tabs: [
              DocTab(view: _view('a', 'Alpha')),
              DocTab(view: _view('b', 'Beta')),
            ],
            activeIndex: 0,
            onSelect: (_) {},
            onClose: (_) {},
            untitledLabel: 'Untitled',
            onNewTab: (p) => at = p,
          ),
        ),
      );
      await tester.tap(find.byIcon(Icons.add));
      expect(at, isNotNull);
    });

    testWidgets('the + stays put when the tabs overflow', (tester) async {
      // It lives outside the horizontal scroll view on purpose: inside, it
      // would scroll off the right edge exactly when there are enough tabs to
      // want another one.
      await tester.pumpWidget(
        _host(
          SizedBox(
            width: 300,
            child: DocTabStrip(
              tabs: [
                for (var i = 0; i < 12; i++)
                  DocTab(view: _view('v$i', 'Page number $i')),
              ],
              activeIndex: 0,
              onSelect: (_) {},
              onClose: (_) {},
              untitledLabel: 'Untitled',
              onNewTab: (_) {},
            ),
          ),
        ),
      );
      expect(find.byIcon(Icons.add), findsOneWidget);
      await tester.tap(find.byIcon(Icons.add));
    });

    testWidgets('the close button reports the index, not the selection', (
      tester,
    ) async {
      // Closing must not go through onSelect: the active tab's close button is
      // visible at rest, so routing it to select would make the X a no-op.
      var closed = -1;
      var selected = -1;
      await tester.pumpWidget(
        _host(
          DocTabStrip(
            tabs: [
              DocTab(view: _view('a', 'Alpha')),
              DocTab(view: _view('b', 'Beta')),
            ],
            activeIndex: 0,
            onSelect: (i) => selected = i,
            onClose: (i) => closed = i,
            untitledLabel: 'Untitled',
          ),
        ),
      );
      // Only the active tab shows its close button at rest.
      await tester.tap(find.byIcon(Icons.close));
      expect(closed, 0);
      expect(selected, -1);
    });
  });
}
