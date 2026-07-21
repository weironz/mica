// Data models and domain enums for the Mica client: auth, workspaces,
// views, documents, blocks, presence. Pure data — no widgets, no I/O.
// Extracted from main.dart (2026-07) and re-exported by it, so existing
// `import 'main.dart'` users keep seeing these symbols.
import 'dart:convert';

import 'package:flutter/material.dart';

import '../l10n/locale_controller.dart';
import '../local/local_offline.dart' show CloudPageTreeCache;

enum AuthMode { login, register }

enum WorkspaceRole {
  admin('admin'),
  editor('editor'),
  commenter('commenter'),
  viewer('viewer');

  const WorkspaceRole(this.apiValue);

  final String apiValue;

  static WorkspaceRole fromApiValue(String value) {
    return WorkspaceRole.values.firstWhere(
      (role) => role.apiValue == value,
      orElse: () => WorkspaceRole.viewer,
    );
  }
}

bool matchesManageRole(String? role) {
  return role == 'owner' || role == 'admin';
}

bool matchesEditRole(String? role) {
  return role == 'owner' || role == 'admin' || role == 'editor';
}

class AuthFormValue {
  const AuthFormValue({
    required this.email,
    required this.displayName,
    required this.password,
  });

  final String email;
  final String displayName;
  final String password;
}

/// Progress of a server-side workspace import job.
class ImportJobStatus {
  const ImportJobStatus({
    required this.status,
    required this.total,
    required this.done,
    this.workspaceId,
    this.error,
  });

  factory ImportJobStatus.fromJson(Map<String, dynamic> json) {
    return ImportJobStatus(
      status: json['status'] as String? ?? 'running',
      total: (json['total'] as num?)?.toInt() ?? 0,
      done: (json['done'] as num?)?.toInt() ?? 0,
      workspaceId: json['workspace_id'] as String?,
      error: json['error'] as String?,
    );
  }

  final String status; // running | done | error
  final int total;
  final int done;
  final String? workspaceId;
  final String? error;
}

class UploadedFile {
  const UploadedFile({
    required this.id,
    required this.name,
    required this.mimeType,
    required this.downloadUrl,
  });

  /// Parse a `{file: {...}, download_url}` payload from presign-complete/resolve.
  factory UploadedFile.fromResponse(Map<String, dynamic> json) {
    final file = json['file'] as Map<String, dynamic>;
    return UploadedFile(
      id: file['id'] as String,
      name: file['original_name'] as String? ?? '',
      mimeType: file['mime_type'] as String? ?? '',
      downloadUrl: json['download_url'] as String? ?? '',
    );
  }

  final String id;
  final String name;
  final String mimeType;
  final String downloadUrl;
}

class AuthSession {
  const AuthSession({
    required this.accessToken,
    required this.user,
    this.refreshToken = '',
  });

  factory AuthSession.fromJson(Map<String, dynamic> json) {
    return AuthSession(
      accessToken: json['access_token'] as String,
      // Absent when talking to a server older than refresh tokens: the session
      // then behaves exactly as it always did — good until the access token
      // expires, then a re-login.
      refreshToken: (json['refresh_token'] as String?) ?? '',
      user: User.fromJson(json['user'] as Map<String, dynamic>),
    );
  }

  final String accessToken;

  /// Spends on `/auth/refresh` for a fresh [accessToken]. **Single-use**: the
  /// server rotates it on every refresh and treats a second spend of the same
  /// token as a theft, killing the whole sign-in. So whoever refreshes must
  /// persist the replacement, and only one refresh may ever be in flight.
  final String refreshToken;

  final User user;

  /// When [accessToken] dies, read from the token itself rather than kept as
  /// separate state that could drift from it.
  DateTime? get expiresAt => jwtExpiry(accessToken);

  AuthSession copyWith({String? accessToken, String? refreshToken, User? user}) {
    return AuthSession(
      accessToken: accessToken ?? this.accessToken,
      refreshToken: refreshToken ?? this.refreshToken,
      user: user ?? this.user,
    );
  }
}

