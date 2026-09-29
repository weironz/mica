import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/api/tree_fetch.dart';
import 'package:mica_flutter/swallowed.dart';

void main() {
  setUp(resetSwallowed);

  test(
    'background tree fetch drops and counts connectivity failures',
    () async {
      final result = await fetchBackgroundTree<int>(
        () async => throw http.ClientException('connection lost'),
      );
      expect(result, isNull);
      expect(swallowedCounts()['views_tree_network'], 1);

      expect(
        await fetchBackgroundTree<int>(
          () async => throw TimeoutException('slow'),
        ),
        isNull,
      );
      expect(swallowedCounts()['views_tree_timeout'], 1);

      expect(
        await fetchBackgroundTree<int>(
          () async => throw const ApiException('offline'),
        ),
        isNull,
      );
      expect(swallowedCounts()['views_tree_api'], 1);
    },
  );

  test(
    'successful fetch returns data; programming errors remain visible',
    () async {
      expect(await fetchBackgroundTree(() async => 42), 42);
      await expectLater(
        fetchBackgroundTree<int>(() async => throw StateError('bad shape')),
        throwsStateError,
      );
    },
  );
}
