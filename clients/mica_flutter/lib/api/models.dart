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
/// One row of the import history: a stored job plus when it started.
///
/// The job itself outlives the process that ran it, so this is what the
/// settings screen lists — including imports that were interrupted by a deploy,
/// which the in-memory map could never have shown.
class ImportHistoryEntry {
  const ImportHistoryEntry({
    required this.jobId,
    required this.startedAt,
    required this.job,
  });

  factory ImportHistoryEntry.fromJson(Map<String, dynamic> json) =>
      ImportHistoryEntry(
        jobId: json['job_id'] as String? ?? '',
        startedAt:
            DateTime.tryParse(json['created_at'] as String? ?? '')?.toLocal(),
        job: ImportJobStatus.fromJson(json),
      );

  final String jobId;
  final DateTime? startedAt;
  final ImportJobStatus job;
}

class ImportJobStatus {
  const ImportJobStatus({
    required this.status,
    required this.total,
    required this.done,
    this.workspaceId,
    this.error,
    this.skipped = const [],
    this.skippedTotal = 0,
    this.imageFailures = const [],
    this.imageFailuresTotal = 0,
  });

  factory ImportJobStatus.fromJson(Map<String, dynamic> json) {
    return ImportJobStatus(
      status: json['status'] as String? ?? 'running',
      total: (json['total'] as num?)?.toInt() ?? 0,
      done: (json['done'] as num?)?.toInt() ?? 0,
      workspaceId: json['workspace_id'] as String?,
      error: json['error'] as String?,
      skipped: (json['skipped'] as List<dynamic>?)?.cast<String>() ?? const [],
      skippedTotal: (json['skipped_total'] as num?)?.toInt() ?? 0,
      // Absent on a server older than migration 0024 — an empty list, which
      // reads as "nothing recorded", not "nothing failed". Those are genuinely
      // different and only the server can tell them apart.
      imageFailures:
          (json['image_failures'] as List<dynamic>?)
              ?.whereType<Map<String, dynamic>>()
              .map(ImportImageFailure.fromJson)
              .toList() ??
          const [],
      imageFailuresTotal: (json['image_failures_total'] as num?)?.toInt() ?? 0,
    );
  }

  /// `running` | `done` | `error` | `cancelled` | `interrupted`.
  ///
  /// `interrupted` means the server restarted mid-import (migration 0023). It
  /// is neither a failure nor a completion: nothing went wrong, and the pages
  /// already written are real and staying — the archive just is not all in.
  final String status;
  final int total;
  final int done;
  final String? workspaceId;
  final String? error;

  /// Archive entries no imported page referenced — the assets that silently did
  /// not make it. Capped by the server; [skippedTotal] is the real count.
  final List<String> skipped;

  /// How many were skipped in total, which can exceed `skipped.length`.
  final int skippedTotal;

  /// Images the server was asked to bring into Mica and could not — those
  /// blocks are still links to somebody else's server. Capped;
  /// [imageFailuresTotal] is the real count.
  final List<ImportImageFailure> imageFailures;

  /// How many images were left as links in total.
  final int imageFailuresTotal;
}

/// One image an import left pointing outside Mica.
///
/// This is the record that was missing when a whole AppFlowy export came in
/// with its images still borrowed: the server could not reach that host (it
/// blocks datacenter IPs), the blocks kept their links, the import reported
/// success, and the source workspace was deleted a few minutes later. The list
/// exists so that never happens silently again — and so the client can retry
/// each one over ITS network, which usually can reach the host the server
/// could not.
class ImportImageFailure {
  const ImportImageFailure({
    required this.url,
    required this.page,
    required this.documentId,
    required this.blockId,
    required this.reason,
    required this.attempted,
  });

  factory ImportImageFailure.fromJson(Map<String, dynamic> json) {
    return ImportImageFailure(
      url: json['url'] as String? ?? '',
      page: json['page'] as String? ?? '',
      documentId: json['document_id'] as String? ?? '',
      blockId: json['block_id'] as String? ?? '',
      reason: json['reason'] as String? ?? '',
      // Defaults TRUE: an old or partial record claiming "never attempted"
      // would send the reader looking for a network problem that may not exist.
      attempted: json['attempted'] as bool? ?? true,
    );
  }

  final String url;

  /// The page it sits on — a list of bare urls is not something anyone can act
  /// on.
  final String page;

  /// The document + block to patch. Exactly what `rehost-image` takes.
  final String documentId;
  final String blockId;

  final String reason;