/// What the sidebar's account tile says: the identity of the world you are
/// LOOKING AT, not "do you happen to hold a session".
///
/// The local world has no account — it is files on this device, owned by
/// nobody — so showing the cloud account there claims you are editing them as
/// that person, which isn't true. The old predicate (`session?.x ?? '本地'`)
/// asked the wrong question: its fallback only ever fired for someone who had
/// never signed in at all, so anyone who had signed in once saw their cloud
/// identity forever, in every world.
///
/// A top-level function so a test can drive THIS, rather than a copy of it
/// living in the test file — a copy stays green while the shipped code drifts,
/// which is exactly how the first attempt at this fix passed its tests while
/// changing the wrong screen entirely.
({String name, String? email, bool canSignOut, bool canSignIn}) accountIdentity({
  required bool local,
  required User? user,
}) {
  if (local) {
    // No sign-out (you are not signed in HERE) and no sign-in (there is no
    // server in this world to sign in to — that is a choice made in Settings).
    return (
      name: l10nNoContext.identityLocalName,
      email: l10nNoContext.identityLocalDevice,
      canSignOut: false,
      canSignIn: false,
    );
  }
  if (user == null) {
    return (
      name: l10nNoContext.identityNotSignedIn,
      email: null,
      canSignOut: false,
      canSignIn: true,
    );
  }
  final name = user.displayName.isNotEmpty ? user.displayName : user.email;
  return (name: name, email: user.email, canSignOut: true, canSignIn: false);
}

/// The `exp` claim of a JWT, or null if it has none / isn't one.
///
/// Not a security check — anyone can write any `exp` — the server is what
/// enforces it. This is only so the client can renew *before* being refused,
/// instead of discovering the expiry as a failed request.
DateTime? jwtExpiry(String token) {
  try {
    final parts = token.split('.');
    if (parts.length != 3) return null;
    var payload = parts[1].replaceAll('-', '+').replaceAll('_', '/');
    payload = payload.padRight((payload.length + 3) ~/ 4 * 4, '=');
    final map =
        jsonDecode(utf8.decode(base64.decode(payload))) as Map<String, dynamic>;
    final exp = map['exp'];
    if (exp is! int) return null;
    return DateTime.fromMillisecondsSinceEpoch(exp * 1000, isUtc: true);
  } catch (_) {
    return null;
  }
}

class User {
  const User({
    required this.id,
    required this.email,
    required this.displayName,
  });

  factory User.fromJson(Map<String, dynamic> json) {
    return User(
      id: json['id'] as String,
      email: json['email'] as String,
      displayName: json['display_name'] as String,
    );
  }

  final String id;
  final String email;
  final String displayName;

  Map<String, dynamic> toJson() => {
    'id': id,
    'email': email,
    'display_name': displayName,
  };
}

class Workspace {
  const Workspace({
    required this.id,
    required this.name,
    required this.ownerId,
    required this.role,
  });

  factory Workspace.fromJson(Map<String, dynamic> json) {
    return Workspace(
      id: json['id'] as String,
      name: json['name'] as String,
      ownerId: json['owner_id'] as String,
      role: json['role'] as String,
    );
  }

  final String id;
  final String name;
  final String ownerId;
  final String role;
}

/// A workspace's globally-unique reference: [origin] is `'local'` or a server
/// URL — the store's origin semantics (P1b-2′), now also the client's (P3).
typedef WorkspaceRef = ({String origin, String id});

/// One entry in the unified workspace list (P3): a workspace plus its
/// provenance. Local and cloud workspaces coexist in one list, each carrying
/// where it lives; the UI groups by [origin] and handlers dispatch on it.
class WorkspaceEntry {
  const WorkspaceEntry({
    required this.origin,
    required this.workspace,
    required this.role,
  });

  /// `'local'` or the server URL this workspace lives on.
  final String origin;
  final Workspace workspace;

  /// The user's membership role — `'owner'` for local workspaces (they are the
  /// user's own), the server/mirrored role for cloud ones (P2d).
  final String role;

  bool get isLocal => origin == 'local';
  WorkspaceRef get ref => (origin: origin, id: workspace.id);
}

class WorkspaceMember {
  const WorkspaceMember({
    required this.userId,
    required this.email,
    required this.displayName,
    required this.role,
  });

  factory WorkspaceMember.fromJson(Map<String, dynamic> json) {
    return WorkspaceMember(
      userId: json['user_id'] as String,
      email: json['email'] as String,
      displayName: json['display_name'] as String,
      role: json['role'] as String,
    );
  }

  final String userId;
  final String email;
  final String displayName;
  final String role;
}

