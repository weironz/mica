import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/home_screen.dart';

// The start pane owns no data and no navigation, so what is worth pinning is
// the contract with the host: every row reaches its callback, a read-only
// workspace gets no "new page", and the recent-page row appears only when there
// is a page to go back to.

const _strings = HomeStrings(
  createTitle: 'CREATE',
  createHint: 'CTRL_N',
  searchTitle: 'SEARCH',
  searchHint: 'CTRL_SHIFT_F',
  recentLabel: 'RECENT',
);

const HomeDocEntry _recent = (
  id: 'p1',
  icon: null,
  name: 'Weekly notes',
  workspaceName: 'ws',
  meta: '2h',
);

void main() {
  Future<void> pumpStart(
    WidgetTester tester, {
    VoidCallback? onCreatePage,
    VoidCallback? onSearch,
    void Function(String id)? onOpen,
    HomeDocEntry? recent,
    bool canCreate = true,
  }) async {
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: HomeScreen(
            strings: _strings,
            onCreatePage: canCreate ? (onCreatePage ?? () {}) : null,
            onSearch: onSearch ?? () {},
            onOpen: onOpen ?? (_) {},
            recent: recent,
          ),
        ),
      ),
    );
  }

  testWidgets('new page and search rows fire their callbacks', (tester) async {
    var created = 0;
    var searched = 0;
    await pumpStart(
      tester,
      onCreatePage: () => created++,
      onSearch: () => searched++,
    );
    expect(find.text('CTRL_N'), findsOneWidget);
    expect(find.text('CTRL_SHIFT_F'), findsOneWidget);
    await tester.tap(find.text('CREATE'));
    await tester.tap(find.text('SEARCH'));
    expect(created, 1);
    expect(searched, 1);
  });

  testWidgets('a read-only workspace offers no new page', (tester) async {
    await pumpStart(tester, canCreate: false);
    expect(find.text('CREATE'), findsNothing);
    expect(find.text('SEARCH'), findsOneWidget);
  });

  testWidgets('the recent row opens that page', (tester) async {
    String? opened;
    await pumpStart(tester, recent: _recent, onOpen: (id) => opened = id);
    expect(find.text('2h'), findsOneWidget);
    await tester.tap(find.textContaining('Weekly notes'));
    expect(opened, 'p1');
  });

  testWidgets('no recent page, no recent row', (tester) async {
    await pumpStart(tester);
    expect(find.textContaining('RECENT'), findsNothing);
    expect(find.byType(Divider), findsNothing);
  });
}
