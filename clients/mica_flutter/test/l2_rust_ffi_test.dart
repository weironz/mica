import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/board/board_bridge.dart';
import 'package:mica_flutter/src/rust/api/board.dart' as rust_board;
import 'package:mica_flutter/src/rust/api/editor.dart' as rust_editor;
import 'package:mica_flutter/src/rust/api/sync.dart' as rust_sync;

import 'support/native_build.dart';

void main() {
  setUpAll(() async {
    await initCheckedNativeRust();
  });

  test('L2 BoardSessionBridge executes stateful interactions via Rust FFI', () async {
    final starter = await rust_board.starterBoardJson(template: 'mindmap');
    final session = await openBoardSession(starter);
    expect(session, isNotNull);
    try {
      final initial =
          jsonDecode(session!.currentStatePatchJson()) as Map<String, dynamic>;
      final beforeCount =
          (initial['activeOverlay'] as Map<String, dynamic>)['nodeCount'] as int;
      expect(beforeCount, greaterThan(0));

      final addPatch = jsonDecode(
        session.dispatchJson(
          jsonEncode({
            'type': 'addNode',
            'worldX': 220.0,
            'worldY': 160.0,
            'kind': 'rounded',
            'text': 'FFI 节点',
          }),
        ),
      ) as Map<String, dynamic>;
      expect(addPatch['snapshotChanged'], isTrue);
      final afterCount =
          (addPatch['activeOverlay'] as Map<String, dynamic>)['nodeCount']
              as int;
      expect(afterCount, beforeCount + 1);

      final undoPatch =
          jsonDecode(session.dispatchJson(jsonEncode({'type': 'undo'})))
              as Map<String, dynamic>;
      expect(
        (undoPatch['activeOverlay'] as Map<String, dynamic>)['nodeCount'],
        beforeCount,
      );

      final svg = session.renderSvg();
      expect(svg, startsWith('<svg '));
    } finally {
      session?.dispose();
    }
  });

  test('L2 EditorSessionHandle executes CJK IME, rules, marks, and undo via Rust FFI', () {
    final ed = rust_editor.EditorSessionHandle.fromMarkdown(
      markdown: '# Mica\n\n初始内容',
      clientId: BigInt.from(777),
    );
    try {
      final patch = jsonDecode(
        ed.dispatchJson(
          cmdJson: jsonEncode({
            'cmd': 'set_selection',
            'selection': {
              'kind': 'caret',
              'block_index': 1,
              'offset': 4,
            },
          }),
        ),
      ) as Map<String, dynamic>;
      expect(patch['content_changed'], isFalse);

      final editPatch = jsonDecode(
        ed.dispatchJson(
          cmdJson: jsonEncode({
            'cmd': 'insert_text',
            'text': ' · 全栈Rust',
          }),
        ),
      ) as Map<String, dynamic>;
      expect(editPatch['content_changed'], isTrue);
      expect(ed.exportMarkdown(), contains('初始内容 · 全栈Rust'));
      expect(ed.encodeState(), isNotEmpty);

      ed.dispatchJson(cmdJson: jsonEncode({'cmd': 'undo'}));
      expect(ed.exportMarkdown(), isNot(contains('全栈Rust')));
    } finally {
      ed.dispose();
    }
  });

  test('L2 SyncEngineHandle runs Sans-I/O connect and bootstrap transitions via Rust FFI', () {
    final engine = rust_sync.SyncEngineHandle.newSession(
      documentId: 'doc-ffi-test',
      clientId: BigInt.from(42),
      selfName: 'Desktop',
      useMemoryStorage: true,
    );
    try {
      expect(engine.isReady(), isFalse);
      final connectEffects =
          jsonDecode(engine.handleEventJson(eventJson: '{"type":"connect"}'))
              as List<dynamic>;
      expect(
        connectEffects.any((e) => (e as Map<String, dynamic>)['type'] == 'open_socket'),
        isTrue,
      );
      expect(engine.phase(), isNotEmpty);
    } finally {
      engine.dispose();
    }
  });
}