class SearchResult {
  const SearchResult({
    required this.viewId,
    required this.objectId,
    required this.name,
    required this.snippet,
    required this.titleMatch,
  });

  factory SearchResult.fromJson(Map<String, dynamic> json) {
    return SearchResult(
      viewId: json['view_id'] as String,
      objectId: json['object_id'] as String,
      name: json['name'] as String? ?? 'Untitled',
      snippet: json['snippet'] as String? ?? '',
      titleMatch: json['title_match'] == true,
    );
  }

  final String viewId;
  final String objectId;
  final String name;
  final String snippet;
  final bool titleMatch;
}

/// A named, restorable checkpoint of a document (the server's version history).
/// One entry in a document's yrs-native version timeline. `label` is null for an
/// AUTO snapshot (captured on a cadence); set for a named user checkpoint.
class DocVersion {
  const DocVersion({
    required this.id,
    required this.label,
    required this.createdAt,
  });

  factory DocVersion.fromJson(Map<String, dynamic> json) {
    return DocVersion(
      id: json['id'] as String,
      label: json['label'] as String?,
      createdAt: json['created_at'] as String? ?? '',
    );
  }

  final String id;
  final String? label;
  final String createdAt;

  /// An auto snapshot has no user-given name (shown by timestamp instead).
  bool get isAuto => label == null || label!.trim().isEmpty;
}

/// Result of moving/copying a view's subtree into another workspace (the
/// server's `/transfer` endpoint). Counts are what the copy touched; a
/// `dryRun` report predicts them without mutating. [danglingLinks] names the
/// documents that hold a link pointing at a page OUTSIDE the transferred
/// subtree — those links break, since the target does not travel along.
class TransferReport {
  const TransferReport({
    required this.newRootViewId,
    required this.documents,
    required this.folders,
    required this.images,
    required this.danglingLinks,
    required this.removedSource,
    required this.dryRun,
  });

  factory TransferReport.fromJson(Map<String, dynamic> json) {
    // Keep only the document name from each dangling-link record — that's all
    // the UI shows ("these pages have links that will break"); the target view
    // id is server-side detail we don't surface.
    final links = (json['dangling_links'] as List<dynamic>? ?? const [])
        .map((e) => (e as Map<String, dynamic>)['document'] as String? ?? '')
        .where((name) => name.isNotEmpty)
        .toList();
    return TransferReport(
      newRootViewId: json['new_root_view_id'] as String?,
      documents: (json['documents'] as num?)?.toInt() ?? 0,
      folders: (json['folders'] as num?)?.toInt() ?? 0,
      images: (json['images'] as num?)?.toInt() ?? 0,
      danglingLinks: links,
      removedSource: json['removed_source'] == true,
      dryRun: json['dry_run'] == true,
    );
  }

  final String? newRootViewId;
  final int documents;
  final int folders;
  final int images;
  final List<String> danglingLinks;
  final bool removedSource;
  final bool dryRun;
}

/// Result of duplicating a view within its own workspace (the server's `/clone`
/// endpoint). [newName] is the sibling-deduped name the copy actually got.
class CloneReport {
  const CloneReport({
    required this.newRootViewId,
    required this.newName,
    required this.documents,
    required this.folders,
    required this.dryRun,
  });

  factory CloneReport.fromJson(Map<String, dynamic> json) {
    return CloneReport(
      newRootViewId: json['new_root_view_id'] as String?,
      newName: json['new_name'] as String? ?? '',
      documents: (json['documents'] as num?)?.toInt() ?? 0,
      folders: (json['folders'] as num?)?.toInt() ?? 0,
      dryRun: json['dry_run'] == true,
    );
  }

  final String? newRootViewId;
  final String newName;
  final int documents;
  final int folders;
  final bool dryRun;
}

class DocumentCreateResult {
  const DocumentCreateResult({required this.document, required this.view});

  factory DocumentCreateResult.fromJson(Map<String, dynamic> json) {
    return DocumentCreateResult(
      document: DocumentRecord.fromJson(
        json['document'] as Map<String, dynamic>,
      ),
      view: DocumentView.fromJson(json['view'] as Map<String, dynamic>),
    );
  }

  final DocumentRecord document;
  final DocumentView view;
}

class DocumentUpdateResult {
  const DocumentUpdateResult({required this.document, required this.snapshot});

