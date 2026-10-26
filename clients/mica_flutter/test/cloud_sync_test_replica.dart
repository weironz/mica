import 'dart:typed_data';

import 'package:mica_flutter/cloud/sync_doc_replica.dart';

/// Byte-only replica for tests of the session's transport/persistence boundary.
class CloudSyncTestReplica implements SyncDocReplica {
  @override
  String rootBlockId() => 'root';
  @override
  Uint8List stateVector() => Uint8List.fromList([0]);
  @override
  Uint8List encodeState() => Uint8List.fromList([0]);
  @override
  Uint8List encodeDiffSince(Uint8List sv) => Uint8List.fromList([1]);
  @override
  bool applyUpdate(Uint8List update) => true;
  @override
  Uint8List mergeUpdates(List<Uint8List> updates) => Uint8List.fromList([2]);
  @override
  List<Map<String, dynamic>> toBlocks() => [
    {
      'id': 'root',
      'type': 'page',
      'text': '',
      'children': <String>[],
      'data': <String, dynamic>{},
    },
  ];
  @override
  void beginHistoryBatch() {}
  @override
  void endHistoryBatch() {}
  @override
  void resetHistory() {}
  @override
  void applyEditorOp(DocOp op) {}
  @override
  bool undo() => false;
  @override
  bool redo() => false;
  @override
  bool canUndo() => false;
  @override
  bool canRedo() => false;
  @override
  (int, int) historyDepths() => (0, 0);
}
