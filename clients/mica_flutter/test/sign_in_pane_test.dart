import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/sign_in_pane.dart';

/// The front door's right half. What matters here is not the pixels but the
/// claims: which world you are about to enter, and whether the server is
/// actually there — the second one is a claim the mockup simply painted green.
void main() {
  const strings = SignInPaneStrings(
    cloudTab: '云端账户',
    localTab: '本地模式',
    connected: '已连接到服务器',
    unreachable: '连不上服务器',
    checking: '正在检测…',
    serversLabel: '服务器',
    addServer: '添加服务器',
    removeServer: '删除',
    retry: '重试',
    localTitle: '在此设备上使用',
    localBody: '无需账户',
    localAction: '开始使用',
    signedIn: '已登录',
  );

  late List<String> selected;
  late List<String> removed;
  late List<int> adds;
  late List<int> locals;
  late List<String> probed;

  setUp(() {
    selected = [];
    removed = [];
    adds = [];
    locals = [];
    probed = [];
  });

  Future<void> pump(
    WidgetTester tester, {
    List<String> origins = const ['https://a.example', 'https://b.example'],
    String active = 'https://a.example',
    Future<bool> Function(String)? probe,
    bool canAdd = true,
    bool canRemove = true,
    Set<String> signedInOrigins = const {},
  }) async {
    await tester.binding.setSurfaceSize(const Size(520, 1000));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: SingleChildScrollView(
            child: SignInPane(
              strings: strings,
              origins: origins,
              active: active,
              signedInOrigins: signedInOrigins,
              onSelect: selected.add,
              onEnterLocal: () => locals.add(1),
              onAdd: canAdd ? () => adds.add(1) : null,
              onRemove: canRemove ? removed.add : null,
              probeHealth: probe == null
                  ? null
                  : (o) {
                      probed.add(o);
                      return probe(o);
                    },
              authForm: const Text('AUTH-FORM'),
            ),
          ),
        ),
      ),
    );
  }

  testWidgets('opens on the tab matching the world you are already in', (
    tester,
  ) async {
    // In 本地模式 the screen should describe where you are, not where you are not.
    await pump(tester, active: kLocalOrigin);
    expect(find.text('在此设备上使用'), findsOneWidget);
    expect(find.text('AUTH-FORM'), findsNothing);
  });

  testWidgets('the cloud tab shows the form; the local tab replaces it', (
    tester,
  ) async {
    await pump(tester);
    expect(find.text('AUTH-FORM'), findsOneWidget);
    await tester.tap(find.text('本地模式'));
    await tester.pumpAndSettle();
    expect(find.text('AUTH-FORM'), findsNothing);
    expect(find.text('开始使用'), findsOneWidget);
  });

  testWidgets('本地模式 is entered by its own action, not by tapping the tab', (
    tester,
  ) async {
    // A tab only changes what the pane describes. Entering the world is a
    // decision that also leaves this screen, so it needs a real button.
    await pump(tester);
    await tester.tap(find.text('本地模式'));
    await tester.pumpAndSettle();
    expect(locals, isEmpty, reason: 'the tab must not switch worlds by itself');
    await tester.tap(find.text('开始使用'));
    expect(locals, hasLength(1));
  });

  testWidgets('only the CURRENT server shows until you open the list', (
    tester,
  ) async {
    // The previous version listed every world all the time — 200px of settings
    // above a login form.
    await pump(tester);
    expect(find.text('a.example'), findsOneWidget);
    expect(find.text('b.example'), findsNothing);

    await tester.tap(find.byIcon(Icons.expand_more));
    await tester.pumpAndSettle();
    expect(find.text('b.example'), findsOneWidget);
    expect(find.text('服务器'), findsOneWidget);
  });

  testWidgets('picking another server closes the list and reports the origin', (
    tester,
  ) async {
    await pump(tester);
    await tester.tap(find.byIcon(Icons.expand_more));
    await tester.pumpAndSettle();
    await tester.tap(find.text('b.example'));
    await tester.pumpAndSettle();
    // The origin, not the label: two servers can share a host.
    expect(selected, ['https://b.example']);
    expect(find.text('服务器'), findsNothing);
  });

  testWidgets('no probe means no claim about the connection', (tester) async {
    // Never a green dot we did not earn.
    await pump(tester);
    expect(find.text('已连接到服务器'), findsNothing);
    expect(find.text('连不上服务器'), findsNothing);
    expect(find.text('正在检测…'), findsNothing);
  });

  testWidgets('a probe that answers yes earns 已连接; one that fails says so', (
    tester,
  ) async {
    await pump(tester, probe: (_) async => true);
    await tester.pumpAndSettle();
    expect(find.text('已连接到服务器'), findsOneWidget);
  });

  testWidgets('an unreachable server says so instead of staying silent', (
    tester,
  ) async {
    await pump(tester, probe: (_) async => false);
    await tester.pumpAndSettle();
    expect(find.text('连不上服务器'), findsOneWidget);
  });

  testWidgets('a probe that throws reads as unreachable, not as a crash', (
    tester,
  ) async {
    await pump(tester, probe: (_) async => throw Exception('dns'));
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
    expect(find.text('连不上服务器'), findsOneWidget);
  });

  testWidgets('本地模式 is never probed — there is no server to ask', (
    tester,
  ) async {
    await pump(tester, active: kLocalOrigin, probe: (_) async => true);
    await tester.pumpAndSettle();
    expect(probed, isEmpty);
  });

  testWidgets('with no server configured, adding one is all that is offered', (
    tester,
  ) async {
    // A fresh install: the form would have nothing to sign in to.
    await pump(tester, origins: const [], active: kLocalOrigin);
    await tester.tap(find.text('云端账户'));
    await tester.pumpAndSettle();
    expect(find.text('AUTH-FORM'), findsNothing);
    await tester.tap(find.text('添加服务器'));
    expect(adds, hasLength(1));
  });

  testWidgets('null callbacks remove the controls rather than disable them', (
    tester,
  ) async {
    await pump(tester, canAdd: false, canRemove: false);
    await tester.tap(find.byIcon(Icons.expand_more));
    await tester.pumpAndSettle();
    expect(find.text('添加服务器'), findsNothing);
    expect(find.byIcon(Icons.delete_outline), findsNothing);
  });

  testWidgets('coming back to the cloud tab asks the server again', (
    tester,
  ) async {
    // The verdict is a moment in time. Starting the server after opening this
    // screen and coming back must not keep showing the old answer — a stale
    // 「连不上」 is the same kind of lie as an unearned green dot.
    var up = false;
    await pump(tester, probe: (_) async => up);
    await tester.pumpAndSettle();
    expect(find.text('连不上服务器'), findsOneWidget);
    expect(probed, hasLength(1));

    up = true;
    await tester.tap(find.text('本地模式'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('云端账户'));
    await tester.pumpAndSettle();

    expect(probed, hasLength(2), reason: 'it has to ask again');
    expect(find.text('已连接到服务器'), findsOneWidget);
  });

  testWidgets('remove reports the origin and closes the list', (tester) async {
    await pump(tester);
    await tester.tap(find.byIcon(Icons.expand_more));
    await tester.pumpAndSettle();
    await tester.tap(find.byIcon(Icons.delete_outline).last);
    await tester.pumpAndSettle();
    expect(removed, ['https://b.example']);
  });

  // ── A failed probe must not be permanent ──────────────────────────────────
  //
  // The bug, as reported: the row says 「连不上服务器」 and the very same screen
  // signs in fine. `_probe()` ran once in `initState` and the only way to ask
  // again was to switch to the local tab and back — so a first check that lost a
  // race with the network (cold start, cold DNS) stuck for the whole session.
  // Worse than no dot: it contradicts something the user can see working.

  testWidgets('a failed probe re-checks itself, without being asked', (
    tester,
  ) async {
    await pump(tester, probe: (_) async => false);
    await tester.pump();
    expect(find.text('连不上服务器'), findsOneWidget);
    expect(probed.length, 1, reason: 'the first check has happened');

    // Nothing yet — the retry is delayed, not a busy loop.
    await tester.pump(const Duration(milliseconds: 500));
    expect(probed.length, 1);

    await tester.pump(const Duration(seconds: 2));
    await tester.pump();
    expect(probed.length, 2, reason: 'it asked again on its own');
  });

  testWidgets('the automatic re-checks are bounded, not forever', (
    tester,
  ) async {
    await pump(tester, probe: (_) async => false);
    await tester.pump();

    // Walk well past the whole backoff schedule.
    for (var i = 0; i < 8; i++) {
      await tester.pump(const Duration(seconds: 15));
      await tester.pump();
    }
    expect(
      probed.length,
      4,
      reason:
          'first check + 3 backed-off retries, then it stops '
          '(a login screen must not probe forever)',
    );
  });

  testWidgets('a recovered server flips to connected and stops retrying', (
    tester,
  ) async {
    var up = false;
    await pump(tester, probe: (_) async => up);
    await tester.pump();
    expect(find.text('连不上服务器'), findsOneWidget);

    // The server comes up between retries — exactly the reported situation.
    up = true;
    await tester.pump(const Duration(seconds: 2));
    await tester.pump();
    expect(find.text('已连接到服务器'), findsOneWidget);

    final after = probed.length;
    await tester.pump(const Duration(seconds: 30));
    await tester.pump();
    expect(after, probed.length, reason: 'reachable = no more retries');
  });

  // Two tests, not one with two `pump`s: re-pumping the same widget at the same
  // position is `didUpdateWidget`, not a fresh `initState`, and that only
  // re-probes when `active` changed — so a second pump with a different probe
  // would silently keep the first verdict. (Caught by this test failing.)
  testWidgets('a reachable server shows no retry affordance', (tester) async {
    await pump(tester, probe: (_) async => true);
    await tester.pump();
    expect(
      find.byTooltip('重试'),
      findsNothing,
      reason: 'a refresh button next to a green dot invites poking at nothing',
    );
  });

  testWidgets('the retry affordance is there while unreachable, and asks now', (
    tester,
  ) async {
    await pump(tester, probe: (_) async => false);
    await tester.pump();
    expect(find.byTooltip('重试'), findsOneWidget);

    // Tapping asks again NOW. The point of having it as WELL as the backoff:
    // "I just started the server", and once the budget is spent it is the only
    // way back.
    final before = probed.length;
    await tester.tap(find.byTooltip('重试'));
    await tester.pump();
    expect(probed.length, before + 1);
  });

  // Reported 2026-08-12: switching worlds while signed in showed an empty
  // password form, so every world in this list looked equally expensive to
  // enter. The badge is what separates "one tap" from "type your password".
  group('已登录 badge', () {
    testWidgets('marks only the servers we hold credentials for', (
      tester,
    ) async {
      await pump(tester, signedInOrigins: {'https://b.example'});
      await tester.tap(find.byIcon(Icons.expand_more));
      await tester.pumpAndSettle();

      expect(find.text('已登录'), findsOneWidget);
      // The badge sits on b, not on the active-but-not-signed-in a.
      final badge = tester.getTopLeft(find.text('已登录'));
      final bRow = tester.getTopLeft(find.text('b.example'));
      final aRow = tester.getTopLeft(find.text('a.example').last);
      expect((badge.dy - bRow.dy).abs(), lessThan(4));
      expect((badge.dy - aRow.dy).abs(), greaterThan(4));
    });

    testWidgets('no badge when nothing is signed in', (tester) async {
      await pump(tester);
      await tester.tap(find.byIcon(Icons.expand_more));
      await tester.pumpAndSettle();
      expect(find.text('已登录'), findsNothing);
    });

    testWidgets('a caller that supplies no badge string shows none', (
      tester,
    ) async {
      // The web gate cannot know which origins have sessions; claiming they all
      // do would be worse than saying nothing.
      await tester.binding.setSurfaceSize(const Size(520, 1000));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: SingleChildScrollView(
              child: SignInPane(
                strings: const SignInPaneStrings(
                  cloudTab: '云端账户',
                  localTab: '本地模式',
                  connected: '已连接到服务器',
                  unreachable: '连不上服务器',
                  checking: '正在检测…',
                  serversLabel: '服务器',
                  addServer: '添加服务器',
                  removeServer: '删除',
                  retry: '重试',
                  localTitle: '在此设备上使用',
                  localBody: '无需账户',
                  localAction: '开始使用',
                ),
                origins: const ['https://a.example'],
                active: 'https://a.example',
                signedInOrigins: const {'https://a.example'},
                onSelect: (_) {},
                onEnterLocal: () {},
                authForm: const Text('AUTH-FORM'),
              ),
            ),
          ),
        ),
      );
      await tester.tap(find.byIcon(Icons.expand_more));
      await tester.pumpAndSettle();
      expect(find.text('已登录'), findsNothing);
    });
  });
}
