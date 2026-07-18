// HTTP client for the Mica REST API. Extracted from main.dart (2026-07).
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:http/http.dart' as http;
import 'package:web_socket_channel/web_socket_channel.dart';

import '../upload/sha256.dart';
import 'models.dart';

/// `scheme://host[:port]` for [base], omitting the port when it IS the
/// scheme's default. `Uri.port` always answers with a number (443 for https),
/// so building an origin from it verbatim produced links like
/// `https://mica.cloudcele.com:443/…` — valid, but noise in a url people copy
/// and paste around. Non-default ports (a dev server on :8080) are kept.
String apiOrigin(Uri base) {
  final isDefault = (base.scheme == 'https' && base.port == 443) ||
      (base.scheme == 'http' && base.port == 80);
  return isDefault
      ? '${base.scheme}://${base.host}'
      : '${base.scheme}://${base.host}:${base.port}';
}

class ApiClient {
  ApiClient() : baseUri = _resolveBaseUri();

  /// HTTP base; also derives the WebSocket endpoint for document rooms. Mutable
  /// so the server switch in Settings can repoint the client at runtime — all
  /// subsequent REST + WebSocket calls use the new base.
  Uri baseUri;

  /// The build-time default base (local dev backend, or the
  /// --dart-define=MICA_API_BASE_URL override).
  static Uri defaultBaseUri() => _resolveBaseUri();

  Future<AuthSession> register(AuthFormValue form) async {
    final response = await _post('/api/auth/register', {
      'email': form.email,
      'display_name': form.displayName,
      'password': form.password,
    });
    return AuthSession.fromJson(response);
  }

  Future<AuthSession> login(AuthFormValue form) async {
    final response = await _post('/api/auth/login', {
      'email': form.email,
      'password': form.password,
    });
    return AuthSession.fromJson(response);
  }

  /// End a sign-in server-side. Best-effort: the caller is signing out either
  /// way, so a failure here must not strand it — but without this the refresh
  /// token stays live for its full 30 days, and "sign out" would mean nothing
  /// to anyone holding a copy of it.
  Future<void> logout(String refreshToken) async {
    if (refreshToken.isEmpty) return;
    await _post('/api/auth/logout', {'refresh_token': refreshToken});
  }

  /// Trade a refresh token for a new session. Needs no access token — the whole
  /// point is that yours is dead.
  ///
  /// The returned session carries a NEW refresh token: the server rotates on
  /// every refresh and treats a second spend of the old one as theft, killing
  /// the sign-in. So the caller must persist what comes back, and must never
  /// run two of these at once — see `_MicaAppState._refreshSession`.
  Future<AuthSession> refreshSession(String refreshToken) async {
    final response = await _post('/api/auth/refresh', {
      'refresh_token': refreshToken,
    });
    return AuthSession.fromJson(response);
  }

  Future<List<Map<String, dynamic>>> listTokens(String token) async {
    final response = await _get('/api/auth/tokens', token);
    return (response['tokens'] as List<dynamic>).cast<Map<String, dynamic>>();
  }

  /// Returns the created token JSON — the `token` secret is present ONCE here.
  Future<Map<String, dynamic>> createToken(
    String token,
    String name,
    List<String> scopes,
    int? expiresInDays,
  ) async {
    final body = <String, dynamic>{'name': name, 'scopes': scopes};
    if (expiresInDays != null) body['expires_in_days'] = expiresInDays;
    return _post('/api/auth/tokens', body, token: token);
  }

  Future<void> revokeToken(String token, String id) async {
    await _delete('/api/auth/tokens/$id', token);
  }

  Future<List<Workspace>> listWorkspaces(String token) async {
    final response = await _get('/api/workspaces', token);
    final items = response['workspaces'] as List<dynamic>;
    return items
        .map((item) => Workspace.fromJson(item as Map<String, dynamic>))
        .toList();
  }

