import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/import/import_controller.dart';

/// Characterization tests for [ImportController], pinned against the behavior
/// the code had when it still lived in `_WorkspaceShellState` (the god-class
/// pilot in docs/reviews/code-optimization/OPTIMIZATION_PLAN_ROUND3.md). These exist so
/// the extraction cannot silently drift; when one fails, that is the point.
void main() {
  group('cleanArchiveName', () {
    test('drops a trailing 32-hex id', () {
      expect(
        ImportController.cleanArchiveName(
          'Export-0123456789abcdef0123456789abcdef.zip',
        ),
        'Export',
      );
    });

    test('drops a leading uuid id', () {
      expect(
        ImportController.cleanArchiveName(
          '01234567-89ab-cdef-0123-456789abcdef_Export.zip',
        ),
        'Export',
      );
    });

    test('separator may be a space or underscore', () {
      expect(
        ImportController.cleanArchiveName(
          'Export 0123456789abcdef0123456789abcdef.zip',
        ),
        'Export',
      );
      expect(
        ImportController.cleanArchiveName(
          '0123456789abcdef0123456789abcdef_Export.zip',
        ),
        'Export',
      );
    });

    test('extension matching is case-insensitive', () {
      expect(ImportController.cleanArchiveName('我的归档.ZIP'), '我的归档');
    });

    test('a plain archive name survives untouched', () {
      expect(ImportController.cleanArchiveName('a.zip'), 'a');
      expect(ImportController.cleanArchiveName('Notes 2026.zip'), 'Notes 2026');
    });
  });

  group('rehostFailures', () {
    ImportImageFailure failure(
      String url, {
      String documentId = 'doc1',
      String blockId = 'blk1',
    }) {
      return ImportImageFailure(
        url: url,
        page: 'Some page',
        documentId: documentId,
        blockId: blockId,
        reason: 'host timed out',
        attempted: true,
      );
    }

    test('without a session or workspace the list is returned untouched', () async {
      final h = _script();
      final input = [failure('https://x.test/a.png')];
      await expectLater(
        h.controller.rehostFailures(null, input, 'ws1'),
        completion(same(input)),
      );
      await expectLater(
        h.controller.rehostFailures('token', input, null),
        completion(same(input)),
      );
    });

    test('a record missing ids or with a non-http url is not retried', () async {
      final h = _script();
      final noBlock = failure('https://x.test/a.png', blockId: '');
      final notHttp = failure('file:///tmp/a.png');
      final remaining = await h.controller.rehostFailures(
        'token',
        [noBlock, notHttp],
        'ws1',
      );
      expect(remaining, [noBlock, notHttp]);
      expect(h.fetchCalls, isEmpty);
    });

    test('a fetch that fails or returns empty keeps the link', () async {
      final h = _script(fetchResults: {'https://x.test/a.png': null});
      expect(
        await h.controller.rehostFailures('token', [failure('https://x.test/a.png')], 'ws1'),
        hasLength(1),
      );
      final h2 = _script(fetchResults: {'https://x.test/a.png': Uint8List(0)});
      expect(
        await h2.controller.rehostFailures('token', [failure('https://x.test/a.png')], 'ws1'),
        hasLength(1),
      );
    });

    test('a refused store/patch keeps the link', () async {
      final h = _script(
        fetchResults: {'https://x.test/a.png': Uint8List.fromList([1, 2])},
        rehostThrows: true,
      );
      expect(
        await h.controller.rehostFailures('token', [failure('https://x.test/a.png')], 'ws1'),
        hasLength(1),
      );
    });

    test('a successful re-host removes the link from the report', () async {
      final h = _script(
        fetchResults: {'https://x.test/a.png': Uint8List.fromList([1, 2])},
      );
      expect(
        await h.controller.rehostFailures('token', [failure('https://x.test/a.png')], 'ws1'),
        isEmpty,
      );
      expect(h.rehostCalls, hasLength(1));
      expect(h.rehostCalls.single.blockId, 'blk1');
    });

    test('one URL is fetched once but posted per block', () async {
      final h = _script(
        fetchResults: {'https://x.test/a.png': Uint8List.fromList([1, 2])},
      );
      final remaining = await h.controller.rehostFailures(
        'token',
        [
          failure('https://x.test/a.png'),
          failure('https://x.test/a.png', blockId: 'blk2'),
        ],
        'ws1',
      );
      expect(remaining, isEmpty);
      expect(h.fetchCalls, ['https://x.test/a.png']);
      expect(h.rehostCalls.map((c) => c.blockId), ['blk1', 'blk2']);
    });
  });

  group('watch', () {
    test('running → done: progress updates, workspace selected, finish fires', () async {
      final ws = _ws('ws1');
      final h = _script(
        jobScript: [
          const ImportJobStatus(status: 'running', total: 5, done: 2),
          const ImportJobStatus(status: 'running', total: 5, done: 4),
          const ImportJobStatus(status: 'done', total: 5, done: 5, workspaceId: 'ws1'),
        ],
        workspaces: [ws],
      );
      final c = h.controller;
      final progressSeen = <({int done, int total})?>[];
      c.addListener(() => progressSeen.add(c.progress));
      var landed = '';
      c.onLanded = (w) async => landed = w.id;
      var finished = false;
      c.onFinished = (_, __) => finished = true;
      final loaded = <List<Workspace>>[];
      c.onWorkspacesLoaded = loaded.add;

      await c.watch('token', 'job1', null, 'fallback', pollDelay: Duration.zero);

      expect(
        progressSeen.whereType<({int done, int total})>().length,
        3,
        reason: 'every poll with total>0 publishes done/total',
      );
      expect(landed, 'ws1');
      expect(loaded, hasLength(1));
      expect(finished, isTrue);
      expect(c.progress, isNull);
      expect(c.jobId, isNull);
    });

    test('an import that landed nothing reports no finish', () async {
      final h = _script(
        jobScript: [
          const ImportJobStatus(status: 'done', total: 0, done: 0, workspaceId: 'ws1'),
        ],
      );
      var finished = false;
      h.controller.onFinished = (_, __) => finished = true;
      await h.controller.watch('token', 'job1', null, 'fallback', pollDelay: Duration.zero);
      expect(finished, isFalse);
    });

    test('cancelled: says how much landed, no finish, no selection', () async {
      final h = _script(
        jobScript: [
          const ImportJobStatus(status: 'cancelled', total: 10, done: 3),
        ],
        workspaces: [_ws('ws1')],
      );
      var cancelled = -1;
      h.controller.onCancelled = (landed) => cancelled = landed;
      var landed = '';
      h.controller.onLanded = (w) async => landed = w.id;
      var finished = false;
      h.controller.onFinished = (_, __) => finished = true;
      final loaded = <List<Workspace>>[];
      h.controller.onWorkspacesLoaded = loaded.add;

      await h.controller.watch('token', 'job1', null, 'fallback', pollDelay: Duration.zero);

      expect(cancelled, 3);
      expect(loaded, hasLength(1), reason: 'a cancelled import still refreshed the list');
      expect(landed, '');
      expect(finished, isFalse);
    });

    test('error status surfaces the server message, or the fallback', () async {
      final h = _script(
        jobScript: [
          const ImportJobStatus(status: 'error', total: 2, done: 1, error: 'boom'),
        ],
      );
      final errors = <String>[];
      h.controller.onError = errors.add;
      await h.controller.watch('token', 'job1', null, 'fallback', pollDelay: Duration.zero);
      expect(errors, ['boom']);

      final h2 = _script(
        jobScript: [const ImportJobStatus(status: 'error', total: 2, done: 1)],
      );
      final errors2 = <String>[];
      h2.controller.onError = errors2.add;
      await h2.controller.watch(
        'token', 'job1', null, 'localized fallback', pollDelay: Duration.zero,
      );
      expect(errors2, ['localized fallback']);
    });

    test('a transient poll failure is retried; the import still finishes', () async {
      // Issue #52: one network blip used to end the watch as a failure.
      var calls = 0;
      final h = _script(
        jobStatusOverride: (id) async {
          calls++;
          if (calls == 2) throw const ApiException('connection reset');
          if (calls == 3) throw const ApiException('bad gateway', statusCode: 502);
          return calls < 5
              ? const ImportJobStatus(status: 'running', total: 4, done: 1)
              : const ImportJobStatus(status: 'done', total: 4, done: 4, workspaceId: 'ws1');
        },
        workspaces: [_ws('ws1')],
      );
      final errors = <String>[];
      h.controller.onError = errors.add;
      var finished = false;
      h.controller.onFinished = (_, _) => finished = true;
      await h.controller.watch('token', 'job1', null, 'fallback', pollDelay: Duration.zero);
      expect(errors, isEmpty);
      expect(finished, isTrue);
    });

    test('a 4xx while polling is final, not retried', () async {
      var calls = 0;
      final h = _script(
        jobStatusOverride: (id) async {
          calls++;
          throw const ApiException('not found', statusCode: 404);
        },
      );
      final errors = <String>[];
      h.controller.onError = errors.add;
      await h.controller.watch('token', 'job1', null, 'fallback', pollDelay: Duration.zero);
      expect(calls, 1);
      expect(errors, ['not found']);
    });

    test('a transport failure during polling lands in onError, state clears', () async {
      final h = _script(jobStatusOverride: (id) => throw ApiException('network down'));
      final errors = <String>[];
      h.controller.onError = errors.add;
      await h.controller.watch('token', 'job1', null, 'fallback', pollDelay: Duration.zero);
      expect(errors, ['network down']);
      expect(h.controller.progress, isNull);
      expect(h.controller.jobId, isNull);
    });

    test('the second image pass runs before finish and reports survivors', () async {
      final dead = _failure('https://x.test/dead.png', 'doc1', 'blk1');
      final alive = _failure('https://x.test/ok.png', 'doc2', 'blk2');
      final h = _script(
        jobScript: [
          ImportJobStatus(
            status: 'done',
            total: 3,
            done: 3,
            workspaceId: 'ws1',
            imageFailures: [dead, alive],
          ),
        ],
        workspaces: [_ws('ws1')],
        fetchResults: {
          'https://x.test/dead.png': null,
          'https://x.test/ok.png': Uint8List.fromList([9]),
        },
      );
      h.controller.onLanded = (_) async {};
      List<ImportImageFailure>? survivors;
      h.controller.onFinished = (_, still) => survivors = still;
      await h.controller.watch('token', 'job1', null, 'fallback', pollDelay: Duration.zero);
      expect(survivors, hasLength(1));
      expect(survivors!.single.url, 'https://x.test/dead.png');
    });

    test('rehost disabled skips the second pass entirely', () async {
      final f = _failure('https://x.test/a.png', 'doc1', 'blk1');
      final h = _script(
        jobScript: [
          ImportJobStatus(
            status: 'done',
            total: 3,
            done: 3,
            workspaceId: 'ws1',
            imageFailures: [f],
          ),
        ],
        workspaces: [_ws('ws1')],
      );
      h.controller.rehostEnabled = false;
      h.controller.onLanded = (_) async {};
      List<ImportImageFailure>? survivors;
      h.controller.onFinished = (_, still) => survivors = still;
      await h.controller.watch('token', 'job1', null, 'fallback', pollDelay: Duration.zero);
      expect(h.fetchCalls, isEmpty);
      expect(survivors, [f]);
    });
  });

  group('cancel', () {
    test('asks the server to stop the current job and swallows failure', () async {
      String? asked;
      final h = _script(cancelJobOverride: (token, id) async {
        asked = id;
        if (id == 'boom') throw ApiException('nope');
      });
      h.controller.attach('job9');
      await h.controller.cancel('token');
      expect(asked, 'job9');

      h.controller.attach('boom');
      await h.controller.cancel('token');
      expect(h.controller.jobId, 'boom', reason: 'a failed cancel leaves the job running');
    });

    test('without a running job there is nothing to ask', () async {
      var asked = false;
      final h = _script(cancelJobOverride: (_, __) async => asked = true);
      await h.controller.cancel('token');
      expect(asked, isFalse);
    });
  });
}

