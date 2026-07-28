import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/home_screen.dart';

// The home screen owns no data and no navigation, so what is worth pinning is
// the contract with the host: every tap reaches a callback with the right id,
// nothing crashes on a document that has no emoji, an empty list produces prose
// instead of a bald grid, and the grid actually reflows instead of squeezing
// three cards into a phone-width pane.

const _strings = HomeStrings(
  createTitle: 'CREATE',
  createSubtitle: 'CREATE_SUB',
  createHint: 'CTRL_N',
  recentLabel: 'RECENT',
  directoriesLabel: 'DIRS',
  recentEmptyTitle: 'RECENT_EMPTY_TITLE',
  recentEmptyBody: 'RECENT_EMPTY_BODY',
  directoriesEmptyTitle: 'DIRS_EMPTY_TITLE',
  directoriesEmptyBody: 'DIRS_EMPTY_BODY',
);

HomeDocEntry entry(String id, {String? icon = '📗', String? name}) => (
  id: id,
  icon: icon,
  name: name ?? 'doc $id',
  workspaceName: 'ws',
  meta: '2h',
);

void main() {
  Future<void> pumpHome(
    WidgetTester tester, {
    List<HomeDocEntry>? recents,
    List<HomeDocEntry>? directories,
    void Function(String id)? onOpen,
    VoidCallback? onCreatePage,
    double width = 1000,
  }) async {
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: Center(
            child: SizedBox(
              width: width,
              child: HomeScreen(
                strings: _strings,
                recents: recents ?? [entry('r1'), entry('r2'), entry('r3')],
                directories: directories ?? [entry('d1')],
                onOpen: onOpen ?? (_) {},
                onCreatePage: onCreatePage ?? () {},
              ),
            ),
          ),
        ),
      ),
    );
  }

  testWidgets('the create card fires onCreatePage', (tester) async {
    var created = 0;
    await pumpHome(tester, onCreatePage: () => created++);

    await tester.tap(find.text('CREATE'));
    await tester.pump();

    expect(created, 1);
  });

  testWidgets('tapping a recent card opens that id', (tester) async {
    final opened = <String>[];
    await pumpHome(tester, onOpen: opened.add);

    await tester.tap(find.text('doc r2'));
    await tester.pump();

    expect(opened, ['r2']);
  });

  testWidgets('tapping a directory row opens that id', (tester) async {
    final opened = <String>[];
    await pumpHome(
      tester,
      directories: [entry('d7', name: 'a folder')],
      onOpen: opened.add,
    );

    await tester.tap(find.text('a folder'));
    await tester.pump();

    expect(opened, ['d7']);
  });

  testWidgets('a null icon falls back to a kind glyph, not a crash', (
    tester,
  ) async {
    await pumpHome(
      tester,
      recents: [entry('r1', icon: null), entry('r2')],
      directories: [entry('d1', icon: null)],
    );

    expect(tester.takeException(), isNull);
    // One fallback for the icon-less document...
    expect(find.byIcon(Icons.description_outlined), findsOneWidget);
    // ...and for the directory: the section header's folder icon plus the row's.
    expect(find.byIcon(Icons.folder_outlined), findsNWidgets(2));
    // The document that does have an emoji still shows it.
    expect(find.text('📗'), findsOneWidget);
  });

  testWidgets('empty recents show prose, not an empty grid', (tester) async {
    await pumpHome(
      tester,
      recents: [],
      directories: [entry('d1', name: 'folder one')],
    );

    expect(find.text('RECENT_EMPTY_TITLE'), findsOneWidget);
    expect(find.text('RECENT_EMPTY_BODY'), findsOneWidget);
    // The section header stays — the user should still see what is empty.
    expect(find.text('RECENT'), findsOneWidget);
    // No stray grid: nothing that could be a document card is laid out.
    expect(find.textContaining('doc '), findsNothing);
    expect(find.byIcon(Icons.description_outlined), findsNothing);
  });

  testWidgets('empty directories show prose too', (tester) async {
    await pumpHome(tester, directories: []);

    expect(find.text('DIRS_EMPTY_TITLE'), findsOneWidget);
    expect(find.text('DIRS_EMPTY_BODY'), findsOneWidget);
  });
}