  Future<Workspace> createWorkspace(String token, String name) async {
    final response = await _post('/api/workspaces', {
      'name': name,
    }, token: token);
    return Workspace.fromJson(response['workspace'] as Map<String, dynamic>);
  }

  Future<Workspace> updateWorkspace(
    String token,
    String workspaceId,
    String name,
  ) async {
    final response = await _patch('/api/workspaces/$workspaceId', {
      'name': name,
    }, token: token);
    return Workspace.fromJson(response['workspace'] as Map<String, dynamic>);
  }

  Future<void> deleteWorkspace(String token, String workspaceId) async {
    await _delete('/api/workspaces/$workspaceId', token);
  }

  Future<List<WorkspaceMember>> listWorkspaceMembers(
    String token,
    String workspaceId,
  ) async {
    final response = await _get('/api/workspaces/$workspaceId/members', token);
    final items = response['members'] as List<dynamic>;
    return items
        .map((item) => WorkspaceMember.fromJson(item as Map<String, dynamic>))
        .toList();
  }

  Future<WorkspaceMember> addWorkspaceMember(
    String token,
    String workspaceId,
    String email,
    String role,
  ) async {
    final response = await _post('/api/workspaces/$workspaceId/members', {
      'email': email,
      'role': role,
    }, token: token);
    return WorkspaceMember.fromJson(response['member'] as Map<String, dynamic>);
  }

  Future<WorkspaceMember> updateWorkspaceMember(
    String token,
    String workspaceId,
    String userId,
    String role,
  ) async {
    final response = await _patch(
      '/api/workspaces/$workspaceId/members/$userId',
      {'role': role},
      token: token,
    );
    return WorkspaceMember.fromJson(response['member'] as Map<String, dynamic>);
  }

  Future<List<WorkspaceMember>> removeWorkspaceMember(
    String token,
    String workspaceId,
    String userId,
  ) async {
    final response = await _delete(
      '/api/workspaces/$workspaceId/members/$userId',
      token,
    );
    final items = response['members'] as List<dynamic>;
    return items
        .map((item) => WorkspaceMember.fromJson(item as Map<String, dynamic>))
        .toList();
  }

  Future<List<DocumentView>> listViews(String token, String workspaceId) async {
    final response = await _get('/api/workspaces/$workspaceId/views', token);
    final items = response['views'] as List<dynamic>;
    return items
        .map((item) => DocumentView.fromJson(item as Map<String, dynamic>))
        .toList();
  }

  Future<DocumentCreateResult> createDocument(
    String token,
    String workspaceId,
    String name, {
    String? parentViewId,
  }) async {
    final body = <String, dynamic>{'name': name};
    if (parentViewId != null) {
      body['parent_view_id'] = parentViewId;
    }

    final response = await _post(
      '/api/workspaces/$workspaceId/documents',
      body,
      token: token,
    );
    return DocumentCreateResult.fromJson(response);
  }

  /// Create a folder view — a pure container (no document). Returns the new
  /// view (object_type='folder'). Mirrors [createDocument] against the folders
  /// endpoint (F1).
  Future<DocumentView> createFolder(
    String token,
    String workspaceId,
    String name, {
    String? parentViewId,
  }) async {
    final body = <String, dynamic>{'name': name};
    if (parentViewId != null) {
      body['parent_view_id'] = parentViewId;
    }
    final response = await _post(
      '/api/workspaces/$workspaceId/folders',
      body,
      token: token,
    );
    return DocumentView.fromJson(response['view'] as Map<String, dynamic>);
  }

  Future<DocumentBootstrap> bootstrapDocument(
    String token,
    String workspaceId,
    String documentId,
  ) async {
    final response = await _get(
      '/api/workspaces/$workspaceId/documents/$documentId/bootstrap',
      token,
    );
    return DocumentBootstrap.fromJson(response);
  }