  factory DocumentUpdateResult.fromJson(Map<String, dynamic> json) {
    return DocumentUpdateResult(
      document: DocumentRecord.fromJson(
        json['document'] as Map<String, dynamic>,
      ),
      snapshot: DocumentSnapshot.fromJson(
        json['snapshot'] as Map<String, dynamic>,
      ),
    );
  }

  final DocumentRecord document;
  final DocumentSnapshot snapshot;
}

class DocumentBootstrap {
  const DocumentBootstrap({
    required this.document,
    required this.view,
    required this.snapshot,
  });

  factory DocumentBootstrap.fromJson(Map<String, dynamic> json) {
    return DocumentBootstrap(
      document: DocumentRecord.fromJson(
        json['document'] as Map<String, dynamic>,
      ),
      view: DocumentView.fromJson(json['view'] as Map<String, dynamic>),
      snapshot: DocumentSnapshot.fromJson(
        json['snapshot'] as Map<String, dynamic>,
      ),
    );
  }

  final DocumentRecord document;
  final DocumentView view;
  final DocumentSnapshot snapshot;

  String get rootBlockText {
    return _blocksById[document.rootBlockId]?.text ?? '';
  }

  List<DocumentBlock> get childBlocks {
    final root = _blocksById[document.rootBlockId];
    if (root == null) {
      return const [];
    }

    return root.children
        .map((childId) => _blocksById[childId])
        .whereType<DocumentBlock>()
        .toList();
  }

  Map<String, DocumentBlock> get _blocksById {
    final blocks = snapshot.payload['blocks'];
    if (blocks is! List<dynamic>) {
      return const {};
    }

    return {
      for (final block in blocks)
        if (block is Map<String, dynamic>)
          DocumentBlock.fromJson(block).id: DocumentBlock.fromJson(block),
    };
  }
}

class DocumentBlock {
  const DocumentBlock({
    required this.id,
    required this.kind,
    required this.text,
    required this.data,
    required this.children,
  });

  factory DocumentBlock.fromJson(Map<String, dynamic> json) {
    final children = json['children'];
    final data = json['data'];
    return DocumentBlock(
      id: json['id'] as String,
      kind: json['type'] as String? ?? 'paragraph',
      text: json['text'] as String? ?? '',
      data: data is Map<String, dynamic> ? data : const {},
      children: children is List<dynamic>
          ? children.whereType<String>().toList()
          : const [],
    );
  }

  final String id;
  final String kind;
  final String text;
  final Map<String, dynamic> data;
  final List<String> children;
}

enum DocumentBlockKind {
  paragraph('paragraph', 'Paragraph'),
  heading('heading', 'Heading'),
  todo('todo', 'Todo'),
  bulletedList('bulleted_list', 'Bulleted list'),
  numberedList('numbered_list', 'Numbered list'),
  quote('quote', 'Quote'),
  codeBlock('code_block', 'Code block');

  const DocumentBlockKind(this.apiValue, this.label);

  final String apiValue;
  final String label;

  static DocumentBlockKind fromApiValue(String value) {
    return DocumentBlockKind.values.firstWhere(
      (kind) => kind.apiValue == value,
      orElse: () => DocumentBlockKind.paragraph,
    );
  }
}

class DocumentView {
  const DocumentView({
    required this.id,
    required this.parentViewId,
    required this.objectId,
    required this.objectType,
    required this.name,
    required this.position,
  });

  factory DocumentView.fromJson(Map<String, dynamic> json) {
    return DocumentView(
      id: json['id'] as String,
      parentViewId: json['parent_view_id'] as String?,
      objectId: json['object_id'] as String,
      objectType: json['object_type'] as String,
      name: json['name'] as String,
      position: json['position'] as String,
    );
  }

  final String id;
  final String? parentViewId;
  final String objectId;
  final String objectType;
  final String name;
  final String position;
}

