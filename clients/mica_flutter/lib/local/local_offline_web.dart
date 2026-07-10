// P2-M3: web stub — local offline is desktop-only (no native FFI on web).
//
// The Settings UI already hides the local-offline option on web; this stub keeps
// `main.dart` compiling for web without pulling the native bridge into the
// bundle. Every method is unavailable.
import 'dart:typed_data';

import '../cloud/cloud_doc_store.dart';

typedef ViewData = ({
  String id,
  String workspaceId,
  String? parentId,
  String objectId,
  String name,
  String position,
  bool trashed,
});

typedef WorkspaceData = ({String id, String name, String position, String role});

typedef CloudPageTreeCache = ({
  List<WorkspaceData> workspaces,
  List<ViewData> views,
});

typedef DocData = ({String rootBlockId, List<Map<String, dynamic>> blocks});

typedef VaultImportResult = ({int docs, int folders, List<String> errors});

class LocalOffline {
  LocalOffline({String? rootDirOverride});

  bool get available => false;

  Future<void> open() async =>
      throw UnsupportedError('local offline is not available on web');

  Future<BigInt?> deviceClientId() async => null;

  /// Web has no on-device store — cloud docs stay online (P2 Phase 1).
  CloudDocStore? cloudDocStore(String docId) => null;

  void rollbackDoc(String docId) {}

  String putBlob(Uint8List bytes) => '';
  void putBlobAs(String fileId, Uint8List bytes) {}
  Uint8List? loadBlob(String fileId) => null;
  bool hasBlob(String fileId) => false;
  String? blobFileUri(String fileId) => null;

  List<WorkspaceData> listWorkspaces({String origin = 'local'}) => const [];

  void saveWorkspace(WorkspaceData w, {String origin = 'local'}) {}

  void deleteWorkspace(String id) {}

  List<ViewData> listViews({String origin = 'local'}) => const [];

  void saveView(ViewData v, {String origin = 'local'}) {}

  void mirrorCloudPageTree(
    String serverUrl,
    List<WorkspaceData> workspaces,
    List<ViewData> views,
  ) {}

  CloudPageTreeCache? cachedCloudPageTree(String serverUrl) => null;

  DocData? openCloudDocMirror(String docId) => null;

  void purgeView(String viewId, String objectId) {}

  ({String docId, String rootBlockId, List<Map<String, dynamic>> blocks})
      newDoc() => throw UnsupportedError('local offline is not available on web');

  DocData? openDoc(String docId) => null;

  Future<void> applyOps(List<Map<String, dynamic>> ops) async {}

  Future<VaultImportResult> importVaultTree(
    List<({String path, List<int> bytes})> entries,
    String workspaceId,
  ) async =>
      (docs: 0, folders: 0, errors: const ['local offline is not available on web']);

  void flush() {}
}