  Future<DocumentView> updateView(
    String token,
    String workspaceId,
    String viewId,
    String name,
  ) async {
    final response = await _patch(
      '/api/workspaces/$workspaceId/views/$viewId',
      {'name': name},
      token: token,
    );
    return DocumentView.fromJson(response['view'] as Map<String, dynamic>);
  }

  Future<DocumentView> moveView(
    String token,
    String workspaceId,
    String viewId, {
    required String? parentViewId,
    required String position,
  }) async {
    final response = await _post(
      '/api/workspaces/$workspaceId/views/$viewId/move',
      {'parent_view_id': parentViewId, 'position': position},
      token: token,
    );
    return DocumentView.fromJson(response['view'] as Map<String, dynamic>);
  }

  Future<List<DocumentView>> deleteView(
    String token,
    String workspaceId,
    String viewId,
  ) async {
    final response = await _delete(
      '/api/workspaces/$workspaceId/views/$viewId',
      token,
    );
    final items = response['views'] as List<dynamic>;
    return items
        .map((item) => DocumentView.fromJson(item as Map<String, dynamic>))
        .toList();
  }

  Future<List<DocumentView>> listTrash(String token, String workspaceId) async {
    final response = await _get('/api/workspaces/$workspaceId/trash', token);
    final items = response['views'] as List<dynamic>;
    return items
        .map((item) => DocumentView.fromJson(item as Map<String, dynamic>))
        .toList();
  }

  Future<List<DocumentView>> restoreView(
    String token,
    String workspaceId,
    String viewId,
  ) async {
    final response = await _post(
      '/api/workspaces/$workspaceId/views/$viewId/restore',
      const {},
      token: token,
    );
    final items = response['views'] as List<dynamic>;
    return items
        .map((item) => DocumentView.fromJson(item as Map<String, dynamic>))
        .toList();
  }

  /// A document's named versions (restorable checkpoints), newest first. The
  /// server's `/history` also returns the raw op log; we take only `versions`.
  Future<List<DocVersion>> listVersions(
    String token,
    String workspaceId,
    String documentId,
  ) async {
    final response = await _get(
      '/api/workspaces/$workspaceId/documents/$documentId/history',
      token,
    );
    final items = (response['versions'] as List<dynamic>?) ?? const [];
    return items
        .map((item) => DocVersion.fromJson(item as Map<String, dynamic>))
        .toList();
  }

  /// Pin the document's CURRENT state as a named, restorable version.
  Future<DocVersion> createVersion(
    String token,
    String workspaceId,
    String documentId,
    String name,
  ) async {
    final response = await _post(
      '/api/workspaces/$workspaceId/documents/$documentId/versions',
      {'name': name},
      token: token,
    );
    return DocVersion.fromJson(response['version'] as Map<String, dynamic>);
  }

  /// Roll the document back to [versionId]. History stays append-only — the
  /// restore is itself a new update the server broadcasts to open editors.
  Future<void> restoreVersion(
    String token,
    String workspaceId,
    String documentId,
    String versionId,
  ) async {
    await _post(
      '/api/workspaces/$workspaceId/documents/$documentId/restore',
      {'version_id': versionId},
      token: token,
    );
  }

  /// A document's public-share status: whether it has an active link, and the
  /// token (compose `{baseUri.origin}/s/{token}` for the shareable URL).
  Future<({bool shared, String? token})> getShare(
    String token,
    String workspaceId,
    String documentId,
  ) async {
    final response = await _get(
      '/api/workspaces/$workspaceId/documents/$documentId/share',
      token,
    );
    return (
      shared: response['shared'] == true,
      token: response['token'] as String?,
    );
  }

  /// Publish the document to a public read-only link. Idempotent — returns the
  /// existing token if already shared.
  Future<String> createShare(
    String token,
    String workspaceId,
    String documentId,
  ) async {
    final response = await _post(
      '/api/workspaces/$workspaceId/documents/$documentId/share',
      const {},
      token: token,
    );
    return response['token'] as String;
  }

