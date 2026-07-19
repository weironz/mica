// The ONE contract both `LocalOffline` variants implement, plus the shared
// plain-data types they speak.
//
// Why this file exists (G7, after the v0.12.0–0.12.3 web-image breakage): the
// conditional import (`local_offline_io.dart` vs `local_offline_web.dart`)
// creates two parallel classes with NO compiler-enforced contract — analyzer
// and `flutter test` resolve only the IO variant, so a member added to IO but
// not mirrored on web compiles clean everywhere except dart2js at release
// time. Both variants now `implements LocalOfflineApi`, so the analyzer flags
// a missing member IN THE EDITOR, the moment the interface gains it. (CI's
// `flutter build web` step remains the backstop for members added outside the
// interface.) The record typedefs used to be duplicated per-variant — a second
// drift surface — and live only here now.
import 'dart:typed_data';

import '../cloud/cloud_doc_store.dart';

/// One page-tree node, as plain data for the UI layer to map onto its own model.
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

/// One local workspace, as plain data. [role] is the user's membership role,
/// mirrored from the server so an offline start knows whether editing is allowed
/// (P2d); local workspaces are the user's own (owner).
typedef WorkspaceData = ({String id, String name, String position, String role});

/// A mirrored page tree read back from the store for one `origin` (a server
/// URL) — workspaces + views, for offline navigation (P2 option C, P1c).
typedef CloudPageTreeCache = ({
  List<WorkspaceData> workspaces,
  List<ViewData> views,
});

/// A loaded document: its root block id and full block list (snapshot payload).
typedef DocData = ({String rootBlockId, List<Map<String, dynamic>> blocks});

/// Outcome of a vault import (S-tier read-only scan): documents + folder-pages
/// created, and any per-file errors (unreadable files, etc.).
typedef VaultImportResult = ({int docs, int folders, List<String> errors});

/// Every public member of the local-offline facade. Abstract members carry no
/// default parameter values — each variant declares its own (they must agree
/// by convention; a differing default is behavior the web stub doesn't have
/// anyway).
abstract interface class LocalOfflineApi {
  bool get available;
  Future<void> open();
  Future<BigInt?> deviceClientId();
  CloudDocStore? cloudDocStore(String docId);

  List<WorkspaceData> listWorkspaces({String origin});
  void saveWorkspace(WorkspaceData w, {String origin});
  void reorderWorkspaces(List<String> ids, {String origin});
  void deleteWorkspace(String id);
  void forgetOrigin(String origin);
  List<ViewData> listViews({String origin});
  void saveView(ViewData v, {String origin});
  void mirrorCloudPageTree(
    String serverUrl,
    List<WorkspaceData> workspaces,
    List<ViewData> views,
  );
  CloudPageTreeCache? cachedCloudPageTree(String serverUrl);
  DocData? openCloudDocMirror(String docId);
  void purgeView(String viewId, String objectId);
  ({String workspaceId, int docs})? detachCloudWorkspace(
    String serverUrl,
    String cloudWorkspaceId,
    String name,
  );

  ({String docId, String rootBlockId, List<Map<String, dynamic>> blocks})
  newDoc();
  DocData? openDoc(String docId);
  Future<void> applyOps(List<Map<String, dynamic>> ops);
  Future<VaultImportResult> importVaultTree(
    List<({String path, List<int> bytes})> entries,
    String workspaceId, {
    String? parentViewId,
  });
  void rollbackDoc(String docId);
  void flush();

  String putBlob(Uint8List bytes);
  void putBlobAs(String fileId, Uint8List bytes);
  Uint8List? loadBlob(String fileId);
  bool hasBlob(String fileId);
  String? blobFileUri(String fileId);

  List<({String id, String? label, int createdAt})> listDocVersions(
    String docId,
  );
  ({String id, String? label, int createdAt})? createDocVersion(
    String docId,
    String label,
  );
  ({String rootBlockId, List<Map<String, dynamic>> blocks})? docVersionContent(
    String docId,
    String versionId,
  );
  bool restoreDocVersion(String docId, String versionId);

  String? exportDocHtml(String docId, String title, {int contentWidth});
  ({Uint8List bytes, String name, String mime})? exportDocMarkdown(
    String docId,
    String base,
  );
  Uint8List? exportFolderZip(String workspaceId, String folderId);
  Future<Uint8List?> htmlToPdf(String html);
}
