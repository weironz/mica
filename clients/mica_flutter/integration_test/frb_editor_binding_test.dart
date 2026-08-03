// P2-M3: the desktop editing loop is closed on-device.
//
// Drives the REAL self-drawn editor controller with its op sink wired to
// LocalDocBackend (yrs doc + SQLite store) — exactly the local-offline path. We
// type, mark, split, change kind, and merge through the controller's normal API,
// then reopen the store in a fresh backend and assert the reloaded document
// matches. This proves the editor's op stream faithfully drives the CRDT doc and
// survives a restart, with no account or network.
//
//   flutter test integration_test/frb_editor_binding_test.dart -d windows
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/local/local_doc.dart';
import 'package:mica_flutter/src/rust/api/store.dart';
import 'package:mica_flutter/src/rust/frb_generated.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async => RustLib.init());
  tearDownAll(() async => RustLib.dispose());

  // Drain the controller's async op chain so all applyOps() have run.
  Future<void> drain() async {
    for (var i = 0; i < 5; i++) {
      await Future<void>.delayed(Duration.zero);
    }
  }

  EditorController controllerFor(LocalDocBackend backend) {
    final c = EditorController(
      rootBlockId: backend.rootBlockId,
      onOps: backend.applyOps,
    );
    c.load([
      for (final b in backend.childBlocks())
        EditorNode(
          id: b['id'] as String,
          kind: b['type'] as String,
          text: b['text'] as String? ?? '',
          data: (b['data'] as Map?)?.cast<String, dynamic>() ?? {},
        ),
    ]);
    return c;
  }

  test('type + mark + split + kind survive a store reopen', () async {
    final dir = Directory.systemTemp.createTempSync('mica_edit');
    final path = '${dir.path}/edit.db';

    // ---- Session 1: edit through the real controller ----
    final store1 = MicaStore.open(path: path)!;
    final backend1 = LocalDocBackend.open(store1, 'doc1');
    final c = controllerFor(backend1);
    final bodyId = backend1.childBlocks().first['id'] as String;

    // Type into the first paragraph.
    c.setSelection(const DocSelection.collapsed(DocPosition(0, 0)));
    c.setFocusedText('Hello world', 11, 11);
    await c.flushPending();

    // Caret after "Hello world", split into a new block, type into it.
    c.setSelection(const DocSelection.collapsed(DocPosition(0, 11)));
    c.splitAtCaret();
    expect(c.nodes.length, 2, reason: 'split produced a second block');
    c.setSelection(DocSelection.collapsed(DocPosition(1, c.nodes[1].text.length)));
    c.setFocusedText('Second line', 11, 11);
    await c.flushPending();

    // Make the first block a heading (a turn-into clears its data/marks — so we
    // bold *after*, exercising marks-on-heading persistence rather than a wipe).
    c.setSelection(const DocSelection.collapsed(DocPosition(0, 0)));
    c.setFocusedKind('heading');

    // Bold "Hello" inside the heading.
    c.setSelection(const DocSelection(
      anchor: DocPosition(0, 0),
      focus: DocPosition(0, 5),
    ));
    c.toggleMark('bold');

    await drain();
    backend1.flush();

    // ---- Session 2: reopen the same db, rebuild from the store ----
    final store2 = MicaStore.open(path: path)!;
    final backend2 = LocalDocBackend.open(store2, 'doc1');
    final blocks = backend2.childBlocks();

    expect(blocks.length, 2, reason: 'two blocks persisted');
    final first = blocks[0];
    expect(first['id'], bodyId);
    expect(first['type'], 'heading');
    expect(first['text'], 'Hello world');
    final marks = (first['data']['marks'] as List).cast<Map<String, dynamic>>();
    expect(marks.length, 1);
    expect(marks.first['type'], 'bold');
    expect(marks.first['start'], 0);
    expect(marks.first['end'], 5);
    expect(blocks[1]['text'], 'Second line');
    expect(blocks[1]['type'], 'paragraph');

    _bestEffortDelete(dir);
  });

  test('merge backward removes a block and joins text', () async {
    final dir = Directory.systemTemp.createTempSync('mica_edit2');
    final path = '${dir.path}/edit.db';

    final store1 = MicaStore.open(path: path)!;
    final backend1 = LocalDocBackend.open(store1, 'doc1');
    final c = controllerFor(backend1);

    c.setSelection(const DocSelection.collapsed(DocPosition(0, 0)));
    c.setFocusedText('alpha', 5, 5);
    await c.flushPending();
    c.setSelection(const DocSelection.collapsed(DocPosition(0, 5)));
    c.splitAtCaret();
    c.setSelection(DocSelection.collapsed(DocPosition(1, 0)));
    c.setFocusedText('beta', 0, 0);
    await c.flushPending();
    expect(c.nodes.length, 2);

    // Backspace at the start of the second block merges it into the first.
    c.setSelection(const DocSelection.collapsed(DocPosition(1, 0)));
    c.mergeBackward();
    await drain();
    backend1.flush();

    final store2 = MicaStore.open(path: path)!;
    final backend2 = LocalDocBackend.open(store2, 'doc1');
    final blocks = backend2.childBlocks();
    expect(blocks.length, 1, reason: 'merge deleted the second block');
    expect(blocks.first['text'], 'alphabeta');

    _bestEffortDelete(dir);
  });

  /// The local backend used to re-encode and rewrite the WHOLE document on a
  /// 400 ms debounce. It appends a CRDT diff per op batch now, so an edit is on
  /// disk when `applyOps` returns — no window, and no `flush()` required.
  ///
  /// Written as "reopen WITHOUT flushing" on purpose: every other test here
  /// calls `backend.flush()` first, which would have passed under the old
  /// behaviour too and so proves nothing about the change.
  test('an edit is durable without flush', () async {
    final dir = Directory.systemTemp.createTempSync('mica_append');
    final path = '${dir.path}/append.db';

    final store1 = MicaStore.open(path: path)!;
    final backend1 = LocalDocBackend.open(store1, 'doc1');
    final c = controllerFor(backend1);
    c.setSelection(const DocSelection.collapsed(DocPosition(0, 0)));
    c.setFocusedText('durable', 7, 7);
    await c.flushPending();
    await drain();
    // Deliberately NO backend1.flush() here.

    final store2 = MicaStore.open(path: path)!;
    expect(
      LocalDocBackend.open(store2, 'doc1').childBlocks().first['text'],
      'durable',
      reason: 'the append must already be on disk when applyOps returned',
    );
    _bestEffortDelete(dir);
  });

  /// A local-only doc has no server, so `pushed_clock` is 0 forever and both
  /// cloud-shaped trims (`squash`, `trimUpdatesThrough`) are no-ops on it. If
  /// compaction were wired to either of those, this log would only grow — and
  /// `loadDoc` replays the whole thing on every open, so the cost would land on
  /// exactly the person with the longest edit history.
  test('the append log stays bounded and the content survives compaction', () async {
    final dir = Directory.systemTemp.createTempSync('mica_compact');
    final path = '${dir.path}/compact.db';

    final store = MicaStore.open(path: path)!;
    final backend = LocalDocBackend.open(store, 'doc1');
    final c = controllerFor(backend);
    // Past the 256-ROW threshold (logSizes counts rows, not bytes) so the
    // every-32-appends check actually fires.
    const edits = 300;
    c.setSelection(const DocSelection.collapsed(DocPosition(0, 0)));
    for (var i = 0; i < edits; i++) {
      c.setFocusedText('x' * (i + 1), i + 1, i + 1);
      await c.flushPending();
    }
    await drain();

    final (localLog, _) = store.logSizes(docId: 'doc1');
    expect(
      localLog,
      lessThan(edits),
      reason: 'compaction must fold the log, not let it track the edit count',
    );

    backend.flush();
    expect(
      store.logSizes(docId: 'doc1').$1,
      0,
      reason: 'flush leaves a clean base',
    );
    final store2 = MicaStore.open(path: path)!;
    expect(
      LocalDocBackend.open(store2, 'doc1').childBlocks().first['text'],
      'x' * edits,
      reason: 'every edit survives the folding',
    );
    _bestEffortDelete(dir);
  });

  /// `rollbackDoc` restores the checkpointed BASE and drops the log. With an
  /// append log that makes the open-time ordering load-bearing: compact first,
  /// then checkpoint — otherwise the recovery point sits before the previous
  /// session's edits and rolling back silently takes them too.
  test('the §10 recovery point includes the previous session', () async {
    final dir = Directory.systemTemp.createTempSync('mica_rollback');
    final path = '${dir.path}/rollback.db';

    final store1 = MicaStore.open(path: path)!;
    final backend1 = LocalDocBackend.open(store1, 'doc1');
    final c = controllerFor(backend1);
    c.setSelection(const DocSelection.collapsed(DocPosition(0, 0)));
    c.setFocusedText('session one', 11, 11);
    await c.flushPending();
    await drain();
    // No flush: the edit is in the log, not the base.

    // A new session checkpoints — which is where the fold has to have happened.
    final store2 = MicaStore.open(path: path)!;
    LocalDocBackend.open(store2, 'doc1');

    final rolled = store2.rollbackDoc(docId: 'doc1');
    expect(rolled, isNotNull);
    final blocks = (jsonDecode(rolled!.toBlocksJson()) as List)
        .cast<Map<String, dynamic>>();
    final body = blocks.firstWhere((b) => b['type'] == 'paragraph');
    expect(
      body['text'],
      'session one',
      reason: 'rolling back must not undo a session that had already ended',
    );
    _bestEffortDelete(dir);
  });

  test('a freshly opened local doc seeds one empty paragraph', () {
    final dir = Directory.systemTemp.createTempSync('mica_edit3');
    final store = MicaStore.open(path: '${dir.path}/edit.db')!;
    final backend = LocalDocBackend.open(store, 'fresh');
    final blocks = backend.childBlocks();
    expect(blocks.length, 1);
    expect(blocks.first['type'], 'paragraph');
    expect(blocks.first['text'], '');
    // Persisted on creation: a second open finds it, not a new seed.
    final reopened = LocalDocBackend.open(store, 'fresh');
    expect(reopened.childBlocks().length, 1);
    _bestEffortDelete(dir);
  });
}

// The open SQLite handle keeps the db file locked on Windows; cleanup is
// best-effort (not under test).
void _bestEffortDelete(Directory dir) {
  try {
    dir.deleteSync(recursive: true);
  } catch (_) {}
}
