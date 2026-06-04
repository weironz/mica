import 'dart:convert';
import 'dart:async';

import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:http/http.dart' as http;
import 'package:web_socket_channel/web_socket_channel.dart';

import 'editor/clipboard_copy.dart';
import 'editor/editor.dart';
import 'editor/image_actions.dart';
import 'editor/pick_file.dart';
import 'widgets/mica_logo.dart';
import 'upload/import_order.dart';
import 'upload/sha256.dart';
import 'upload/unzip.dart';

/// Dev convenience: when true, the app signs in automatically on startup so you
/// don't have to log in on every reload. Turn it off for a real login screen:
/// `flutter run --dart-define=MICA_DEV_AUTOLOGIN=false`.
const bool kDevAutoLogin = bool.fromEnvironment(
  'MICA_DEV_AUTOLOGIN',
  defaultValue: true,
);
const String kDevEmail = String.fromEnvironment(
  'MICA_DEV_EMAIL',
  defaultValue: 'demo@mica.dev',
);
const String kDevPassword = String.fromEnvironment(
  'MICA_DEV_PASSWORD',
  defaultValue: 'password123',
);

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  // Suppress the browser's native right-click menu so the editor can show its
  // own (e.g. image actions) on web.
  if (kIsWeb) BrowserContextMenu.disableContextMenu();
  _warmUpFonts();
  runApp(const MicaApp());
}

/// Flutter Web doesn't bundle CJK fonts — the engine downloads a Noto fallback
/// on first use, which makes the custom-painted editor briefly show ".notdef"
/// boxes. Kick that download off at startup (during login/loading) so the font
/// is cached before any document renders.
void _warmUpFonts() {
  const samples = [
    '中文字体预热示例 ABCabc 0123 ，。！',
    '繁體字預熱 測試',
  ];
  for (final text in samples) {
    final painter = TextPainter(
      text: TextSpan(text: text, style: const TextStyle(fontSize: 16)),
      textDirection: TextDirection.ltr,
    )..layout();
    painter.dispose();
  }
  // Also warm the icon font used by the editor's painted toolbars.
  for (final icon in [Icons.content_copy, Icons.wrap_text, Icons.add]) {
    final painter = TextPainter(
      text: TextSpan(
        text: String.fromCharCode(icon.codePoint),
        style: TextStyle(
          fontFamily: icon.fontFamily,
          package: icon.fontPackage,
          fontSize: 16,
        ),
      ),
      textDirection: TextDirection.ltr,
    )..layout();
    painter.dispose();
  }
}

class MicaApp extends StatelessWidget {
  const MicaApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Mica',
      debugShowCheckedModeBanner: false,
      theme: ThemeData(
        colorScheme: ColorScheme.fromSeed(
          seedColor: const Color(0xFF2563EB),
          brightness: Brightness.light,
        ),
        scaffoldBackgroundColor: const Color(0xFFF8FAFC),
        useMaterial3: true,
        // Bundled CJK fallback so Chinese text never waits on an on-demand
        // web-font download (which flashed ".notdef" boxes).
        fontFamilyFallback: const ['CJKFallback'],
      ),
      home: const WorkspaceShell(),
    );
  }
}

class WorkspaceShell extends StatefulWidget {
  const WorkspaceShell({super.key});

  @override
  State<WorkspaceShell> createState() => _WorkspaceShellState();
}

class _WorkspaceShellState extends State<WorkspaceShell> {
  final ApiClient _api = ApiClient();

  AuthSession? _session;
  List<Workspace> _workspaces = const [];
  Map<String, List<WorkspaceMember>> _membersByWorkspace = const {};
  Map<String, List<DocumentView>> _viewsByWorkspace = const {};
  Workspace? _selectedWorkspace;
  DocumentView? _selectedView;
  DocumentBootstrap? _selectedBootstrap;
  String? _selectedMarkdown;
  String? _message;
  bool _isBusy = false;

  // Editor appearance (in-memory; applied live to the editor).
  EditorAppearance _appearance = const EditorAppearance();
  double _pageWidth = 1160;
  // When on, pasted external image URLs are re-hosted into Mica storage.
  bool _reHostImages = true;

  DocumentSyncClient? _sync;
  List<PresenceUser> _presence = const [];
  Timer? _syncRefetchTimer;

  @override
  void initState() {
    super.initState();
    if (kDevAutoLogin) {
      WidgetsBinding.instance.addPostFrameCallback((_) => _devAutoLogin());
    }
  }

  /// Sign in with the dev account on startup. Falls back to registering it the
  /// first time, and silently leaves the login screen up if the API is down.
  Future<void> _devAutoLogin() async {
    if (_session != null) {
      return;
    }
    const form = AuthFormValue(
      email: kDevEmail,
      displayName: 'Demo User',
      password: kDevPassword,
    );
    await _authenticate(AuthMode.login, form);
    if (mounted && _session == null) {
      await _authenticate(AuthMode.register, form);
    }
  }

  Future<void> _run(Future<void> Function() action) async {
    setState(() {
      _isBusy = true;
      _message = null;
    });

    try {
      await action();
      _reconcileSync();
    } catch (error) {
      setState(() {
        _message = error.toString();
      });
    } finally {
      setState(() {
        _isBusy = false;
      });
    }
  }

  /// Open, switch, or close the document WebSocket so it always tracks the
  /// currently selected document.
  void _reconcileSync() {
    final session = _session;
    final workspace = _selectedWorkspace;
    final documentId = _selectedBootstrap?.document.id;

    if (session == null || workspace == null || documentId == null) {
      _closeDocumentSync();
      return;
    }

    if (_sync?.documentId == documentId) {
      return;
    }

    _closeDocumentSync();
    final sync = DocumentSyncClient(
      documentId: documentId,
      uri: documentSocketUri(
        _api.baseUri,
        workspace.id,
        documentId,
        session.accessToken,
      ),
      selfName: session.user.displayName,
      onRemoteSeq: _handleRemoteSeq,
      onPresence: (users) {
        if (mounted) {
          setState(() => _presence = users);
        }
      },
    );
    _sync = sync;
    setState(() => _presence = const []);
    sync.connect();
  }

  void _closeDocumentSync() {
    _syncRefetchTimer?.cancel();
    _syncRefetchTimer = null;
    _sync?.dispose();
    _sync = null;
    if (_presence.isNotEmpty) {
      setState(() => _presence = const []);
    }
  }

  /// A remote (or our own, echoed) accepted update advanced the server
  /// sequence. If it is ahead of what we hold, pull the latest snapshot. Our
  /// own edits already updated `currentSeq` via their REST response, so their
  /// echo is ignored here.
  void _handleRemoteSeq(String documentId, int serverSeq) {
    final bootstrap = _selectedBootstrap;
    if (bootstrap == null ||
        bootstrap.document.id != documentId ||
        serverSeq <= bootstrap.document.currentSeq) {
      return;
    }

    _syncRefetchTimer?.cancel();
    _syncRefetchTimer = Timer(const Duration(milliseconds: 120), () {
      _refreshSelectedBootstrap(documentId);
    });
  }

  Future<void> _refreshSelectedBootstrap(String documentId) async {
    final session = _session;
    final workspace = _selectedWorkspace;
    if (session == null ||
        workspace == null ||
        _selectedBootstrap?.document.id != documentId) {
      return;
    }

    try {
      final fresh = await _api.bootstrapDocument(
        session.accessToken,
        workspace.id,
        documentId,
      );
      if (!mounted || _selectedBootstrap?.document.id != documentId) {
        return;
      }
      setState(() {
        _selectedBootstrap = fresh;
        _selectedMarkdown = null;
      });
    } catch (_) {
      // Transient sync refetch failures are non-fatal; the next update retries.
    }
  }

  @override
  void dispose() {
    _closeDocumentSync();
    super.dispose();
  }

  Future<void> _authenticate(AuthMode mode, AuthFormValue form) {
    return _run(() async {
      final session = mode == AuthMode.register
          ? await _api.register(form)
          : await _api.login(form);
      final workspaces = await _api.listWorkspaces(session.accessToken);

      setState(() {
        _session = session;
        _workspaces = workspaces;
        _selectedWorkspace = workspaces.firstOrNull;
      });

      await _loadSelectedWorkspaceMembers();
      await _loadSelectedWorkspaceViews();
    });
  }

  Future<void> _refreshWorkspaces() {
    return _run(() async {
      final session = _requireSession();
      final workspaces = await _api.listWorkspaces(session.accessToken);
      setState(() {
        _workspaces = workspaces;
        _selectedWorkspace = _selectedWorkspace == null
            ? workspaces.firstOrNull
            : workspaces
                  .where((workspace) => workspace.id == _selectedWorkspace!.id)
                  .firstOrNull;
      });

      await _loadSelectedWorkspaceMembers();
      await _loadSelectedWorkspaceViews();
    });
  }

  Future<void> _createWorkspace(String name) {
    return _run(() async {
      final session = _requireSession();
      final workspace = await _api.createWorkspace(session.accessToken, name);
      final created = await _api.createDocument(
        session.accessToken,
        workspace.id,
        'Untitled',
      );
      final bootstrap = await _api.bootstrapDocument(
        session.accessToken,
        workspace.id,
        created.document.id,
      );
      setState(() {
        _workspaces = [..._workspaces, workspace];
        _selectedWorkspace = workspace;
        _viewsByWorkspace = {
          ..._viewsByWorkspace,
          workspace.id: [created.view],
        };
        _selectedView = created.view;
        _selectedBootstrap = bootstrap;
        _selectedMarkdown = null;
      });

      await _loadSelectedWorkspaceMembers();
    });
  }

  Future<void> _renameWorkspace(Workspace workspace, String name) {
    return _run(() async {
      final session = _requireSession();
      final renamed = await _api.updateWorkspace(
        session.accessToken,
        workspace.id,
        name,
      );
      setState(() {
        _workspaces = _workspaces
            .map((item) => item.id == renamed.id ? renamed : item)
            .toList();
        _selectedWorkspace = renamed;
      });
    });
  }

  Future<void> _deleteWorkspace(Workspace workspace) {
    return _run(() async {
      final session = _requireSession();
      await _api.deleteWorkspace(session.accessToken, workspace.id);
      final remaining = _workspaces
          .where((item) => item.id != workspace.id)
          .toList();
      final wasSelected = _selectedWorkspace?.id == workspace.id;
      setState(() {
        _workspaces = remaining;
        _viewsByWorkspace = {..._viewsByWorkspace}..remove(workspace.id);
        if (wasSelected) {
          _selectedWorkspace = remaining.isNotEmpty ? remaining.first : null;
          _selectedView = null;
          _selectedBootstrap = null;
          _selectedMarkdown = null;
        }
      });
      if (wasSelected && _selectedWorkspace != null) {
        await _loadSelectedWorkspaceMembers();
        await _loadSelectedWorkspaceViews();
      }
    });
  }

  Future<void> _selectWorkspace(Workspace workspace) {
    return _run(() async {
      setState(() {
        _selectedWorkspace = workspace;
        _selectedView = null;
        _selectedBootstrap = null;
        _selectedMarkdown = null;
      });
      await _loadSelectedWorkspaceMembers();
      await _loadSelectedWorkspaceViews();
    });
  }

