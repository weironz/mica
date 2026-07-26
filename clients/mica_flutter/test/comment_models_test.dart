import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';

// The comment DTOs decide what the editor draws, and the load-bearing rule is
// negative: a thread whose anchor the server could NOT resolve (orphaned), or one
// already resolved, must never be highlighted — drawing a comment over unrelated
// words is worse than not drawing it (docs/comments-plan.md).

const Object _absent = Object();

void main() {
  Map<String, dynamic> threadJson({
    Object? anchor = _absent,
    String status = 'open',
    List<Map<String, dynamic>>? comments,
  }) {
    return {
      'id': 'aaaaaaaa-0000-0000-0000-000000000001',
      'status': status,
      'quote': 'Hello',
      'created_by': 'bbbbbbbb-0000-0000-0000-000000000002',
      'created_at': '2026-07-26T07:00:00Z',
      'resolved_by': null,
      'resolved_at': null,
      'anchor': identical(anchor, _absent)
          ? {
              'start_block': 'a',
              'start_offset': 0,
              'end_block': 'a',
              'end_offset': 5,
            }
          : anchor,
      'comments': comments ??
          [
            {
              'id': 'cccccccc-0000-0000-0000-000000000003',
              'author_id': 'bbbbbbbb-0000-0000-0000-000000000002',
              'body': 'why greet?',
              'created_at': '2026-07-26T07:00:01Z',
              'edited_at': null,
            },
          ],
    };
  }

  test('a live thread parses its anchor and is highlightable', () {
    final thread = CommentThread.fromJson(threadJson());
    expect(thread.status, 'open');
    expect(thread.quote, 'Hello');
    expect(thread.anchor!.startOffset, 0);
    expect(thread.anchor!.endOffset, 5);
    expect(thread.anchor!.isSingleBlock, isTrue);
    expect(thread.comments.single.body, 'why greet?');
    expect(thread.createdAt, isNotNull);
    expect(thread.isOrphaned, isFalse);
    expect(thread.isResolved, isFalse);
    expect(thread.isHighlightable, isTrue);
  });

  test('a null anchor means orphaned and is never highlighted', () {
    // The server sends anchor:null when the anchored text is gone.
    final thread = CommentThread.fromJson(threadJson(anchor: null));
    expect(thread.anchor, isNull);
    expect(thread.isOrphaned, isTrue);
    expect(thread.isHighlightable, isFalse);
    // The quote is what the UI can still show.
    expect(thread.quote, 'Hello');
  });

  test('status orphaned is honoured even if an anchor came along', () {
    final thread = CommentThread.fromJson(threadJson(status: 'orphaned'));
    expect(thread.isOrphaned, isTrue);
    expect(thread.isHighlightable, isFalse);
  });

  test('a resolved thread keeps its anchor but is not highlighted', () {
    final thread = CommentThread.fromJson(threadJson(status: 'resolved'));
    expect(thread.isResolved, isTrue);
    expect(thread.anchor, isNotNull,
        reason: 'kept, so "show resolved" can draw it again');
    expect(thread.isHighlightable, isFalse);
  });

  test('a cross-block anchor is not single-block', () {
    final thread = CommentThread.fromJson(threadJson(anchor: {
      'start_block': 'a',
      'start_offset': 2,
      'end_block': 'b',
      'end_offset': 3,
    }));
    expect(thread.anchor!.isSingleBlock, isFalse);
    expect(thread.isHighlightable, isTrue);
  });

  test('missing optional fields degrade instead of throwing', () {
    final thread = CommentThread.fromJson({
      'id': 'aaaaaaaa-0000-0000-0000-000000000001',
      // no status / quote / created_by / timestamps / comments / anchor
    });
    expect(thread.status, 'open');
    expect(thread.quote, '');
    expect(thread.comments, isEmpty);
    expect(thread.createdAt, isNull);
    // No anchor → treated as orphaned, i.e. NOT drawn. Failing safe matters here.
    expect(thread.isOrphaned, isTrue);
    expect(thread.isHighlightable, isFalse);
  });
}
