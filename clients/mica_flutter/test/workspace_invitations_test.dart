import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:mica_flutter/api/client.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/ui/workspace_invitations.dart';
import 'package:mica_flutter/views/invitations_controller.dart';

/// "Add a member" became an invitation the invitee accepts
/// (docs/features/workspace-invitations.md). These pin the client half: the
/// wire shapes it parses, and the two UI surfaces built on them.
void main() {
  const workspaceJson = {
    'id': 'w9',
    'name': 'Team',
    'owner_id': 'u1',
    'role': 'editor',
    'created_at': '2026-10-21T00:00:00Z',
    'updated_at': '2026-10-21T00:00:00Z',
  };
  const receivedJson = {
    'id': 'i1',
    'workspace_id': 'w9',
    'workspace_name': 'Team',
    'role': 'editor',
    'invited_by_name': 'Ada',
    'created_at': '2026-10-21T00:00:00Z',
  };

  /// Every request the fake server saw, as "METHOD path [body]".
  late List<String> calls;

  /// Answers by "METHOD path"; anything unlisted is a 404.
  void serve(Map<String, http.Response Function()> routes) {
    useHttpClientForTesting(
      MockClient((request) async {
        final key = '${request.method} ${request.url.path}';
        calls.add(request.body.isEmpty ? key : '$key ${request.body}');
        final answer = routes[key];
        return answer == null
            ? http.Response('{"code":"not_found","message":"not found"}', 404)
            : answer();
      }),
    );
  }

  http.Response json(Object body, [int status = 200]) =>
      http.Response(jsonEncode(body), status);

  setUp(() => calls = []);
  tearDown(resetHttpClientForTesting);

  InvitationsController controller({List<Workspace>? joined}) =>
      InvitationsController(
        api: ApiClient()..baseUri = Uri.parse('http://test'),
        token: () => 'tok',
        onJoined: (w) async => joined?.add(w),
      );

  Widget host(Widget child) => MaterialApp(
    locale: const Locale('en'),
    localizationsDelegates: AppLocalizations.localizationsDelegates,
    supportedLocales: AppLocalizations.supportedLocales,
    home: Scaffold(body: SingleChildScrollView(child: child)),
  );

  group('client parsing', () {
    test('invite posts to the old members URL and parses an invitation',
        () async {
      serve({
        'POST /api/workspaces/w9/members': () => json({
          'invitation': {
            'id': 'i1',
            'workspace_id': 'w9',
            'email': 'a@b.dev',
            'role': 'viewer',
            'invited_by': 'u1',
            'created_at': '2026-10-21T00:00:00Z',
          },
        }),
      });
      final inv = await controller().invite('w9', 'a@b.dev', 'viewer');
      expect(calls.single, contains('"email":"a@b.dev"'));
      expect(
        (inv.id, inv.workspaceId, inv.email, inv.role),
        ('i1', 'w9', 'a@b.dev', 'viewer'),
      );
    });

    test('received invitations parse; a deleted inviter reads as empty',
        () async {
      serve({
        'GET /api/invitations': () => json({
          'invitations': [
            receivedJson,
            {...receivedJson, 'id': 'i2', 'invited_by_name': null},
          ],
        }),
      });
      final c = controller();
      await c.refreshReceived();
      expect(c.received.map((i) => i.id), ['i1', 'i2']);
      expect(c.received.first.workspaceName, 'Team');
      expect(c.received.first.invitedByName, 'Ada');
      expect(c.received.last.invitedByName, '');
    });

    test('signed out: the inbox is empty and nothing is requested', () async {
      serve({});
      final c = InvitationsController(
        api: ApiClient()..baseUri = Uri.parse('http://test'),
        token: () => null,
        onJoined: (_) async {},
      )..received = const [
          ReceivedInvitation(
            id: 'x',
            workspaceId: 'w',
            workspaceName: 'W',
            role: 'viewer',
            invitedByName: '',
          ),
        ];
      await c.refreshReceived();
      expect(c.received, isEmpty);
      expect(calls, isEmpty);
    });
  });

  group('ReceivedInvitationsCard', () {
    testWidgets('accept joins, hands the workspace to the shell, and clears',
        (tester) async {
      serve({
        'GET /api/invitations': () => json({'invitations': [receivedJson]}),
        'POST /api/invitations/i1/accept': () =>
            json({'workspace': workspaceJson}),
      });
      final joined = <Workspace>[];
      final c = controller(joined: joined);
      await tester.runAsync(c.refreshReceived);
      await tester.pumpWidget(host(ReceivedInvitationsCard(controller: c)));

      expect(find.text('Invitations for you'), findsOneWidget);
      expect(find.text('Team'), findsOneWidget);
      expect(find.text('Ada invited you as editor'), findsOneWidget);

      await tester.runAsync(() async {
        await tester.tap(find.text('Accept'));
        await Future<void>.delayed(const Duration(milliseconds: 50));
      });
      await tester.pumpAndSettle();

      expect(calls, contains('POST /api/invitations/i1/accept {}'));
      expect(joined.single.id, 'w9');
      expect(find.text('Invitations for you'), findsNothing,
          reason: 'nothing left to answer — the card goes away');
    });

    testWidgets('decline removes the invitation without joining',
        (tester) async {
      serve({
        'GET /api/invitations': () => json({'invitations': [receivedJson]}),
        'POST /api/invitations/i1/decline': () => http.Response('', 204),
      });
      final joined = <Workspace>[];
      final c = controller(joined: joined);
      await tester.runAsync(c.refreshReceived);
      await tester.pumpWidget(host(ReceivedInvitationsCard(controller: c)));

      await tester.runAsync(() async {
        await tester.tap(find.text('Decline'));
        await Future<void>.delayed(const Duration(milliseconds: 50));
      });
      await tester.pumpAndSettle();

      expect(calls, contains('POST /api/invitations/i1/decline {}'));
      expect(joined, isEmpty);
      expect(find.text('Team'), findsNothing);
    });

    testWidgets('renders nothing when there are no invitations',
        (tester) async {
      await tester.pumpWidget(
        host(ReceivedInvitationsCard(controller: controller())),
      );
      expect(find.text('Invitations for you'), findsNothing);
    });
  });

  group('MemberInviteSection', () {
    const pendingJson = {
      'id': 'i7',
      'workspace_id': 'w9',
      'email': 'new@b.dev',
      'role': 'editor',
      'invited_by': 'u1',
      'created_at': '2026-10-21T00:00:00Z',
    };

    testWidgets('invite says it was sent and lists it as pending; revoke works',
        (tester) async {
      var pending = <Map<String, Object>>[];
      serve({
        'GET /api/workspaces/w9/invitations': () =>
            json({'invitations': pending}),
        'POST /api/workspaces/w9/members': () {
          pending = [pendingJson];
          return json({'invitation': pendingJson});
        },
        'DELETE /api/workspaces/w9/invitations/i7': () {
          pending = [];
          return http.Response('', 204);
        },
      });
      final c = controller();
      await tester.runAsync(() async {
        await tester.pumpWidget(
          host(MemberInviteSection(controller: c, workspaceId: 'w9')),
        );
        await Future<void>.delayed(const Duration(milliseconds: 20));
      });
      await tester.pump();
      expect(find.text('Pending invitations'), findsNothing);

      await tester.enterText(find.byType(TextField), 'new@b.dev');
      await tester.runAsync(() async {
        await tester.tap(find.text('Invite'));
        await Future<void>.delayed(const Duration(milliseconds: 50));
      });
      await tester.pumpAndSettle();

      expect(
        find.text('Invitation sent. They join once they accept.'),
        findsOneWidget,
      );
      expect(find.text('new@b.dev · editor'), findsOneWidget);
      expect(find.text('Pending'), findsOneWidget);

      await tester.runAsync(() async {
        await tester.tap(find.byTooltip('Revoke invitation'));
        await Future<void>.delayed(const Duration(milliseconds: 50));
      });
      await tester.pumpAndSettle();
      expect(calls, contains('DELETE /api/workspaces/w9/invitations/i7'));
      expect(find.text('new@b.dev · editor'), findsNothing);
    });

    testWidgets('an address that already is a member says so', (tester) async {
      serve({
        'GET /api/workspaces/w9/invitations': () => json({'invitations': []}),
        'POST /api/workspaces/w9/members': () => json({
          'code': 'conflict',
          'message': 'conflict: already a member of this workspace',
        }, 409),
      });
      await tester.runAsync(() async {
        await tester.pumpWidget(
          host(MemberInviteSection(controller: controller(), workspaceId: 'w9')),
        );
        await Future<void>.delayed(const Duration(milliseconds: 20));
      });
      await tester.enterText(find.byType(TextField), 'member@b.dev');
      await tester.runAsync(() async {
        await tester.tap(find.text('Invite'));
        await Future<void>.delayed(const Duration(milliseconds: 50));
      });
      await tester.pumpAndSettle();
      expect(find.text('That address is already a member.'), findsOneWidget);
    });
  });
}