  Future<void> _createDocument(String name, {String? parentViewId}) {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      final created = await _api.createDocument(
        session.accessToken,
        workspace.id,
        name,
        parentViewId: parentViewId,
      );
      final bootstrap = await _api.bootstrapDocument(
        session.accessToken,
        workspace.id,
        created.document.id,
      );

      setState(() {
        final views = _viewsByWorkspace[workspace.id] ?? const [];
        _viewsByWorkspace = {
          ..._viewsByWorkspace,
          workspace.id: [...views, created.view],
        };
        _selectedView = created.view;
        _selectedBootstrap = bootstrap;
        _selectedMarkdown = null;
      });
    });
  }

  Future<void> _createChildDocument(DocumentView parent, String name) {
    return _createDocument(name, parentViewId: parent.id);
  }

  /// Persist a new sibling order: assign evenly spaced, zero-padded positions to
  /// [orderedSiblings] (all sharing [parentViewId]) and push the ones that
  /// changed. Ordering is per-parent, so renumbering one group is self-contained.
  Future<void> _reorderViews(
    String? parentViewId,
    List<DocumentView> orderedSiblings,
  ) {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      String pad(int n) => n.toString().padLeft(10, '0');

      final moved = <DocumentView>[];
      for (var i = 0; i < orderedSiblings.length; i++) {
        final desired = pad((i + 1) * 10);
        final view = orderedSiblings[i];
        if (view.position != desired || view.parentViewId != parentViewId) {
          moved.add(
            await _api.moveView(
              session.accessToken,
              workspace.id,
              view.id,
              parentViewId: parentViewId,
              position: desired,
            ),
          );
        }
      }
      if (moved.isEmpty) {
        return;
      }
      setState(() {
        final views = [
          ...?_viewsByWorkspace[workspace.id],
        ];
        for (final m in moved) {
          final idx = views.indexWhere((v) => v.id == m.id);
          if (idx >= 0) {
            views[idx] = m;
          }
        }
        _viewsByWorkspace = {..._viewsByWorkspace, workspace.id: views};
      });
    });
  }

  Future<List<DocumentView>> _loadTrash() async {
    final session = _requireSession();
    final workspace = _requireWorkspace();
    return _api.listTrash(session.accessToken, workspace.id);
  }

  Future<void> _restoreView(DocumentView view) {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      final views = await _api.restoreView(
        session.accessToken,
        workspace.id,
        view.id,
      );
      setState(() {
        _viewsByWorkspace = {..._viewsByWorkspace, workspace.id: views};
      });
    });
  }

  Future<void> _purgeView(DocumentView view) {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      await _api.purgeView(session.accessToken, workspace.id, view.id);
    });
  }

  // ---------------------------------------------------------------------------
  // AI
  // ---------------------------------------------------------------------------

  Stream<String> _aiStream(String prompt, {String? system}) {
    final session = _session;
    if (session == null) {
      return Stream<String>.error(const ApiException('Not signed in.'));
    }
    return _api.aiStream(session.accessToken, prompt, system: system);
  }

  Future<String> _exportPageMarkdown() async {
    final session = _requireSession();
    final workspace = _requireWorkspace();
    final bootstrap = _selectedBootstrap;
    if (bootstrap == null) {
      throw const ApiException('Open a page first.');
    }
    return _api.exportMarkdown(
      session.accessToken,
      workspace.id,
      bootstrap.document.id,
    );
  }

  Future<Uint8List> _exportPageZip() async {
    final session = _requireSession();
    final workspace = _requireWorkspace();
    final bootstrap = _selectedBootstrap;
    if (bootstrap == null) {
      throw const ApiException('Open a page first.');
    }
    return _api.exportDocumentZip(
      session.accessToken,
      workspace.id,
      bootstrap.document.id,
    );
  }

  Future<String> _exportWorkspaceMarkdown() async {
    final session = _requireSession();
    final workspace = _requireWorkspace();
    return _api.exportWorkspaceMarkdown(session.accessToken, workspace.id);
  }

  Future<Uint8List> _exportWorkspaceZip(String workspaceId) async {
    final session = _requireSession();
    return _api.exportWorkspaceZip(session.accessToken, workspaceId);
  }

  Future<String> _exportAllMarkdown() async {
    final session = _requireSession();
    return _api.exportAllMarkdown(session.accessToken);
  }

  Future<List<SearchResult>> _searchWorkspace(String query) async {
    final session = _session;
    final workspace = _selectedWorkspace;
    if (session == null || workspace == null) return const [];
    return _api.searchWorkspace(session.accessToken, workspace.id, query);
  }

  Future<void> _openViewById(String viewId) async {
    final workspace = _selectedWorkspace;
    if (workspace == null) return;
    final views = _viewsByWorkspace[workspace.id] ?? const [];
    final view = views.where((v) => v.id == viewId).firstOrNull;
    if (view != null) {
      await _selectView(view);
    }
  }

  Future<void> _updateProfile(String displayName) async {
    final session = _requireSession();
    final user = await _api.updateMe(session.accessToken, displayName);
    if (mounted) {
      setState(() {
        _session = AuthSession(accessToken: session.accessToken, user: user);
      });
    }
  }

  Future<void> _changePassword(String current, String next) async {
    final session = _requireSession();
    await _api.changePassword(session.accessToken, current, next);
  }

  Future<Map<String, dynamic>> _loadAiSettings() async {
    final session = _requireSession();
    return _api.getAiSettings(session.accessToken);
  }

  Future<void> _saveAiSettings({
    required String provider,
    required String baseUrl,
    required String model,
    String? apiKey,
  }) async {
    final session = _requireSession();
    await _api.updateAiSettings(
      session.accessToken,
      provider: provider,
      baseUrl: baseUrl,
      model: model,
      apiKey: apiKey,
    );
  }

  String _titleFromMarkdown(String markdown, String fallback) {
    for (final line in markdown.split('\n')) {
      final trimmed = line.trim();
      final heading = RegExp(r'^#{1,6}\s+(.*)$').firstMatch(trimmed);
      if (heading != null && heading.group(1)!.trim().isNotEmpty) {
        return heading.group(1)!.trim();
      }
      if (trimmed.isNotEmpty) {
        return trimmed.length > 60 ? trimmed.substring(0, 60) : trimmed;
      }
    }
    return fallback;
  }

  Future<void> _aiNewPageFromMarkdown(String markdown) {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      final title = _titleFromMarkdown(markdown, 'Untitled');
      final bootstrap = await _api.importMarkdown(
        session.accessToken,
        workspace.id,
        title,
        markdown,
      );
      setState(() {
        final views = _viewsByWorkspace[workspace.id] ?? const [];
        _viewsByWorkspace = {
          ..._viewsByWorkspace,
          workspace.id: [...views, bootstrap.view],
        };
        _selectedView = bootstrap.view;
        _selectedBootstrap = bootstrap;
        _selectedMarkdown = null;
      });
    });
  }

  /// Import a Markdown file as a new page (title from its H1, else the filename).
  Future<void> _importMarkdownAsPage(String fileName, String markdown) {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      final base = fileName
          .replaceAll(RegExp(r'\.(md|markdown|txt)$', caseSensitive: false), '')
          .trim();
      final title =
          _titleFromMarkdown(markdown, base.isEmpty ? 'Imported' : base);
      final bootstrap = await _api.importMarkdown(
        session.accessToken,
        workspace.id,
        title,
        markdown,
      );
      setState(() {
        final views = _viewsByWorkspace[workspace.id] ?? const [];
        _viewsByWorkspace = {
          ..._viewsByWorkspace,
          workspace.id: [...views, bootstrap.view],
        };
        _selectedView = bootstrap.view;
        _selectedBootstrap = bootstrap;
        _selectedMarkdown = null;
      });
    });
  }

  /// Import a workspace ZIP: rebuild the page tree from the folder layout.
  /// Works for Mica exports and Notion "Markdown & CSV" exports alike.
  Future<void> _importWorkspaceZip(String fileName, Uint8List zipBytes) {
    final wsName = stripNotionId(fileName
        .replaceAll(RegExp(r'\.zip$', caseSensitive: false), '')
        .trim());
    return _importWorkspaceTree(
      wsName.isEmpty ? 'Imported' : wsName,
      () => readZip(zipBytes),
    );
  }

  /// Import a file tree (relative path → bytes) as a NEW workspace. Entries
  /// come lazily so ZIP parsing also runs inside the guarded flow.
  Future<void> _importWorkspaceTree(
    String wsName,
    List<ZipFileEntry> Function() loadEntries,
  ) {
    return _run(() async {
      final session = _requireSession();
      final entries = loadEntries();
      final workspace = await _api.createWorkspace(
        session.accessToken,
        wsName,
      );
      await _importEntriesInto(session, workspace, entries);
      final workspaces = await _api.listWorkspaces(session.accessToken);
      if (mounted) setState(() => _workspaces = workspaces);
      await _selectWorkspace(workspace);
    });
  }

  /// Import a file tree into an EXISTING workspace (pages appended at the
  /// root level), then reload it.
  Future<void> _importTreeIntoWorkspace(
    Workspace workspace,
    List<ZipFileEntry> entries,
  ) {
    return _run(() async {
      final session = _requireSession();
      await _importEntriesInto(session, workspace, entries);
      await _selectWorkspace(workspace);
    });
  }

  /// Shared import core: rebuild the page tree from the folder layout
  /// (folders without a matching `.md` become empty directory pages), and
  /// upload images referenced by relative path, rewiring them to Mica's
  /// `{file_id, name}` image format.
  Future<void> _importEntriesInto(
    AuthSession session,
    Workspace workspace,
    List<ZipFileEntry> rawEntries,
  ) async {
    // Peel wrapper folders (zipped folder / Notion export shell) and drop
    // OS metadata before anything else looks at the paths.
    final entries = normalizeZipEntries(rawEntries);

    // Mica's export writes a manifest.json carrying the page-tree order.
    String? manifestJson;
    for (final e in entries) {
      if (e.name == 'manifest.json') {
        manifestJson = utf8.decode(e.bytes, allowMalformed: true);
        break;
      }
    }

    // Non-markdown files in the ZIP, addressable by path. Uploaded lazily
    // when a page actually references them, each at most once.
    final filesByPath = <String, Uint8List>{
      for (final e in entries)
        if (!e.name.toLowerCase().endsWith('.md') &&
            e.name != 'manifest.json')
          e.name: e.bytes,
    };
    final zipPaths = filesByPath.keys.toSet();
    final uploadedByPath = <String, ({String fileId, String name})>{};

    Future<({String fileId, String name})?> uploadImageRef(
        String mdPath, String url) async {
      final zipPath = resolveZipPath(mdPath, url, zipPaths);
      if (zipPath == null) return null; // external URL or not in the ZIP
      final cached = uploadedByPath[zipPath];
      if (cached != null) return cached;
      final base = zipPath.split('/').last;
      final file = await _api.uploadImage(
        session.accessToken,
        workspace.id,
        fileName: base,
        mimeType: _guessImageMime(base),
        bytes: filesByPath[zipPath]!,
      );
      return uploadedByPath[zipPath] = (fileId: file.id, name: file.name);
    }

    // Create pages in manifest (page-tree) order so sibling order is
    // restored; files unknown to the manifest follow, parents-first.
    final mdByPath = {
      for (final e in entries)
        if (e.name.toLowerCase().endsWith('.md')) e.name: e,
    };
    final mds = [
      for (final p in orderPagePaths(mdByPath.keys, manifestJson))
        mdByPath[p]!,
    ];
    final viewByPath = <String, String>{};
    final folderView = <String, String>{}; // folder path -> its page's view
    final stamp = DateTime.now().microsecondsSinceEpoch;
    for (final e in mds) {
      final parts = e.name.split('/');
      // Walk the folder chain. A folder maps to the page exported as
      // `<folder>.md` when present; otherwise create an empty page to act
      // as the directory node.
      String? parentViewId;
      var folderPath = '';
      for (var d = 0; d + 1 < parts.length; d++) {
        folderPath =
            folderPath.isEmpty ? parts[d] : '$folderPath/${parts[d]}';
        var v = viewByPath['$folderPath.md'] ?? folderView[folderPath];
        if (v == null) {
          final dirPage = await _api.createDocument(
            session.accessToken,
            workspace.id,
            stripNotionId(parts[d]),
            parentViewId: parentViewId,
          );
          v = dirPage.view.id;
          folderView[folderPath] = v;
        }
        parentViewId = v;
      }
      final markdown = utf8.decode(e.bytes, allowMalformed: true);
      final fallback = stripNotionId(parts.last.replaceAll(
        RegExp(r'\.md$', caseSensitive: false),
        '',
      ));
      final title = _titleFromMarkdown(markdown, fallback);
      final created = await _api.createDocument(
        session.accessToken,
        workspace.id,
        title,
        parentViewId: parentViewId,
      );
      viewByPath[e.name] = created.view.id;

      final specs = markdownToBlocks(markdown);
      final ops = <Map<String, dynamic>>[];
      var index = 0;
      for (var k = 0; k < specs.length; k++) {
        // Skip a leading H1 that duplicates the page title.
        if (k == 0 &&
            specs[k].kind == 'heading' &&
            specs[k].text.trim() == title) {
          continue;
        }
        var data = specs[k].data;
        if (specs[k].kind == 'image') {
          final url = data['url'] as String?;
          if (url != null) {
            final up = await uploadImageRef(e.name, url);
            if (up != null) {
              data = {'file_id': up.fileId, 'name': up.name};
            }
          }
        }
        ops.add({
          'type': 'insert_block',
          'parent_id': created.document.rootBlockId,
          'index': index,
          'block': {
            'id': 'block_${stamp}_${viewByPath.length}_$k',
            'type': specs[k].kind,
            'text': specs[k].text,
            'data': data,
            'children': <String>[],
          },
        });
        index++;
      }
      if (ops.isNotEmpty) {
        await _api.applyDocumentUpdate(
          session.accessToken,
          workspace.id,
          created.document.id,
          ops,
        );
      }
    }
  }

  String _guessImageMime(String name) {
    final ext = name.contains('.') ? name.split('.').last.toLowerCase() : '';
    switch (ext) {
      case 'jpg':
      case 'jpeg':
        return 'image/jpeg';
      case 'gif':
        return 'image/gif';
      case 'webp':
        return 'image/webp';
      case 'svg':
        return 'image/svg+xml';
      case 'bmp':
        return 'image/bmp';
      default:
        return 'image/png';
    }
  }

  Future<void> _aiCurrentFromMarkdown(String markdown) {
    return _run(() async {
      final bootstrap = _selectedBootstrap;
      if (bootstrap == null) {
        throw const ApiException('Open a page first.');
      }
      final specs = markdownToBlocks(markdown);
      final root = bootstrap.document.rootBlockId;
      var index = bootstrap.childBlocks.length;
      final stamp = DateTime.now().microsecondsSinceEpoch;
      final ops = <Map<String, dynamic>>[];
      for (var i = 0; i < specs.length; i++) {
        ops.add({
          'type': 'insert_block',
          'parent_id': root,
          'index': index,
          'block': {
            'id': 'block_${stamp}_$i',
            'type': specs[i].kind,
            'text': specs[i].text,
            'data': specs[i].data,
            'children': <String>[],
          },
        });
        index++;
      }
      await _applyEditorOperations(ops);
    });
  }

  Future<void> _aiNewWorkspaceFromMarkdown(String markdown) {
    return _run(() async {
      final session = _requireSession();
      final title = _titleFromMarkdown(markdown, 'AI workspace');
      final workspace = await _api.createWorkspace(session.accessToken, title);
      final bootstrap = await _api.importMarkdown(
        session.accessToken,
        workspace.id,
        title,
        markdown,
      );
      setState(() {
        _workspaces = [..._workspaces, workspace];
        _selectedWorkspace = workspace;
        _viewsByWorkspace = {
          ..._viewsByWorkspace,
          workspace.id: [bootstrap.view],
        };
        _selectedView = bootstrap.view;
        _selectedBootstrap = bootstrap;
        _selectedMarkdown = null;
      });
      await _loadSelectedWorkspaceMembers();
    });
  }

  Future<void> _selectView(DocumentView view) {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      final bootstrap = await _api.bootstrapDocument(
        session.accessToken,
        workspace.id,
        view.objectId,
      );
      setState(() {
        _selectedView = view;
        _selectedBootstrap = bootstrap;
        _selectedMarkdown = null;
      });
    });
  }

  Future<void> _renameView(DocumentView view, String name) {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      final renamed = await _api.updateView(
        session.accessToken,
        workspace.id,
        view.id,
        name,
      );

      setState(() {
        final views = _viewsByWorkspace[workspace.id] ?? const [];
        _viewsByWorkspace = {
          ..._viewsByWorkspace,
          workspace.id: views
              .map((item) => item.id == renamed.id ? renamed : item)
              .toList(),
        };

        if (_selectedView?.id == renamed.id) {
          _selectedView = renamed;
        }

        final bootstrap = _selectedBootstrap;
        if (bootstrap != null && bootstrap.view.id == renamed.id) {
          _selectedBootstrap = DocumentBootstrap(
            document: bootstrap.document,
            view: renamed,
            snapshot: bootstrap.snapshot,
          );
        }
      });
    });
  }

  Future<void> _deleteView(DocumentView view) {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      final views = await _api.deleteView(
        session.accessToken,
        workspace.id,
        view.id,
      );

      setState(() {
        _viewsByWorkspace = {..._viewsByWorkspace, workspace.id: views};
        if (_selectedView?.id == view.id) {
          _selectedView = null;
          _selectedBootstrap = null;
          _selectedMarkdown = null;
        }
      });
    });
  }

  Future<void> _updateRootBlockText(String text) {
    final bootstrap = _selectedBootstrap;
    if (bootstrap == null) {
      return _run(() async {
        throw const ApiException('Select a page first.');
      });
    }

    return _applySelectedDocumentOperations([
      {
        'type': 'update_block',
        'block_id': bootstrap.document.rootBlockId,
        'text': text,
      },
    ]);
  }

  Future<void> _addBlock(DocumentBlockKind kind, String text) {
    final bootstrap = _selectedBootstrap;
    if (bootstrap == null) {
      return _run(() async {
        throw const ApiException('Select a page first.');
      });
    }

    return _applySelectedDocumentOperations([
      {
        'type': 'insert_block',
        'parent_id': bootstrap.document.rootBlockId,
        'block': {
          'id': 'block_${DateTime.now().microsecondsSinceEpoch}',
          'type': kind.apiValue,
          'text': text,
          'children': <String>[],
        },
      },
    ]);
  }

  Future<void> _updateBlock(
    DocumentBlock block,
    DocumentBlockKind kind,
    String text,
  ) {
    return _applySelectedDocumentOperations([
      {
        'type': 'update_block',
        'block_id': block.id,
        'kind': kind.apiValue,
        'text': text,
      },
    ]);
  }

  Future<void> _deleteBlock(DocumentBlock block) {
    return _applySelectedDocumentOperations([
      {'type': 'delete_block', 'block_id': block.id},
    ]);
  }

  Future<void> _moveBlock(DocumentBlock block, int targetIndex) {
    final bootstrap = _selectedBootstrap;
    if (bootstrap == null) {
      return _run(() async {
        throw const ApiException('Select a page first.');
      });
    }

    return _applySelectedDocumentOperations([
      {
        'type': 'move_block',
        'block_id': block.id,
        'parent_id': bootstrap.document.rootBlockId,
        'index': targetIndex,
      },
    ]);
  }

  Future<void> _applySelectedDocumentOperations(
    List<Map<String, dynamic>> operations,
  ) {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      final bootstrap = _selectedBootstrap;
      if (bootstrap == null) {
        throw const ApiException('Select a page first.');
      }

      final result = await _api.applyDocumentUpdate(
        session.accessToken,
        workspace.id,
        bootstrap.document.id,
        operations,
      );

      setState(() {
        _selectedBootstrap = DocumentBootstrap(
          document: result.document,
          view: bootstrap.view,
          snapshot: result.snapshot,
        );
        _selectedMarkdown = null;
      });
    });
  }

  /// Apply editor operations without toggling the global busy state, so inline
  /// typing in the block editor stays smooth. Errors surface in the banner.
  Future<void> _applyEditorOperations(List<Map<String, dynamic>> operations) async {
    final session = _session;
    final workspace = _selectedWorkspace;
    final bootstrap = _selectedBootstrap;
    if (session == null || workspace == null || bootstrap == null) {
      return;
    }

    try {
      final result = await _api.applyDocumentUpdate(
        session.accessToken,
        workspace.id,
        bootstrap.document.id,
        operations,
      );
      if (!mounted || _selectedBootstrap?.document.id != bootstrap.document.id) {
        return;
      }
      setState(() {
        _selectedBootstrap = DocumentBootstrap(
          document: result.document,
          view: _selectedBootstrap!.view,
          snapshot: result.snapshot,
        );
        _selectedMarkdown = null;
      });
    } catch (error) {
      if (mounted) {
        setState(() => _message = error.toString());
      }
    }
  }

  /// Upload image bytes for the editor, returning the new file id + name.
  Future<({String fileId, String name})?> _uploadEditorImage(
    Uint8List bytes,
    String fileName,
    String mimeType,
  ) async {
    final session = _session;
    final workspace = _selectedWorkspace;
    if (session == null || workspace == null) return null;
    try {
      final file = await _api.uploadImage(
        session.accessToken,
        workspace.id,
        fileName: fileName,
        mimeType: mimeType,
        bytes: bytes,
      );
      return (fileId: file.id, name: file.name);
    } catch (error) {
      if (mounted) setState(() => _message = error.toString());
      return null;
    }
  }

  /// Re-host a pasted image URL server-side, returning the new file id + name.
  Future<({String fileId, String name})?> _importEditorImageUrl(
    String url,
  ) async {
    final session = _session;
    final workspace = _selectedWorkspace;
    if (session == null || workspace == null) return null;
    try {
      final file = await _api.importImageUrl(
        session.accessToken,
        workspace.id,
        url,
      );
      return (fileId: file.id, name: file.name);
    } catch (error) {
      if (mounted) setState(() => _message = error.toString());
      return null;
    }
  }

  /// Map image file ids to their permanent Mica blob links (stable, never
  /// expiring — the endpoint re-signs storage on each request). Used by copy so
  /// pasted images keep displaying anywhere.
  Future<Map<String, String>> _resolveEditorImageUrls(List<String> ids) async {
    final workspace = _selectedWorkspace;
    if (workspace == null || ids.isEmpty) return {};
    final base = _api.baseUri;
    final origin = '${base.scheme}://${base.host}:${base.port}';
    return {
      for (final id in ids)
        id: '$origin/api/workspaces/${workspace.id}/files/$id/blob',
    };
  }

  /// Fetch an image's bytes for the canvas. The key is either an external URL
  /// (markdown image) — fetched directly — or a file id, resolved to a fresh
  /// signed URL first.
  Future<Uint8List?> _loadEditorImageBytes(String key) async {
    final session = _session;
    final workspace = _selectedWorkspace;
    if (session == null || workspace == null) return null;
    try {
      String? url;
      if (key.startsWith('http://') || key.startsWith('https://')) {
        url = key;
      } else {
        final urls = await _api.resolveFiles(
          session.accessToken,
          workspace.id,
          [key],
        );
        url = urls[key];
      }
      if (url == null) return null;
      final resp = await http.get(Uri.parse(url));
      if (resp.statusCode != 200) return null;
      return resp.bodyBytes;
    } catch (_) {
      return null;
    }
  }

  Future<void> _exportSelectedMarkdown() {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      final bootstrap = _selectedBootstrap;
      if (bootstrap == null) {
        throw const ApiException('Select a page first.');
      }

      final markdown = await _api.exportMarkdown(
        session.accessToken,
        workspace.id,
        bootstrap.document.id,
      );

      setState(() {
        _selectedMarkdown = markdown;
      });
    });
  }

  Future<void> _addWorkspaceMember(String email, WorkspaceRole role) {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      await _api.addWorkspaceMember(
        session.accessToken,
        workspace.id,
        email,
        role.apiValue,
      );
      await _loadSelectedWorkspaceMembers();
    });
  }

  Future<void> _updateWorkspaceMember(
    WorkspaceMember member,
    WorkspaceRole role,
  ) {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      await _api.updateWorkspaceMember(
        session.accessToken,
        workspace.id,
        member.userId,
        role.apiValue,
      );
      await _loadSelectedWorkspaceMembers();
    });
  }

  Future<void> _removeWorkspaceMember(WorkspaceMember member) {
    return _run(() async {
      final session = _requireSession();
      final workspace = _requireWorkspace();
      final members = await _api.removeWorkspaceMember(
        session.accessToken,
        workspace.id,
        member.userId,
      );
      setState(() {
        _membersByWorkspace = {..._membersByWorkspace, workspace.id: members};
      });
    });
  }

  AuthSession _requireSession() {
    final session = _session;
    if (session == null) {
      throw const ApiException('Sign in first.');
    }
    return session;
  }

  Workspace _requireWorkspace() {
    final workspace = _selectedWorkspace;
    if (workspace == null) {
      throw const ApiException('Select a workspace first.');
    }
    return workspace;
  }

  Future<void> _loadSelectedWorkspaceMembers() async {
    final session = _session;
    final workspace = _selectedWorkspace;
    if (session == null || workspace == null) {
      return;
    }

    final members = await _api.listWorkspaceMembers(
      session.accessToken,
      workspace.id,
    );
    setState(() {
      _membersByWorkspace = {..._membersByWorkspace, workspace.id: members};
    });
  }

  void _signOut() {
    _closeDocumentSync();
    setState(() {
      _session = null;
      _workspaces = const [];
      _membersByWorkspace = const {};
      _viewsByWorkspace = const {};
      _selectedWorkspace = null;
      _selectedView = null;
      _selectedBootstrap = null;
      _selectedMarkdown = null;
      _message = null;
    });
  }

  Future<void> _loadSelectedWorkspaceViews() async {
    final session = _session;
    final workspace = _selectedWorkspace;
    if (session == null || workspace == null) {
      return;
    }

    final views = await _api.listViews(session.accessToken, workspace.id);
    final selectedView = views
        .where((view) => view.id == _selectedView?.id)
        .firstOrNull;
    final viewToOpen = selectedView ?? views.firstOrNull;
    final bootstrap = viewToOpen == null
        ? null
        : await _api.bootstrapDocument(
            session.accessToken,
            workspace.id,
            viewToOpen.objectId,
          );

    setState(() {
      _viewsByWorkspace = {..._viewsByWorkspace, workspace.id: views};
      _selectedView = viewToOpen;
      _selectedBootstrap = bootstrap;
      _selectedMarkdown = null;
    });
  }

  @override
  Widget build(BuildContext context) {
    final session = _session;

    return Scaffold(
      body: SafeArea(
        child: session == null
            ? Row(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  SizedBox(
                    width: 360,
                    child: SidePanel(
                      session: session,
                      isBusy: _isBusy,
                      onAuthenticate: _authenticate,
                      onCreateWorkspace: _createWorkspace,
                    ),
                  ),
                  const VerticalDivider(width: 1),
                  const Expanded(
                    child: EmptyState(
                      icon: Icons.description_outlined,
                      title: 'Mica',
                      detail: 'Sign in to open your workspace.',
                    ),
                  ),
                ],
              )
            : WorkspaceView(
                session: session,
                isBusy: _isBusy,
                onRefresh: () {
                  if (!_isBusy) _refreshWorkspaces();
                },
                onSignOut: () {
                  if (!_isBusy) _signOut();
                },
                workspaces: _workspaces,
                selectedWorkspace: _selectedWorkspace,
                members: _selectedWorkspace == null
                    ? const []
                    : _membersByWorkspace[_selectedWorkspace!.id] ?? const [],
                views: _selectedWorkspace == null
                    ? const []
                    : _viewsByWorkspace[_selectedWorkspace!.id] ?? const [],
                selectedView: _selectedView,
                selectedBootstrap: _selectedBootstrap,
                selectedMarkdown: _selectedMarkdown,
                presence: _presence,
                message: _message,
                onSelectWorkspace: _selectWorkspace,
                onCreateWorkspace: _createWorkspace,
                onRenameWorkspace: _renameWorkspace,
                onDeleteWorkspace: _deleteWorkspace,
                onCreateDocument: _createDocument,
                onCreateChildDocument: _createChildDocument,
                onReorderViews: _reorderViews,
                onLoadTrash: _loadTrash,
                onRestoreView: _restoreView,
                onPurgeView: _purgeView,
                onSelectView: _selectView,
                onRenameView: _renameView,
                onDeleteView: _deleteView,
                onUpdateRootBlockText: _updateRootBlockText,
                onAddBlock: _addBlock,
                onUpdateBlock: _updateBlock,
                onDeleteBlock: _deleteBlock,
                onMoveBlock: _moveBlock,
                onApplyOperations: _applyEditorOperations,
                onUploadImage: _uploadEditorImage,
                onImportImageUrl: _importEditorImageUrl,
                onLoadImageBytes: _loadEditorImageBytes,
                onResolveImageUrls: _resolveEditorImageUrls,
                onAiStream: _aiStream,
                onAiNewPage: _aiNewPageFromMarkdown,
                onAiCurrentPage: _selectedBootstrap == null
                    ? null
                    : _aiCurrentFromMarkdown,
                onAiNewWorkspace: _aiNewWorkspaceFromMarkdown,
                onLoadAiSettings: _loadAiSettings,
                onSaveAiSettings: _saveAiSettings,
                userName: _session?.user.displayName ?? '',
                userEmail: _session?.user.email ?? '',
                onUpdateProfile: _updateProfile,
                onChangePassword: _changePassword,
                appearance: _appearance,
                pageWidth: _pageWidth,
                reHostImages: _reHostImages,
                onReHostImagesChanged: (value) =>
                    setState(() => _reHostImages = value),
                onAppearanceChanged: (appearance, pageWidth) {
                  setState(() {
                    _appearance = appearance;
                    _pageWidth = pageWidth;
                  });
                },
                onSearch: _searchWorkspace,
                onOpenSearchResult: _openViewById,
                onExportPageMarkdown: _exportPageMarkdown,
                onExportPageZip: _exportPageZip,
                onImportMarkdown: _importMarkdownAsPage,
                onExportWorkspaceMarkdown: _exportWorkspaceMarkdown,
                onExportWorkspaceZip: _exportWorkspaceZip,
                onImportWorkspaceZip: _importWorkspaceZip,
                onImportWorkspaceTreeInto: _importTreeIntoWorkspace,
                onExportAllMarkdown: _exportAllMarkdown,
                onExportMarkdown: _exportSelectedMarkdown,
                onAddMember: _addWorkspaceMember,
                onUpdateMember: _updateWorkspaceMember,
                onRemoveMember: _removeWorkspaceMember,
              ),
      ),
    );
  }
}