  /// Turn off the public link (it 404s immediately).
  Future<void> deleteShare(
    String token,
    String workspaceId,
    String documentId,
  ) async {
    await _delete(
      '/api/workspaces/$workspaceId/documents/$documentId/share',
      token,
    );
  }

  /// Move (`removeSource: true`) or copy (`false`) the view's subtree into
  /// [destWorkspaceId], under [parentViewId] there (null = destination root).
  /// With [dryRun] the server reports what it WOULD do and mutates nothing —
  /// the dialog uses that for its preview before the user commits.
  Future<TransferReport> transferView({
    required String token,
    required String workspaceId,
    required String viewId,
    required String destWorkspaceId,
    String? parentViewId,
    required bool removeSource,
    required bool dryRun,
  }) async {
    final response = await _post(
      '/api/workspaces/$workspaceId/views/$viewId/transfer',
      {
        'dest_workspace_id': destWorkspaceId,
        'parent_view_id': parentViewId,
        'remove_source': removeSource,
        'dry_run': dryRun,
      },
      token: token,
    );
    return TransferReport.fromJson(response);
  }

  /// Duplicate [viewId] within its own workspace. [name] is the caller's
  /// locale-aware copy name (e.g. "X 副本"); the server dedupes it against
  /// siblings. [parentViewId] null = beside the original (its own parent).
  Future<CloneReport> cloneView({
    required String token,
    required String workspaceId,
    required String viewId,
    String? name,
    String? parentViewId,
    required bool dryRun,
  }) async {
    final response = await _post(
      '/api/workspaces/$workspaceId/views/$viewId/clone',
      {
        'name': name,
        'parent_view_id': parentViewId,
        'dry_run': dryRun,
      },
      token: token,
    );
    return CloneReport.fromJson(response);
  }

  Future<void> purgeView(
    String token,
    String workspaceId,
    String viewId,
  ) async {
    await _delete('/api/workspaces/$workspaceId/trash/$viewId', token);
  }

  Future<String> aiComplete(
    String token,
    String prompt, {
    String? system,
  }) async {
    final body = <String, dynamic>{'prompt': prompt};
    if (system != null) {
      body['system'] = system;
    }
    final response = await _post('/api/ai/complete', body, token: token);
    return response['text'] as String? ?? '';
  }

  /// Stream an AI completion over WebSocket, yielding text deltas as they arrive.
  /// (Flutter Web's HTTP client can't stream responses, so AI streaming uses WS.)
  Stream<String> aiStream(
    String token,
    String prompt, {
    String? system,
  }) async* {
    final uri = baseUri.replace(
      scheme: baseUri.scheme == 'https' ? 'wss' : 'ws',
      path: '/ws/ai',
      queryParameters: {'token': token},
    );
    final channel = WebSocketChannel.connect(uri);
    try {
      channel.sink.add(jsonEncode({'prompt': prompt, 'system': ?system}));
      await for (final raw in channel.stream) {
        final message = jsonDecode(raw as String) as Map<String, dynamic>;
        switch (message['type']) {
          case 'delta':
            yield message['text'] as String? ?? '';
          case 'error':
            throw ApiException(message['message'] as String? ?? 'AI error');
          case 'done':
            return;
        }
      }
    } finally {
      await channel.sink.close();
    }
  }

  Future<List<SearchResult>> searchWorkspace(
    String token,
    String workspaceId,
    String query,
  ) async {
    final response = await _get(
      '/api/workspaces/$workspaceId/search?q=${Uri.encodeQueryComponent(query)}',
      token,
    );
    final items = response['results'] as List<dynamic>;
    return items
        .map((item) => SearchResult.fromJson(item as Map<String, dynamic>))
        .toList();
  }