Workspace _ws(String id) => Workspace(id: id, name: 'W-$id', ownerId: 'o', role: 'owner');

ImportImageFailure _failure(String url, String documentId, String blockId) {
  return ImportImageFailure(
    url: url,
    page: 'P',
    documentId: documentId,
    blockId: blockId,
    reason: 'unreachable',
    attempted: true,
  );
}

/// A controller wired to scriptable fakes, with the call log the assertions
/// read. Every remote hook records what it was asked.
class _Harness {
  _Harness({
    List<ImportJobStatus> jobScript = const [],
    Future<ImportJobStatus> Function(String)? jobStatusOverride,
    Future<void> Function(String, String)? cancelJobOverride,
    List<Workspace> workspaces = const [],
    Map<String, Uint8List?> fetchResults = const {},
    bool rehostThrows = false,
  }) {
    var poll = 0;
    final fetchCalls = this.fetchCalls;
    final rehostCalls = this.rehostCalls;
    controller = ImportController(
      jobStatus: jobStatusOverride ?? (id) async => jobScript[poll++],
      listWorkspaces: () async => workspaces,
      rehostImage: (token, ws, doc, {required blockId, required fileName, required bytes}) async {
        rehostCalls.add((blockId: blockId, fileName: fileName));
        if (rehostThrows) throw ApiException('refused');
      },
      cancelJob: cancelJobOverride ?? (token, id) async {},
      fetchImageBytes: (url) async {
        fetchCalls.add(url);
        return fetchResults[url];
      },
    );
  }

  late final ImportController controller;
  final fetchCalls = <String>[];
  final rehostCalls = <({String blockId, String fileName})>[];
}

_Harness _script({
  List<ImportJobStatus> jobScript = const [],
  Future<ImportJobStatus> Function(String)? jobStatusOverride,
  Future<void> Function(String, String)? cancelJobOverride,
  List<Workspace> workspaces = const [],
  Map<String, Uint8List?> fetchResults = const {},
  bool rehostThrows = false,
}) {
  return _Harness(
    jobScript: jobScript,
    jobStatusOverride: jobStatusOverride,
    cancelJobOverride: cancelJobOverride,
    workspaces: workspaces,
    fetchResults: fetchResults,
    rehostThrows: rehostThrows,
  );
}