class SidePanel extends StatefulWidget {
  const SidePanel({
    required this.session,
    required this.isBusy,
    required this.onAuthenticate,
    required this.onCreateWorkspace,
    super.key,
  });

  final AuthSession? session;
  final bool isBusy;
  final Future<void> Function(AuthMode mode, AuthFormValue form) onAuthenticate;
  final Future<void> Function(String name) onCreateWorkspace;

  @override
  State<SidePanel> createState() => _SidePanelState();
}

class _SidePanelState extends State<SidePanel> {
  AuthMode _mode = AuthMode.login;
  final _email = TextEditingController();
  final _displayName = TextEditingController();
  final _password = TextEditingController();
  final _workspaceName = TextEditingController();

  @override
  void dispose() {
    _email.dispose();
    _displayName.dispose();
    _password.dispose();
    _workspaceName.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final session = widget.session;

    return ColoredBox(
      color: Colors.white,
      child: Padding(
        padding: const EdgeInsets.all(20),
        child: session == null ? _authForm(context) : _workspaceForm(context),
      ),
    );
  }

  Widget _authForm(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text('Account', style: Theme.of(context).textTheme.titleLarge),
        const SizedBox(height: 16),
        SegmentedButton<AuthMode>(
          segments: const [
            ButtonSegment(
              value: AuthMode.login,
              icon: Icon(Icons.login),
              label: Text('Login'),
            ),
            ButtonSegment(
              value: AuthMode.register,
              icon: Icon(Icons.person_add),
              label: Text('Register'),
            ),
          ],
          selected: {_mode},
          onSelectionChanged: widget.isBusy
              ? null
              : (selection) {
                  setState(() {
                    _mode = selection.single;
                  });
                },
        ),
        const SizedBox(height: 18),
        TextField(
          controller: _email,
          enabled: !widget.isBusy,
          keyboardType: TextInputType.emailAddress,
          decoration: const InputDecoration(
            labelText: 'Email',
            prefixIcon: Icon(Icons.alternate_email),
            border: OutlineInputBorder(),
          ),
        ),
        const SizedBox(height: 12),
        if (_mode == AuthMode.register) ...[
          TextField(
            controller: _displayName,
            enabled: !widget.isBusy,
            decoration: const InputDecoration(
              labelText: 'Display name',
              prefixIcon: Icon(Icons.badge),
              border: OutlineInputBorder(),
            ),
          ),
          const SizedBox(height: 12),
        ],
        TextField(
          controller: _password,
          enabled: !widget.isBusy,
          obscureText: true,
          decoration: const InputDecoration(
            labelText: 'Password',
            prefixIcon: Icon(Icons.lock),
            border: OutlineInputBorder(),
          ),
        ),
        const SizedBox(height: 16),
        FilledButton.icon(
          onPressed: widget.isBusy ? null : _submitAuth,
          icon: Icon(
            _mode == AuthMode.register ? Icons.person_add : Icons.login,
          ),
          label: Text(_mode == AuthMode.register ? 'Register' : 'Login'),
        ),
      ],
    );
  }

  Widget _workspaceForm(BuildContext context) {
    final session = widget.session!;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text('Workspace', style: Theme.of(context).textTheme.titleLarge),
        const SizedBox(height: 4),
        Text(
          session.user.email,
          style: Theme.of(context).textTheme.bodySmall,
          overflow: TextOverflow.ellipsis,
        ),
        const SizedBox(height: 20),
        TextField(
          controller: _workspaceName,
          enabled: !widget.isBusy,
          decoration: const InputDecoration(
            labelText: 'Workspace name',
            prefixIcon: Icon(Icons.workspaces),
            border: OutlineInputBorder(),
          ),
        ),
        const SizedBox(height: 12),
        FilledButton.icon(
          onPressed: widget.isBusy ? null : _submitWorkspace,
          icon: const Icon(Icons.add),
          label: const Text('Create'),
        ),
      ],
    );
  }

  void _submitAuth() {
    widget.onAuthenticate(
      _mode,
      AuthFormValue(
        email: _email.text,
        displayName: _displayName.text,
        password: _password.text,
      ),
    );
  }

  void _submitWorkspace() {
    widget.onCreateWorkspace(_workspaceName.text);
    _workspaceName.clear();
  }
}

class WorkspaceView extends StatefulWidget {
  const WorkspaceView({
    required this.session,
    required this.isBusy,
    required this.onRefresh,
    required this.onSignOut,
    required this.workspaces,
    required this.selectedWorkspace,
    required this.members,
    required this.views,
    required this.selectedView,
    required this.selectedBootstrap,
    required this.selectedMarkdown,
    required this.presence,
    required this.message,
    required this.onSelectWorkspace,
    required this.onCreateWorkspace,
    required this.onRenameWorkspace,
    required this.onDeleteWorkspace,
    required this.onCreateDocument,
    required this.onCreateChildDocument,
    required this.onReorderViews,
    required this.onLoadTrash,
    required this.onRestoreView,
    required this.onPurgeView,
    required this.onSelectView,
    required this.onRenameView,
    required this.onDeleteView,
    required this.onUpdateRootBlockText,
    required this.onAddBlock,
    required this.onUpdateBlock,
    required this.onDeleteBlock,
    required this.onMoveBlock,
    required this.onApplyOperations,
    required this.onUploadImage,
    required this.onImportImageUrl,
    required this.onLoadImageBytes,
    required this.onResolveImageUrls,
    required this.onAiStream,
    required this.onAiNewPage,
    required this.onAiCurrentPage,
    required this.onAiNewWorkspace,
    required this.onLoadAiSettings,
    required this.onSaveAiSettings,
    required this.userName,
    required this.userEmail,
    required this.onUpdateProfile,
    required this.onChangePassword,
    required this.appearance,
    required this.pageWidth,
    required this.reHostImages,
    required this.onReHostImagesChanged,
    required this.onAppearanceChanged,
    required this.onSearch,
    required this.onOpenSearchResult,
    required this.onExportPageMarkdown,
    required this.onExportPageZip,
    required this.onImportMarkdown,
    required this.onExportWorkspaceMarkdown,
    required this.onExportWorkspaceZip,
    required this.onImportWorkspaceZip,
    required this.onImportWorkspaceTreeInto,
    required this.onExportAllMarkdown,
    required this.onExportMarkdown,
    required this.onAddMember,
    required this.onUpdateMember,
    required this.onRemoveMember,
    super.key,
  });

  final AuthSession? session;
  final bool isBusy;
  final VoidCallback onRefresh;
  final VoidCallback onSignOut;
  final List<Workspace> workspaces;
  final Workspace? selectedWorkspace;
  final List<WorkspaceMember> members;
  final List<DocumentView> views;
  final DocumentView? selectedView;
  final DocumentBootstrap? selectedBootstrap;
  final String? selectedMarkdown;
  final List<PresenceUser> presence;
  final String? message;
  final Future<void> Function(Workspace workspace) onSelectWorkspace;
  final Future<void> Function(String name) onCreateWorkspace;
  final Future<void> Function(Workspace workspace, String name)
  onRenameWorkspace;
  final Future<void> Function(Workspace workspace) onDeleteWorkspace;
  final Future<void> Function(String name) onCreateDocument;
  final Future<void> Function(DocumentView parent, String name)
  onCreateChildDocument;
  final Future<void> Function(String? parentViewId, List<DocumentView> ordered)
  onReorderViews;
  final Future<List<DocumentView>> Function() onLoadTrash;
  final Future<void> Function(DocumentView view) onRestoreView;
  final Future<void> Function(DocumentView view) onPurgeView;
  final Future<void> Function(DocumentView view) onSelectView;
  final Future<void> Function(DocumentView view, String name) onRenameView;
  final Future<void> Function(DocumentView view) onDeleteView;
  final Future<void> Function(String text) onUpdateRootBlockText;
  final Future<void> Function(DocumentBlockKind kind, String text) onAddBlock;
  final Future<void> Function(
    DocumentBlock block,
    DocumentBlockKind kind,
    String text,
  )
  onUpdateBlock;
  final Future<void> Function(DocumentBlock block) onDeleteBlock;
  final Future<void> Function(DocumentBlock block, int targetIndex) onMoveBlock;
  final Future<void> Function(List<Map<String, dynamic>> operations)
  onApplyOperations;
  final Future<({String fileId, String name})?> Function(
    Uint8List bytes,
    String fileName,
    String mimeType,
  )
  onUploadImage;
  final Future<({String fileId, String name})?> Function(String url)
  onImportImageUrl;
  final Future<Uint8List?> Function(String fileId) onLoadImageBytes;
  final Future<Map<String, String>> Function(List<String> fileIds)
  onResolveImageUrls;
  final Stream<String> Function(String prompt, {String? system}) onAiStream;
  final Future<void> Function(String markdown) onAiNewPage;
  final Future<void> Function(String markdown)? onAiCurrentPage;
  final Future<void> Function(String markdown) onAiNewWorkspace;
  final Future<Map<String, dynamic>> Function() onLoadAiSettings;
  final Future<void> Function({
    required String provider,
    required String baseUrl,
    required String model,
    String? apiKey,
  })
  onSaveAiSettings;
  final String userName;
  final String userEmail;
  final Future<void> Function(String displayName) onUpdateProfile;
  final Future<void> Function(String current, String next) onChangePassword;
  final EditorAppearance appearance;
  final double pageWidth;
  final bool reHostImages;
  final void Function(bool value) onReHostImagesChanged;
  final void Function(EditorAppearance appearance, double pageWidth)
  onAppearanceChanged;
  final Future<List<SearchResult>> Function(String query) onSearch;
  final Future<void> Function(String viewId) onOpenSearchResult;
  final Future<String> Function() onExportPageMarkdown;
  final Future<Uint8List> Function() onExportPageZip;
  final Future<void> Function(String fileName, String markdown) onImportMarkdown;
  final Future<String> Function() onExportWorkspaceMarkdown;
  final Future<Uint8List> Function(String workspaceId) onExportWorkspaceZip;
  final Future<void> Function(String fileName, Uint8List bytes)
  onImportWorkspaceZip;
  final Future<void> Function(Workspace workspace, List<ZipFileEntry> entries)
  onImportWorkspaceTreeInto;
  final Future<String> Function() onExportAllMarkdown;
  final Future<void> Function() onExportMarkdown;
  final Future<void> Function(String email, WorkspaceRole role) onAddMember;
  final Future<void> Function(WorkspaceMember member, WorkspaceRole role)
  onUpdateMember;
  final Future<void> Function(WorkspaceMember member) onRemoveMember;

  @override
  State<WorkspaceView> createState() => _WorkspaceViewState();
}

class _WorkspaceViewState extends State<WorkspaceView> {
  final _rename = TextEditingController();
  final _memberEmail = TextEditingController();
  final _pageTitle = TextEditingController();
  final FocusNode _editorFocus = FocusNode(debugLabel: 'MicaEditorBody');
  Timer? _pageTitleSaveTimer;
  final Set<String> _collapsedViewIds = {};
  // True only while a page is being dragged in the tree. The drop zones overlay
  // each row, so they are mounted only during a drag — otherwise they would
  // intercept ordinary taps on the page rows.
  bool _draggingTree = false;
  WorkspaceRole _memberRole = WorkspaceRole.editor;
  bool _toolsExpanded = false;
  bool _workspaceSettingsOpen = false;
  final EditorScrollHook _scrollHook = EditorScrollHook();

  @override
  void didUpdateWidget(covariant WorkspaceView oldWidget) {
    super.didUpdateWidget(oldWidget);
    final selected = widget.selectedWorkspace;
    if (selected != null && selected.id != oldWidget.selectedWorkspace?.id) {
      _rename.text = selected.name;
      _collapsedViewIds.clear();
    }

    final bootstrap = widget.selectedBootstrap;
    if (bootstrap?.view.id != oldWidget.selectedBootstrap?.view.id ||
        bootstrap?.view.name != oldWidget.selectedBootstrap?.view.name) {
      _pageTitle.text = bootstrap?.view.name ?? '';
    }
  }