/// Rebuild the cloud workspace list + per-workspace views from an on-device
/// page-tree mirror ([CloudPageTreeCache]) when the server is unreachable —
/// the P1c offline-read reconstruction. `role` is the real mirrored membership
/// role (P2d), so an editor can edit a cached cloud doc offline (its edits queue
/// in the append-log outbox and push on reconnect); a viewer stays read-only via
/// the existing `matchesEditRole` gate. `objectType` is carried through from the
/// mirror (F3 — so a mirrored folder stays a folder offline); only `ownerId` is
/// defaulted to [ownerId] (the current user), the real value returning on the
/// next successful online load. Views are grouped by their `workspaceId` and keep
/// the mirror's position order. Pure + testable.
({List<Workspace> workspaces, Map<String, List<DocumentView>> views})
rebuildCloudNavFromCache(CloudPageTreeCache cache, String ownerId) {
  final workspaces = [
    for (final w in cache.workspaces)
      Workspace(id: w.id, name: w.name, ownerId: ownerId, role: w.role),
  ];
  final views = <String, List<DocumentView>>{};
  for (final v in cache.views) {
    (views[v.workspaceId] ??= <DocumentView>[]).add(
      DocumentView(
        id: v.id,
        parentViewId: v.parentId,
        objectId: v.objectId,
        objectType: v.objectType,
        name: v.name,
        position: v.position,
      ),
    );
  }
  return (workspaces: workspaces, views: views);
}

/// The first view worth auto-opening: a folder is a pure container with no
/// content to bootstrap (opening one would 404 on its unbacked object_id), so
/// every auto-open path — online, offline mirror, and local — skips folders and
/// lands on the first document. Shared + testable so the three worlds can't
/// drift back to `.firstOrNull` (which would land on a folder → blank editor).
/// Does tapping [targetViewId] need a bootstrap, or is that page already up?
///
/// Re-tapping the open row used to re-run the whole load. Two taps inside the
/// double-click window then had two bootstraps in flight; the loser hit a
/// disposed session, fell back to the on-device mirror, got nothing (a doc
/// opened online has no mirror) and blanked the page being read. Switching away
/// and back reloaded it — which is exactly why the content always "came back"
/// and the bug looked like a rename gone wrong.
///
/// Pure + named so the guard reads as a rule instead of an inlined `&&` that
/// the next person deletes as redundant.
bool needsBootstrapOnSelect({
  required String? openViewId,
  required bool openViewHasContent,
  required String targetViewId,
}) => !(openViewId == targetViewId && openViewHasContent);

/// May a finished load write its result over what is on screen?
///
/// No, when it has nothing and the same page is already rendering: an empty
/// pane is strictly worse than content that is merely a moment old. This only
/// ever declines to blank a view that had something to lose.
bool mayReplaceBootstrap({
  required bool haveNewBootstrap,
  required bool wasShowingSameView,
}) => haveNewBootstrap || !wasShowingSameView;

DocumentView? firstOpenableView(Iterable<DocumentView> views) =>
    views.where((v) => v.objectType == 'document').firstOrNull;

/// The default name a freshly-created page carries (the server rejects empty
/// view names, so a new page must be named). The title field renders this — and
/// the legacy English 'Untitled' — as an empty placeholder so a new page shows a
/// grey hint + caret rather than solid, pre-selected text.
const String kUntitledPage = '未命名页面';

/// True when [name] is an untouched default page name (new page never renamed),
/// so the title field should show its placeholder instead of the literal text.
bool isUntitledPageName(String name) {
  final t = name.trim();
  return t == kUntitledPage || t == 'Untitled';
}

/// Whether a view may be nested under [parentId] (null = workspace root). A page
/// is a leaf: nothing may live under a document — only a folder (or the root)
/// accepts children. Pure + testable; shared by the drag-drop gate so it stays
/// consistent with the menu (which offers "new child" on folders only). Existing
/// or imported document-with-children still renders; this only blocks NEW nesting.
bool canNestUnder(Iterable<DocumentView> views, String? parentId) {
  if (parentId == null) return true; // workspace root always accepts children
  final parent = views.where((v) => v.id == parentId).firstOrNull;
  return parent != null && parent.objectType == 'folder';
}

/// The view a new page/folder should be created UNDER, given the sidebar node
/// the user has "located". A folder holds children → create INSIDE it; a page is
/// a leaf → create BESIDE it (under its own parent, so it lands in the same
/// group in order); nothing located → the workspace root (null). Pure + testable
/// core of the top-of-sidebar New page/folder buttons.
DocumentView? createParentForLocated(
  Iterable<DocumentView> views,
  DocumentView? located,
) {
  if (located == null) return null;
  if (located.objectType == 'folder') return located;
  final parentId = located.parentViewId;
  if (parentId == null) return null;
  return views.where((v) => v.id == parentId).firstOrNull;
}

