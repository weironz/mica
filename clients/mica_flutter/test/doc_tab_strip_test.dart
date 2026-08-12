// Tabs: the close-index arithmetic, and what the strip does and does not draw.
//
// The arithmetic is tested apart from the widget because it is the only part
// that can be wrong in a way the user notices — close a tab, land on the wrong
// page — and a widget test would only reach it through a pile of pumping.

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
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

void main() {
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
    testWidgets('draws nothing for a single tab', (tester) async {
      // A one-tab strip would be a permanent height tax on the editor for a
      // control with nothing to switch between.
      await tester.pumpWidget(
        _host(
          DocTabStrip(
            tabs: [DocTab(view: _view('a', 'Alpha'))],
            activeIndex: 0,
            onSelect: (_) {},
            onClose: (_) {},
            untitledLabel: 'Untitled',
          ),
        ),
      );
      expect(find.text('Alpha'), findsNothing);
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