  @override
  void dispose() {
    _rename.dispose();
    _memberEmail.dispose();
    _pageTitle.dispose();
    _editorFocus.dispose();
    _pageTitleSaveTimer?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    if (widget.session == null) {
      return const EmptyState(
        icon: Icons.login,
        title: 'Sign in',
        detail: 'Register or log in to open your workspace list.',
      );
    }

    return Row(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SizedBox(width: 280, child: _navigationPane(context)),
        const VerticalDivider(width: 1),
        Expanded(child: _editorPane(context)),
        if (_toolsExpanded) ...[
          const VerticalDivider(width: 1),
          SizedBox(width: 300, child: _workspaceTools(context)),
        ],
      ],
    );
  }

  Widget _navigationPane(BuildContext context) {
    final canEdit = matchesEditRole(widget.selectedWorkspace?.role);

    return ColoredBox(
      color: Colors.white,
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            // App header — top-left, grouped with the sidebar (no global AppBar).
            Row(
              children: [
                const MicaLogo(size: 24),
                const SizedBox(width: 8),
                const Text(
                  'Mica',
                  style: TextStyle(fontSize: 20, fontWeight: FontWeight.w700),
                ),
                const Spacer(),
                if (widget.isBusy)
                  const Padding(
                    padding: EdgeInsets.only(right: 4),
                    child: SizedBox(
                      height: 16,
                      width: 16,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    ),
                  ),
                IconButton(
                  tooltip: 'Refresh',
                  visualDensity: VisualDensity.compact,
                  onPressed: widget.onRefresh,
                  icon: const Icon(Icons.refresh, size: 20),
                ),
              ],
            ),
            const SizedBox(height: 14),
            SizedBox(
              width: double.infinity,
              child: FilledButton.tonalIcon(
                onPressed: _openAiDialog,
                icon: const Icon(Icons.auto_awesome, size: 18),
                label: const Text('Ask AI'),
              ),
            ),
            const SizedBox(height: 16),
            Row(
              children: [
                Text('Workspace',
                    style: Theme.of(context).textTheme.titleMedium),
                const Spacer(),
                IconButton(
                  tooltip: 'Workspace settings',
                  visualDensity: VisualDensity.compact,
                  isSelected: _workspaceSettingsOpen,
                  onPressed: widget.selectedWorkspace == null
                      ? null
                      : () => setState(
                          () => _workspaceSettingsOpen = !_workspaceSettingsOpen),
                  icon: const Icon(Icons.tune, size: 20),
                ),
              ],
            ),
            const SizedBox(height: 10),
            _WorkspaceSelector(
              workspaces: widget.workspaces,
              selected: widget.selectedWorkspace,
              onSelect: widget.onSelectWorkspace,
              onRename: _promptRenameWorkspace,
              onDelete: _confirmDeleteWorkspace,
              onExport: _exportWorkspaceFile,
              onCreate: _promptCreateWorkspace,
              onImport: _importWorkspaceFile,
              onImportFilesInto: _importFilesIntoWorkspace,
              onImportFolderInto: _importFolderIntoWorkspace,
            ),
            if (_workspaceSettingsOpen) _workspaceSettings(context),
            if (widget.message != null) ...[
              const SizedBox(height: 12),
              ErrorBanner(widget.message!),
            ],
            const SizedBox(height: 20),
            Row(
              children: [
                const Icon(Icons.account_tree_outlined),
                const SizedBox(width: 8),
                Text('Pages', style: Theme.of(context).textTheme.titleMedium),
                const Spacer(),
                IconButton(
                  tooltip: 'Search',
                  visualDensity: VisualDensity.compact,
                  onPressed: widget.selectedWorkspace == null
                      ? null
                      : _openSearch,
                  icon: const Icon(Icons.search),
                ),
                if (canEdit) ...[
                  IconButton(
                    tooltip: 'Recycle bin',
                    visualDensity: VisualDensity.compact,
                    onPressed: _openRecycleBin,
                    icon: const Icon(Icons.delete_outline),
                  ),
                  IconButton(
                    tooltip: 'New page',
                    visualDensity: VisualDensity.compact,
                    onPressed: () {
                      widget.onCreateDocument('Untitled');
                    },
                    icon: const Icon(Icons.note_add_outlined),
                  ),
                ],
              ],
            ),
            const SizedBox(height: 12),
            Expanded(child: _pageTree(context, canEdit)),
            const Divider(height: 24),
            _accountTile(context),
          ],
        ),
      ),
    );
  }

  /// Account row pinned to the bottom of the left sidebar. Tapping opens a menu
  /// with Settings and Sign out.
  Widget _accountTile(BuildContext context) {
    final user = widget.session?.user;
    final name = user?.displayName.isNotEmpty == true
        ? user!.displayName
        : (user?.email ?? 'Account');
    final initial = name.isNotEmpty ? name.substring(0, 1).toUpperCase() : '?';
    return PopupMenuButton<String>(
      tooltip: 'Account',
      position: PopupMenuPosition.under,
      // Match the sidebar content width (280 panel − 16 padding each side).
      constraints: const BoxConstraints(minWidth: 248, maxWidth: 248),
      onSelected: (value) {
        switch (value) {
          case 'settings':
            _openSettings();
          case 'signout':
            widget.onSignOut();
        }
      },
      itemBuilder: (context) => const [
        PopupMenuItem(
          value: 'settings',
          child: ListTile(
            dense: true,
            contentPadding: EdgeInsets.zero,
            leading: Icon(Icons.settings_outlined),
            title: Text('Settings'),
          ),
        ),
        PopupMenuItem(
          value: 'signout',
          child: ListTile(
            dense: true,
            contentPadding: EdgeInsets.zero,
            leading: Icon(Icons.logout),
            title: Text('Sign out'),
          ),
        ),
      ],
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: 6, horizontal: 4),
        child: Row(
          children: [
            CircleAvatar(
              radius: 16,
              backgroundColor: const Color(0xFFE2E8F0),
              child: Text(
                initial,
                style: const TextStyle(
                  fontSize: 14,
                  fontWeight: FontWeight.w600,
                  color: Color(0xFF334155),
                ),
              ),
            ),
            const SizedBox(width: 10),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                mainAxisSize: MainAxisSize.min,
                children: [
                  Text(
                    name,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(
                      fontSize: 14,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                  if (user?.email != null)
                    Text(
                      user!.email,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: const TextStyle(
                        fontSize: 12,
                        color: Color(0xFF64748B),
                      ),
                    ),
                ],
              ),
            ),
            const Icon(Icons.more_vert, size: 18, color: Color(0xFF94A3B8)),
          ],
        ),
      ),
    );
  }

  Widget _pageTree(BuildContext context, bool canEdit) {
    if (widget.selectedWorkspace == null) {
      return const EmptyState(
        icon: Icons.ads_click,
        title: 'Select workspace',
        detail: 'Pages appear after selecting a workspace.',
      );
    }

    if (widget.views.isEmpty) {
      return const EmptyState(
        icon: Icons.note_add,
        title: 'No pages',
        detail: 'Create a page to start writing.',
      );
    }

    return ListView(
      children: _visibleDocumentTree().map((item) {
        final row = DocumentListItem(
          view: item.view,
          depth: item.depth,
          hasChildren: item.hasChildren,
          isCollapsed: _collapsedViewIds.contains(item.view.id),
          isSelected: item.view.id == widget.selectedView?.id,
          canEdit: canEdit,
          onToggle: () => _toggleViewCollapse(item.view),
          onPressed: () => widget.onSelectView(item.view),
          onCreateChild: () {
            setState(() => _collapsedViewIds.remove(item.view.id));
            widget.onCreateChildDocument(item.view, 'Untitled');
          },
          onRename: () => _promptRenameDocument(item.view),
          onDelete: () => widget.onDeleteView(item.view),
        );
        if (!canEdit) {
          return Padding(
            padding: const EdgeInsets.only(bottom: 8),
            child: row,
          );
        }
        return _draggableTreeRow(item.view, row);
      }).toList(),
    );
  }

  /// Wrap a page row so it can be dragged to reorder among its siblings (its
  /// subtree follows, since children render under their parent). Long-press to
  /// start dragging; the top/bottom half of each sibling row is a drop slot
  /// (insert before / after).
  Widget _draggableTreeRow(DocumentView view, Widget row) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 8),
      child: LongPressDraggable<DocumentView>(
        data: view,
        dragAnchorStrategy: pointerDragAnchorStrategy,
        onDragStarted: () => setState(() => _draggingTree = true),
        onDragEnd: (_) => setState(() => _draggingTree = false),
        onDraggableCanceled: (_, _) => setState(() => _draggingTree = false),
        onDragCompleted: () => setState(() => _draggingTree = false),
        feedback: Material(
          elevation: 6,
          borderRadius: BorderRadius.circular(6),
          child: Container(
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
            constraints: const BoxConstraints(maxWidth: 260),
            decoration: BoxDecoration(
              color: Colors.white,
              borderRadius: BorderRadius.circular(6),
            ),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                const Icon(
                  Icons.description_outlined,
                  size: 18,
                  color: Color(0xFF2563EB),
                ),
                const SizedBox(width: 8),
                Flexible(
                  child: Text(view.name, overflow: TextOverflow.ellipsis),
                ),
              ],
            ),
          ),
        ),
        childWhenDragging: Opacity(opacity: 0.4, child: row),
        child: Stack(
          children: [
            row,
            // Drop zones exist only during a drag, so they never block taps.
            if (_draggingTree)
              Positioned.fill(
                child: Column(
                  children: [
                    Expanded(flex: 3, child: _dropSlot(view, _DropMode.before)),
                    Expanded(flex: 4, child: _dropSlot(view, _DropMode.into)),
                    Expanded(flex: 3, child: _dropSlot(view, _DropMode.after)),
                  ],
                ),
              ),
          ],
        ),
      ),
    );
  }

  Widget _dropSlot(DocumentView target, _DropMode mode) {
    return DragTarget<DocumentView>(
      // The whole zone must be droppable; DragTarget defaults to deferToChild,
      // which would limit the hit area to the thin indicator. These overlays
      // only exist during a drag, so opaque is safe (no taps to intercept).
      hitTestBehavior: HitTestBehavior.opaque,
      onWillAcceptWithDetails: (details) =>
          !_isSelfOrDescendant(target.id, details.data.id),
      onAcceptWithDetails: (details) => _handleDrop(details.data, target, mode),
      builder: (context, candidate, rejected) {
        final active = candidate.isNotEmpty;
        if (mode == _DropMode.into) {
          // Nesting: highlight the whole target row.
          return Container(
            decoration: BoxDecoration(
              borderRadius: BorderRadius.circular(6),
              border: Border.all(
                color: active ? const Color(0xFF2563EB) : Colors.transparent,
                width: 2,
              ),
              color: active ? const Color(0x142563EB) : Colors.transparent,
            ),
          );
        }
        return Align(
          alignment: mode == _DropMode.before
              ? Alignment.topCenter
              : Alignment.bottomCenter,
          child: Container(
            height: 2,
            color: active ? const Color(0xFF2563EB) : Colors.transparent,
          ),
        );
      },
    );
  }

  /// True when [candidateId] is [rootId] itself or anywhere inside its subtree —
  /// dropping there would create a cycle, so it must be rejected.
  bool _isSelfOrDescendant(String candidateId, String rootId) {
    if (candidateId == rootId) {
      return true;
    }
    final parents = {for (final v in widget.views) v.id: v.parentViewId};
    String? cursor = candidateId;
    final seen = <String>{};
    while (cursor != null && seen.add(cursor)) {
      final parent = parents[cursor];
      if (parent == rootId) {
        return true;
      }
      cursor = parent;
    }
    return false;
  }

  void _handleDrop(DocumentView dragged, DocumentView target, _DropMode mode) {
    if (mode == _DropMode.into) {
      final children =
          widget.views
              .where((v) => v.parentViewId == target.id && v.id != dragged.id)
              .toList()
            ..sort((a, b) => a.position.compareTo(b.position));
      children.add(dragged);
      setState(() => _collapsedViewIds.remove(target.id)); // reveal new child
      widget.onReorderViews(target.id, children);
      return;
    }

    final parentId = target.parentViewId;
    final siblings =
        widget.views
            .where((v) => v.parentViewId == parentId && v.id != dragged.id)
            .toList()
          ..sort((a, b) => a.position.compareTo(b.position));
    final targetIndex = siblings.indexWhere((v) => v.id == target.id);
    if (targetIndex < 0) {
      return;
    }
    siblings.insert(
      mode == _DropMode.before ? targetIndex : targetIndex + 1,
      dragged,
    );
    widget.onReorderViews(parentId, siblings);
  }

  List<({DocumentView view, int depth, bool hasChildren})>
  _visibleDocumentTree() {
    final childrenByParent = <String?, List<DocumentView>>{};
    for (final view in widget.views) {
      childrenByParent.putIfAbsent(view.parentViewId, () => []).add(view);
    }

    for (final children in childrenByParent.values) {
      children.sort((left, right) => left.position.compareTo(right.position));
    }

    final ordered = <({DocumentView view, int depth, bool hasChildren})>[];
    final visited = <String>{};

    void appendChildren(String? parentId, int depth) {
      final children = childrenByParent[parentId] ?? const <DocumentView>[];
      for (final child in children) {
        if (!visited.add(child.id)) {
          continue;
        }

        final hasChildren = (childrenByParent[child.id] ?? const []).isNotEmpty;
        ordered.add((view: child, depth: depth, hasChildren: hasChildren));
        if (!_collapsedViewIds.contains(child.id)) {
          appendChildren(child.id, depth + 1);
        }
      }
    }

    appendChildren(null, 0);
    // Surface genuine orphans (parent missing) at the top level. A node that is
    // unvisited only because an ancestor is collapsed must stay hidden — not be
    // dumped at depth 0 (which made new children of a collapsed parent appear as
    // siblings of it).
    final viewIds = {for (final view in widget.views) view.id};
    for (final view in widget.views) {
      if (visited.contains(view.id)) {
        continue;
      }
      final parentId = view.parentViewId;
      if (parentId != null && viewIds.contains(parentId)) {
        continue; // hidden under a collapsed ancestor
      }
      if (visited.add(view.id)) {
        final hasChildren = (childrenByParent[view.id] ?? const []).isNotEmpty;
        ordered.add((view: view, depth: 0, hasChildren: hasChildren));
      }
    }

    return ordered;
  }

  void _toggleViewCollapse(DocumentView view) {
    setState(() {
      if (!_collapsedViewIds.add(view.id)) {
        _collapsedViewIds.remove(view.id);
      }
    });
  }

  void _schedulePageTitleSave() {
    _pageTitleSaveTimer?.cancel();
    _pageTitleSaveTimer = Timer(const Duration(milliseconds: 700), () {
      final bootstrap = widget.selectedBootstrap;
      final title = _pageTitle.text.trim();
      if (bootstrap == null || title.isEmpty || title == bootstrap.view.name) {
        return;
      }

      widget.onRenameView(bootstrap.view, title);
    });
  }

  Widget _editorPane(BuildContext context) {
    final workspace = widget.selectedWorkspace;
    if (workspace == null) {
      return const EmptyState(
        icon: Icons.ads_click,
        title: 'Select a workspace',
        detail: 'Choose a workspace and open a page from the document tree.',
      );
    }

    final bootstrap = widget.selectedBootstrap;
    if (bootstrap == null) {
      return const EmptyState(
        icon: Icons.description_outlined,
        title: 'Select a page',
        detail: 'Open a page from the document tree to edit it.',
      );
    }

    final canEdit = matchesEditRole(workspace.role);

    return ColoredBox(
      color: Colors.white,
      child: SingleChildScrollView(
        padding: const EdgeInsets.symmetric(horizontal: 28, vertical: 28),
        child: Center(
          child: ConstrainedBox(
            constraints: BoxConstraints(maxWidth: widget.pageWidth),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Row(
                  crossAxisAlignment: CrossAxisAlignment.center,
                  children: [
                    Expanded(
                      child: TextField(
                        controller: _pageTitle,
                        style: Theme.of(context).textTheme.headlineMedium,
                        textInputAction: TextInputAction.next,
                        onChanged: (_) => _schedulePageTitleSave(),
                        // Enter in the title drops into the first body line.
                        onSubmitted: (_) => _editorFocus.requestFocus(),
                        decoration: const InputDecoration(
                          hintText: 'Untitled',
                          border: InputBorder.none,
                        ),
                      ),
                    ),
                    PopupMenuButton<String>(
                      tooltip: 'Page menu',
                      icon: const Icon(Icons.expand_more),
                      onSelected: _onPageMenu,
                      itemBuilder: (context) => const [
                        PopupMenuItem(
                          value: 'export-md',
                          child: ListTile(
                            dense: true,
                            contentPadding: EdgeInsets.zero,
                            leading: Icon(Icons.download_outlined),
                            title: Text('Export Markdown'),
                          ),
                        ),
                        PopupMenuItem(
                          value: 'export-zip',
                          child: ListTile(
                            dense: true,
                            contentPadding: EdgeInsets.zero,
                            leading: Icon(Icons.folder_zip_outlined),
                            title: Text('Export ZIP (with images)'),
                          ),
                        ),
                        PopupMenuDivider(),
                        PopupMenuItem(
                          value: 'import-md',
                          child: ListTile(
                            dense: true,
                            contentPadding: EdgeInsets.zero,
                            leading: Icon(Icons.upload_file_outlined),
                            title: Text('Import Markdown…'),
                          ),
                        ),
                      ],
                    ),
                    const SizedBox(width: 4),
                    IconButton(
                      tooltip: _toolsExpanded
                          ? 'Hide side panel'
                          : 'Show side panel',
                      onPressed: () =>
                          setState(() => _toolsExpanded = !_toolsExpanded),
                      icon: Icon(
                        _toolsExpanded
                            ? Icons.chevron_right
                            : Icons.chevron_left,
                      ),
                    ),
                  ],
                ),
                const SizedBox(height: 8),
                Row(
                  children: [
                    Text(
                      'seq ${bootstrap.document.currentSeq} · snapshot ${bootstrap.snapshot.versionSeq}',
                      style: Theme.of(context).textTheme.bodySmall?.copyWith(
                        color: const Color(0xFF64748B),
                      ),
                    ),
                    const Spacer(),
                    _PresenceBar(presence: widget.presence),
                  ],
                ),
                const SizedBox(height: 8),
                Padding(
                  padding: const EdgeInsets.only(top: 4),
                  child: MicaEditor(
                      key: ValueKey(bootstrap.document.id),
                      focusNode: _editorFocus,
                      rootBlockId: bootstrap.document.rootBlockId,
                      nodes: [
                        for (final b in bootstrap.childBlocks)
                          EditorNode(
                            id: b.id,
                            kind: b.kind,
                            text: b.text,
                            data: Map<String, dynamic>.from(b.data),
                          ),
                      ],
                      version: bootstrap.snapshot.versionSeq,
                      canEdit: canEdit,
                      onApplyOperations: widget.onApplyOperations,
                      onUploadImage: widget.onUploadImage,
                      onImportImageUrl: widget.onImportImageUrl,
                      onLoadImageBytes: widget.onLoadImageBytes,
                      onResolveImageUrls: widget.onResolveImageUrls,
                      onAiStream: widget.onAiStream,
                      reHostImages: widget.reHostImages,
                      scrollHook: _scrollHook,
                      appearance: widget.appearance,
                    ),
                ),
                if (widget.selectedMarkdown != null) ...[
                  const SizedBox(height: 28),
                  Text(
                    'Markdown',
                    style: Theme.of(context).textTheme.titleLarge,
                  ),
                  const SizedBox(height: 12),
                  DecoratedBox(
                    decoration: BoxDecoration(
                      color: Colors.white,
                      borderRadius: BorderRadius.circular(8),
                      border: Border.all(color: const Color(0xFFE2E8F0)),
                    ),
                    child: Padding(
                      padding: const EdgeInsets.all(16),
                      child: SelectableText(widget.selectedMarkdown!),
                    ),
                  ),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }

  /// Tappable outline entries for the current page's headings (no section
  /// header). Tapping scrolls the editor to that heading.
  List<Widget> _pageOutlineItems(BuildContext context) {
    final blocks = widget.selectedBootstrap?.childBlocks ?? const [];
    return [
      for (final b in blocks)
        if (b.kind == 'heading' && b.text.trim().isNotEmpty)
          InkWell(
            onTap: () => _scrollHook.scrollToBlock(b.id),
            borderRadius: BorderRadius.circular(6),
            child: Padding(
              padding: EdgeInsets.only(
                left: 4.0 + 14 * ((_headingLevel(b) - 1).clamp(0, 5)),
                top: 5,
                bottom: 5,
                right: 4,
              ),
              child: Text(
                b.text.trim(),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  fontSize: _headingLevel(b) <= 1 ? 14 : 13,
                  fontWeight:
                      _headingLevel(b) <= 1 ? FontWeight.w600 : FontWeight.w400,
                  color: const Color(0xFF334155),
                ),
              ),
            ),
          ),
    ];
  }

  int _headingLevel(DocumentBlock b) {
    final level = b.data['level'];
    if (level is int) return level.clamp(1, 6);
    if (level is num) return level.toInt().clamp(1, 6);
    return 1;
  }

  /// Right panel — the current page's outline (table of contents).
  Widget _workspaceTools(BuildContext context) {
    final outline = _pageOutlineItems(context);
    return ColoredBox(
      color: Colors.white,
      child: outline.isEmpty
          ? const EmptyState(
              icon: Icons.toc,
              title: 'Outline',
              detail: 'Headings in this page appear here.',
            )
          : SingleChildScrollView(
              padding: const EdgeInsets.all(16),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Row(
                    children: [
                      const Icon(Icons.toc, size: 18),
                      const SizedBox(width: 8),
                      Text('Outline',
                          style: Theme.of(context).textTheme.titleLarge),
                    ],
                  ),
                  const SizedBox(height: 8),
                  ...outline,
                ],
              ),
            ),
    );
  }

  /// Inline workspace settings (rename + members), shown in the left panel when
  /// its gear is toggled — kept in the tree so member edits refresh live.
  Widget _workspaceSettings(BuildContext context) {
    final workspace = widget.selectedWorkspace;
    if (workspace == null) return const SizedBox.shrink();
    if (_rename.text.isEmpty) _rename.text = workspace.name;
    final canManage = matchesManageRole(workspace.role);
    return Container(
      margin: const EdgeInsets.only(top: 10),
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(
        color: const Color(0xFFF8FAFC),
        borderRadius: BorderRadius.circular(8),
        border: Border.all(color: const Color(0xFFE2E8F0)),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          DetailRow(label: 'Role', value: workspace.role),
          DetailRow(label: 'ID', value: workspace.id),
          const SizedBox(height: 12),
          TextField(
            controller: _rename,
            decoration: const InputDecoration(
              labelText: 'Rename workspace',
              prefixIcon: Icon(Icons.edit),
              border: OutlineInputBorder(),
              isDense: true,
            ),
          ),
          const SizedBox(height: 10),
          FilledButton.icon(
            onPressed: () => widget.onRenameWorkspace(workspace, _rename.text),
            icon: const Icon(Icons.save, size: 18),
            label: const Text('Save'),
          ),
          const SizedBox(height: 20),
          Row(
            children: [
              const Icon(Icons.group, size: 18),
              const SizedBox(width: 8),
              Text('Members', style: Theme.of(context).textTheme.titleMedium),
            ],
          ),
          const SizedBox(height: 12),
          if (canManage) _compactAddMemberForm(),
          if (canManage) const SizedBox(height: 14),
          if (widget.members.isEmpty)
            const Text('No members loaded.',
                style: TextStyle(color: Color(0xFF94A3B8)))
          else
            Column(
              children: widget.members
                  .map(
                    (member) => Padding(
                      padding: const EdgeInsets.only(bottom: 8),
                      child: MemberListItem(
                        member: member,
                        canManage: canManage,
                        canRemove: member.role != 'owner',
                        onRoleChanged: (role) =>
                            widget.onUpdateMember(member, role),
                        onRemove: () => widget.onRemoveMember(member),
                      ),
                    ),
                  )
                  .toList(),
            ),
        ],
      ),
    );
  }

  Widget _compactAddMemberForm() {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        TextField(
          controller: _memberEmail,
          keyboardType: TextInputType.emailAddress,
          decoration: const InputDecoration(
            labelText: 'Member email',
            prefixIcon: Icon(Icons.alternate_email),
            border: OutlineInputBorder(),
          ),
        ),
        const SizedBox(height: 10),
        DropdownButtonFormField<WorkspaceRole>(
          initialValue: _memberRole,
          decoration: const InputDecoration(
            labelText: 'Role',
            border: OutlineInputBorder(),
          ),
          items: WorkspaceRole.values
              .map(
                (role) =>
                    DropdownMenuItem(value: role, child: Text(role.apiValue)),
              )
              .toList(),
          onChanged: (role) {
            if (role == null) {
              return;
            }
            setState(() {
              _memberRole = role;
            });
          },
        ),
        const SizedBox(height: 10),
        FilledButton.icon(
          onPressed: () {
            widget.onAddMember(_memberEmail.text, _memberRole);
            _memberEmail.clear();
          },
          icon: const Icon(Icons.person_add),
          label: const Text('Add'),
        ),
      ],
    );
  }

  Future<void> _promptCreateWorkspace() async {
    final controller = TextEditingController();
    final name = await showDialog<String>(
      context: context,
      builder: (context) {
        return AlertDialog(
          title: const Text('New workspace'),
          content: TextField(
            controller: controller,
            autofocus: true,
            decoration: const InputDecoration(
              labelText: 'Workspace name',
              border: OutlineInputBorder(),
            ),
            onSubmitted: (value) => Navigator.of(context).pop(value),
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.of(context).pop(),
              child: const Text('Cancel'),
            ),
            FilledButton(
              onPressed: () => Navigator.of(context).pop(controller.text),
              child: const Text('Create'),
            ),
          ],
        );
      },
    );
    controller.dispose();

    final trimmed = name?.trim() ?? '';
    if (trimmed.isNotEmpty) {
      await widget.onCreateWorkspace(trimmed);
    }
  }

  Future<void> _promptRenameWorkspace(Workspace workspace) async {
    final controller = TextEditingController(text: workspace.name);
    final name = await showDialog<String>(
      context: context,
      builder: (context) {
        return AlertDialog(
          title: const Text('Rename workspace'),
          content: TextField(
            controller: controller,
            autofocus: true,
            decoration: const InputDecoration(
              labelText: 'Workspace name',
              border: OutlineInputBorder(),
            ),
            onSubmitted: (value) => Navigator.of(context).pop(value),
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.of(context).pop(),
              child: const Text('Cancel'),
            ),
            FilledButton.icon(
              onPressed: () => Navigator.of(context).pop(controller.text),
              icon: const Icon(Icons.save),
              label: const Text('Save'),
            ),
          ],
        );
      },
    );
    controller.dispose();

    final trimmed = name?.trim() ?? '';
    if (trimmed.isNotEmpty && trimmed != workspace.name) {
      await widget.onRenameWorkspace(workspace, trimmed);
    }
  }

  Future<void> _promptRenameDocument(DocumentView view) async {
    final controller = TextEditingController(text: view.name);
    final name = await showDialog<String>(
      context: context,
      builder: (context) {
        return AlertDialog(
          title: const Text('Rename page'),
          content: TextField(
            controller: controller,
            autofocus: true,
            decoration: const InputDecoration(
              labelText: 'Page name',
              border: OutlineInputBorder(),
            ),
            onSubmitted: (value) => Navigator.of(context).pop(value),
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.of(context).pop(),
              child: const Text('Cancel'),
            ),
            FilledButton.icon(
              onPressed: () => Navigator.of(context).pop(controller.text),
              icon: const Icon(Icons.save),
              label: const Text('Save'),
            ),
          ],
        );
      },
    );
    controller.dispose();

    if (name == null) {
      return;
    }

    await widget.onRenameView(view, name);
  }

  Future<void> _confirmDeleteWorkspace(Workspace workspace) async {
    final shouldDelete = await showDialog<bool>(
      context: context,
      builder: (context) {
        return AlertDialog(
          title: const Text('Delete workspace'),
          content: Text(
            'Delete "${workspace.name}" and all of its pages? '
            'This cannot be undone.',
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.of(context).pop(false),
              child: const Text('Cancel'),
            ),
            FilledButton.icon(
              style: FilledButton.styleFrom(
                backgroundColor: const Color(0xFFDC2626),
              ),
              onPressed: () => Navigator.of(context).pop(true),
              icon: const Icon(Icons.delete_outline),
              label: const Text('Delete'),
            ),
          ],
        );
      },
    );

    if (shouldDelete != true) {
      return;
    }

    await widget.onDeleteWorkspace(workspace);
  }

  void _openRecycleBin() {
    showDialog<void>(
      context: context,
      builder: (context) => _RecycleBinDialog(
        onLoad: widget.onLoadTrash,
        onRestore: widget.onRestoreView,
        onPurge: widget.onPurgeView,
      ),
    );
  }

  /// Page title menu: Markdown export/import + ZIP export.
  Future<void> _onPageMenu(String value) async {
    switch (value) {
      case 'export-md':
        await _downloadPageMarkdown();
      case 'export-zip':
        await _onExport('page-zip');
      case 'import-md':
        await _importMarkdownFile();
    }
  }

  Future<void> _downloadPageMarkdown() async {
    try {
      final markdown = await widget.onExportPageMarkdown();
      final title = _pageTitle.text.trim().isEmpty
          ? 'page'
          : _pageTitle.text.trim();
      downloadImage(
        Uint8List.fromList(utf8.encode(markdown)),
        '$title.md',
        'text/markdown',
      );
    } catch (error) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Export failed: $error')),
        );
      }
    }
  }

  Future<void> _importMarkdownFile() async {
    final picked = await pickTextFile();
    if (picked == null || !mounted) return;
    await widget.onImportMarkdown(picked.name, picked.text);
  }

  /// Download a whole workspace as a Markdown ZIP (page-tree folders + assets).
  Future<void> _exportWorkspaceFile(Workspace workspace) async {
    try {
      final bytes = await widget.onExportWorkspaceZip(workspace.id);
      final name = workspace.name.trim().isEmpty ? 'workspace' : workspace.name.trim();
      downloadImage(bytes, '$name.zip', 'application/zip');
    } catch (error) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Export failed: $error')),
        );
      }
    }
  }

  Future<void> _onExport(String kind) async {
    if (kind == 'page-zip') {
      try {
        final bytes = await widget.onExportPageZip();
        downloadImage(bytes, 'page.zip', 'application/zip');
      } catch (error) {
        if (mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            SnackBar(content: Text('Export failed: $error')),
          );
        }
      }
      return;
    }
    final (title, future) = switch (kind) {
      'page' => ('Export current page', widget.onExportPageMarkdown()),
      'workspace' => ('Export workspace', widget.onExportWorkspaceMarkdown()),
      _ => ('Export all workspaces', widget.onExportAllMarkdown()),
    };
    String markdown;
    try {
      markdown = await future;
    } catch (error) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Export failed: $error')),
        );
      }
      return;
    }
    if (!mounted) return;
    await showDialog<void>(
      context: context,
      builder: (context) => _ExportDialog(title: title, markdown: markdown),
    );
  }

  void _openSearch() {
    showDialog<void>(
      context: context,
      builder: (context) => _SearchDialog(
        onSearch: widget.onSearch,
        onOpen: (viewId) {
          Navigator.of(context).pop();
          widget.onOpenSearchResult(viewId);
        },
      ),
    );
  }

  void _openSettings() {
    showDialog<void>(
      context: context,
      builder: (context) => _SettingsDialog(
        onLoadAiSettings: widget.onLoadAiSettings,
        onSaveAiSettings: widget.onSaveAiSettings,
        userName: widget.userName,
        userEmail: widget.userEmail,
        onUpdateProfile: widget.onUpdateProfile,
        onChangePassword: widget.onChangePassword,
        appearance: widget.appearance,
        pageWidth: widget.pageWidth,
        reHostImages: widget.reHostImages,
        onReHostImagesChanged: widget.onReHostImagesChanged,
        onAppearanceChanged: widget.onAppearanceChanged,
        onImportWorkspace: () => _importWorkspaceFile(fromSettings: true),
      ),
    );
  }

  /// Pick a workspace ZIP (Mica export or a Notion "Markdown & CSV" export —
  /// both flow through the same tree import) and rebuild it as a new
  /// workspace.
  Future<void> _importWorkspaceFile({bool fromSettings = false}) async {
    final picked = await pickImportFile(zipOnly: true);
    if (picked == null || !mounted) return;
    if (fromSettings) {
      Navigator.of(context).pop(); // close settings before the import flow runs
    }
    await widget.onImportWorkspaceZip(picked.name, picked.bytes);
  }

  /// Multi-select import into an existing workspace: .md files (plus images
  /// they reference) append pages at the root; a ZIP's whole tree unpacks in.
  Future<void> _importFilesIntoWorkspace(Workspace workspace) async {
    final picked = await pickImportFiles();
    if (picked.isEmpty || !mounted) return;
    final entries = <ZipFileEntry>[];
    for (final f in picked) {
      if (f.name.toLowerCase().endsWith('.zip')) {
        entries.addAll(readZip(f.bytes));
      } else {
        entries.add(ZipFileEntry(f.name, f.bytes));
      }
    }
    await widget.onImportWorkspaceTreeInto(workspace, entries);
  }

  /// Folder import (recursive) into an existing workspace: the folder's
  /// contents become pages, its subfolders the page tree.
  Future<void> _importFolderIntoWorkspace(Workspace workspace) async {
    final picked = await pickImportFolder();
    if (picked.isEmpty || !mounted) return;
    final entries = <ZipFileEntry>[];
    for (final f in picked) {
      // The picker includes the chosen folder itself as the first segment —
      // drop it so the folder's contents land at the workspace root.
      final parts = f.path.split('/');
      entries.add(ZipFileEntry(
        parts.length > 1 ? parts.sublist(1).join('/') : f.path,
        f.bytes,
      ));
    }
    await widget.onImportWorkspaceTreeInto(workspace, entries);
  }

  void _openAiDialog() {
    showDialog<void>(
      context: context,
      builder: (context) => _AiDialog(
        canEdit: matchesEditRole(widget.selectedWorkspace?.role),
        hasWorkspace: widget.selectedWorkspace != null,
        onStream: widget.onAiStream,
        onNewPage: widget.onAiNewPage,
        onCurrentPage: widget.onAiCurrentPage,
        onNewWorkspace: widget.onAiNewWorkspace,
      ),
    );
  }
}