  Future<User> updateMe(String token, String displayName) async {
    final response = await _patch('/api/auth/me', {
      'display_name': displayName,
    }, token: token);
    return User.fromJson(response['user'] as Map<String, dynamic>);
  }

  Future<void> changePassword(
    String token,
    String currentPassword,
    String newPassword,
  ) async {
    await _post('/api/auth/password', {
      'current_password': currentPassword,
      'new_password': newPassword,
    }, token: token);
  }

  Future<Map<String, dynamic>> getAiSettings(String token) async {
    return _get('/api/ai/settings', token);
  }

  Future<Map<String, dynamic>> updateAiSettings(
    String token, {
    required String provider,
    required String baseUrl,
    required String model,
    String? apiKey,
    int? maxTokens,
  }) async {
    final body = <String, dynamic>{
      'provider': provider,
      'base_url': baseUrl,
      'model': model,
    };
    if (apiKey != null && apiKey.isNotEmpty) {
      body['api_key'] = apiKey;
    }
    if (maxTokens != null) {
      body['max_tokens'] = maxTokens;
    }
    return _patch('/api/ai/settings', body, token: token);
  }

  Future<DocumentBootstrap> importMarkdown(
    String token,
    String workspaceId,
    String name,
    String markdown, {
    String? parentViewId,
  }) async {
    final body = <String, dynamic>{'name': name, 'markdown': markdown};
    if (parentViewId != null) {
      body['parent_view_id'] = parentViewId;
    }
    final response = await _post(
      '/api/workspaces/$workspaceId/documents/import/markdown',
      body,
      token: token,
    );
    return DocumentBootstrap.fromJson(response);
  }

  Future<DocumentUpdateResult> applyDocumentUpdate(
    String token,
    String workspaceId,
    String documentId,
    List<Map<String, dynamic>> operations,
  ) async {
    final response = await _post(
      '/api/workspaces/$workspaceId/documents/$documentId/updates',
      {'operations': operations},
      token: token,
    );
    return DocumentUpdateResult.fromJson(response);
  }

  Future<String> exportMarkdown(
    String token,
    String workspaceId,
    String documentId,
  ) async {
    final response = await _get(
      '/api/workspaces/$workspaceId/documents/$documentId/export/markdown',
      token,
    );
    return response['markdown'] as String;
  }

  Future<String> exportWorkspaceMarkdown(
    String token,
    String workspaceId,
  ) async {
    final response = await _get(
      '/api/workspaces/$workspaceId/export/markdown',
      token,
    );
    return response['markdown'] as String? ?? '';
  }

  Future<String> exportAllMarkdown(String token) async {
    final response = await _get('/api/export/markdown', token);
    return response['markdown'] as String? ?? '';
  }

  /// Upload an image: presign a content-addressed key, PUT the bytes directly to
  /// object storage, then record metadata. Returns the new file id + name.
  Future<UploadedFile> uploadImage(
    String token,
    String workspaceId, {
    required String fileName,
    required String mimeType,
    required Uint8List bytes,
  }) async {
    final hash = sha256Hex(bytes);
    final presign = await _post('/api/workspaces/$workspaceId/files/presign', {
      'file_name': fileName,
      'mime_type': mimeType,
      'byte_size': bytes.length,
      'content_hash': hash,
    }, token: token);
    final objectKey = presign['object_key'] as String;
    final uploadUrl = presign['upload_url'] as String;

    final put = await http.put(
      Uri.parse(uploadUrl),
      headers: {'content-type': mimeType},
      body: bytes,
    );
    if (put.statusCode < 200 || put.statusCode >= 300) {
      throw ApiException('upload failed (HTTP ${put.statusCode})');
    }

    final complete =
        await _post('/api/workspaces/$workspaceId/files/complete', {
          'object_key': objectKey,
          'file_name': fileName,
          'mime_type': mimeType,
          'byte_size': bytes.length,
        }, token: token);
    return UploadedFile.fromResponse(complete);
  }

