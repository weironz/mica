import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

/// The workspace switcher's menu, reduced to the shape that broke it.
///
/// `just app` on Windows flooded the console with
///
///     RenderBox was not laid out: RenderTapRegion#… relayoutBoundary=up1
///     Failed assertion: '!semantics.parentDataDirty': is not true.
///
/// and the menu never appeared. It had been verified on web, where it worked —
/// because Flutter web does not build the semantics tree until something asks
/// for it, and desktop always does. So the web check could not have caught it.
/// `ensureSemantics()` here is what makes this test equivalent to the desktop
/// run rather than to the web one.
///
/// `_WorkspaceSelector` is private (`part of main.dart`) and takes 17 required
/// callbacks, so this reproduces its SHAPE: a MenuAnchor whose children are a
/// text field and a height-capped ListView. That pair is what was added; if the
/// shape is safe, the widget built from it is too.
void main() {
  Widget menu({required List<Widget> children}) => MaterialApp(
    home: Scaffold(
      body: Center(
        child: MenuAnchor(
          menuChildren: children,
          builder: (context, controller, _) => TextButton(
            onPressed: controller.open,
            child: const Text('open'),
          ),
        ),
      ),
    ),
  );

  List<Widget> rows(int n) => [
    for (var i = 0; i < n; i++)
      SizedBox(width: 320, child: ListTile(title: Text('workspace $i'))),
  ];

  /// The shape `_worldList` builds. `SingleChildScrollView` + `Column` and NOT
  /// `ListView`: a menu asks its children for intrinsic dimensions, and a lazy
  /// viewport answers that by throwing. Swapping this back to `ListView` is the
  /// bug, and these tests fail when you do.
  /// [controller] is passed because `_worldList` passes one. Without it the
  /// scroll view grabs the PrimaryScrollController, which the menu panel is
  /// already using, and the failure that reports is the test's, not the
  /// product's.
  Widget cappedList(int n, ScrollController controller) => SizedBox(
    width: 320,
    child: ConstrainedBox(
      constraints: const BoxConstraints(maxHeight: 320),
      child: SingleChildScrollView(
        controller: controller,
        child: Column(mainAxisSize: MainAxisSize.min, children: rows(n)),
      ),
    ),
  );

  testWidgets('a height-capped scroller opens inside a menu', (tester) async {
    final semantics = tester.ensureSemantics();
    final scroll = ScrollController();
    addTearDown(scroll.dispose);
    await tester.pumpWidget(menu(children: [cappedList(13, scroll)]));
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
    expect(find.text('workspace 0'), findsOneWidget);
    semantics.dispose();
  });

  testWidgets('…and still opens with an autofocusing field above it', (
    tester,
  ) async {
    final semantics = tester.ensureSemantics();
    final scroll = ScrollController();
    addTearDown(scroll.dispose);
    await tester.pumpWidget(
      menu(
        children: [
          const SizedBox(width: 320, child: TextField(autofocus: true)),
          cappedList(13, scroll),
        ],
      ),
    );
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
    expect(find.text('workspace 0'), findsOneWidget);
    semantics.dispose();
  });
}
