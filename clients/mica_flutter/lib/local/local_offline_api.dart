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
import 'cache_stats.dart';
import 'dart:typed_data';
import '../editor/operation_history.dart';

import '../cloud/cloud_doc_store.dart';
import '../api/models.dart';

/// Thrown by the local backend when a document's on-device snapshot is PRESENT
/// but corrupt/unreadable (distinct from simply absent). The app surfaces it
/// and offers rollback / version history instead of clobbering the doc with a
/// fresh blank page (which would also destroy the §10 recovery checkpoint).
class LocalDocCorruptException implements Exception {
  LocalDocCorruptException(this.docId);
  final String docId;
  @override
  String toString() => 'LocalDocCorruptException($docId)';
}

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
typedef WorkspaceData = ({
  String id,
  String name,
  String position,
  String role,
});

/// A mirrored page tree read back from the store for one `origin` (a server
/// URL) — workspaces + views, for offline navigation (P2 option C, P1c).
typedef CloudPageTreeCache = ({
  List<WorkspaceData> workspaces,
  List<ViewData> views,
});

/// A loaded document: its root block id and full block list (snapshot payload).
typedef DocData = ({String rootBlockId, List<Map<String, dynamic>> blocks});

/// Outcome of a vault import (S-tier read-only scan): documents + folder-pages
/// created, any per-file errors (unreadable files, etc.), and the id of every
/// view the import created in plan order. The ids are what lets a single-page
/// Markdown import open the page it just made.
typedef VaultImportResult = ({
  int docs,
  int folders,
  List<String> errors,
  List<String> viewIds,
});

/// Every public member of the local-offline facade. Abstract members carry no
/// default parameter values — each variant declares its own (they must agree
/// by convention; a differing default is behavior the web stub doesn't have
/// anyway).
abstract interface class LocalOfflineApi {
  bool get available;
  Future<void> open();
  Future<BigInt?> deviceClientId();

  /// The durable per-document mirror for a cloud document, scoped to [origin]
  /// (see `cloudCacheOrigin`) so two accounts on one server cannot read each
  /// other's replica, outbox or cursor.
  CloudDocStore? cloudDocStore(String origin, String docId);

  List<WorkspaceData> listWorkspaces({String origin});
  void saveWorkspace(WorkspaceData w, {String origin});
  void reorderWorkspaces(List<String> ids, {String origin});
  void forgetOrigin(String origin);
  List<ViewData> listViews({String origin});

  /// Search local workspace [workspaceId]'s pages by title and body.
  ///
  /// Scoped, like the two below: the store holds every local workspace AND the
  /// cloud mirrors, and an unscoped query answered with all of them.
  ///
  /// Async where most of this facade is sync: it reads every indexed document,
  /// and the one thing a search box must not do is freeze the frame being typed
  /// into. Empty on web — there is no on-device store there at all, which is a
  /// different thing from "found nothing" and the caller has to keep telling
  /// them apart (see `onSearch: null` in main.dart).
  Future<List<SearchResult>> searchLocal(
    String workspaceId,
    String query, {
    int limit,
  });

  /// The pages in local workspace [workspaceId] that link TO [viewId] — the
  /// backlinks panel.
  ///
  /// Async for the same reason as [searchLocal], and empty on web for the same
  /// reason too: there is no on-device store there. Unlike search, the caller
  /// does NOT have to tell "no store" from "no backlinks" apart — a page with no
  /// backlinks renders nothing either way, so the panel simply stays hidden.
  Future<List<Backlink>> backlinksLocal(String workspaceId, String viewId);

  /// The page-link graph of local workspace [workspaceId], for the graph view.
  /// Empty on web — no on-device store.
  Future<PageGraph> graphLocal(String workspaceId);

  /// Index up to [limit] documents saved before the search projection existed.
  /// Returns how many it did; fewer than [limit] means the backlog is done.
  Future<int> backfillSearchIndex(int limit);

  /// What the on-device store is holding for [accountId] on [servers], split
  /// into re-downloadable mirrors and local-only originals. See
  /// `cache_stats.dart` for why the split matters.
  Future<LocalCacheStats> cacheStats({
    required List<String> servers,
    required String accountId,
  });

  /// Delete the re-downloadable half of this ACCOUNT's cache and report what is
  /// left. Documents that still hold un-pushed offline edits are KEPT (with
  /// their page/view rows, so the user can open and keep editing them) and
  /// reported via [ClearMirrorResult.retainedDocs]. Legacy server-only rows
  /// (pre-#111, owner unverifiable) are quarantined, never deleted, and listed
  /// in [ClearMirrorResult.quarantinedOrigins].
  Future<ClearMirrorResult> clearMirrorCache({
    required List<String> servers,
    required String accountId,
  });

  /// Legacy (pre-#111) caches that exist for [servers] under the old
  /// server-only key. They are NOT visible to any account — we cannot verify
  /// which account wrote them — but they are retained on disk and listed here
  /// (ids and page counts only, never names or bodies) so Settings can offer an
  /// explicit export instead of leaving the user with silently invisible data.
  List<({String origin, String workspaceId, int pages})> quarantinedWorkspaces({
    required List<String> servers,
  });
  /// Upsert a view row (create / rename / move / trash-toggle) in [origin].
  /// Web has no on-device tree; see the web variant for what it does there.
  void saveView(ViewData v, {String origin});

