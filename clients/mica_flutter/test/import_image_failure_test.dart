import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/editor/file_names.dart';

/// The record that says "this image is still borrowed".
///
/// It exists because the failure used to be silent: a re-host the server could
/// not do left the block on its original url, the import reported success, and
/// the user deleted the AppFlowy workspace they had just exported from. The
/// links died with it. So the parsing here is not cosmetic — it decides whether
/// the user is warned before that deletion, and whether the retry can address
/// the right block.
void main() {
  Map<String, dynamic> json({bool? attempted}) => {
    'url': 'https://appflowy.example/blob/abc=.',
    'page': '架构笔记',
    'document_id': 'doc-1',
    'block_id': 'block_7',
    'reason': 'connection timed out',
    if (attempted != null) 'attempted': attempted,
  };

  group('ImportImageFailure', () {
    test('carries the pair a retry has to post to', () {
      final f = ImportImageFailure.fromJson(json());
      expect(f.documentId, 'doc-1');
      expect(f.blockId, 'block_7');
      expect(f.page, '架构笔记');
      expect(f.isRetryable, isTrue);
    });

    test('a record with no attempted flag is read as attempted', () {
      // An older server, or a partial record. Claiming "never tried" would send
      // the reader hunting for a network problem that may not exist; the other
      // way round only costs a slightly vaguer sentence.
      expect(ImportImageFailure.fromJson(json()).attempted, isTrue);
      expect(
        ImportImageFailure.fromJson(json(attempted: false)).attempted,
        isFalse,
      );
    });

    test('a record missing its ids is not retryable', () {
      // `rehost-image` is addressed by document + block. Without both there is
      // nothing to post to, and offering a retry that cannot work is worse than
      // offering none — the user would believe the image was saved.
      for (final broken in [
        {...json(), 'document_id': ''},
        {...json(), 'block_id': ''},
        {...json(), 'url': 'data:image/png;base64,AAAA'},
      ]) {
        expect(
          ImportImageFailure.fromJson(broken).isRetryable,
          isFalse,
          reason: 'must not offer to retry $broken',
        );
      }
    });
  });

  group('ImportJobStatus image failures', () {
    test('a server without the field reports an empty list, not a crash', () {
      // Migration 0024 added the column. Against an older server the key is
      // absent, and that means "nothing recorded" — NOT "nothing failed". Only
      // the server can tell those apart, so the client must not invent an
      // all-clear.
      final job = ImportJobStatus.fromJson({
        'status': 'done',
        'total': 3,
        'done': 3,
      });
      expect(job.imageFailures, isEmpty);
      expect(job.imageFailuresTotal, 0);
    });

    test('the list is parsed and the total kept separately', () {
      // They differ on purpose: the list is capped by the server, the total is
      // exact. Reading the count off the list would under-report the damage.
      final job = ImportJobStatus.fromJson({
        'status': 'done',
        'total': 3,
        'done': 3,
        'image_failures': [json(), json(attempted: false)],
        'image_failures_total': 137,
      });
      expect(job.imageFailures.length, 2);
      expect(job.imageFailuresTotal, 137);
    });
  });

  /// Moved out of the editor so the import's recovery pass names its uploads the
  /// same way. These cases are the ones that were wrong before.
  group('fileNameFromUrl', () {
    test('an ordinary url keeps its name', () {
      expect(fileNameFromUrl('https://x.example/a/b/photo.png'), 'photo.png');
    });

    test('a query string is not part of the name', () {
      expect(fileNameFromUrl('https://x.example/photo.jpg?w=800'), 'photo.jpg');
    });

    test('percent escapes are decoded', () {
      expect(
        fileNameFromUrl('https://x.example/%E8%A7%A3%E9%94%81.png'),
        '解锁.png',
      );
    });

    test('an AppFlowy blob url gets a real extension', () {
      // Its last segment ends in `=.` — a dot with nothing after it. Taken
      // verbatim it uploaded an extension-less file that displayed as `abc=.`.
      expect(fileNameFromUrl('https://af.example/blob/abc='), 'abc=.png');
      expect(fileNameFromUrl('https://af.example/blob/abc=.'), 'abc=.png');
    });

    test('a url with no path segment falls back to a generic name', () {
      expect(fileNameFromUrl('https://x.example/'), 'image.png');
      expect(fileNameFromUrl(''), 'image.png');
    });

    test('a string that is not a url keeps its text and gains an extension', () {
      // `Uri.tryParse` accepts it as a relative reference, so there IS a last
      // segment. Asserted rather than assumed: the first version of this test
      // expected 'image.png' and was simply wrong about what the parser does.
      // Naming it after itself is the better outcome anyway — the server
      // sanitizes the name and re-derives the type from the bytes.
      expect(fileNameFromUrl('not a url at all'), 'not a url at all.png');
    });
  });
}