  /// False when the server's circuit breaker refused it WITHOUT a request,
  /// because that host had already timed out repeatedly. Worth distinguishing:
  /// it means the link was never actually tested, so a retry from here is more
  /// likely to work, not less.
  final bool attempted;

  /// True when this block still needs fixing — the id pair is what a retry
  /// posts to, and a record missing either cannot be retried at all.
  bool get isRetryable =>
      documentId.isNotEmpty && blockId.isNotEmpty && url.startsWith('http');
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

  AuthSession copyWith({
    String? accessToken,
    String? refreshToken,
    User? user,
  }) {
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
({String name, String? email, bool canSignOut, bool canSignIn})
accountIdentity({required bool local, required User? user}) {
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
    this.avatarVersion,
  });

  factory User.fromJson(Map<String, dynamic> json) {
    return User(
      id: json['id'] as String,
      email: json['email'] as String,
      displayName: json['display_name'] as String,
      avatarVersion: json['avatar_version'] as String?,
    );
  }

  final String id;
  final String email;
  final String displayName;

  /// Null means no profile picture. Not a URL — see `ui/avatar_url.dart` for why
  /// the address is composed in one place instead of being sent per payload.
  final String? avatarVersion;

  User withAvatarVersion(String? version) => User(
    id: id,
    email: email,
    displayName: displayName,
    avatarVersion: version,
  );

  Map<String, dynamic> toJson() => {
    'id': id,
    'email': email,
    'display_name': displayName,
    // Persisted with the session so a restart draws the picture on the first
    // frame instead of popping it in after /me returns.
    if (avatarVersion != null) 'avatar_version': avatarVersion,
  };
}

class Workspace {
  const Workspace({
    required this.id,
    required this.name,
    required this.ownerId,
    required this.role,
    this.pageCount = 0,
  });

  factory Workspace.fromJson(Map<String, dynamic> json) {
    return Workspace(
      id: json['id'] as String,
      name: json['name'] as String,
      ownerId: json['owner_id'] as String,
      role: json['role'] as String,
      pageCount: (json['page_count'] as num?)?.toInt() ?? 0,
    );
  }

  final String id;
  final String name;
  final String ownerId;
  final String role;

  /// Live pages in this workspace — folders and the recycle bin excluded, the
  /// same definition [countPages] uses on a loaded view tree.
  ///
  /// Only the workspace LIST endpoint fills this in; anything else (a local
  /// workspace, a single-workspace response, a server too old to send it) leaves
  /// it 0. So 0 means "not known here", not "empty" — a caller that wants to show
  /// it must decide what to do with 0 rather than print it.
  final int pageCount;
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
    this.avatarVersion,
  });

  factory WorkspaceMember.fromJson(Map<String, dynamic> json) {
    return WorkspaceMember(
      userId: json['user_id'] as String,
      email: json['email'] as String,
      displayName: json['display_name'] as String,
      role: json['role'] as String,
      avatarVersion: json['avatar_version'] as String?,
    );
  }

  final String userId;
  final String email;
  final String displayName;
  final String role;

  /// Null means this member has no picture — the row draws their initial.
  final String? avatarVersion;
}

class SearchResult {
  const SearchResult({
    required this.viewId,
    required this.objectId,
    required this.name,
    required this.snippet,
    required this.titleMatch,
    this.isFolder = false,
    this.parentViewId,
    this.workspaceId,
  });

  factory SearchResult.fromJson(Map<String, dynamic> json) {
    return SearchResult(
      viewId: json['view_id'] as String,
      objectId: json['object_id'] as String,
      name: json['name'] as String? ?? 'Untitled',
      snippet: json['snippet'] as String? ?? '',
      titleMatch: json['title_match'] == true,
      // Absent on a server older than this field — and absent has to read as
      // "page", because guessing "folder" would route a real page to the
      // handler that refuses to open it.
      isFolder: json['is_folder'] == true,
      parentViewId: json['parent_view_id'] as String?,
      // Null against a server older than this field. Callers must treat null as
      // "the workspace I searched", NOT as "unknown" — the per-workspace route
      // has always returned hits from exactly one workspace, so falling back to
      // the searched one is correct there and is the only thing that keeps this
      // client working against an older server.
      workspaceId: json['workspace_id'] as String?,
    );
  }

  final String viewId;
  final String objectId;
  final String name;
  final String snippet;
  final bool titleMatch;