/// Workspace switcher. The anchor button shows the current workspace; the menu
/// lists every workspace (each row selects it and carries inline rename/delete
/// actions) and ends with a "New workspace" row.
class _WorkspaceSelector extends StatefulWidget {
  const _WorkspaceSelector({
    required this.workspaces,
    required this.selected,
    required this.onSelect,
    required this.onRename,
    required this.onDelete,
    required this.onExport,
    required this.onCreate,
    required this.onImport,
    required this.onImportFilesInto,
    required this.onImportFolderInto,
  });

  final List<Workspace> workspaces;
  final Workspace? selected;
  final Future<void> Function(Workspace workspace) onSelect;
  final void Function(Workspace workspace) onRename;
  final void Function(Workspace workspace) onDelete;
  final void Function(Workspace workspace) onExport;
  final VoidCallback onCreate;
  final VoidCallback onImport;
  final void Function(Workspace workspace) onImportFilesInto;
  final void Function(Workspace workspace) onImportFolderInto;

  @override
  State<_WorkspaceSelector> createState() => _WorkspaceSelectorState();
}

class _WorkspaceSelectorState extends State<_WorkspaceSelector> {
  final MenuController _menu = MenuController();

  @override
  Widget build(BuildContext context) {
    return MenuAnchor(
      controller: _menu,
      style: const MenuStyle(
        minimumSize: WidgetStatePropertyAll(Size(300, 0)),
        padding: WidgetStatePropertyAll(EdgeInsets.symmetric(vertical: 6)),
      ),
      menuChildren: [
        for (final workspace in widget.workspaces) _row(workspace),
        if (widget.workspaces.isNotEmpty) const Divider(height: 8),
        _createRow(),
        SizedBox(
          width: 320,
          child: SubmenuButton(
            leadingIcon: const Icon(
              Icons.upload_file_outlined,
              size: 18,
              color: Color(0xFF475569),
            ),
            menuChildren: [
              _importChoice(Icons.folder_zip_outlined, 'From ZIP (Mica export)'),
              _importChoice(
                Icons.cloud_download_outlined,
                'From Notion (Markdown & CSV ZIP)',
              ),
            ],
            child: const Text(
              'Import workspace',
              style: TextStyle(
                color: Color(0xFF475569),
                fontWeight: FontWeight.w600,
              ),
            ),
          ),
        ),
      ],
      builder: (context, controller, child) {
        final label = widget.selected?.name ?? 'Select workspace';
        return SizedBox(
          width: double.infinity,
          child: OutlinedButton(
            style: OutlinedButton.styleFrom(
              padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 12),
              alignment: Alignment.centerLeft,
              side: const BorderSide(color: Color(0xFFCBD5E1)),
              shape: RoundedRectangleBorder(
                borderRadius: BorderRadius.circular(8),
              ),
            ),
            onPressed: () =>
                controller.isOpen ? controller.close() : controller.open(),
            child: Row(
              children: [
                const Icon(
                  Icons.workspaces_outline,
                  size: 20,
                  color: Color(0xFF475569),
                ),
                const SizedBox(width: 8),
                Expanded(
                  child: Text(
                    label,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(color: Color(0xFF0F172A)),
                  ),
                ),
                const Icon(Icons.arrow_drop_down, color: Color(0xFF475569)),
              ],
            ),
          ),
        );
      },
    );
  }

  Widget _row(Workspace workspace) {
    final selected = workspace.id == widget.selected?.id;
    return SizedBox(
      width: 320,
      child: Row(
        children: [
          Expanded(
            child: InkWell(
              onTap: () {
                _menu.close();
                widget.onSelect(workspace);
              },
              child: Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: 12,
                  vertical: 10,
                ),
                child: Row(
                  children: [
                    Icon(
                      selected ? Icons.check : Icons.workspaces_outline,
                      size: 18,
                      color: selected
                          ? const Color(0xFF2563EB)
                          : const Color(0xFF94A3B8),
                    ),
                    const SizedBox(width: 10),
                    Expanded(
                      child: Text(
                        workspace.name,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontWeight: selected
                              ? FontWeight.w600
                              : FontWeight.w400,
                          color: const Color(0xFF0F172A),
                        ),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
          MenuAnchor(
            menuChildren: [
              _wsAction(
                Icons.edit_outlined,
                'Rename',
                () => widget.onRename(workspace),
              ),
              _wsAction(
                Icons.folder_zip_outlined,
                'Export (ZIP)',
                () => widget.onExport(workspace),
              ),
              // One Import entry; the native picker can't mix files and
              // folders, so the choice lives in a submenu.
              SubmenuButton(
                leadingIcon: const Icon(
                  Icons.download_outlined,
                  size: 18,
                  color: Color(0xFF475569),
                ),
                menuChildren: [
                  _wsAction(
                    Icons.upload_file_outlined,
                    'Files (.md / .zip)',
                    () => widget.onImportFilesInto(workspace),
                  ),
                  _wsAction(
                    Icons.drive_folder_upload_outlined,
                    'Folder',
                    () => widget.onImportFolderInto(workspace),
                  ),
                ],
                child: const Text('Import'),
              ),
              _wsAction(
                Icons.delete_outline,
                'Delete',
                () => widget.onDelete(workspace),
                color: const Color(0xFFDC2626),
              ),
            ],
            builder: (context, controller, child) => IconButton(
              tooltip: 'Workspace menu',
              icon: const Icon(Icons.more_horiz, size: 18),
              onPressed: () =>
                  controller.isOpen ? controller.close() : controller.open(),
            ),
          ),
          const SizedBox(width: 4),
        ],
      ),
    );
  }

  Widget _createRow() {
    return SizedBox(
      width: 320,
      child: InkWell(
        onTap: () {
          _menu.close();
          widget.onCreate();
        },
        child: const Padding(
          padding: EdgeInsets.symmetric(horizontal: 12, vertical: 12),
          child: Row(
            children: [
              Icon(Icons.add, size: 18, color: Color(0xFF2563EB)),
              SizedBox(width: 10),
              Text(
                'New workspace',
                style: TextStyle(
                  color: Color(0xFF2563EB),
                  fontWeight: FontWeight.w600,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  /// A per-workspace menu action that also closes the outer dropdown.
  Widget _wsAction(
    IconData icon,
    String label,
    VoidCallback onTap, {
    Color? color,
  }) {
    return MenuItemButton(
      leadingIcon: Icon(icon, size: 18, color: color ?? const Color(0xFF475569)),
      onPressed: () {
        _menu.close();
        onTap();
      },
      child: Text(
        label,
        style: TextStyle(color: color ?? const Color(0xFF0F172A)),
      ),
    );
  }

  /// Both submenu entries run the same ZIP import — Notion's "Markdown & CSV"
  /// export is a markdown ZIP too; its ID-suffixed names are auto-detected.
  Widget _importChoice(IconData icon, String label) {
    return MenuItemButton(
      leadingIcon: Icon(icon, size: 18, color: const Color(0xFF475569)),
      onPressed: () {
        _menu.close();
        widget.onImport();
      },
      child: Text(
        label,
        style: const TextStyle(color: Color(0xFF475569)),
      ),
    );
  }
}

class WorkspaceListItem extends StatelessWidget {
  const WorkspaceListItem({
    required this.workspace,
    required this.isSelected,
    required this.onPressed,
    super.key,
  });

  final Workspace workspace;
  final bool isSelected;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    return Material(
      color: isSelected ? const Color(0xFFEFF6FF) : Colors.white,
      shape: RoundedRectangleBorder(
        side: BorderSide(
          color: isSelected ? const Color(0xFF2563EB) : const Color(0xFFE2E8F0),
        ),
        borderRadius: BorderRadius.circular(8),
      ),
      child: ListTile(
        onTap: onPressed,
        leading: const Icon(Icons.workspaces),
        title: Text(workspace.name, overflow: TextOverflow.ellipsis),
        subtitle: Text(workspace.role),
      ),
    );
  }
}

class MemberListItem extends StatelessWidget {
  const MemberListItem({
    required this.member,
    required this.canManage,
    required this.canRemove,
    required this.onRoleChanged,
    required this.onRemove,
    super.key,
  });

  final WorkspaceMember member;
  final bool canManage;
  final bool canRemove;
  final Future<void> Function(WorkspaceRole role) onRoleChanged;
  final Future<void> Function() onRemove;

  @override
  Widget build(BuildContext context) {
    return DecoratedBox(
      decoration: BoxDecoration(
        color: const Color(0xFFF8FAFC),
        borderRadius: BorderRadius.circular(8),
        border: Border.all(color: const Color(0xFFE2E8F0)),
      ),
      child: Padding(
        padding: const EdgeInsets.all(12),
        child: Row(
          children: [
            const CircleAvatar(child: Icon(Icons.person)),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    member.displayName,
                    style: Theme.of(context).textTheme.titleMedium,
                    overflow: TextOverflow.ellipsis,
                  ),
                  Text(
                    member.email,
                    style: Theme.of(context).textTheme.bodySmall,
                    overflow: TextOverflow.ellipsis,
                  ),
                ],
              ),
            ),
            const SizedBox(width: 12),
            if (canManage && member.role != 'owner')
              SizedBox(
                width: 150,
                child: DropdownButtonFormField<WorkspaceRole>(
                  initialValue: WorkspaceRole.fromApiValue(member.role),
                  decoration: const InputDecoration(
                    labelText: 'Role',
                    border: OutlineInputBorder(),
                  ),
                  items: WorkspaceRole.values
                      .map(
                        (role) => DropdownMenuItem(
                          value: role,
                          child: Text(role.apiValue),
                        ),
                      )
                      .toList(),
                  onChanged: (role) {
                    if (role != null) {
                      onRoleChanged(role);
                    }
                  },
                ),
              )
            else
              Chip(label: Text(member.role)),
            const SizedBox(width: 8),
            IconButton(
              tooltip: 'Remove',
              onPressed: canManage && canRemove ? onRemove : null,
              icon: const Icon(Icons.delete_outline),
            ),
          ],
        ),
      ),
    );
  }
}

class BlockListItem extends StatelessWidget {
  const BlockListItem({
    required this.block,
    required this.canMoveUp,
    required this.canMoveDown,
    required this.onMoveUp,
    required this.onMoveDown,
    required this.onEdit,
    required this.onDelete,
    super.key,
  });

  final DocumentBlock block;
  final bool canMoveUp;
  final bool canMoveDown;
  final VoidCallback onMoveUp;
  final VoidCallback onMoveDown;
  final VoidCallback onEdit;
  final VoidCallback onDelete;

  @override
  Widget build(BuildContext context) {
    final kind = DocumentBlockKind.fromApiValue(block.kind);
    return DecoratedBox(
      decoration: BoxDecoration(
        color: kind == DocumentBlockKind.codeBlock
            ? const Color(0xFFF1F5F9)
            : Colors.white,
        borderRadius: BorderRadius.circular(8),
        border: Border.all(color: const Color(0xFFE2E8F0)),
      ),
      child: Padding(
        padding: const EdgeInsets.all(12),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Icon(_iconFor(kind), color: const Color(0xFF64748B)),
            const SizedBox(width: 12),
            Expanded(child: _contentFor(context, kind)),
            IconButton(
              tooltip: 'Move up',
              onPressed: canMoveUp ? onMoveUp : null,
              icon: const Icon(Icons.arrow_upward),
            ),
            IconButton(
              tooltip: 'Move down',
              onPressed: canMoveDown ? onMoveDown : null,
              icon: const Icon(Icons.arrow_downward),
            ),
            IconButton(
              tooltip: 'Edit',
              onPressed: onEdit,
              icon: const Icon(Icons.edit_outlined),
            ),
            IconButton(
              tooltip: 'Delete',
              onPressed: onDelete,
              icon: const Icon(Icons.delete_outline),
            ),
          ],
        ),
      ),
    );
  }

  Widget _contentFor(BuildContext context, DocumentBlockKind kind) {
    final text = block.text.isEmpty ? '(empty)' : block.text;
    switch (kind) {
      case DocumentBlockKind.heading:
        return SelectableText(
          text,
          style: Theme.of(context).textTheme.headlineSmall,
        );
      case DocumentBlockKind.todo:
        return Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const Icon(Icons.check_box_outline_blank, size: 18),
            const SizedBox(width: 8),
            Expanded(child: SelectableText(text)),
          ],
        );
      case DocumentBlockKind.bulletedList:
        return Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const Text('•'),
            const SizedBox(width: 10),
            Expanded(child: SelectableText(text)),
          ],
        );
      case DocumentBlockKind.numberedList:
        return Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const Text('1.'),
            const SizedBox(width: 8),
            Expanded(child: SelectableText(text)),
          ],
        );
      case DocumentBlockKind.quote:
        return DecoratedBox(
          decoration: const BoxDecoration(
            border: Border(
              left: BorderSide(color: Color(0xFF94A3B8), width: 3),
            ),
          ),
          child: Padding(
            padding: const EdgeInsets.only(left: 12),
            child: SelectableText(
              text,
              style: Theme.of(
                context,
              ).textTheme.bodyLarge?.copyWith(color: const Color(0xFF475569)),
            ),
          ),
        );
      case DocumentBlockKind.codeBlock:
        return SelectableText(
          text,
          style: const TextStyle(fontFamily: 'monospace'),
        );
      case DocumentBlockKind.paragraph:
        return SelectableText(
          text,
          style: Theme.of(context).textTheme.bodyLarge,
        );
    }
  }

  IconData _iconFor(DocumentBlockKind kind) {
    return switch (kind) {
      DocumentBlockKind.heading => Icons.title,
      DocumentBlockKind.todo => Icons.check_box_outlined,
      DocumentBlockKind.bulletedList => Icons.format_list_bulleted,
      DocumentBlockKind.numberedList => Icons.format_list_numbered,
      DocumentBlockKind.quote => Icons.format_quote,
      DocumentBlockKind.codeBlock => Icons.code,
      DocumentBlockKind.paragraph => Icons.notes,
    };
  }
}