/// The ids of [rootId] plus all its descendants, given parent-linked [nodes].
/// Pure + testable core of the local delete/restore/purge subtree cascade — the
/// part that must walk the WHOLE subtree (not just direct children) so trashing a
/// folder carries its deep descendants, matching the server's recursive-CTE
/// handlers. Returns empty if [rootId] isn't present; cycle-safe.
/// The ids of every ANCESTOR of [id] (walking parentViewId up). Pure + testable
/// core of the sidebar "reveal a nested node" logic — expanding these makes a
/// deep node visible. Cycle-safe; returns empty if [id] is a root/unknown.
Set<String> ancestorIds(Iterable<DocumentView> views, String id) {
  final parents = {for (final v in views) v.id: v.parentViewId};
  final out = <String>{};
  var cursor = parents[id];
  while (cursor != null && out.add(cursor)) {
    cursor = parents[cursor];
  }
  return out;
}

Set<String> collectSubtreeIds(
  Iterable<({String id, String? parentId})> nodes,
  String rootId,
) {
  final byParent = <String?, List<String>>{};
  var hasRoot = false;
  for (final n in nodes) {
    (byParent[n.parentId] ??= <String>[]).add(n.id);
    if (n.id == rootId) hasRoot = true;
  }
  if (!hasRoot) return <String>{};
  final out = <String>{};
  final stack = <String>[rootId];
  while (stack.isNotEmpty) {
    final id = stack.removeLast();
    if (!out.add(id)) continue; // already visited → cycle guard
    final kids = byParent[id];
    if (kids != null) stack.addAll(kids);
  }
  return out;
}

class DocumentRecord {
  const DocumentRecord({
    required this.id,
    required this.rootBlockId,
    required this.currentSeq,
  });

  factory DocumentRecord.fromJson(Map<String, dynamic> json) {
    return DocumentRecord(
      id: json['id'] as String,
      rootBlockId: json['root_block_id'] as String,
      currentSeq: json['current_seq'] as int,
    );
  }

  final String id;
  final String rootBlockId;
  final int currentSeq;
}

class DocumentSnapshot {
  const DocumentSnapshot({
    required this.versionSeq,
    required this.schemaVersion,
    required this.payload,
  });

  factory DocumentSnapshot.fromJson(Map<String, dynamic> json) {
    return DocumentSnapshot(
      versionSeq: json['version_seq'] as int,
      schemaVersion: json['schema_version'] as int,
      payload: json['payload'] as Map<String, dynamic>,
    );
  }

  final int versionSeq;
  final int schemaVersion;
  final Map<String, dynamic> payload;
}

class ApiException implements Exception {
  const ApiException(this.message, {this.statusCode});

  final String message;

  /// The HTTP status, when the failure came from a response. Null for the
  /// hand-thrown cases (no server involved).
  ///
  /// Carried because the server's *message* is not something a person should be
  /// shown or reasoned about: dropping the status is why an expired session
  /// surfaced as a bare `unauthorized` banner with no hint that signing in again
  /// was the fix.
  final int? statusCode;

  /// The session is gone (expired / revoked / the JWT secret rotated), as
  /// opposed to any other failure. See `_MicaAppState._endExpiredSession`.
  bool get isUnauthorized => statusCode == 401;

  @override
  String toString() => message;
}

/// Stable collaborator colors (avatar + remote caret share one per connection).
const List<Color> kPresencePalette = [
  Color(0xFF2563EB),
  Color(0xFF16A34A),
  Color(0xFFDB2777),
  Color(0xFFD97706),
  Color(0xFF7C3AED),
  Color(0xFF0891B2),
];

Color presenceColor(String connectionId) =>
    kPresencePalette[connectionId.hashCode.abs() % kPresencePalette.length];

/// A collaborator currently connected to the same document room, with their
/// live caret position (block id + UTF-16 offset) for awareness rendering.
class PresenceUser {
  const PresenceUser({
    required this.connectionId,
    required this.userId,
    required this.name,
    this.cursorBlockId,
    this.cursorOffset,
  });

  final String connectionId;
  final String userId;
  final String name;
  final String? cursorBlockId;
  final int? cursorOffset;

  Color get color => presenceColor(connectionId);
  bool get hasCursor => cursorBlockId != null && cursorOffset != null;
}