  /// Download a page as a portable ZIP (markdown + bundled image assets).
  Future<Uint8List> exportDocumentZip(
    String token,
    String workspaceId,
    String documentId,
  ) async {
    final response = await http.get(
      baseUri.replace(
        path: '/api/workspaces/$workspaceId/documents/$documentId/export.zip',
      ),
      headers: {'authorization': 'Bearer $token'},
    );
    if (response.statusCode < 200 || response.statusCode >= 300) {
      throw ApiException('export failed (HTTP ${response.statusCode})');
    }
    return response.bodyBytes;
  }

  /// Download one page as a self-contained HTML file — a full document with an
  /// embedded stylesheet and images inlined as `data:` URIs (see the server's
  /// `export_document_html`). Returned as a string so the caller writes it under
  /// the page's own name; the endpoint also sets a `filename` for browsers.
  Future<String> exportDocumentHtml(
    String token,
    String workspaceId,
    String documentId,
  ) async {
    final response = await http.get(
      baseUri.replace(
        path: '/api/workspaces/$workspaceId/documents/$documentId/export/html',
      ),
      headers: {'authorization': 'Bearer $token'},
    );
    if (response.statusCode < 200 || response.statusCode >= 300) {
      throw ApiException('export failed (HTTP ${response.statusCode})');
    }
    return utf8.decode(response.bodyBytes);
  }

  /// Download one folder's subtree as a Markdown ZIP (same shape as the
  /// workspace export: relative paths + shared `assets/` + manifest).
  Future<Uint8List> exportFolderZip(
    String token,
    String workspaceId,
    String viewId,
  ) async {
    final response = await http.get(
      baseUri.replace(
        path: '/api/workspaces/$workspaceId/views/$viewId/export.zip',
      ),
      headers: {'authorization': 'Bearer $token'},
    );
    if (response.statusCode < 200 || response.statusCode >= 300) {
      throw ApiException('export failed (HTTP ${response.statusCode})');
    }
    return response.bodyBytes;
  }

  /// Download a whole workspace as a Markdown ZIP (page-tree folders + assets).
  Future<Uint8List> exportWorkspaceZip(String token, String workspaceId) async {
    final response = await http.get(
      baseUri.replace(path: '/api/workspaces/$workspaceId/export.zip'),
      headers: {'authorization': 'Bearer $token'},
    );
    if (response.statusCode < 200 || response.statusCode >= 300) {
      throw ApiException('export failed (HTTP ${response.statusCode})');
    }
    return response.bodyBytes;
  }

  /// Re-host a remote image by URL (server-side fetch + store), avoiding dead
  /// links. Returns the new file id + name.
  Future<UploadedFile> importImageUrl(
    String token,
    String workspaceId,
    String url,
  ) async {
    final response = await _post(
      '/api/workspaces/$workspaceId/files/import-url',
      {'url': url},
      token: token,
    );
    return UploadedFile.fromResponse(response);
  }

  /// Resolve image file ids to fresh (signed) download URLs. Returns a
  /// `fileId -> url` map; unknown ids are simply absent.
  Future<Map<String, String>> resolveFiles(
    String token,
    String workspaceId,
    List<String> ids,
  ) async {
    if (ids.isEmpty) return {};
    final response = await _post('/api/workspaces/$workspaceId/files/resolve', {
      'ids': ids,
    }, token: token);
    final files = (response['files'] as List<dynamic>? ?? []);
    final out = <String, String>{};
    for (final raw in files) {
      final f = UploadedFile.fromResponse(raw as Map<String, dynamic>);
      out[f.id] = f.downloadUrl;
    }
    return out;
  }

