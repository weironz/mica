import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/main.dart';

/// Cutting one of our own images and pasting it back used to look like an
/// external link — copy can only put a url on the clipboard — so the re-host
/// ladder made the server fetch its own blob back over the public internet,
/// hash it, and PUT it again just to arrive at the file_id the url already
/// carried. [micaBlobLink] reads the ids back out so that round trip is skipped.
const _origin = 'https://mica.example.com';
const _ws = '11111111-1111-1111-1111-111111111111';
const _file = '22222222-2222-2222-2222-222222222222';

void main() {
  group('micaBlobLink: recognises our own image links', () {
    test('bare /blob', () {
      final r =
          micaBlobLink('$_origin/api/workspaces/$_ws/files/$_file/blob', _origin);
      expect(r, isNotNull);
      expect(r!.workspaceId, _ws);
      expect(r.fileId, _file);
      expect(r.name, '', reason: 'no cosmetic segment to read');
    });

    test('with the cosmetic filename copy hangs off it', () {
      final r = micaBlobLink(
        '$_origin/api/workspaces/$_ws/files/$_file/blob/diagram.png',
        _origin,
      );
      expect(r!.name, 'diagram.png');
      expect(r.fileId, _file);
    });

    test('a percent-encoded filename is decoded', () {
      final r = micaBlobLink(
        '$_origin/api/workspaces/$_ws/files/$_file/blob/my%20shot.png',
        _origin,
      );
      expect(r!.name, 'my shot.png');
    });

    test('surrounding whitespace is tolerated', () {
      expect(
        micaBlobLink(
            '  $_origin/api/workspaces/$_ws/files/$_file/blob  ', _origin),
        isNotNull,
      );
    });

    // The builder is `_resolveEditorImageUrls`; both must agree on one shape.
    test('matches the url the copy path actually builds', () {
      final built = '${apiOrigin(Uri.parse('$_origin/api/'))}'
          '/api/workspaces/$_ws/files/$_file/blob';
      final r = micaBlobLink(built, apiOrigin(Uri.parse(_origin)));
      expect(r!.fileId, _file);
    });
  });

  group('micaBlobLink: everything else is a genuine external link', () {
    test('another host wearing the same path', () {
      // A document can carry any url; a look-alike must not be able to name a
      // file_id inside this workspace.
      expect(
        micaBlobLink(
          'https://evil.example.net/api/workspaces/$_ws/files/$_file/blob',
          _origin,
        ),
        isNull,
      );
    });

    test('a plain external image', () {
      expect(micaBlobLink('https://imgur.com/a/photo.png', _origin), isNull);
    });

    test('our host, but not a blob path', () {
      expect(
          micaBlobLink('$_origin/api/workspaces/$_ws/views', _origin), isNull);
    });

    test('right shape, wrong last segment', () {
      expect(
        micaBlobLink('$_origin/api/workspaces/$_ws/files/$_file/meta', _origin),
        isNull,
      );
    });

    test('too deep to be a blob link', () {
      expect(
        micaBlobLink(
            '$_origin/api/workspaces/$_ws/files/$_file/blob/a/b', _origin),
        isNull,
      );
    });

    test('empty ids', () {
      expect(
          micaBlobLink('$_origin/api/workspaces//files//blob', _origin), isNull);
    });

    test('not a url at all', () {
      expect(micaBlobLink('just some text', _origin), isNull);
      expect(micaBlobLink('', _origin), isNull);
    });

    // The caller compares this against the OPEN workspace and re-hosts on a
    // mismatch: object keys are `workspaces/<id>/<sha256>`, and quota and
    // membership are per workspace, so a file_id must not cross the boundary.
    test('another workspace still parses — the caller is what rejects it', () {
      const other = '33333333-3333-3333-3333-333333333333';
      final r = micaBlobLink(
        '$_origin/api/workspaces/$other/files/$_file/blob',
        _origin,
      );
      expect(r!.workspaceId, other);
      expect(r.workspaceId == _ws, isFalse);
    });
  });
}