/// Where a dragged page lands relative to the row it is dropped on: as the
/// sibling before it, nested as its child, or the sibling after it.
enum _DropMode { before, into, after }

/// Shows exported Markdown in a selectable box with a one-tap copy.
class _ExportDialog extends StatelessWidget {
  const _ExportDialog({required this.title, required this.markdown});

  final String title;
  final String markdown;

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: Text(title),
      content: SizedBox(
        width: 560,
        height: 460,
        child: Container(
          width: double.infinity,
          padding: const EdgeInsets.all(12),
          decoration: BoxDecoration(
            color: const Color(0xFFF8FAFC),
            borderRadius: BorderRadius.circular(8),
            border: Border.all(color: const Color(0xFFE2E8F0)),
          ),
          child: SingleChildScrollView(
            child: SelectableText(
              markdown.isEmpty ? '(empty)' : markdown,
              style: const TextStyle(fontFamily: 'monospace', fontSize: 13, height: 1.5),
            ),
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Close'),
        ),
        FilledButton.icon(
          onPressed: () async {
            await copyTextToClipboard(markdown);
            if (context.mounted) {
              ScaffoldMessenger.of(context).showSnackBar(
                const SnackBar(content: Text('Copied to clipboard')),
              );
            }
          },
          icon: const Icon(Icons.copy, size: 18),
          label: const Text('Copy'),
        ),
      ],
    );
  }
}

/// Workspace search: type to find pages by title or body text; click to open.
class _SearchDialog extends StatefulWidget {
  const _SearchDialog({required this.onSearch, required this.onOpen});

  final Future<List<SearchResult>> Function(String query) onSearch;
  final void Function(String viewId) onOpen;

  @override
  State<_SearchDialog> createState() => _SearchDialogState();
}

class _SearchDialogState extends State<_SearchDialog> {
  final _query = TextEditingController();
  Timer? _debounce;
  bool _loading = false;
  List<SearchResult> _results = const [];
  String _lastQuery = '';

  @override
  void dispose() {
    _debounce?.cancel();
    _query.dispose();
    super.dispose();
  }

  void _onChanged(String value) {
    _debounce?.cancel();
    _debounce = Timer(const Duration(milliseconds: 280), () => _run(value));
  }

