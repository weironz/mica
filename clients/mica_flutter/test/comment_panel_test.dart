import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/ui/comment_panel.dart';

// The panel is the only place an ORPHANED thread can still be read (its anchored
// text is gone, so nothing is highlighted on the canvas). That, and "every action
// reaches the host", are what these pin — the host owns the server round trip, so
// the panel must not swallow a tap (docs/comments-plan.md).

CommentThread thread({
  String id = 'thread-1',
  String status = 'open',
  String quote = 'Hello',
  bool anchored = true,
  String createdBy = 'user-1',
  List<CommentEntry>? comments,
}) {
  return CommentThread(
    id: id,
    status: status,
    quote: quote,
    createdBy: createdBy,
    createdAt: DateTime.parse('2026-07-26T07:00:00Z'),
    anchor: anchored
        ? const CommentAnchorRange(
            startBlock: 'a',
            startOffset: 0,
            endBlock: 'a',
            endOffset: 5,
          )
        : null,
    comments:
        comments ??
        const [
          CommentEntry(
            id: 'c1',
            authorId: 'user-1',
            body: 'why greet?',
            createdAt: null,
          ),
        ],
  );
}

void main() {
  Future<void> pumpPanel(
    WidgetTester tester, {
    required List<CommentThread> threads,
    void Function(String id, String body)? onReply,
    void Function(String id, bool resolved)? onSetResolved,
    void Function(String id)? onDelete,
    String? currentUserId,
  }) async {
    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        locale: const Locale('zh'),
        home: Scaffold(
          body: SingleChildScrollView(
            child: CommentPanel(
              threads: threads,
              currentUserId: currentUserId,
              onReply: (id, body) async => onReply?.call(id, body),
              onSetResolved: (id, resolved) async =>
                  onSetResolved?.call(id, resolved),
              onDelete: (id) async => onDelete?.call(id),
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  testWidgets('an empty document explains how to start a comment', (
    tester,
  ) async {
    await pumpPanel(tester, threads: const []);
    expect(find.textContaining('还没有评论'), findsOneWidget);
  });

  testWidgets('a live thread shows its quote and messages', (tester) async {
    await pumpPanel(tester, threads: [thread()]);
    expect(find.text('Hello'), findsOneWidget);
    expect(find.text('why greet?'), findsOneWidget);
    // Open thread → the action offered is "resolve", not "reopen".
    expect(find.text('标记已解决'), findsOneWidget);
    expect(find.text('重新打开'), findsNothing);
  });

  testWidgets('an orphaned thread is still readable and says why', (
    tester,
  ) async {
    // The anchored text was deleted: no highlight exists on the canvas, so this
    // panel is the only way the discussion survives.
    await pumpPanel(
      tester,
      threads: [thread(status: 'orphaned', anchored: false)],
    );
    expect(find.text('Hello'), findsOneWidget, reason: 'the quote remains');
    expect(find.textContaining('已被删除'), findsOneWidget);
    expect(find.text('why greet?'), findsOneWidget);
  });

  testWidgets('a resolved thread offers reopen instead', (tester) async {
    await pumpPanel(tester, threads: [thread(status: 'resolved')]);
    expect(find.text('重新打开'), findsOneWidget);
    expect(find.text('标记已解决'), findsNothing);
    expect(find.text('已解决'), findsOneWidget);
  });

  testWidgets('replying reaches the host with the typed body', (tester) async {
    String? gotId;
    String? gotBody;
    await pumpPanel(
      tester,
      threads: [thread()],
      onReply: (id, body) {
        gotId = id;
        gotBody = body;
      },
    );

    await tester.tap(find.text('回复'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), '  because manners  ');
    await tester.tap(find.text('评论'));
    await tester.pumpAndSettle();

    expect(gotId, 'thread-1');
    expect(gotBody, 'because manners', reason: 'trimmed before sending');
  });

  testWidgets('an empty reply is not sent', (tester) async {
    var called = false;
    await pumpPanel(
      tester,
      threads: [thread()],
      onReply: (_, _) => called = true,
    );
    await tester.tap(find.text('回复'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), '   ');
    await tester.tap(find.text('评论'));
    await tester.pumpAndSettle();
    expect(called, isFalse);
  });

  testWidgets('resolve and delete reach the host', (tester) async {
    String? resolvedId;
    bool? resolvedTo;
    String? deletedId;
    await pumpPanel(
      tester,
      threads: [thread()],
      onSetResolved: (id, resolved) {
        resolvedId = id;
        resolvedTo = resolved;
      },
      onDelete: (id) => deletedId = id,
    );

    await tester.tap(find.text('标记已解决'));
    await tester.pumpAndSettle();
    expect(resolvedId, 'thread-1');
    expect(resolvedTo, isTrue);

    await tester.tap(find.byIcon(Icons.delete_outline));
    await tester.pumpAndSettle();
    expect(deletedId, 'thread-1');
  });

  testWidgets("someone else's thread offers no delete button", (tester) async {
    // The server enforces this too (author, or write access); the UI just does
    // not dangle an action that would be refused.
    await pumpPanel(
      tester,
      threads: [thread(createdBy: 'someone-else')],
      currentUserId: 'me',
    );
    expect(find.byIcon(Icons.delete_outline), findsNothing);
  });

  testWidgets('open threads sort ahead of resolved ones', (tester) async {
    await pumpPanel(
      tester,
      threads: [
        thread(id: 't-resolved', status: 'resolved', quote: 'RESOLVED'),
        thread(id: 't-open', quote: 'OPEN'),
      ],
    );
    final open = tester.getTopLeft(find.text('OPEN'));
    final resolved = tester.getTopLeft(find.text('RESOLVED'));
    expect(open.dy, lessThan(resolved.dy));
  });
}
