import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/world_picker.dart';

/// Adding or choosing a server was reachable ONLY from a menu inside the app
/// shell — behind the door you were trying to open. These pin the rules that
/// make putting it on the front door safe.
void main() {
  const strings = WorldPickerStrings(
    heading: '进入哪个世界',
    localName: '本地模式',
    localSubtitle: '不联网也能用',
    addServer: '添加服务器',
    removeServer: '删除',
  );

  Future<({List<String> selected, List<String> removed, List<int> adds})> pump(
    WidgetTester tester, {
    List<String> origins = const ['https://a.example', 'https://b.example'],
    String active = 'https://a.example',
    bool canAdd = true,
    bool canRemove = true,
  }) async {
    final selected = <String>[];
    final removed = <String>[];
    // A LIST, not an int: a record captures an int by value, so the first
    // version of this asserted `adds == 0` — which is true no matter what the
    // button does. A test that cannot fail is not a test.
    final adds = <int>[];
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: WorldPicker(
            origins: origins,
            active: active,
            strings: strings,
            onSelect: selected.add,
            onAdd: canAdd ? () => adds.add(1) : null,
            onRemove: canRemove ? removed.add : null,
          ),
        ),
      ),
    );
    return (selected: selected, removed: removed, adds: adds);
  }

  testWidgets('本地模式 is always there, and it is listed first', (tester) async {
    // It is not a server, and on desktop it is a complete way to use the app —
    // so the front door has to offer it, not just the servers.
    await pump(tester);
    final local = tester.getCenter(find.text('本地模式'));
    final first = tester.getCenter(find.text('a.example'));
    expect(local.dy, lessThan(first.dy));
  });

  testWidgets('servers are shown by host, not by raw origin', (tester) async {
    await pump(tester);
    expect(find.text('a.example'), findsOneWidget);
    expect(find.text('https://a.example'), findsNothing);
  });

  testWidgets('本地模式 has no remove button — its content exists nowhere else', (
    tester,
  ) async {
    // A server's mirror can be re-fetched; the local world cannot.
    await pump(tester);
    expect(find.byIcon(Icons.delete_outline), findsNWidgets(2));
    final localRow = find.ancestor(
      of: find.text('本地模式'),
      matching: find.byType(Row),
    );
    expect(
      find.descendant(
        of: localRow.first,
        matching: find.byIcon(Icons.delete_outline),
      ),
      findsNothing,
    );
  });

  testWidgets('tapping a row selects that world', (tester) async {
    final r = await pump(tester);
    await tester.tap(find.text('b.example'));
    expect(r.selected, ['https://b.example']);
    await tester.tap(find.text('本地模式'));
    expect(r.selected.last, kLocalOrigin);
  });

  testWidgets('remove reports the origin, not the label', (tester) async {
    // The label is lossy (two servers can share a host); the origin is the key.
    final r = await pump(tester);
    await tester.tap(find.byIcon(Icons.delete_outline).last);
    expect(r.removed, ['https://b.example']);
  });

  testWidgets('null callbacks remove the controls instead of disabling them', (
    tester,
  ) async {
    await pump(tester, canAdd: false, canRemove: false);
    expect(find.text('添加服务器'), findsNothing);
    expect(find.byIcon(Icons.delete_outline), findsNothing);
  });

  testWidgets('with no servers yet, adding one is still offered', (
    tester,
  ) async {
    // The empty case is the one that matters: a fresh install has no server, and
    // this row is the only way to get one before signing in.
    final r = await pump(tester, origins: const [], active: kLocalOrigin);
    expect(find.text('本地模式'), findsOneWidget);
    await tester.tap(find.text('添加服务器'));
    expect(
      r.adds,
      hasLength(1),
      reason: 'the only way to get a server before signing in',
    );
  });
}