  /// Start a server-side workspace import: the body is the raw archive.
  /// Returns the job id to poll with [importJobStatus].
  Future<String> startWorkspaceImport(
    String token,
    Uint8List zipBytes, {
    String? name,
    bool notion = false,
    String? workspaceId,
  }) async {
    final response = await http.post(
      baseUri.replace(
        path: '/api/workspaces/import',
        queryParameters: {
          if (name != null && name.isNotEmpty) 'name': name,
          if (notion) 'notion': 'true',
          'workspace_id': ?workspaceId,
        },
      ),
      headers: {
        'content-type': 'application/zip',
        'authorization': 'Bearer $token',
      },
      body: zipBytes,
    );
    final body = _decode(response);
    return body['job_id'] as String;
  }

  Future<ImportJobStatus> importJobStatus(String token, String jobId) async {
    final response = await _get('/api/import/jobs/$jobId', token);
    return ImportJobStatus.fromJson(response);
  }

  /// Build the request URI. [path] may carry a query string
  /// ('/x/search?q=…') — `Uri.replace(path:)` would percent-encode the '?',
  /// shipping the whole query as part of the path (and 404ing).
  Uri _apiUri(String path) {
    final q = path.indexOf('?');
    if (q < 0) return baseUri.replace(path: path);
    return baseUri.replace(
      path: path.substring(0, q),
      query: path.substring(q + 1),
    );
  }

  Future<Map<String, dynamic>> _get(String path, String token) async {
    final response = await http.get(
      _apiUri(path),
      headers: {'authorization': 'Bearer $token'},
    );
    return _decode(response);
  }

  Future<Map<String, dynamic>> _post(
    String path,
    Map<String, dynamic> body, {
    String? token,
  }) async {
    final response = await http.post(
      _apiUri(path),
      headers: _headers(token),
      body: jsonEncode(body),
    );
    return _decode(response);
  }

  Future<Map<String, dynamic>> _patch(
    String path,
    Map<String, dynamic> body, {
    required String token,
  }) async {
    final response = await http.patch(
      _apiUri(path),
      headers: _headers(token),
      body: jsonEncode(body),
    );
    return _decode(response);
  }

  Future<Map<String, dynamic>> _delete(String path, String token) async {
    final response = await http.delete(
      _apiUri(path),
      headers: {'authorization': 'Bearer $token'},
    );
    return _decode(response);
  }

  Map<String, String> _headers(String? token) {
    return {
      'content-type': 'application/json',
      if (token != null) 'authorization': 'Bearer $token',
    };
  }

  Map<String, dynamic> _decode(http.Response response) {
    final body = response.body.isEmpty
        ? <String, dynamic>{}
        : jsonDecode(response.body) as Map<String, dynamic>;
    if (response.statusCode < 200 || response.statusCode >= 300) {
      final message =
          body['message'] as String? ?? 'HTTP ${response.statusCode}';
      throw ApiException(message, statusCode: response.statusCode);
    }
    return body;
  }

  static Uri _resolveBaseUri() {
    const configured = String.fromEnvironment('MICA_API_BASE_URL');
    if (configured.isNotEmpty) {
      return Uri.parse(configured);
    }

    // Desktop/mobile have no serving origin — Uri.base is a file:// cwd, not a
    // web page — so the page-relative logic below doesn't apply. Default to the
    // local dev backend; point at a deployed server with
    // --dart-define=MICA_API_BASE_URL=https://host.
    if (!kIsWeb) {
      return Uri.parse('http://127.0.0.1:8080');
    }

    final page = Uri.base;
    // Served from a standard port (production behind the reverse proxy):
    // the API is same-origin — nginx routes /api and /ws to the backend, so
    // the same static bundle works on any server IP or domain.
    if (page.scheme.isNotEmpty && (page.port == 80 || page.port == 443)) {
      return Uri(scheme: page.scheme, host: page.host, port: page.port);
    }
    // Dev: the API listens on :8080 of the same host.
    return Uri(
      scheme: page.scheme.isEmpty ? 'http' : page.scheme,
      host: page.host.isEmpty ? '127.0.0.1' : page.host,
      port: 8080,
    );
  }
}
