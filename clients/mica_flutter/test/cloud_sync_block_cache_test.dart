// The session's block list is patched from the replica's change log instead
// of a full read per remote update (2026-11-01). Whatever reaches the replica
// — a peer's keystroke, a local edit, undo, a structural change — the patched
// list must equal a fresh full read. Real FFI engine: the change log is
// mica-core's, so a fake would test nothing.
@TestOn('vm')
library;

import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/cloud/cloud_sync_session.dart';
import 'package:mica_flutter/cloud/sync_replica_io.dart';
import 'package:mica_flutter/src/rust/api/document.dart';
import 'package:mica_flutter/src/rust/frb_generated.dart';

import 'support/native_build.dart';

void main() {
  setUpAll(initCheckedNativeRust);
  tearDownAll(() async => RustLib.dispose());

  test('the patched block list always equals a full read', () {
    final base = MicaDocument.fromBlocksJson(
      rootId: 'r',
      blocksJson: jsonEncode([
        {
          'id': 'r',
          'type': 'page',
          'children': ['a', 'b'],
        },
        {'id': 'a', 'type': 'paragraph', 'text': 'alpha'},
        {'id': 'b', 'type': 'paragraph', 'text': 'beta'},
      ]),
    ).encodeState();
    final replica = replicaFromState(base, BigInt.from(7))!;
    final peer = MicaDocument.fromStateWithClientId(
      bytes: base,
      clientId: BigInt.from(9),
    )!;
    var remoteChanges = 0;
    final session = CloudSyncSession(
      uri: () async => Uri.parse('ws://127.0.0.1:1/ws'),
      clientId: BigInt.from(7),
      onReady: (_, _) {},
      onRemoteChange: () => remoteChanges++,
    )..debugAttachReplica(replica);
    addTearDown(session.dispose);

    var rid = 0;
    void fromPeer(void Function() edit) {
      final sv = peer.stateVector();
      edit();
      session.debugHandleFrame(
        jsonEncode({
          'type': 'sync.update',
          'rid': ++rid,
          'update': base64.encode(peer.encodeDiffSince(stateVector: sv)),
        }),
      );
    }

    void expectInSync(String step) {
      expect(
        jsonEncode(session.allBlocks()),
        jsonEncode(replica.toBlocks()),
        reason: step,
      );
    }

    expectInSync('first read');
    final first = session.allBlocks();
    expect(
      identical(session.allBlocks(), first),
      isTrue,
      reason: 'nothing changed: no read at all',
    );

    fromPeer(() => peer.textInsert(id: 'a', at: 5, text: '!'));
    expectInSync('peer keystroke');
    expect(
      first[1]['text'],
      'alpha',
      reason: 'a list already handed out is not mutated',
    );

    session.applyLocalOps([
      {'type': 'update_block', 'block_id': 'b', 'text': 'beta 2'},
    ], trackHistory: true);
    expectInSync('local edit');

    expect(replica.undo(), isTrue);
    expectInSync('undo');

    fromPeer(
      () => peer.insertBlockJson(
        parentId: 'r',
        index: 1,
        blockJson: jsonEncode({'id': 'c', 'type': 'paragraph', 'text': 'new'}),
      ),
    );
    expectInSync('peer inserts a block');

    fromPeer(() => peer.deleteBlock(id: 'a', bringChildren: false));
    expectInSync('peer deletes a block');

    fromPeer(() => peer.textInsert(id: 'c', at: 0, text: '>'));
    expectInSync('keystroke after a shape change');
    expect(session.childBlocks().map((b) => b['id']), ['c', 'b']);
    expect(remoteChanges, 4);
  });
}
