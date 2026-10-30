// Issue #86, gap 1: whole-shell widget tests for the shell-level editor flush
// and the tab-load races.
//
// P1-2 (fixed in 0.13.64, review 2026-10-21): switching a tab, opening a tab
// and switching a workspace must commit the mounted editor's debounced
// (≤400ms) typing BEFORE the active document changes — the editor's own
// dispose does not flush, so anything switching away used to drop the last
// input. These tests pump the FULL app (MicaApp → WorkspaceShell →
// WorkspaceView) against a scripted HTTP server (MockClient) and interleave
// three things into ONE event list — server requests, the shell-flush probe
// (`debugProbeShellFlush`) and door events — so "flush first" is an index
// comparison, not a timing guess.
//
// U1: `_selectView` writes its result into the tab it STARTED in; a second
// navigation on that tab invalidates the first load. Both were queue-jumping
// races with no widget-level pin.
// U3: a background tab whose bootstrap fails is removed through `_closeTab`
// (index-correct), not by a bare `removeAt` that left `_activeTabIndex` past
// the end.
//
// U2 (`_restoreSession` server-switch staleness) is NOT covered here: driving
// a server switch mid-restore needs the settings server-list UI on a second
// origin plus a live session swap — the guard itself (`_cloudOrigin != origin`
// on every write) sits inside a private closure with no observable seam.
// See the handoff note for that trade-off.
import 'dart:async';
import 'dart:convert';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:mica_flutter/api/client.dart';
import 'package:mica_flutter/main.dart' as app;
import 'package:mica_flutter/prefs.dart';
import 'package:mica_flutter/ui/doc_tab_strip.dart';

import 'support/online_only_local_offline.dart';

/// Bounded settle. With a document open the shell keeps background work alive
/// (sync session reconnect backoff), so `pumpAndSettle` never settles; a fixed
/// number of frames is what lets the mocked HTTP futures and the taps play
/// out.
Future<void> settle(WidgetTester tester, [int frames = 12]) async {
  for (var i = 0; i < frames; i++) {
    await tester.pump(const Duration(milliseconds: 50));
  }
}

/// Open a page by tapping its sidebar row and waiting it in.
Future<void> openPage(WidgetTester tester, String name) async {
  await tester.tap(find.text(name).first);
  await settle(tester);
  expect(
    tester.takeException(),
    isNull,
    reason: 'opening $name must not throw',
  );
}

DocTabStrip stripOf(WidgetTester tester) =>
    tester.widget<DocTabStrip>(find.byType(DocTabStrip));