  /// Which workspace the hit lives in; null from a server that predates it.
  ///
  /// Load-bearing for cross-workspace search: opening a page is workspace-scoped
  /// and this client only holds page trees for workspaces it has visited, so a
  /// hit from an unvisited workspace cannot be located any other way.
  final String? workspaceId;

  /// A folder rather than a page. Folders match on their NAME only — they carry
  /// no body — so a folder hit never has a snippet.
  final bool isFolder;

  /// The containing folder, or null at the workspace root.
  final String? parentViewId;
}

/// One page that links TO the page being viewed — the source of a reverse
/// reference. `viewId` is the source page's view (open it / follow a
/// `mica://page/<viewId>` link the same way a forward link resolves). Cloud
/// workspaces only; the local world has no backlinks endpoint.
class Backlink {
  const Backlink({
    required this.viewId,
    required this.documentId,
    required this.title,
  });

  factory Backlink.fromJson(Map<String, dynamic> json) {
    return Backlink(
      viewId: json['view_id'] as String,
      documentId: json['document_id'] as String,
      title: json['title'] as String? ?? 'Untitled',
    );
  }

  final String viewId;
  final String documentId;
  final String title;
}

/// The page-link graph of a workspace, for the graph view.
///
/// [unlinked] is a COUNT, not nodes. Most pages in a real library link to
/// nothing (a production snapshot: 798 documents, 136 with any link), so drawing
/// them would bury the structure the view exists to show — but omitting them
/// silently would misrepresent the workspace, hence the number.
class PageGraph {
  const PageGraph({
    required this.nodes,
    required this.edges,
    required this.unlinked,
  });

  factory PageGraph.fromJson(Map<String, dynamic> json) {
    return PageGraph(
      nodes: [
        for (final n in json['nodes'] as List<dynamic>)
          GraphNode.fromJson(n as Map<String, dynamic>),
      ],
      edges: [
        for (final e in json['edges'] as List<dynamic>)
          GraphEdge.fromJson(e as Map<String, dynamic>),
      ],
      unlinked: json['unlinked'] as int? ?? 0,
    );
  }

  static const empty = PageGraph(nodes: [], edges: [], unlinked: 0);

  final List<GraphNode> nodes;
  final List<GraphEdge> edges;
  final int unlinked;

  bool get isEmpty => nodes.isEmpty;
}

/// One page in the graph. [degree] is how many edges touch it (in + out) — the
/// view sizes hubs by it rather than counting edges itself.
class GraphNode {
  const GraphNode({
    required this.viewId,
    required this.name,
    required this.degree,
  });

  factory GraphNode.fromJson(Map<String, dynamic> json) {
    return GraphNode(
      viewId: json['view_id'] as String,
      name: json['name'] as String? ?? '',
      degree: json['degree'] as int? ?? 0,
    );
  }

  final String viewId;
  final String name;
  final int degree;
}

/// A link, in the direction it was written: [source] links to [target].
class GraphEdge {
  const GraphEdge({required this.source, required this.target});

  factory GraphEdge.fromJson(Map<String, dynamic> json) {
    return GraphEdge(
      source: json['source'] as String,
      target: json['target'] as String,
    );
  }

  final String source;
  final String target;
}

/// Where a comment sits in the document RIGHT NOW, resolved by the server from a
/// yrs sticky index. UTF-16 offsets, so they index Dart strings directly.
///
/// Null on a [CommentThread] means ORPHANED: the anchored text is gone, so the
/// thread is shown against its `quote` and NO highlight is drawn — never guess a
/// position (docs/comments-plan.md).
class CommentAnchorRange {
  const CommentAnchorRange({
    required this.startBlock,
    required this.startOffset,
    required this.endBlock,
    required this.endOffset,
  });

  factory CommentAnchorRange.fromJson(Map<String, dynamic> json) {
    return CommentAnchorRange(
      startBlock: json['start_block'] as String,
      startOffset: (json['start_offset'] as num).toInt(),
      endBlock: json['end_block'] as String,
      endOffset: (json['end_offset'] as num).toInt(),
    );
  }

  final String startBlock;
  final int startOffset;
  final String endBlock;
  final int endOffset;

  /// True when the range stays inside one block — the only case the editor can
  /// highlight with a single-block run today.
  bool get isSingleBlock => startBlock == endBlock;
}

/// One message in a comment thread. `body` is Markdown, like the document itself.
class CommentEntry {
  const CommentEntry({
    required this.id,
    required this.authorId,
    required this.body,
    required this.createdAt,
    this.editedAt,
  });

