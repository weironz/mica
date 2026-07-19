// P2-M3: web stub — local offline (the local WORLD) is desktop-only (no native
// FFI on web). The Settings UI already hides the local-offline option on web;
// this stub keeps `main.dart` compiling for web without pulling the native
// bridge into the bundle.
//
// P4-2 exception: the CLOUD page-tree mirror is real here — backed by
// localStorage (small JSON, synchronous — which is exactly what the offline-nav
// fallback needs at cold start). Cloud DOC replicas live in IndexedDB via
// `WebIdbDocStore` (see doc_store_platform_web.dart); only the nav skeleton is
// mirrored through this class.
import 'dart:convert';
import 'dart:typed_data';

import '../cloud/cloud_doc_store.dart';
import '../prefs.dart';

typedef ViewData = ({
  String id,
  String workspaceId,
  String? parentId,
  String objectId,
  String name,
  String position,
  bool trashed,
  String objectType,
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

  String? exportDocHtml(String docId, String title, {int contentWidth = 1160}) =>
      null;

  /// Web has no local pages, so nothing to export (the caller only reaches
  /// this in 本地模式, which doesn't exist on web).
  ({Uint8List bytes, String name, String mime})? exportDocMarkdown(
    String docId,
    String base,
  ) => null;

  /// Web has no local folders — folder ZIP export is cloud-side there.
  Uint8List? exportFolderZip(String workspaceId, String folderId) => null;

  /// No native WebView2 on web — PDF export is a desktop capability. (The web
  /// build routes PDF through the browser's own print, not this path.)
  Future<Uint8List?> htmlToPdf(String html) async => null;

  List<({String id, String? label, int createdAt})> listDocVersions(String docId) =>
      const [];

  ({String id, String? label, int createdAt})? createDocVersion(
    String docId,
    String label,
  ) => null;

  bool restoreDocVersion(String docId, String versionId) => false;

  ({String rootBlockId, List<Map<String, dynamic>> blocks})? docVersionContent(
    String docId,
    String versionId,
  ) => null;

  List<WorkspaceData> listWorkspaces({String origin = 'local'}) => const [];

  void saveWorkspace(WorkspaceData w, {String origin = 'local'}) {}

  /// Web has no local store — cloud workspace order is persisted via the API
  /// (`reorderWorkspaces`), not here.
  void reorderWorkspaces(List<String> ids, {String origin = 'local'}) {}

  void deleteWorkspace(String id) {}

  /// No-op: web has no local store (the mirror lives in IndexedDB, cleared with
  /// the doc mirrors, not here). Present so the caller needn't branch on
  /// platform. See the io variant for what this actually does.
  void forgetOrigin(String origin) {}

  List<ViewData> listViews({String origin = 'local'}) => const [];

  void saveView(ViewData v, {String origin = 'local'}) {}

  /// P4-2: mirror the cloud page tree into localStorage (clean replace per
  /// server origin — same semantics as the desktop store's mirror).
  void mirrorCloudPageTree(
    String serverUrl,
    List<WorkspaceData> workspaces,
    List<ViewData> views,
  ) {
    savePref(
      'cloudPageTree:$serverUrl',
      jsonEncode({
        'workspaces': [
          for (final w in workspaces)
            {
              'id': w.id,
              'name': w.name,
              'position': w.position,
              'role': w.role,
            },
        ],
        'views': [
          for (final v in views)
            {
              'id': v.id,
              'workspaceId': v.workspaceId,
              'parentId': v.parentId,
              'objectId': v.objectId,
              'name': v.name,
              'position': v.position,
              'trashed': v.trashed,
              'objectType': v.objectType,
            },
        ],
      }),
    );
  }

  /// P4-2: the localStorage page-tree mirror for [serverUrl] (null when never
  /// mirrored / unparsable — the caller then stays on the online-only path).
  CloudPageTreeCache? cachedCloudPageTree(String serverUrl) {
    final raw = loadPref('cloudPageTree:$serverUrl');
    if (raw == null || raw.isEmpty) return null;
    try {
      final m = jsonDecode(raw) as Map<String, dynamic>;
      return (
        workspaces: [
          for (final w in (m['workspaces'] as List).cast<Map<String, dynamic>>())
            (
              id: w['id'] as String,
              name: w['name'] as String,
              position: w['position'] as String,
              role: (w['role'] as String?) ?? 'viewer',
            ),
        ],
        views: [
          for (final v in (m['views'] as List).cast<Map<String, dynamic>>())
            (
              id: v['id'] as String,
              workspaceId: v['workspaceId'] as String,
              parentId: v['parentId'] as String?,
              objectId: v['objectId'] as String,
              name: v['name'] as String,
              position: v['position'] as String,
              trashed: (v['trashed'] as bool?) ?? false,
              objectType: (v['objectType'] as String?) ?? 'document',
            ),
        ],
      );
    } catch (_) {
      return null;
    }
  }

  DocData? openCloudDocMirror(String docId) => null;

  ({String workspaceId, int docs})? detachCloudWorkspace(
    String serverUrl,
    String cloudWorkspaceId,
    String name,
  ) => null;

  ({String rootViewId, String newName, int docs})? cloneView({
    required String viewId,
    required String rootName,
  }) => null;

  void purgeView(String viewId, String objectId) {}

  ({String docId, String rootBlockId, List<Map<String, dynamic>> blocks})
      newDoc() => throw UnsupportedError('local offline is not available on web');

  DocData? openDoc(String docId) => null;

  Future<void> applyOps(List<Map<String, dynamic>> ops) async {}

  Future<VaultImportResult> importVaultTree(
    List<({String path, List<int> bytes})> entries,
    String workspaceId, {
    String? parentViewId,
  }) async =>
      (docs: 0, folders: 0, errors: const ['local offline is not available on web']);

  void flush() {}
}