void main() {
  late _FakeMicaServer server;

  setUp(() {
    useOnlineOnlyLocalOffline();
    server = _FakeMicaServer();
  });

  tearDown(() {
    app.debugProbeShellFlush = null;
    resetHttpClientForTesting();
    resetLocalOfflineFactory();
  });

  /// Boot the full app against the fake server with a persisted session and
  /// settle into the shell. [workspaces]/[viewsByWorkspace] are written in a
  /// compact form ({id, name, object_id?, name}).
  Future<void> boot(
    WidgetTester tester, {
    required List<Map<String, String>> workspaces,
    required Map<String, List<Map<String, String>>> viewsByWorkspace,
  }) async {
    // Chinese locale: the popup menus size to their content and the wider
    // test-font metrics push the EN labels a few pixels past the route's cap;
    // the zh labels fit (this app is zh-first anyway), which keeps the tests
    // about behavior, not font metrics.
    tester.platformDispatcher.localesTestValue = [const Locale('zh', 'CN')];
    addTearDown(tester.platformDispatcher.clearLocalesTestValue);
    // A desktop-sized surface: the 800×600 default leaves the shell a few
    // pixels short in places, surfacing layout noise that is not the scenario.
    tester.view.physicalSize = const Size(1280, 800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    server.script(workspaces: workspaces, viewsByWorkspace: viewsByWorkspace);
    app.debugProbeShellFlush = server.probeFlush;
    await tester.pumpWidget(const app.MicaApp());
    await settle(tester);
    expect(
      tester.takeException(),
      isNull,
      reason: 'the scenario cannot mean anything if startup itself threw',
    );
    expect(
      find.byType(app.ErrorBanner),
      findsNothing,
      reason: 'the cloud fixture must boot without a native-store error',
    );
  }

  testWidgets('P1-2: opening a page in a new tab flushes first', (
    tester,
  ) async {
    await boot(
      tester,
      workspaces: [
        {'id': 'ws1', 'name': 'One'},
      ],
      viewsByWorkspace: {
        'ws1': [
          {'id': 'va', 'name': 'Alpha'},
          {'id': 'vb', 'name': 'Beta'},
        ],
      },
    );
    await openPage(tester, 'Alpha');
    expect(stripOf(tester).tabs, hasLength(1));
    server.events.clear();

    // Right-click the sidebar row → "Open in new tab".
    final row = tester.getCenter(find.text('Beta').first);
    await tester.tapAt(row, buttons: kSecondaryMouseButton);
    await settle(tester);
    await tester.tap(find.text('在新标签页打开'));
    await settle(tester);
    expect(tester.takeException(), isNull);

    server.expectFlushBefore(
      'GET /api/workspaces/ws1/documents/doc-vb/bootstrap',
      reason:
          'the new tab must commit the editor that was showing Alpha before '
          'Beta starts loading into it (P1-2: _inNewTab → _flushActiveEditor)',
    );
    final s = stripOf(tester);
    expect(s.tabs, hasLength(2));
    expect(s.activeIndex, 1);
    expect(s.tabs[1].view?.name, 'Beta');
  });

  testWidgets('open tabs come back after a restart, loading lazily', (
    tester,
  ) async {
    await boot(
      tester,
      workspaces: [
        {'id': 'ws1', 'name': 'One'},
      ],
      viewsByWorkspace: {
        'ws1': [
          {'id': 'va', 'name': 'Alpha'},
          {'id': 'vb', 'name': 'Beta'},
        ],
      },
    );
    await openPage(tester, 'Alpha');
    final row = tester.getCenter(find.text('Beta').first);
    await tester.tapAt(row, buttons: kSecondaryMouseButton);
    await settle(tester);
    await tester.tap(find.text('在新标签页打开'));
    await settle(tester);
    expect(stripOf(tester).tabs, hasLength(2));
    expect(loadPref('openTabs'), contains('"v":"va"'));

    // Restart: same server, same prefs, a fresh app.
    await tester.pumpWidget(const SizedBox());
    await settle(tester);
    server.events.clear();
    await tester.pumpWidget(const app.MicaApp());
    await settle(tester);
    expect(tester.takeException(), isNull);

    var s = stripOf(tester);
    expect(s.tabs.map((t) => t.view?.name), ['Alpha', 'Beta']);
    expect(s.activeIndex, 1, reason: 'the last viewed page is active again');
    expect(
      server.events.where((e) => e.contains('documents/doc-va/bootstrap')),
      isEmpty,
      reason: 'a restored background tab is a label until shown: '
          '${server.events}',
    );

    // First activation fetches it, once.
    await tester.tap(
      find.descendant(
        of: find.byType(DocTabStrip),
        matching: find.text('Alpha'),
      ),
    );
    await settle(tester);
    expect(tester.takeException(), isNull);
    expect(
      server.events
          .where((e) => e.contains('documents/doc-va/bootstrap'))
          .length,
      1,
      reason: '${server.events}',
    );
    s = stripOf(tester);
    expect(s.activeIndex, 0);
    expect(s.tabs[0].bootstrap, isNotNull);
  });

  testWidgets(
    'P1-2: selecting another tab flushes first, and never refetches',
    (tester) async {
      await boot(
        tester,
        workspaces: [
          {'id': 'ws1', 'name': 'One'},
        ],
        viewsByWorkspace: {
          'ws1': [
            {'id': 'va', 'name': 'Alpha'},
            {'id': 'vb', 'name': 'Beta'},
          ],
        },
      );
      await openPage(tester, 'Alpha');
      final row = tester.getCenter(find.text('Beta').first);
      await tester.tapAt(row, buttons: kSecondaryMouseButton);
      await settle(tester);
      await tester.tap(find.text('在新标签页打开'));
      await settle(tester);
      expect(stripOf(tester).activeIndex, 1);
      server.events.clear();

      // Click tab 0 in the strip (scope to the strip — 'Alpha' is also the
      // sidebar row and the home recents card).
      await tester.tap(
        find.descendant(
          of: find.byType(DocTabStrip),
          matching: find.text('Alpha'),
        ),
      );
      await settle(tester);
      expect(tester.takeException(), isNull);

      expect(
        server.events.where((e) => e == _FakeMicaServer.flushEvent).length,
        1,
        reason: 'exactly one shell flush for one tab switch: ${server.events}',
      );
      expect(
        server.events.where((e) => e.contains('documents/doc-va/bootstrap')),
        isEmpty,
        reason:
            'a tab switch reads the bootstrap already in memory — the '
            'fix is wrong if it turned into a refetch cycle: ${server.events}',
      );
      final s = stripOf(tester);
      expect(s.activeIndex, 0);
      expect(s.tabs[0].view?.name, 'Alpha');
      expect(s.tabs[1].view?.name, 'Beta');
    },
  );

  testWidgets('P1-2: the "+" menu\'s New page flushes before creating', (
    tester,
  ) async {
    await boot(
      tester,
      workspaces: [
        {'id': 'ws1', 'name': 'One'},
      ],
      viewsByWorkspace: {
        'ws1': [
          {'id': 'va', 'name': 'Alpha'},
        ],
      },
    );
    await openPage(tester, 'Alpha');
    server.events.clear();

    await tester.tap(
      find.descendant(
        of: find.byType(DocTabStrip),
        matching: find.byIcon(Icons.add),
      ),
    );
    await settle(tester);
    await tester.tap(find.text('新建页面'));
    await settle(tester);
    expect(tester.takeException(), isNull);

    server.expectFlushBefore(
      'POST /api/workspaces/ws1/documents',
      reason:
          'creating into a fresh tab must commit the editor first (P1-2: '
          '_inNewTab → _flushActiveEditor): ${server.events}',
    );
    final s = stripOf(tester);
    expect(s.tabs, hasLength(2));
    expect(s.activeIndex, 1);
    expect(s.tabs[1].bootstrap?.document.id, 'doc-created');
  });

  testWidgets('P1-2: switching workspaces flushes before its tree loads', (
    tester,
  ) async {
    await boot(
      tester,
      workspaces: [
        {'id': 'ws1', 'name': 'One'},
        {'id': 'ws2', 'name': 'Two'},
      ],
      viewsByWorkspace: {
        'ws1': [
          {'id': 'va', 'name': 'Alpha'},
        ],
        'ws2': [
          {'id': 'vg', 'name': 'Gamma'},
        ],
      },
    );
    await openPage(tester, 'Alpha');
    server.events.clear();

    // The workspace selector shows the CURRENT workspace's name; the pick is
    // the other one in its menu.
    await tester.tap(find.text('One').first);
    await settle(tester);
    await tester.tap(find.text('Two').last);
    await settle(tester);
    expect(tester.takeException(), isNull);

    server.expectFlushBefore(
      'GET /api/workspaces/ws2/views',
      reason:
          'switching workspace must commit the editor before the switch '
          'discards the open page (P1-2: _selectWorkspace): ${server.events}',
    );
  });

  testWidgets('U1: a load lands in the tab it started in, not the active one', (
    tester,
  ) async {
    await boot(
      tester,
      workspaces: [
        {'id': 'ws1', 'name': 'One'},
      ],
      viewsByWorkspace: {
        'ws1': [
          {'id': 'va', 'name': 'Alpha'},
          {'id': 'vb', 'name': 'Beta'},
          {'id': 'vg', 'name': 'Gamma'},
        ],
      },
    );
    await openPage(tester, 'Alpha');
    // Beta in a second tab, active.
    final row = tester.getCenter(find.text('Beta').first);
    await tester.tapAt(row, buttons: kSecondaryMouseButton);
    await settle(tester);
    await tester.tap(find.text('在新标签页打开'));
    await settle(tester);
    // Back to Alpha's tab, then start Gamma's load — GATED on the server.
    await tester.tap(
      find.descendant(
        of: find.byType(DocTabStrip),
        matching: find.text('Alpha'),
      ),
    );
    await settle(tester);
    final gate = server.gateBootstrap('doc-vg');
    await tester.tap(find.text('Gamma').first);
    await tester.pump();
    // While Gamma is loading, the user switches to Beta's tab.
    await tester.tap(
      find.descendant(
        of: find.byType(DocTabStrip),
        matching: find.text('Beta'),
      ),
    );
    await settle(tester);
    expect(stripOf(tester).activeIndex, 1);

    gate.complete(server.bootstrapResponse('doc-vg', 'vg', 'Gamma'));
    await settle(tester);
    expect(tester.takeException(), isNull);

    final s = stripOf(tester);
    expect(
      s.tabs[0].view?.name,
      'Gamma',
      reason:
          'the load was started from this tab — it owns the result (U1): '
          '${server.events}',
    );
    expect(
      s.tabs[1].view?.name,
      'Beta',
      reason:
          'the merely-active-at-completion tab must not be overwritten '
          '(U1): ${server.events}',
    );
    expect(s.activeIndex, 1, reason: 'the user never moved');
  });

  testWidgets('U1: a re-navigation retires the load still in flight', (
    tester,
  ) async {
    await boot(
      tester,
      workspaces: [
        {'id': 'ws1', 'name': 'One'},
      ],
      viewsByWorkspace: {
        'ws1': [
          {'id': 'va', 'name': 'Alpha'},
          {'id': 'vg', 'name': 'Gamma'},
        ],
      },
    );
    // Landing on Alpha first: tapping a sidebar row reliably drives the load
    // once a page is open (the tree row is unambiguous), and the race below
    // is about replacing THAT page — the shape the user actually races.
    await openPage(tester, 'Alpha');
    server.events.clear();
    final first = server.gateBootstrap('doc-vg');
    final second = server.gateBootstrap('doc-vg');

    // Two quick taps on Gamma: the second re-issues the load (navSeq bumps) —
    // the first response must be dropped whenever it arrives.
    await tester.tap(find.text('Gamma').first);
    await settle(tester, 4);
    await tester.tap(find.text('Gamma').first);
    await settle(tester, 4);
    expect(
      server.events.where((e) => e.contains('documents/doc-vg/bootstrap')),
      hasLength(2),
      reason:
          'a re-tap during the load re-requests, it does not share the '
          'in-flight one: ${server.events}',
    );

    // The NEWER load completes first; then the stale one comes home.
    second.complete(server.bootstrapResponse('doc-vg', 'vg', 'Gamma'));
    await settle(tester);
    first.complete(
      server.bootstrapResponse(
        'doc-vg',
        'vg',
        'Gamma (stale)',
        objectId: 'doc-vg-stale',
      ),
    );
    await settle(tester);
    expect(tester.takeException(), isNull);

    final s = stripOf(tester);
    expect(
      s.tabs.single.view?.objectId,
      'doc-vg',
      reason:
          'the stale response arrived LAST and must be ignored (U1 '
          'navSeq guard), not land over the newer one: ${server.events}',
    );
  });

  testWidgets('U3: a background tab whose bootstrap fails is removed cleanly', (
    tester,
  ) async {
    await boot(
      tester,
      workspaces: [
        {'id': 'ws1', 'name': 'One'},
        {'id': 'ws2', 'name': 'Two'},
      ],
      viewsByWorkspace: {
        'ws1': [
          {'id': 'va', 'name': 'Alpha'},
        ],
        'ws2': [
          {'id': 'vg', 'name': 'Gamma'},
        ],
      },
    );
    await openPage(tester, 'Alpha');
    server.failBootstrap('doc-vg');
    server.searchAnswer = {
      'results': [
        {
          'view_id': 'vg',
          'object_id': 'doc-vg',
          'name': 'Gamma',
          'snippet': '',
          'workspace_id': 'ws2',
        },
      ],
    };

    // The "+" menu → search across workspaces → tap the hit: queues a
    // BACKGROUND tab from workspace Two whose bootstrap then 500s.
    await tester.tap(
      find.descendant(
        of: find.byType(DocTabStrip),
        matching: find.byIcon(Icons.add),
      ),
    );
    await settle(tester);
    await tester.tap(find.text('打开已有页面…'));
    await settle(tester);
    // The field inside the DIALOG, not the page-title field of the open doc —
    // otherwise the query lands in the document title and nothing searches.
    await tester.enterText(
      find.descendant(
        of: find.byType(Dialog),
        matching: find.byType(TextField),
      ),
      'Gam',
    );
    await settle(tester);
    await tester.tap(find.text('Gamma').first);
    await settle(tester);

    expect(
      tester.takeException(),
      isNull,
      reason:
          'before the fix, the failed background tab left '
          '_activeTabIndex past the end of _tabs (RangeError) and leaked '
          'its socket: ${server.events}',
    );
    final s = stripOf(tester);
    expect(
      s.tabs,
      hasLength(1),
      reason:
          'a tab that can never load is removed, not parked as a '
          'spinner: ${server.events}',
    );
    expect(s.tabs.single.view?.name, 'Alpha');
    expect(s.activeIndex, 0);
  });
}

/// The scripted server + event recorder for the tests above. Everything the
/// shell asks gets an answer here; the assertions then read the event list.
class _FakeMicaServer {
  static const flushEvent = 'FLUSH';

  final events = <String>[];
  late List<Map<String, String>> _workspaces;
  late Map<String, List<Map<String, String>>> _views;
  final _bootstrapGates = <String, List<Completer<http.Response>>>{};
  final _failingBootstrapDocs = <String>{};
  Map<String, dynamic>? searchAnswer;

  void probeFlush() => events.add(flushEvent);

  void script({
    required List<Map<String, String>> workspaces,
    required Map<String, List<Map<String, String>>> viewsByWorkspace,
  }) {
    _workspaces = workspaces;
    _views = viewsByWorkspace;
    final origin =
        'https://shell-flush-${DateTime.now().microsecondsSinceEpoch}.test';
    savePref('cloudOrigin', origin);
    savePref('activeOrigin', origin);
    savePref('servers', jsonEncode([origin]));
    savePref(
      'authUser:$origin',
      jsonEncode({
        'id': 'u1',
        'email': 'shell@mica.test',
        'display_name': 'Shell User',
      }),
    );
    savePref('authToken:$origin', _nonExpiredToken());
    // The prefs store is the real file-backed one on desktop, so this key
    // survives across TEST PROCESSES too — a previous run's workspace switch
    // would steer this boot onto the other workspace. Clear it: every test
    // here boot scripts its world and expects to land on the first one.
    removePref('lastWorkspaceId');
    // Same reason: a saved tab strip from an earlier test would come back.
    removePref('openTabs');
    useHttpClientForTesting(MockClient(_handle));
  }

  /// Bootstrap calls for [docId] hang until the returned Completer completes —
  /// one per call, so a test with two in-flight loads gates them separately.
  Completer<http.Response> gateBootstrap(String docId) {
    final c = Completer<http.Response>();
    _bootstrapGates.putIfAbsent(docId, () => []).add(c);
    return c;
  }

  void failBootstrap(String docId) => _failingBootstrapDocs.add(docId);

  http.Response bootstrapResponse(
    String docId,
    String viewId,
    String pageName, {
    String? objectId,
  }) {
    return http.Response(
      jsonEncode({
        'document': {
          'id': objectId ?? docId,
          'root_block_id': 'root-$docId',
          'current_seq': 0,
        },
        'view': _viewJson({'id': viewId, 'name': pageName}),
        'snapshot': {
          'version_seq': 1,
          'schema_version': 1,
          'payload': {
            'blocks': [
              {
                'id': 'root-$docId',
                'type': 'page',
                'text': '',
                'data': <String, dynamic>{},
                'children': <dynamic>[],
              },
            ],
          },
        },
      }),
      200,
      headers: {'content-type': 'application/json; charset=utf-8'},
    );
  }

  /// The P1-2 shape: "flush first" as an event-index comparison.
  void expectFlushBefore(String request, {String? reason}) {
    final flushAt = events.indexOf(flushEvent);
    final requestAt = events.indexOf(request);
    expect(
      flushAt,
      isNonNegative,
      reason: 'no shell flush recorded at all: $events',
    );
    expect(
      requestAt,
      isNonNegative,
      reason: 'the expected request never went out: $events',
    );
    expect(
      flushAt,
      lessThan(requestAt),
      reason: reason ?? 'flush must precede $request: $events',
    );
  }

  Map<String, dynamic> _viewJson(Map<String, String> v) => {
    'id': v['id'],
    'parent_view_id': null,
    'object_id': 'doc-${v['id']}',
    'object_type': 'document',
    'name': v['name'],
    'position': '0000000010',
  };

  Future<http.Response> _handle(http.Request request) async {
    final path = request.url.path;
    events.add('${request.method} $path');
    if (path == '/api/search' || path.startsWith('/api/search?')) {
      return http.Response(jsonEncode(searchAnswer ?? {'results': []}), 200);
    }
    if (path == '/api/workspaces') {
      return http.Response(
        jsonEncode({
          'workspaces': [
            for (final w in _workspaces)
              {
                'id': w['id'],
                'name': w['name'],
                'owner_id': 'u1',
                'role': 'owner',
              },
          ],
        }),
        200,
      );
    }
    if (path == '/api/auth/me') {
      return http.Response(
        jsonEncode({
          'id': 'u1',
          'email': 'shell@mica.test',
          'display_name': 'Shell User',
        }),
        200,
      );
    }
    for (final w in _workspaces) {
      final wsId = w['id']!;
      if (path == '/api/workspaces/$wsId/views') {
        return http.Response(
          jsonEncode({
            'views': [for (final v in _views[wsId] ?? const []) _viewJson(v)],
          }),
          200,
          headers: {'content-type': 'application/json; charset=utf-8'},
        );
      }
      if (path == '/api/workspaces/$wsId/members') {
        return http.Response(jsonEncode({'members': []}), 200);
      }
      if (path == '/api/workspaces/$wsId/documents' &&
          request.method == 'POST') {
        // Echo the requested name as the new view/doc (the "+" menu's New page).
        final name =
            (jsonDecode(request.body) as Map<String, dynamic>)['name']
                as String;
        // A real create commits the view before the shell refreshes the tree
        // and bootstraps the new document. Keep the scripted server stateful.
        _views[wsId] = [
          ...?_views[wsId],
          {'id': 'created', 'name': name},
        ];
        return http.Response(
          jsonEncode({
            'document': {
              'id': 'doc-created',
              'root_block_id': 'root-doc-created',
              'current_seq': 0,
            },
            'view': {
              'id': 'created',
              'parent_view_id': null,
              'object_id': 'doc-created',
              'object_type': 'document',
              'name': name,
              'position': '0000000020',
            },
          }),
          200,
          headers: {'content-type': 'application/json; charset=utf-8'},
        );
      }
      if (path.startsWith('/api/workspaces/$wsId/documents/') &&
          path.endsWith('/bootstrap')) {
        // /api/workspaces/:ws/documents/:docId/bootstrap → segment 5.
        final docId = path.split('/')[5];
        if (_failingBootstrapDocs.contains(docId)) {
          return http.Response(
            jsonEncode({'error': 'gone', 'message': 'deleted on the server'}),
            404,
          );
        }
        final gates = _bootstrapGates[docId];
        if (gates != null && gates.isNotEmpty) {
          return gates.removeAt(0).future;
        }
        final view = _views[wsId]!.firstWhere((v) => 'doc-${v['id']}' == docId);
        return bootstrapResponse(docId, view['id']!, view['name']!);
      }
    }
    return http.Response('{}', 200);
  }
}

/// A JWT with a far-future exp — the shell only reads the exp claim here;
/// signature is never verified client-side (the server does that).
String _nonExpiredToken() {
  final payload = base64UrlEncode(
    utf8.encode(
      jsonEncode({
        'sub': 'u1',
        'exp': DateTime.now().millisecondsSinceEpoch ~/ 1000 + 86400,
      }),
    ),
  );
  return 'eyJhbGciOiJIUzI1NiJ9.$payload.unsigned-shell';
}