  /// Mirror [views] for the workspaces in [viewWorkspaceIds] (defaulting to the
  /// workspaces the views themselves mention). Workspaces NOT in that set keep
  /// their mirrored rows — a refresh of one workspace must not erase another's
  /// offline navigation (issue #109). An EMPTY incoming workspace needs its id
  /// in [viewWorkspaceIds] to have its stale rows cleared; it cannot be
  /// inferred from an empty view list.
  void mirrorCloudPageTree(
    String origin,
    List<WorkspaceData> workspaces,
    List<ViewData> views, {
    Set<String> viewWorkspaceIds,
  });
  CloudPageTreeCache? cachedCloudPageTree(String origin);
  DocData? openCloudDocMirror(String origin, String docId);
  void purgeView(String viewId, String objectId);

  /// Trash / restore / permanently remove a view AND its whole subtree,
  /// returning the ids touched so the caller can drop an open editor that was
  /// inside it. The cascade — and the "lift a restored root whose parent is
  /// still trashed" rule — live in Rust, so the local world and the server
  /// enforce one implementation instead of two.
  List<String> trashViewSubtree(String viewId);
  List<String> restoreViewSubtree(String viewId);
  List<String> purgeViewSubtree(String viewId);

  /// Create a page or folder row and return its id. Position is assigned by
  /// Rust (after the last live sibling, in steps of ten) — the same rule
  /// clone and reorder use, so all three agree on where things land.
  String createView({
    required String workspaceId,
    String? parentId,
    required String objectId,
    required String name,
    required String objectType,
  });

  /// Renumber [orderedIds] as consecutive children of [parentId], reparenting
  /// them if they came from elsewhere.
  void reorderViews(String? parentId, List<String> orderedIds);

  /// Create a local workspace and return its id (position assigned by Rust).
  String createLocalWorkspace(String name);

  /// Rename, keeping position and role.
  void renameLocalWorkspace(String id, String name);

  /// Delete a workspace with its views and documents. Returns false — and
  /// changes nothing — when it is the last one on the device, so the caller
  /// only has to decide what to SAY about it.
  bool deleteLocalWorkspace(String id);
  ({String workspaceId, int docs})? detachCloudWorkspace(
    String origin,
    String cloudWorkspaceId,
    String name,
  );

  ({String docId, String rootBlockId, List<Map<String, dynamic>> blocks})
  newDoc();
  DocData? openDoc(String docId);
  Future<void> applyOps(
    List<Map<String, dynamic>> ops, {
    bool trackHistory = false,
  });
  Future<EditorHistoryState> applyHistory(HistoryAction action);
  Future<VaultImportResult> importVaultTree(
    List<({String path, List<int> bytes})> entries,
    String workspaceId, {
    String? parentViewId,
  });

  /// Import ONE Markdown file as a new page in [workspaceId] — the local
  /// counterpart of the cloud `importMarkdown` (issue #108). Returns the same
  /// outcome as [importVaultTree], including the created view id.
  Future<VaultImportResult> importMarkdownPage(
    String fileName,
    String markdown,
    String workspaceId, {
    required String fallbackTitle,
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

  /// The page's Markdown TEXT, with no packaging decision — for the clipboard.
  /// Distinct from [exportDocMarkdown], which switches to a ZIP once the page
  /// has images: a clipboard cannot carry an assets folder.
  String? exportDocMarkdownText(String docId, String title);

  /// Write a page's title into its local document — the offline counterpart of
  /// what the server does inside PATCH /views/{id}. See the io implementation.
  void setDocTitle(String docId, String title);
  /// Export one folder's subtree as a Markdown ZIP (folder + pages + images).
  Uint8List? exportFolderZip(String workspaceId, String folderId);

  /// Export a WHOLE workspace as a Markdown ZIP — the same Rust tree builder
  /// folder/page export use. [origin] defaults to `'local'`; a quarantined
  /// legacy origin can be exported the same way for explicit recovery. Never
  /// returns an empty archive for a workspace that has pages; the UI treats
  /// empty as a failure (issue #108).
  Uint8List? exportWorkspaceZip(String workspaceId, {String origin});
  Future<Uint8List?> htmlToPdf(String html);
}

/// Sniff an image's MIME from its magic bytes — the local blob CAS keys by
/// sha256 and keeps no extension, so a `data:` URI (or a cloud upload during
/// migration) has nowhere else to learn the type. Defaults to PNG (the format
/// editor paste/upload produces).
String sniffImageMime(Uint8List b) {
  if (b.length >= 3 && b[0] == 0xFF && b[1] == 0xD8 && b[2] == 0xFF) {
    return 'image/jpeg';
  }
  if (b.length >= 4 && b[0] == 0x47 && b[1] == 0x49 && b[2] == 0x46) {
    return 'image/gif';
  }
  if (b.length >= 12 &&
      b[0] == 0x52 &&
      b[1] == 0x49 &&
      b[2] == 0x46 &&
      b[8] == 0x57 &&
      b[9] == 0x45 &&
      b[10] == 0x42 &&
      b[11] == 0x50) {
    return 'image/webp';
  }
  if (b.isNotEmpty && b[0] == 0x3C) {
    return 'image/svg+xml';
  }
  return 'image/png';
}
