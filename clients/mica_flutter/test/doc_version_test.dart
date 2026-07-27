import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';

/// `created_by` was in the server's response the whole time (`YrsVersionMeta`),
/// and `DocVersion.fromJson` read the row and threw the field away — so on a
/// shared document the history timeline could not say who rolled what back.
/// Same failure shape as the import job's `done`/`total`: the data arrived and
/// the client dropped it.
void main() {
  group('DocVersion.fromJson', () {
    test('keeps created_by instead of dropping it', () {
      final v = DocVersion.fromJson({
        'id': 'ver-1',
        'label': 'before the migration',
        'created_at': '2026-07-27T10:00:00Z',
        'created_by': 'user-42',
      });

      expect(v.createdBy, 'user-42');
      expect(v.id, 'ver-1');
      expect(v.label, 'before the migration');
      expect(v.createdAt, '2026-07-27T10:00:00Z');
    });

    test('a missing created_by is null, not a crash', () {
      // Local history has no user ids at all, and cloud rows written before the
      // column existed omit it.
      final v = DocVersion.fromJson({
        'id': 'ver-2',
        'label': null,
        'created_at': '2026-07-27T10:00:00Z',
      });

      expect(v.createdBy, isNull);
      expect(v.isAuto, isTrue, reason: 'no label means an auto snapshot');
    });

    test('a null created_by is null', () {
      final v = DocVersion.fromJson({
        'id': 'ver-3',
        'label': '  ',
        'created_at': '',
        'created_by': null,
      });

      expect(v.createdBy, isNull);
      // Whitespace is not a name either — it must still read as an auto snapshot,
      // otherwise the row shows a blank title where a timestamp belongs.
      expect(v.isAuto, isTrue);
    });
  });
}
