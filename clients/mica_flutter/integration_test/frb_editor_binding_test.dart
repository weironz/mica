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