  Future<void> _run(String value) async {
    final query = value.trim();
    if (query.isEmpty) {
      setState(() {
        _results = const [];
        _loading = false;
      });
      return;
    }
    setState(() => _loading = true);
    try {
      final results = await widget.onSearch(query);
      if (!mounted || _query.text.trim() != query) return;
      setState(() {
        _results = results;
        _lastQuery = query;
        _loading = false;
      });
    } catch (_) {
      if (mounted) setState(() => _loading = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: const Text('Search pages'),
      content: SizedBox(
        width: 480,
        height: 420,
        child: Column(
          children: [
            TextField(
              controller: _query,
              autofocus: true,
              onChanged: _onChanged,
              decoration: InputDecoration(
                hintText: 'Search titles and content…',
                prefixIcon: const Icon(Icons.search),
                suffixIcon: _loading
                    ? const Padding(
                        padding: EdgeInsets.all(12),
                        child: SizedBox(
                          width: 16,
                          height: 16,
                          child: CircularProgressIndicator(strokeWidth: 2),
                        ),
                      )
                    : null,
                border: const OutlineInputBorder(),
              ),
            ),
            const SizedBox(height: 12),
            Expanded(child: _buildResults(context)),
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Close'),
        ),
      ],
    );
  }

  Widget _buildResults(BuildContext context) {
    if (_query.text.trim().isEmpty) {
      return const EmptyState(
        icon: Icons.search,
        title: 'Search this workspace',
        detail: 'Find pages by title or content.',
      );
    }
    if (!_loading && _results.isEmpty) {
      return EmptyState(
        icon: Icons.search_off,
        title: 'No matches',
        detail: 'Nothing found for "$_lastQuery".',
      );
    }
    return ListView.separated(
      itemCount: _results.length,
      separatorBuilder: (_, _) => const Divider(height: 1),
      itemBuilder: (context, i) {
        final result = _results[i];
        return ListTile(
          contentPadding: const EdgeInsets.symmetric(horizontal: 4),
          leading: const Icon(Icons.description_outlined, size: 18),
          title: Text(result.name, overflow: TextOverflow.ellipsis),
          subtitle: result.snippet.isEmpty
              ? (result.titleMatch ? const Text('Title match') : null)
              : Text(result.snippet, maxLines: 2, overflow: TextOverflow.ellipsis),
          onTap: () => widget.onOpen(result.viewId),
        );
      },
    );
  }
}

/// System prompt for AI that should produce a whole document with a title line.
const String kAiDocSystem =
    'You are a writing assistant inside a Markdown document editor. Respond with '
    'clean GitHub-Flavored Markdown only — no preamble or commentary. Begin the '
    'document with a single level-1 heading (a "# Title" line).';

enum _AiTarget { newPage, currentPage, newWorkspace }

/// Preset AI providers. Each maps to a backend provider dialect (openai/anthropic)
/// plus default base URL + model; "Local / Custom" lets the user point at any
/// OpenAI-compatible server (Ollama, LM Studio, vLLM, …).
enum _AiPreset { deepseek, openai, anthropic, custom }

extension _AiPresetInfo on _AiPreset {
  String get label => switch (this) {
    _AiPreset.deepseek => 'DeepSeek',
    _AiPreset.openai => 'OpenAI',
    _AiPreset.anthropic => 'Anthropic (Claude)',
    _AiPreset.custom => 'Local / Custom',
  };

  String get provider =>
      this == _AiPreset.anthropic ? 'anthropic' : 'openai';

  String get baseUrl => switch (this) {
    _AiPreset.deepseek => 'https://api.deepseek.com',
    _AiPreset.openai => 'https://api.openai.com/v1',
    _AiPreset.anthropic => 'https://api.anthropic.com',
    _AiPreset.custom => 'http://localhost:11434/v1',
  };

  String get model => switch (this) {
    _AiPreset.deepseek => 'deepseek-chat',
    _AiPreset.openai => 'gpt-4o-mini',
    _AiPreset.anthropic => 'claude-sonnet-4-6',
    _AiPreset.custom => '',
  };
}

/// Settings dialog. Currently hosts the AI provider configuration; appearance and
/// account sections will slot in alongside it.
class _SettingsDialog extends StatefulWidget {
  const _SettingsDialog({
    required this.onLoadAiSettings,
    required this.onSaveAiSettings,
    required this.userName,
    required this.userEmail,
    required this.onUpdateProfile,
    required this.onChangePassword,
    required this.appearance,
    required this.pageWidth,
    required this.reHostImages,
    required this.onReHostImagesChanged,
    required this.onAppearanceChanged,
    required this.onImportWorkspace,
  });

  final String userName;
  final String userEmail;
  final Future<void> Function(String displayName) onUpdateProfile;
  final Future<void> Function(String current, String next) onChangePassword;
  final Future<Map<String, dynamic>> Function() onLoadAiSettings;
  final Future<void> Function({
    required String provider,
    required String baseUrl,
    required String model,
    String? apiKey,
  })
  onSaveAiSettings;
  final EditorAppearance appearance;
  final double pageWidth;
  final bool reHostImages;
  final void Function(bool value) onReHostImagesChanged;
  final void Function(EditorAppearance appearance, double pageWidth)
  onAppearanceChanged;
  final Future<void> Function() onImportWorkspace;

  @override
  State<_SettingsDialog> createState() => _SettingsDialogState();
}

class _SettingsDialogState extends State<_SettingsDialog> {
  final _baseUrl = TextEditingController();
  final _model = TextEditingController();
  final _apiKey = TextEditingController();
  _AiPreset _preset = _AiPreset.deepseek;
  int _tab = 0; // 0 Appearance, 1 AI provider, 2 Account
  bool _loading = true;
  bool _saving = false;
  bool _hasKey = false;
  String? _error;
  String? _saved;

  late final _name = TextEditingController(text: widget.userName);
  final _curPass = TextEditingController();
  final _newPass = TextEditingController();
  bool _accountBusy = false;
  String? _accountMsg;

  late double _fontScale = widget.appearance.fontScale;
  late String? _fontFamily = widget.appearance.fontFamily;
  late double _pageWidth = widget.pageWidth;
  late bool _reHostImages = widget.reHostImages;

  @override
  void initState() {
    super.initState();
    _load();
  }

  void _applyAppearance() {
    widget.onAppearanceChanged(
      EditorAppearance(fontScale: _fontScale, fontFamily: _fontFamily),
      _pageWidth,
    );
  }

  Widget _sliderRow({
    required String label,
    required double value,
    required double min,
    required double max,
    required String display,
    required ValueChanged<double> onChanged,
  }) {
    return Row(
      children: [
        SizedBox(width: 90, child: Text(label)),
        Expanded(
          child: Slider(value: value, min: min, max: max, onChanged: onChanged),
        ),
        SizedBox(
          width: 56,
          child: Text(
            display,
            textAlign: TextAlign.right,
            style: const TextStyle(color: Color(0xFF64748B), fontSize: 13),
          ),
        ),
      ],
    );
  }

  Widget _fontChip(String label, String? family) {
    return ChoiceChip(
      label: Text(label),
      selected: _fontFamily == family,
      onSelected: (_) {
        setState(() => _fontFamily = family);
        _applyAppearance();
      },
    );
  }

  @override
  void dispose() {
    _baseUrl.dispose();
    _model.dispose();
    _apiKey.dispose();
    _name.dispose();
    _curPass.dispose();
    _newPass.dispose();
    super.dispose();
  }

  Future<void> _saveProfile() async {
    setState(() {
      _accountBusy = true;
      _accountMsg = null;
    });
    try {
      await widget.onUpdateProfile(_name.text.trim());
      if (mounted) setState(() => _accountMsg = 'Profile saved');
    } catch (error) {
      if (mounted) setState(() => _accountMsg = error.toString());
    } finally {
      if (mounted) setState(() => _accountBusy = false);
    }
  }

  Future<void> _changeAccountPassword() async {
    if (_newPass.text.length < 8) {
      setState(() => _accountMsg = 'New password must be at least 8 characters');
      return;
    }
    setState(() {
      _accountBusy = true;
      _accountMsg = null;
    });
    try {
      await widget.onChangePassword(_curPass.text, _newPass.text);
      if (mounted) {
        setState(() {
          _accountMsg = 'Password changed';
          _curPass.clear();
          _newPass.clear();
        });
      }
    } catch (error) {
      if (mounted) setState(() => _accountMsg = error.toString());
    } finally {
      if (mounted) setState(() => _accountBusy = false);
    }
  }

  Future<void> _load() async {
    try {
      final settings = await widget.onLoadAiSettings();
      if (!mounted) return;
      final provider = settings['provider'] as String? ?? 'openai';
      final base = settings['base_url'] as String? ?? '';
      final model = settings['model'] as String? ?? '';
      setState(() {
        _preset = _presetFor(provider, base);
        _baseUrl.text = base.isEmpty ? _preset.baseUrl : base;
        _model.text = model.isEmpty ? _preset.model : model;
        _hasKey = settings['has_key'] == true;
        _loading = false;
      });
    } catch (error) {
      if (!mounted) return;
      setState(() {
        _error = error.toString();
        _loading = false;
      });
    }
  }

  _AiPreset _presetFor(String provider, String base) {
    if (provider == 'anthropic') return _AiPreset.anthropic;
    if (base.contains('deepseek')) return _AiPreset.deepseek;
    if (base.contains('openai.com')) return _AiPreset.openai;
    return base.isEmpty ? _AiPreset.deepseek : _AiPreset.custom;
  }

  void _applyPreset(_AiPreset preset) {
    setState(() {
      _preset = preset;
      if (preset != _AiPreset.custom) {
        _baseUrl.text = preset.baseUrl;
        _model.text = preset.model;
      }
    });
  }

  Future<void> _save() async {
    setState(() {
      _saving = true;
      _error = null;
      _saved = null;
    });
    try {
      await widget.onSaveAiSettings(
        provider: _preset.provider,
        baseUrl: _baseUrl.text.trim(),
        model: _model.text.trim(),
        apiKey: _apiKey.text.trim().isEmpty ? null : _apiKey.text.trim(),
      );
      if (!mounted) return;
      setState(() {
        _saving = false;
        _saved = 'Saved';
        if (_apiKey.text.trim().isNotEmpty) _hasKey = true;
        _apiKey.clear();
      });
    } catch (error) {
      if (!mounted) return;
      setState(() {
        _saving = false;
        _error = error.toString();
      });
    }
  }

  Widget _sectionTitle(BuildContext context, IconData icon, String label, Color color) {
    return Row(
      children: [
        Icon(icon, size: 18, color: color),
        const SizedBox(width: 8),
        Text(label, style: Theme.of(context).textTheme.titleMedium),
      ],
    );
  }

  List<Widget> _appearanceSection(BuildContext context) => [
    _sectionTitle(context, Icons.tune, 'Appearance', const Color(0xFF2563EB)),
    const SizedBox(height: 12),
    _sliderRow(
      label: 'Page width',
      value: _pageWidth,
      min: 640,
      max: 1440,
      display: '${_pageWidth.round()} px',
      onChanged: (value) {
        setState(() => _pageWidth = value);
        _applyAppearance();
      },
    ),
    _sliderRow(
      label: 'Font size',
      value: _fontScale,
      min: 0.85,
      max: 1.4,
      display: '${(_fontScale * 100).round()}%',
      onChanged: (value) {
        setState(() => _fontScale = value);
        _applyAppearance();
      },
    ),
    const SizedBox(height: 4),
    Row(
      children: [
        const SizedBox(width: 90, child: Text('Font')),
        Expanded(
          child: Wrap(
            spacing: 8,
            children: [
              _fontChip('System', null),
              _fontChip('Serif', 'serif'),
              _fontChip('Mono', 'monospace'),
            ],
          ),
        ),
      ],
    ),
    const SizedBox(height: 8),
    SwitchListTile(
      contentPadding: EdgeInsets.zero,
      dense: true,
      value: _reHostImages,
      title: const Text('Re-host network images'),
      subtitle: const Text(
        'Pasted image links are saved into Mica storage. Off: keep '
        'them as standard external Markdown links.',
      ),
      onChanged: (value) {
        setState(() => _reHostImages = value);
        widget.onReHostImagesChanged(value);
      },
    ),
  ];

  List<Widget> _aiSection(BuildContext context) => [
    _sectionTitle(context, Icons.auto_awesome, 'AI provider', const Color(0xFF7C3AED)),
    const SizedBox(height: 12),
    DropdownButtonFormField<_AiPreset>(
                      initialValue: _preset,
                      decoration: const InputDecoration(
                        labelText: 'Provider',
                        border: OutlineInputBorder(),
                      ),
                      items: _AiPreset.values
                          .map(
                            (preset) => DropdownMenuItem(
                              value: preset,
                              child: Text(preset.label),
                            ),
                          )
                          .toList(),
                      onChanged: _saving
                          ? null
                          : (preset) {
                              if (preset != null) _applyPreset(preset);
                            },
                    ),
                    const SizedBox(height: 12),
                    TextField(
                      controller: _baseUrl,
                      enabled: !_saving,
                      decoration: const InputDecoration(
                        labelText: 'API base URL',
                        hintText: 'https://api.deepseek.com',
                        border: OutlineInputBorder(),
                      ),
                    ),
                    const SizedBox(height: 12),
                    TextField(
                      controller: _model,
                      enabled: !_saving,
                      decoration: const InputDecoration(
                        labelText: 'Model',
                        hintText: 'deepseek-chat',
                        border: OutlineInputBorder(),
                      ),
                    ),
                    const SizedBox(height: 12),
                    TextField(
                      controller: _apiKey,
                      enabled: !_saving,
                      obscureText: true,
                      decoration: InputDecoration(
                        labelText: 'API key',
                        hintText: _hasKey
                            ? '•••••••• (leave blank to keep)'
                            : 'Required for hosted providers',
                        border: const OutlineInputBorder(),
                      ),
                    ),
                    const SizedBox(height: 6),
                    Text(
                      'Local models (Ollama, LM Studio) usually need no key. '
                      'The key is stored on the server and never returned.',
                      style: Theme.of(context).textTheme.bodySmall?.copyWith(
                        color: const Color(0xFF64748B),
                      ),
                    ),
    if (_error != null) ...[
      const SizedBox(height: 12),
      ErrorBanner(_error!),
    ],
    if (_saved != null) ...[
      const SizedBox(height: 12),
      Row(
        children: [
          const Icon(Icons.check_circle, color: Color(0xFF16A34A), size: 18),
          const SizedBox(width: 6),
          Text(_saved!),
        ],
      ),
    ],
  ];

  List<Widget> _accountSection(BuildContext context) => [
    _sectionTitle(context, Icons.person_outline, 'Account', const Color(0xFF2563EB)),
    const SizedBox(height: 4),
    Text(
      widget.userEmail,
      style: Theme.of(context).textTheme.bodySmall?.copyWith(
        color: const Color(0xFF64748B),
      ),
    ),
    const SizedBox(height: 12),
    TextField(
      controller: _name,
      enabled: !_accountBusy,
      decoration: const InputDecoration(
        labelText: 'Display name',
        border: OutlineInputBorder(),
      ),
    ),
    const SizedBox(height: 8),
    Align(
      alignment: Alignment.centerLeft,
      child: OutlinedButton.icon(
        onPressed: _accountBusy ? null : _saveProfile,
        icon: const Icon(Icons.save, size: 16),
        label: const Text('Save name'),
      ),
    ),
    const SizedBox(height: 16),
    TextField(
      controller: _curPass,
      enabled: !_accountBusy,
      obscureText: true,
      decoration: const InputDecoration(
        labelText: 'Current password',
        border: OutlineInputBorder(),
      ),
    ),
    const SizedBox(height: 8),
    TextField(
      controller: _newPass,
      enabled: !_accountBusy,
      obscureText: true,
      decoration: const InputDecoration(
        labelText: 'New password (min 8 chars)',
        border: OutlineInputBorder(),
      ),
    ),
    const SizedBox(height: 8),
    Align(
      alignment: Alignment.centerLeft,
      child: OutlinedButton.icon(
        onPressed: _accountBusy ? null : _changeAccountPassword,
        icon: const Icon(Icons.lock_outline, size: 16),
        label: const Text('Change password'),
      ),
    ),
    if (_accountMsg != null) ...[
      const SizedBox(height: 10),
      Text(
        _accountMsg!,
        style: const TextStyle(color: Color(0xFF64748B), fontSize: 13),
      ),
    ],
  ];

  List<Widget> _dataSection(BuildContext context) => [
    _sectionTitle(context, Icons.import_export, 'Data', const Color(0xFF0EA5E9)),
    const SizedBox(height: 12),
    const Text(
      'Import a workspace from a ZIP — a Mica export or a Notion '
      '"Markdown & CSV" export. The page tree, ordering and images are '
      'rebuilt.',
      style: TextStyle(color: Color(0xFF64748B)),
    ),
    const SizedBox(height: 12),
    Align(
      alignment: Alignment.centerLeft,
      child: OutlinedButton.icon(
        onPressed: () => widget.onImportWorkspace(),
        icon: const Icon(Icons.upload_file_outlined, size: 18),
        label: const Text('Import workspace (ZIP)'),
      ),
    ),
    const SizedBox(height: 16),
    Text(
      'Tip: export a single page or a whole workspace from the page menu (▾) '
      'or a workspace’s ⋯ menu.',
      style: Theme.of(context).textTheme.bodySmall?.copyWith(
        color: const Color(0xFF94A3B8),
      ),
    ),
  ];

  @override
  Widget build(BuildContext context) {
    const titles = ['Appearance', 'AI provider', 'Account', 'Data'];
    const icons = [
      Icons.tune,
      Icons.auto_awesome,
      Icons.person_outline,
      Icons.import_export,
    ];
    final sections = [
      _appearanceSection(context),
      _aiSection(context),
      _accountSection(context),
      _dataSection(context),
    ];
    return AlertDialog(
      title: const Row(
        children: [
          Icon(Icons.settings_outlined, size: 22),
          SizedBox(width: 8),
          Text('Settings'),
        ],
      ),
      contentPadding: const EdgeInsets.fromLTRB(0, 12, 0, 0),
      content: SizedBox(
        width: 720,
        height: 460,
        child: _loading
            ? const Center(child: CircularProgressIndicator())
            : Row(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  SizedBox(
                    width: 180,
                    child: ListView(
                      padding: const EdgeInsets.symmetric(vertical: 4),
                      children: [
                        for (var i = 0; i < titles.length; i++)
                          ListTile(
                            dense: true,
                            selected: _tab == i,
                            leading: Icon(icons[i], size: 20),
                            title: Text(titles[i]),
                            onTap: () => setState(() => _tab = i),
                          ),
                      ],
                    ),
                  ),
                  const VerticalDivider(width: 1),
                  Expanded(
                    child: SingleChildScrollView(
                      padding: const EdgeInsets.fromLTRB(20, 16, 20, 16),
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        mainAxisSize: MainAxisSize.min,
                        children: sections[_tab],
                      ),
                    ),
                  ),
                ],
              ),
      ),
      actions: [
        TextButton(
          onPressed: _saving ? null : () => Navigator.of(context).pop(),
          child: const Text('Close'),
        ),
        FilledButton.icon(
          onPressed: _saving || _loading ? null : _save,
          icon: _saving
              ? const SizedBox(
                  width: 16,
                  height: 16,
                  child: CircularProgressIndicator(strokeWidth: 2),
                )
              : const Icon(Icons.save, size: 18),
          label: const Text('Save'),
        ),
      ],
    );
  }
}

/// Global AI dialog: type an instruction and pick where the generated content
/// goes — a new page, the current page, or a brand-new workspace.
class _AiDialog extends StatefulWidget {
  const _AiDialog({
    required this.canEdit,
    required this.hasWorkspace,
    required this.onStream,
    required this.onNewPage,
    required this.onCurrentPage,
    required this.onNewWorkspace,
  });

  final bool canEdit;
  final bool hasWorkspace;
  final Stream<String> Function(String prompt, {String? system}) onStream;
  final Future<void> Function(String markdown) onNewPage;
  final Future<void> Function(String markdown)? onCurrentPage;
  final Future<void> Function(String markdown) onNewWorkspace;

  @override
  State<_AiDialog> createState() => _AiDialogState();
}

class _AiDialogState extends State<_AiDialog> {
  final _prompt = TextEditingController();
  final _scroll = ScrollController();
  late _AiTarget _target = widget.hasWorkspace
      ? _AiTarget.newPage
      : _AiTarget.newWorkspace;
  StreamSubscription<String>? _sub;
  bool _streaming = false;
  bool _applying = false;
  bool _done = false;
  final StringBuffer _buffer = StringBuffer();
  String? _error;

  bool get _busy => _streaming || _applying;

  @override
  void dispose() {
    _sub?.cancel();
    _prompt.dispose();
    _scroll.dispose();
    super.dispose();
  }