  factory CommentEntry.fromJson(Map<String, dynamic> json) {
    return CommentEntry(
      id: json['id'] as String,
      authorId: json['author_id'] as String,
      body: json['body'] as String,
      createdAt: DateTime.tryParse(
        json['created_at'] as String? ?? '',
      )?.toLocal(),
      editedAt: DateTime.tryParse(
        json['edited_at'] as String? ?? '',
      )?.toLocal(),
    );
  }

  final String id;
  final String authorId;
  final String body;
  final DateTime? createdAt;
  final DateTime? editedAt;
}

/// A comment thread: an anchored range plus its messages.
class CommentThread {
  const CommentThread({
    required this.id,
    required this.status,
    required this.quote,
    required this.createdBy,
    required this.comments,
    this.anchor,
    this.createdAt,
    this.resolvedBy,
    this.resolvedAt,
  });

  factory CommentThread.fromJson(Map<String, dynamic> json) {
    final anchor = json['anchor'];
    return CommentThread(
      id: json['id'] as String,
      status: json['status'] as String? ?? 'open',
      quote: json['quote'] as String? ?? '',
      createdBy: json['created_by'] as String? ?? '',
      anchor: anchor is Map<String, dynamic>
          ? CommentAnchorRange.fromJson(anchor)
          : null,
      createdAt: DateTime.tryParse(
        json['created_at'] as String? ?? '',
      )?.toLocal(),
      resolvedBy: json['resolved_by'] as String?,
      resolvedAt: DateTime.tryParse(
        json['resolved_at'] as String? ?? '',
      )?.toLocal(),
      comments: ((json['comments'] as List<dynamic>?) ?? const [])
          .map((c) => CommentEntry.fromJson(c as Map<String, dynamic>))
          .toList(),
    );
  }

  final String id;

  /// `open` | `resolved` | `orphaned` — the server derives `orphaned` from the
  /// anchor on every listing, so it never drifts from the text.
  final String status;

  /// The anchored text as it read when the thread was created. The only thing
  /// left to show once a thread is orphaned.
  final String quote;
  final String createdBy;
  final CommentAnchorRange? anchor;
  final DateTime? createdAt;
  final String? resolvedBy;
  final DateTime? resolvedAt;
  final List<CommentEntry> comments;

  bool get isResolved => status == 'resolved';

  /// The anchored text is gone: show the discussion, draw no highlight.
  bool get isOrphaned => status == 'orphaned' || anchor == null;

  /// Only unresolved, still-anchored threads get a highlight on the canvas.
  bool get isHighlightable => !isResolved && !isOrphaned;
}

/// A named, restorable checkpoint of a document (the server's version history).
/// One entry in a document's yrs-native version timeline. `label` is null for an
/// AUTO snapshot (captured on a cadence); set for a named user checkpoint.
class DocVersion {
  const DocVersion({
    required this.id,
    required this.label,
    required this.createdAt,
    this.createdBy,
  });

  factory DocVersion.fromJson(Map<String, dynamic> json) {
    return DocVersion(
      id: json['id'] as String,
      label: json['label'] as String?,
      createdAt: json['created_at'] as String? ?? '',
      createdBy: json['created_by'] as String?,
    );
  }

  final String id;
  final String? label;
  final String createdAt;

  /// Who saved it, as a user id — resolved to a name against the workspace's
  /// members. The server has been sending this all along (`YrsVersionMeta`); the
  /// client parsed the row and dropped the field, so a shared document's history
  /// could not tell you who rolled what back. Null for local-only history and
  /// for rows written before the column existed.
  final String? createdBy;

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

  /// The root block's full `data` map — where document-level metadata lives.
  /// The page-properties panel edits `data['front_matter']` inside this and
  /// writes the whole map back (preserving any other root-data keys).
  Map<String, dynamic> get rootData {
    return _blocksById[document.rootBlockId]?.data ?? const {};
  }