  void _generate() {
    final prompt = _prompt.text.trim();
    if (prompt.isEmpty) return;
    // New page / new workspace want a document with a title line.
    final system = _target == _AiTarget.currentPage ? null : kAiDocSystem;
    setState(() {
      _streaming = true;
      _done = false;
      _error = null;
      _buffer.clear();
    });
    _sub = widget.onStream(prompt, system: system).listen(
      (delta) {
        setState(() => _buffer.write(delta));
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (_scroll.hasClients) {
            _scroll.jumpTo(_scroll.position.maxScrollExtent);
          }
        });
      },
      onError: (Object error) {
        if (mounted) {
          setState(() {
            _streaming = false;
            _error = error.toString();
          });
        }
      },
      onDone: () {
        if (mounted) {
          setState(() {
            _streaming = false;
            _done = true;
          });
        }
      },
    );
  }

  Future<void> _apply() async {
    final markdown = _buffer.toString().trim();
    if (markdown.isEmpty) return;
    setState(() {
      _applying = true;
      _error = null;
    });
    try {
      switch (_target) {
        case _AiTarget.newPage:
          await widget.onNewPage(markdown);
        case _AiTarget.currentPage:
          await widget.onCurrentPage?.call(markdown);
        case _AiTarget.newWorkspace:
          await widget.onNewWorkspace(markdown);
      }
      if (mounted) Navigator.of(context).pop();
    } catch (error) {
      if (mounted) setState(() => _error = error.toString());
    } finally {
      if (mounted) setState(() => _applying = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final canWriteCurrent = widget.onCurrentPage != null;
    final hasOutput = _buffer.isNotEmpty;
    return AlertDialog(
      title: const Row(
        children: [
          Icon(Icons.auto_awesome, size: 22, color: Color(0xFF7C3AED)),
          SizedBox(width: 8),
          Text('Ask AI'),
        ],
      ),
      content: SizedBox(
        width: 520,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            TextField(
              controller: _prompt,
              autofocus: true,
              minLines: 2,
              maxLines: 5,
              enabled: !_busy,
              decoration: const InputDecoration(
                hintText:
                    'e.g. Write a project kickoff plan with goals, milestones, and risks',
                border: OutlineInputBorder(),
              ),
            ),
            const SizedBox(height: 12),
            Wrap(
              spacing: 8,
              children: [
                if (widget.canEdit)
                  ChoiceChip(
                    label: const Text('New page'),
                    selected: _target == _AiTarget.newPage,
                    onSelected: widget.hasWorkspace && !_busy
                        ? (_) => setState(() => _target = _AiTarget.newPage)
                        : null,
                  ),
                if (widget.canEdit && canWriteCurrent)
                  ChoiceChip(
                    label: const Text('Current page'),
                    selected: _target == _AiTarget.currentPage,
                    onSelected: _busy
                        ? null
                        : (_) => setState(() => _target = _AiTarget.currentPage),
                  ),
                ChoiceChip(
                  label: const Text('New workspace'),
                  selected: _target == _AiTarget.newWorkspace,
                  onSelected: _busy
                      ? null
                      : (_) => setState(() => _target = _AiTarget.newWorkspace),
                ),
              ],
            ),
            if (hasOutput || _streaming) ...[
              const SizedBox(height: 12),
              Container(
                height: 220,
                width: double.infinity,
                padding: const EdgeInsets.all(12),
                decoration: BoxDecoration(
                  color: const Color(0xFFF8FAFC),
                  borderRadius: BorderRadius.circular(8),
                  border: Border.all(color: const Color(0xFFE2E8F0)),
                ),
                child: SingleChildScrollView(
                  controller: _scroll,
                  child: Text(
                    _buffer.isEmpty ? '…' : _buffer.toString(),
                    style: const TextStyle(fontSize: 13, height: 1.5),
                  ),
                ),
              ),
              const SizedBox(height: 6),
              Row(
                children: [
                  if (_streaming) ...[
                    const SizedBox(
                      width: 14,
                      height: 14,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    ),
                    const SizedBox(width: 8),
                    const Text('Generating…'),
                  ] else if (_done)
                    const Text(
                      'Done — review, then insert.',
                      style: TextStyle(color: Color(0xFF64748B)),
                    ),
                ],
              ),
            ],
            if (_error != null) ...[
              const SizedBox(height: 12),
              ErrorBanner(_error!),
            ],
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: _busy ? null : () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        if (!_done)
          FilledButton.icon(
            onPressed: _busy ? null : _generate,
            icon: const Icon(Icons.auto_awesome, size: 18),
            label: Text(hasOutput ? 'Regenerate' : 'Generate'),
          )
        else ...[
          TextButton.icon(
            onPressed: _applying ? null : _generate,
            icon: const Icon(Icons.refresh, size: 18),
            label: const Text('Regenerate'),
          ),
          FilledButton.icon(
            onPressed: _applying ? null : _apply,
            icon: _applying
                ? const SizedBox(
                    width: 16,
                    height: 16,
                    child: CircularProgressIndicator(strokeWidth: 2),
                  )
                : const Icon(Icons.check, size: 18),
            label: const Text('Insert'),
          ),
        ],
      ],
    );
  }
}

/// Recycle bin: lists soft-deleted pages and offers restore / delete-forever.
/// Only the roots of each deleted subtree are shown; restoring a root brings its
/// whole subtree back.
class _RecycleBinDialog extends StatefulWidget {
  const _RecycleBinDialog({
    required this.onLoad,
    required this.onRestore,
    required this.onPurge,
  });

  final Future<List<DocumentView>> Function() onLoad;
  final Future<void> Function(DocumentView view) onRestore;
  final Future<void> Function(DocumentView view) onPurge;

  @override
  State<_RecycleBinDialog> createState() => _RecycleBinDialogState();
}

class _RecycleBinDialogState extends State<_RecycleBinDialog> {
  bool _loading = true;
  String? _error;
  List<DocumentView> _roots = const [];

  @override
  void initState() {
    super.initState();
    _refresh();
  }

  Future<void> _refresh() async {
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final all = await widget.onLoad();
      final ids = {for (final v in all) v.id};
      // Show only subtree roots: a deleted page whose parent is not itself in
      // the bin (children come back with their parent on restore).
      final roots = all
          .where((v) => v.parentViewId == null || !ids.contains(v.parentViewId))
          .toList();
      if (!mounted) return;
      setState(() {
        _roots = roots;
        _loading = false;
      });
    } catch (error) {
      if (!mounted) return;
      setState(() {
        _error = error.toString();
        _loading = false;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: Row(
        children: [
          const Icon(Icons.delete_outline, size: 22),
          const SizedBox(width: 8),
          const Text('Recycle bin'),
          const Spacer(),
          IconButton(
            tooltip: 'Refresh',
            onPressed: _loading ? null : _refresh,
            icon: const Icon(Icons.refresh, size: 20),
          ),
        ],
      ),
      content: SizedBox(
        width: 420,
        height: 360,
        child: _buildBody(context),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Close'),
        ),
      ],
    );
  }

  Widget _buildBody(BuildContext context) {
    if (_loading) {
      return const Center(child: CircularProgressIndicator());
    }
    if (_error != null) {
      return Center(child: ErrorBanner(_error!));
    }
    if (_roots.isEmpty) {
      return const EmptyState(
        icon: Icons.delete_outline,
        title: 'Recycle bin is empty',
        detail: 'Deleted pages appear here and can be restored.',
      );
    }
    return ListView.separated(
      itemCount: _roots.length,
      separatorBuilder: (_, _) => const Divider(height: 1),
      itemBuilder: (context, i) {
        final view = _roots[i];
        return ListTile(
          contentPadding: const EdgeInsets.symmetric(horizontal: 4),
          leading: const Icon(Icons.description_outlined, size: 18),
          title: Text(view.name, overflow: TextOverflow.ellipsis),
          trailing: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              IconButton(
                tooltip: 'Restore',
                icon: const Icon(Icons.restore, size: 20),
                onPressed: () async {
                  await widget.onRestore(view);
                  await _refresh();
                },
              ),
              IconButton(
                tooltip: 'Delete forever',
                color: const Color(0xFFDC2626),
                icon: const Icon(Icons.delete_forever, size: 20),
                onPressed: () async {
                  await widget.onPurge(view);
                  await _refresh();
                },
              ),
            ],
          ),
        );
      },
    );
  }
}

class DocumentListItem extends StatelessWidget {
  const DocumentListItem({
    required this.view,
    required this.depth,
    required this.hasChildren,
    required this.isCollapsed,
    required this.isSelected,
    required this.canEdit,
    required this.onToggle,
    required this.onPressed,
    required this.onCreateChild,
    required this.onRename,
    required this.onDelete,
    super.key,
  });

  final DocumentView view;
  final int depth;
  final bool hasChildren;
  final bool isCollapsed;
  final bool isSelected;
  final bool canEdit;
  final VoidCallback onToggle;
  final VoidCallback onPressed;
  final VoidCallback onCreateChild;
  final VoidCallback onRename;
  final VoidCallback onDelete;

  @override
  Widget build(BuildContext context) {
    return Material(
      color: isSelected ? const Color(0xFFEFF6FF) : Colors.transparent,
      borderRadius: BorderRadius.circular(6),
      child: InkWell(
        borderRadius: BorderRadius.circular(6),
        onTap: onPressed,
        child: ConstrainedBox(
          constraints: const BoxConstraints(minHeight: 38),
          child: Padding(
            padding: EdgeInsets.only(left: 8 + (depth * 18), right: 4),
            child: Row(
              children: [
                SizedBox(
                  width: 24,
                  height: 30,
                  child: hasChildren
                      ? IconButton(
                          tooltip: isCollapsed ? 'Expand' : 'Collapse',
                          onPressed: onToggle,
                          padding: EdgeInsets.zero,
                          iconSize: 18,
                          icon: Icon(
                            isCollapsed
                                ? Icons.chevron_right
                                : Icons.expand_more,
                          ),
                        )
                      : const SizedBox.shrink(),
                ),
                Icon(
                  Icons.description_outlined,
                  size: 18,
                  color: isSelected
                      ? const Color(0xFF2563EB)
                      : const Color(0xFF64748B),
                ),
                const SizedBox(width: 8),
                Expanded(
                  child: Text(
                    view.name,
                    overflow: TextOverflow.ellipsis,
                    style: Theme.of(context).textTheme.bodyMedium?.copyWith(
                      fontWeight: isSelected
                          ? FontWeight.w600
                          : FontWeight.w400,
                    ),
                  ),
                ),
                if (canEdit) ...[
                  SizedBox(
                    width: 30,
                    height: 30,
                    child: IconButton(
                      tooltip: 'Create child page',
                      onPressed: onCreateChild,
                      padding: EdgeInsets.zero,
                      iconSize: 17,
                      icon: const Icon(Icons.add),
                    ),
                  ),
                  SizedBox(
                    width: 30,
                    height: 30,
                    child: IconButton(
                      tooltip: 'Rename',
                      onPressed: onRename,
                      padding: EdgeInsets.zero,
                      iconSize: 17,
                      icon: const Icon(Icons.edit_outlined),
                    ),
                  ),
                  SizedBox(
                    width: 30,
                    height: 30,
                    child: IconButton(
                      tooltip: 'Delete',
                      onPressed: onDelete,
                      padding: EdgeInsets.zero,
                      iconSize: 17,
                      icon: const Icon(Icons.delete_outline),
                    ),
                  ),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class EmptyState extends StatelessWidget {
  const EmptyState({
    required this.icon,
    required this.title,
    required this.detail,
    super.key,
  });

  final IconData icon;
  final String title;
  final String detail;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 360),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(icon, size: 40, color: const Color(0xFF64748B)),
            const SizedBox(height: 12),
            Text(title, style: Theme.of(context).textTheme.titleLarge),
            const SizedBox(height: 6),
            Text(
              detail,
              textAlign: TextAlign.center,
              style: Theme.of(
                context,
              ).textTheme.bodyMedium?.copyWith(color: const Color(0xFF64748B)),
            ),
          ],
        ),
      ),
    );
  }
}

class DetailRow extends StatelessWidget {
  const DetailRow({required this.label, required this.value, super.key});

  final String label;
  final String value;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 6),
      child: Row(
        children: [
          SizedBox(
            width: 88,
            child: Text(label, style: Theme.of(context).textTheme.labelLarge),
          ),
          Expanded(child: SelectableText(value)),
        ],
      ),
    );
  }
}

class ErrorBanner extends StatelessWidget {
  const ErrorBanner(this.message, {super.key});

  final String message;

  @override
  Widget build(BuildContext context) {
    return DecoratedBox(
      decoration: BoxDecoration(
        color: const Color(0xFFFEF2F2),
        borderRadius: BorderRadius.circular(8),
        border: Border.all(color: const Color(0xFFFCA5A5)),
      ),
      child: Padding(
        padding: const EdgeInsets.all(12),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const Icon(Icons.error_outline, color: Color(0xFFDC2626)),
            const SizedBox(width: 10),
            Expanded(child: SelectableText(message)),
          ],
        ),
      ),
    );
  }
}

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

class ApiClient {
  ApiClient() : _baseUri = _resolveBaseUri();

  final Uri _baseUri;

  /// HTTP base used to derive the WebSocket endpoint for document rooms.
  Uri get baseUri => _baseUri;

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

  Future<void> purgeView(String token, String workspaceId, String viewId) async {
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
    final uri = _baseUri.replace(
      scheme: _baseUri.scheme == 'https' ? 'wss' : 'ws',
      path: '/ws/ai',
      queryParameters: {'token': token},
    );
    final channel = WebSocketChannel.connect(uri);
    try {
      channel.sink.add(
        jsonEncode({'prompt': prompt, 'system': ?system}),
      );
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
    final presign = await _post(
      '/api/workspaces/$workspaceId/files/presign',
      {
        'file_name': fileName,
        'mime_type': mimeType,
        'byte_size': bytes.length,
        'content_hash': hash,
      },
      token: token,
    );
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

    final complete = await _post(
      '/api/workspaces/$workspaceId/files/complete',
      {
        'object_key': objectKey,
        'file_name': fileName,
        'mime_type': mimeType,
        'byte_size': bytes.length,
      },
      token: token,
    );
    return UploadedFile.fromResponse(complete);
  }

  /// Download a page as a portable ZIP (markdown + bundled image assets).
  Future<Uint8List> exportDocumentZip(
    String token,
    String workspaceId,
    String documentId,
  ) async {
    final response = await http.get(
      _baseUri.replace(
        path: '/api/workspaces/$workspaceId/documents/$documentId/export.zip',
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
      _baseUri.replace(path: '/api/workspaces/$workspaceId/export.zip'),
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
    final response = await _post(
      '/api/workspaces/$workspaceId/files/resolve',
      {'ids': ids},
      token: token,
    );
    final files = (response['files'] as List<dynamic>? ?? []);
    final out = <String, String>{};
    for (final raw in files) {
      final f = UploadedFile.fromResponse(raw as Map<String, dynamic>);
      out[f.id] = f.downloadUrl;
    }
    return out;
  }

  Future<Map<String, dynamic>> _get(String path, String token) async {
    final response = await http.get(
      _baseUri.replace(path: path),
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
      _baseUri.replace(path: path),
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
      _baseUri.replace(path: path),
      headers: _headers(token),
      body: jsonEncode(body),
    );
    return _decode(response);
  }

  Future<Map<String, dynamic>> _delete(String path, String token) async {
    final response = await http.delete(
      _baseUri.replace(path: path),
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
      throw ApiException(message);
    }
    return body;
  }

  static Uri _resolveBaseUri() {
    const configured = String.fromEnvironment('MICA_API_BASE_URL');
    if (configured.isNotEmpty) {
      return Uri.parse(configured);
    }

    final page = Uri.base;
    return Uri(
      scheme: page.scheme.isEmpty ? 'http' : page.scheme,
      host: page.host.isEmpty ? '127.0.0.1' : page.host,
      port: 8080,
    );
  }
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
  const AuthSession({required this.accessToken, required this.user});

  factory AuthSession.fromJson(Map<String, dynamic> json) {
    return AuthSession(
      accessToken: json['access_token'] as String,
      user: User.fromJson(json['user'] as Map<String, dynamic>),
    );
  }

  final String accessToken;
  final User user;
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

/// Live collaborator indicator shown in the document header. Renders an avatar
/// per other connected user, or "Only you" when alone.
class _PresenceBar extends StatelessWidget {
  const _PresenceBar({required this.presence});

  final List<PresenceUser> presence;

  static const List<Color> _palette = [
    Color(0xFF2563EB),
    Color(0xFF16A34A),
    Color(0xFFDB2777),
    Color(0xFFD97706),
    Color(0xFF7C3AED),
    Color(0xFF0891B2),
  ];

  @override
  Widget build(BuildContext context) {
    if (presence.isEmpty) {
      return Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          const Icon(Icons.circle, size: 8, color: Color(0xFF94A3B8)),
          const SizedBox(width: 6),
          Text(
            'Only you',
            style: Theme.of(
              context,
            ).textTheme.bodySmall?.copyWith(color: const Color(0xFF64748B)),
          ),
        ],
      );
    }

    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        const Icon(Icons.circle, size: 8, color: Color(0xFF16A34A)),
        const SizedBox(width: 8),
        for (var i = 0; i < presence.length && i < 5; i++)
          Padding(
            padding: const EdgeInsets.only(right: 4),
            child: Tooltip(
              message: presence[i].name,
              child: CircleAvatar(
                radius: 12,
                backgroundColor: _palette[i % _palette.length],
                child: Text(
                  _initial(presence[i].name),
                  style: const TextStyle(fontSize: 11, color: Colors.white),
                ),
              ),
            ),
          ),
        const SizedBox(width: 4),
        Text(
          presence.length == 1 ? '1 editing' : '${presence.length} editing',
          style: Theme.of(
            context,
          ).textTheme.bodySmall?.copyWith(color: const Color(0xFF16A34A)),
        ),
      ],
    );
  }

  String _initial(String name) {
    final trimmed = name.trim();
    return trimmed.isEmpty ? '?' : trimmed.characters.first.toUpperCase();
  }
}

class ApiException implements Exception {
  const ApiException(this.message);

  final String message;

  @override
  String toString() => message;
}

/// A collaborator currently connected to the same document room.
class PresenceUser {
  const PresenceUser({
    required this.connectionId,
    required this.userId,
    required this.name,
  });

  final String connectionId;
  final String userId;
  final String name;
}

typedef RemoteSeqCallback = void Function(String documentId, int serverSeq);
typedef PresenceCallback = void Function(List<PresenceUser> users);

/// WebSocket client for a single document room.
///
/// Receives the server's accepted-update sequence (so the shell can pull the
/// latest snapshot) and tracks presence of other collaborators. Local edits
/// continue to flow over REST, which the backend broadcasts here.
class DocumentSyncClient {
  DocumentSyncClient({
    required this.documentId,
    required this.uri,
    required this.selfName,
    required this.onRemoteSeq,
    required this.onPresence,
  });

  final String documentId;
  final Uri uri;
  final String selfName;
  final RemoteSeqCallback onRemoteSeq;
  final PresenceCallback onPresence;

  WebSocketChannel? _channel;
  StreamSubscription<dynamic>? _subscription;
  String? _connectionId;
  final Map<String, PresenceUser> _presence = {};
  bool _disposed = false;

  void connect() {
    final channel = WebSocketChannel.connect(uri);
    _channel = channel;
    _subscription = channel.stream.listen(
      _onMessage,
      onError: (_) {},
      onDone: () {},
      cancelOnError: false,
    );
  }

  void _onMessage(dynamic raw) {
    if (raw is! String) {
      return;
    }

    final Map<String, dynamic> message;
    try {
      message = jsonDecode(raw) as Map<String, dynamic>;
    } catch (_) {
      return;
    }

    switch (message['type']) {
      case 'document.bootstrap':
        _connectionId = message['connection_id'] as String?;
        _sendPresence();
      case 'document.update.accepted':
        final seq = message['server_seq'];
        if (seq is int) {
          onRemoteSeq(documentId, seq);
        }
      case 'presence.state':
        _presence.clear();
        final list = message['presences'];
        if (list is List<dynamic>) {
          for (final entry in list) {
            if (entry is Map<String, dynamic>) {
              _upsertPresence(entry);
            }
          }
        }
        _emitPresence();
      case 'presence.update':
        _upsertPresence(message);
        _emitPresence();
      case 'presence.leave':
        final connectionId = message['connection_id'];
        if (connectionId is String) {
          _presence.remove(connectionId);
        }
        _emitPresence();
    }
  }

  void _upsertPresence(Map<String, dynamic> message) {
    final connectionId = message['connection_id'];
    final userId = message['user_id'];
    if (connectionId is! String || userId is! String) {
      return;
    }

    var name = userId;
    final data = message['data'];
    if (data is Map<String, dynamic> && data['name'] is String) {
      name = data['name'] as String;
    }

    _presence[connectionId] = PresenceUser(
      connectionId: connectionId,
      userId: userId,
      name: name,
    );
  }

  void _emitPresence() {
    if (_disposed) {
      return;
    }
    final others = _presence.values
        .where((user) => user.connectionId != _connectionId)
        .toList();
    onPresence(others);
  }

  void _sendPresence() {
    _channel?.sink.add(
      jsonEncode({
        'type': 'presence.update',
        'payload': {'name': selfName},
      }),
    );
  }

  void dispose() {
    _disposed = true;
    _subscription?.cancel();
    _channel?.sink.close();
    _channel = null;
  }
}

Uri documentSocketUri(
  Uri base,
  String workspaceId,
  String documentId,
  String token,
) {
  final scheme = base.scheme == 'https' ? 'wss' : 'ws';
  return base.replace(
    scheme: scheme,
    path: '/ws/workspaces/$workspaceId/documents/$documentId',
    queryParameters: {'token': token},
  );
}