  /// The document's raw YAML front matter (the inner text, no `---` fences),
  /// or `''` if none. This is the sole authority the property panel reads;
  /// see `editor/properties.dart`.
  String get rootFrontMatter {
    final fm = rootData['front_matter'];
    return fm is String ? fm : '';
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
    this.icon,
    this.updatedAt,
  });

  factory DocumentView.fromJson(Map<String, dynamic> json) {
    final icon = (json['icon'] as String?)?.trim();
    return DocumentView(
      id: json['id'] as String,
      parentViewId: json['parent_view_id'] as String?,
      objectId: json['object_id'] as String,
      objectType: json['object_type'] as String,
      name: json['name'] as String,
      position: json['position'] as String,
      // The server has always sent these (views.icon / views.updated_at); the
      // client simply never read them. They are what the sidebar's emoji and the
      // home screen's "recently edited" ordering are built from.
      icon: (icon == null || icon.isEmpty) ? null : icon,
      updatedAt: DateTime.tryParse(
        json['updated_at'] as String? ?? '',
      )?.toLocal(),
    );
  }

  final String id;
  final String? parentViewId;
  final String objectId;
  final String objectType;
  final String name;
  final String position;

  /// The user-chosen emoji, or null for none — then the UI falls back to a
  /// kind-based glyph, so a folder looks like a folder without anyone setting one.
  final String? icon;

  /// Server-side last-modified. Optional because the offline page-tree mirror and
  /// the local world build views without one.
  final DateTime? updatedAt;

  bool get isFolder => objectType == 'folder';
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

/// True when [name] is an untouched default page name (a new page nobody
/// renamed), so the title field shows its placeholder instead of the literal
/// text — a grey hint + caret rather than solid, pre-selected words.
///
/// Both spellings are matched on purpose. The default is written by whichever
/// client created the page (`l10n.untitledPage`), because the server rejects
/// empty view names — so this string is persisted DATA, and a Chinese client
/// saves one spelling while an English client saves the other. A page created in
/// either language, by any version, has to keep rendering as untouched.
///
/// Literals rather than a shared constant: there is no longer a single "the
/// default name" to point at, and a constant would invite someone to reuse it as
/// one — which is precisely the bug this replaced, where every client persisted
/// the Chinese string no matter what language it was running in.
bool isUntitledPageName(String name) {
  final t = name.trim();
  return t == '未命名页面' || t == 'Untitled';
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
/// The sibling order that puts [created] directly below [anchor], or null when
/// nothing needs to move.
///
/// Creating a page/folder carries no position — neither the HTTP API nor the
/// local store takes one — so a new row always lands LAST in its group. When
/// Which workspace holds [viewId], or null when none of them does.
///
/// Exists because losing this answer is a bug that does not look like one.
/// Home lists pages from EVERY workspace; the open path is workspace-scoped
/// (`GET /workspaces/{workspace_id}/documents/{id}`, and the server's
/// `fetch_document` is scoped too). Code that flattens `viewsByWorkspace.values`
/// to find a view by id throws away the one field the open needs, and the
/// result is a 404 that reads as "that row is broken" rather than "the app
/// asked the wrong workspace".
String? workspaceIdOfView({
  required Map<String, List<DocumentView>> viewsByWorkspace,
  required String viewId,
}) {
  for (final entry in viewsByWorkspace.entries) {
    for (final view in entry.value) {
      if (view.id == viewId) return entry.key;
    }
  }
  return null;
}

/// the user located a page, "new page" means the next line, not the end of the
/// folder, and the row has to be slid up afterwards.
///
/// Null is a real answer, not a failure: when [anchor] is already the last
/// sibling, the create put the row exactly where it belongs and reordering
/// would be a round trip that changes nothing. Also null when [anchor] is no
/// longer among [views] (moved or deleted while the create was in flight) —
/// leaving the row at the end beats reordering around a stale anchor.
List<DocumentView>? orderedSiblingsPlacingAfter({
  required Iterable<DocumentView> views,
  required DocumentView anchor,
  required DocumentView created,
}) {
  final parentId = anchor.parentViewId;
  final siblings =
      views
          .where((v) => v.parentViewId == parentId && v.id != created.id)
          .toList()
        ..sort((a, b) => a.position.compareTo(b.position));
  final index = siblings.indexWhere((v) => v.id == anchor.id);
  if (index < 0) return null;
  if (index == siblings.length - 1) return null;
  return [...siblings.take(index + 1), created, ...siblings.skip(index + 1)];
}

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
  const ApiException(this.message, {this.statusCode, this.code});

  final String message;

  /// The server's machine-readable error code (the `code` field of the error
  /// body), when the failure came from a response. This is the only honest way
  /// to react to a *specific* failure: [message] is English server prose, and
  /// matching on its text means the server can silently break the client by
  /// rewording a sentence.
  ///
  /// Mostly the generic HTTP category (`bad_request`, `forbidden`, …); a few
  /// cases the user can actually act on carry a specific one, e.g.
  /// `import_no_markdown`.
  final String? code;

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
