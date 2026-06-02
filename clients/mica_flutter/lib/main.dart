import 'dart:convert';
import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:http/http.dart' as http;
import 'package:web_socket_channel/web_socket_channel.dart';

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
  runApp(const MicaApp());
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
      appBar: AppBar(
        title: const Text('Mica'),
        actions: [
          if (_isBusy)
            const Padding(
              padding: EdgeInsets.only(right: 16),
              child: SizedBox(
                height: 18,
                width: 18,
                child: CircularProgressIndicator(strokeWidth: 2),
              ),
            ),
          if (session != null)
            IconButton(
              tooltip: 'Refresh',
              onPressed: _isBusy ? null : _refreshWorkspaces,
              icon: const Icon(Icons.refresh),
            ),
          if (session != null)
            IconButton(
              tooltip: 'Sign out',
              onPressed: _isBusy ? null : _signOut,
              icon: const Icon(Icons.logout),
            ),
        ],
      ),
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
                onCreateDocument: _createDocument,
                onCreateChildDocument: _createChildDocument,
                onSelectView: _selectView,
                onRenameView: _renameView,
                onDeleteView: _deleteView,
                onUpdateRootBlockText: _updateRootBlockText,
                onAddBlock: _addBlock,
                onUpdateBlock: _updateBlock,
                onDeleteBlock: _deleteBlock,
                onMoveBlock: _moveBlock,
                onApplyOperations: _applyEditorOperations,
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
    required this.onCreateDocument,
    required this.onCreateChildDocument,
    required this.onSelectView,
    required this.onRenameView,
    required this.onDeleteView,
    required this.onUpdateRootBlockText,
    required this.onAddBlock,
    required this.onUpdateBlock,
    required this.onDeleteBlock,
    required this.onMoveBlock,
    required this.onApplyOperations,
    required this.onExportMarkdown,
    required this.onAddMember,
    required this.onUpdateMember,
    required this.onRemoveMember,
    super.key,
  });

  final AuthSession? session;
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
  final Future<void> Function(String name) onCreateDocument;
  final Future<void> Function(DocumentView parent, String name)
  onCreateChildDocument;
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
  Timer? _pageTitleSaveTimer;
  final Set<String> _collapsedViewIds = {};
  WorkspaceRole _memberRole = WorkspaceRole.editor;
  bool _toolsExpanded = true;

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
            Text('Workspace', style: Theme.of(context).textTheme.titleMedium),
            const SizedBox(height: 10),
            if (widget.workspaces.isEmpty)
              const EmptyState(
                icon: Icons.workspaces,
                title: 'No workspaces',
                detail: 'Create one below.',
              )
            else
              Row(
                children: [
                  Expanded(
                    child: DropdownButtonFormField<String>(
                      isExpanded: true,
                      initialValue: widget.selectedWorkspace?.id,
                      decoration: const InputDecoration(
                        border: OutlineInputBorder(),
                        contentPadding: EdgeInsets.symmetric(
                          horizontal: 12,
                          vertical: 8,
                        ),
                        prefixIcon: Icon(Icons.workspaces_outline, size: 20),
                      ),
                      items: widget.workspaces
                          .map(
                            (workspace) => DropdownMenuItem(
                              value: workspace.id,
                              child: Text(
                                workspace.name,
                                overflow: TextOverflow.ellipsis,
                              ),
                            ),
                          )
                          .toList(),
                      onChanged: (id) {
                        if (id == null) {
                          return;
                        }
                        final workspace = widget.workspaces
                            .where((item) => item.id == id)
                            .firstOrNull;
                        if (workspace != null &&
                            workspace.id != widget.selectedWorkspace?.id) {
                          widget.onSelectWorkspace(workspace);
                        }
                      },
                    ),
                  ),
                  const SizedBox(width: 8),
                  IconButton.filledTonal(
                    tooltip: 'New workspace',
                    onPressed: _promptCreateWorkspace,
                    icon: const Icon(Icons.add),
                  ),
                ],
              ),
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
                if (canEdit)
                  IconButton(
                    tooltip: 'New page',
                    visualDensity: VisualDensity.compact,
                    onPressed: () {
                      widget.onCreateDocument('Untitled');
                    },
                    icon: const Icon(Icons.note_add_outlined),
                  ),
              ],
            ),
            const SizedBox(height: 12),
            Expanded(child: _pageTree(context, canEdit)),
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
      children: _visibleDocumentTree()
          .map(
            (item) => Padding(
              padding: const EdgeInsets.only(bottom: 8),
              child: DocumentListItem(
                view: item.view,
                depth: item.depth,
                hasChildren: item.hasChildren,
                isCollapsed: _collapsedViewIds.contains(item.view.id),
                isSelected: item.view.id == widget.selectedView?.id,
                canEdit: canEdit,
                onToggle: () => _toggleViewCollapse(item.view),
                onPressed: () => widget.onSelectView(item.view),
                onCreateChild: () => _promptCreateChildDocument(item.view),
                onRename: () => _promptRenameDocument(item.view),
                onDelete: () => _confirmDeleteDocument(item.view),
              ),
            ),
          )
          .toList(),
    );
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
    for (final view in widget.views) {
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
      color: const Color(0xFFF8FAFC),
      child: SingleChildScrollView(
        padding: const EdgeInsets.symmetric(horizontal: 28, vertical: 28),
        child: Center(
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 1160),
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
                        onChanged: (_) => _schedulePageTitleSave(),
                        decoration: const InputDecoration(
                          hintText: 'Untitled',
                          border: InputBorder.none,
                        ),
                      ),
                    ),
                    OutlinedButton.icon(
                      onPressed: widget.onExportMarkdown,
                      icon: const Icon(Icons.download_outlined),
                      label: const Text('Export'),
                    ),
                    const SizedBox(width: 8),
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
                const SizedBox(height: 20),
                DecoratedBox(
                  decoration: BoxDecoration(
                    color: Colors.white,
                    borderRadius: BorderRadius.circular(8),
                    border: Border.all(color: const Color(0xFFE2E8F0)),
                  ),
                  child: Padding(
                    padding: const EdgeInsets.symmetric(
                      horizontal: 24,
                      vertical: 20,
                    ),
                    child: BlockEditor(
                      key: ValueKey(bootstrap.document.id),
                      documentId: bootstrap.document.id,
                      rootBlockId: bootstrap.document.rootBlockId,
                      blocks: bootstrap.childBlocks,
                      version: bootstrap.snapshot.versionSeq,
                      canEdit: canEdit,
                      onApplyOperations: widget.onApplyOperations,
                    ),
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

  Widget _workspaceTools(BuildContext context) {
    final workspace = widget.selectedWorkspace;
    if (workspace == null) {
      return const ColoredBox(
        color: Colors.white,
        child: EmptyState(
          icon: Icons.workspaces,
          title: 'Workspace',
          detail: 'Select a workspace.',
        ),
      );
    }

    if (_rename.text.isEmpty) {
      _rename.text = workspace.name;
    }

    final canManage = matchesManageRole(workspace.role);

    return ColoredBox(
      color: Colors.white,
      child: SingleChildScrollView(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text('Workspace', style: Theme.of(context).textTheme.titleLarge),
            const SizedBox(height: 12),
            DetailRow(label: 'Role', value: workspace.role),
            DetailRow(label: 'ID', value: workspace.id),
            const SizedBox(height: 16),
            TextField(
              controller: _rename,
              decoration: const InputDecoration(
                labelText: 'Rename workspace',
                prefixIcon: Icon(Icons.edit),
                border: OutlineInputBorder(),
              ),
            ),
            const SizedBox(height: 12),
            FilledButton.icon(
              onPressed: () =>
                  widget.onRenameWorkspace(workspace, _rename.text),
              icon: const Icon(Icons.save),
              label: const Text('Save'),
            ),
            const SizedBox(height: 28),
            Row(
              children: [
                const Icon(Icons.group),
                const SizedBox(width: 10),
                Text('Members', style: Theme.of(context).textTheme.titleLarge),
              ],
            ),
            const SizedBox(height: 14),
            if (canManage) _compactAddMemberForm(),
            if (canManage) const SizedBox(height: 16),
            if (widget.members.isEmpty)
              const EmptyState(
                icon: Icons.group,
                title: 'No members loaded',
                detail: 'Members appear here after the workspace is selected.',
              )
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

  Future<void> _promptCreateChildDocument(DocumentView parent) async {
    final controller = TextEditingController();
    final name = await showDialog<String>(
      context: context,
      builder: (context) {
        return AlertDialog(
          title: const Text('Create child page'),
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
              icon: const Icon(Icons.add),
              label: const Text('Create'),
            ),
          ],
        );
      },
    );
    controller.dispose();

    if (name == null) {
      return;
    }

    await widget.onCreateChildDocument(parent, name);
  }

  Future<void> _confirmDeleteDocument(DocumentView view) async {
    final shouldDelete = await showDialog<bool>(
      context: context,
      builder: (context) {
        return AlertDialog(
          title: const Text('Delete page'),
          content: Text(view.name, overflow: TextOverflow.ellipsis),
          actions: [
            TextButton(
              onPressed: () => Navigator.of(context).pop(false),
              child: const Text('Cancel'),
            ),
            FilledButton.icon(
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

    await widget.onDeleteView(view);
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
                  depth == 0
                      ? Icons.description_outlined
                      : Icons.subdirectory_arrow_right,
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

/// One editable block: its server id, current kind/data, and the controllers
/// backing its text field.
class _EditorBlock {
  _EditorBlock({
    required this.id,
    required this.kind,
    required String text,
    required this.data,
  }) : controller = TextEditingController(text: text),
       focus = FocusNode();

  final String id;
  String kind;
  Map<String, dynamic> data;
  final TextEditingController controller;
  final FocusNode focus;
  final LayerLink link = LayerLink();
  Timer? saveTimer;

  void dispose() {
    saveTimer?.cancel();
    controller.dispose();
    focus.dispose();
  }
}

/// An entry in the slash (`/`) insert menu.
class _SlashOption {
  const _SlashOption(this.label, this.icon, this.kind, [this.data]);

  final String label;
  final IconData icon;
  final String kind;
  final Map<String, dynamic>? data;
}

const List<_SlashOption> _slashOptions = [
  _SlashOption('Text', Icons.notes, 'paragraph'),
  _SlashOption('Heading 1', Icons.title, 'heading', {'level': 1}),
  _SlashOption('Heading 2', Icons.title, 'heading', {'level': 2}),
  _SlashOption('Heading 3', Icons.title, 'heading', {'level': 3}),
  _SlashOption('Bulleted list', Icons.format_list_bulleted, 'bulleted_list'),
  _SlashOption('Numbered list', Icons.format_list_numbered, 'numbered_list'),
  _SlashOption('To-do', Icons.check_box_outlined, 'todo', {'checked': false}),
  _SlashOption('Quote', Icons.format_quote, 'quote'),
  _SlashOption('Code', Icons.code, 'code_block'),
];

/// A block-based document editor with Markdown shortcuts.
///
/// Typing `# `, `- `, `1. `, `> `, `- [ ] `, or ``` ``` ``` at the start of a
/// block converts it to the matching block type. Enter creates a new block (and
/// continues lists); Backspace at the start of a block merges it into the
/// previous one. Edits persist through [onApplyOperations]; remote snapshots are
/// reconciled without disturbing the block currently being edited.
class BlockEditor extends StatefulWidget {
  const BlockEditor({
    required this.documentId,
    required this.rootBlockId,
    required this.blocks,
    required this.version,
    required this.canEdit,
    required this.onApplyOperations,
    super.key,
  });

  final String documentId;
  final String rootBlockId;
  final List<DocumentBlock> blocks;
  final int version;
  final bool canEdit;
  final Future<void> Function(List<Map<String, dynamic>> operations)
  onApplyOperations;

  @override
  State<BlockEditor> createState() => _BlockEditorState();
}

class _BlockEditorState extends State<BlockEditor> {
  final List<_EditorBlock> _blocks = [];
  Future<void> _sendChain = Future.value();
  int _idCounter = 0;

  // Hover + slash-menu transient UI state. None of this is shown at rest.
  String? _hoveredId;
  String? _slashBlockId;
  String _slashQuery = '';
  OverlayEntry? _slashEntry;

  @override
  void initState() {
    super.initState();
    _syncFromWidget();
    _ensureInitialBlock();
  }

  @override
  void didUpdateWidget(covariant BlockEditor oldWidget) {
    super.didUpdateWidget(oldWidget);
    // A new snapshot arrived (our own write or a remote edit); reconcile.
    if (oldWidget.version != widget.version) {
      setState(_syncFromWidget);
      _ensureInitialBlock();
      // Close the slash menu if its block vanished in the new snapshot.
      if (_slashBlockId != null &&
          !_blocks.any((block) => block.id == _slashBlockId)) {
        _closeSlash();
      }
    }
  }

  @override
  void dispose() {
    _slashEntry?.remove();
    _slashEntry = null;
    for (final block in _blocks) {
      block.dispose();
    }
    super.dispose();
  }

  /// Reconcile local editor blocks against the latest server snapshot.
  /// Controllers are reused by id; the focused block is never overwritten so
  /// in-flight typing is preserved.
  void _syncFromWidget() {
    final existing = {for (final block in _blocks) block.id: block};
    final next = <_EditorBlock>[];

    for (final src in widget.blocks) {
      final current = existing.remove(src.id);
      if (current == null) {
        final block = _EditorBlock(
          id: src.id,
          kind: src.kind,
          text: src.text,
          data: Map<String, dynamic>.from(src.data),
        );
        _attachKeyHandler(block);
        next.add(block);
      } else {
        current.kind = src.kind;
        current.data = Map<String, dynamic>.from(src.data);
        if (!current.focus.hasFocus && current.controller.text != src.text) {
          current.controller.text = src.text;
        }
        next.add(current);
      }
    }

    for (final leftover in existing.values) {
      leftover.dispose();
    }

    _blocks
      ..clear()
      ..addAll(next);
  }

  void _ensureInitialBlock() {
    if (_blocks.isNotEmpty || !widget.canEdit) {
      return;
    }
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted || _blocks.isNotEmpty) {
        return;
      }
      final block = _EditorBlock(id: _genId(), kind: 'paragraph', text: '', data: {});
      _attachKeyHandler(block);
      setState(() => _blocks.add(block));
      _send([_insertOp(block, 0)]);
    });
  }

  void _attachKeyHandler(_EditorBlock block) {
    block.focus.onKeyEvent = (node, event) => _onKey(block, event);
  }

  KeyEventResult _onKey(_EditorBlock block, KeyEvent event) {
    if (!widget.canEdit) {
      return KeyEventResult.ignored;
    }
    if (event is! KeyDownEvent && event is! KeyRepeatEvent) {
      return KeyEventResult.ignored;
    }

    // While the slash menu is open, Enter applies the top match and Escape
    // dismisses it.
    if (_slashEntry != null && _slashBlockId == block.id) {
      if (event.logicalKey == LogicalKeyboardKey.escape) {
        _closeSlash();
        return KeyEventResult.handled;
      }
      if (event.logicalKey == LogicalKeyboardKey.enter) {
        final matches = _filteredSlashOptions();
        if (matches.isNotEmpty) {
          _applySlash(block, matches.first);
        } else {
          _closeSlash();
        }
        return KeyEventResult.handled;
      }
    }

    if (event.logicalKey == LogicalKeyboardKey.enter) {
      if (HardwareKeyboard.instance.isShiftPressed || block.kind == 'code_block') {
        return KeyEventResult.ignored; // soft newline
      }
      _handleEnter(block);
      return KeyEventResult.handled;
    }

    if (event.logicalKey == LogicalKeyboardKey.backspace) {
      final selection = block.controller.selection;
      if (selection.isCollapsed &&
          selection.baseOffset == 0 &&
          _blocks.indexOf(block) > 0) {
        _handleBackspaceMerge(block);
        return KeyEventResult.handled;
      }
    }

    // Cross-block cursor movement, so the whole document navigates like one
    // continuous page. Only for unmodified arrows; Shift/Ctrl/Alt/Meta keep
    // their native in-field behavior.
    final keyboard = HardwareKeyboard.instance;
    final plain = !keyboard.isShiftPressed &&
        !keyboard.isControlPressed &&
        !keyboard.isAltPressed &&
        !keyboard.isMetaPressed;
    if (plain) {
      final selection = block.controller.selection;
      if (event.logicalKey == LogicalKeyboardKey.arrowLeft) {
        if (selection.isCollapsed && selection.baseOffset == 0) {
          return _moveToPrevBlock(block, atEnd: true)
              ? KeyEventResult.handled
              : KeyEventResult.ignored;
        }
      } else if (event.logicalKey == LogicalKeyboardKey.arrowRight) {
        if (selection.isCollapsed &&
            selection.baseOffset == block.controller.text.length) {
          return _moveToNextBlock(block, atStart: true)
              ? KeyEventResult.handled
              : KeyEventResult.ignored;
        }
      } else if (event.logicalKey == LogicalKeyboardKey.arrowUp) {
        // Let the field move within its own wrapped lines first; if the cursor
        // didn't move, it was on the top line, so hop to the previous block.
        _hopIfCursorStuck(block, toPrevious: true);
      } else if (event.logicalKey == LogicalKeyboardKey.arrowDown) {
        _hopIfCursorStuck(block, toPrevious: false);
      }
    }

    return KeyEventResult.ignored;
  }

  void _hopIfCursorStuck(_EditorBlock block, {required bool toPrevious}) {
    final before = block.controller.selection.baseOffset;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted || !block.focus.hasFocus) {
        return;
      }
      if (block.controller.selection.baseOffset == before) {
        if (toPrevious) {
          _moveToPrevBlock(block, atEnd: true);
        } else {
          _moveToNextBlock(block, atStart: true);
        }
      }
    });
  }

  bool _moveToPrevBlock(_EditorBlock block, {required bool atEnd}) {
    final index = _blocks.indexOf(block);
    if (index <= 0) {
      return false;
    }
    final previous = _blocks[index - 1];
    previous.focus.requestFocus();
    final offset = atEnd ? previous.controller.text.length : 0;
    previous.controller.selection = TextSelection.collapsed(offset: offset);
    return true;
  }

  bool _moveToNextBlock(_EditorBlock block, {required bool atStart}) {
    final index = _blocks.indexOf(block);
    if (index < 0 || index >= _blocks.length - 1) {
      return false;
    }
    final next = _blocks[index + 1];
    next.focus.requestFocus();
    final offset = atStart ? 0 : next.controller.text.length;
    next.controller.selection = TextSelection.collapsed(offset: offset);
    return true;
  }

  void _handleEnter(_EditorBlock block) {
    final index = _blocks.indexOf(block);
    final text = block.controller.text;
    final selection = block.controller.selection;
    final offset = selection.isValid
        ? selection.baseOffset.clamp(0, text.length)
        : text.length;
    final before = text.substring(0, offset);
    final after = text.substring(offset);

    final isList =
        block.kind == 'bulleted_list' ||
        block.kind == 'numbered_list' ||
        block.kind == 'todo';

    // Enter on an empty list item exits the list.
    if (isList && text.trim().isEmpty) {
      block.kind = 'paragraph';
      block.data = {};
      block.controller.text = '';
      setState(() {});
      _send([
        {
          'type': 'update_block',
          'block_id': block.id,
          'kind': 'paragraph',
          'text': '',
          'data': <String, dynamic>{},
        },
      ]);
      return;
    }

    final newKind = isList ? block.kind : 'paragraph';
    final newData = block.kind == 'todo'
        ? <String, dynamic>{'checked': false}
        : <String, dynamic>{};

    block.controller.text = before;
    final created = _EditorBlock(
      id: _genId(),
      kind: newKind,
      text: after,
      data: Map<String, dynamic>.from(newData),
    );
    _attachKeyHandler(created);
    setState(() => _blocks.insert(index + 1, created));
    _send([
      {'type': 'update_block', 'block_id': block.id, 'text': before},
      _insertOp(created, index + 1, text: after, data: newData),
    ]);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      created.focus.requestFocus();
      created.controller.selection = const TextSelection.collapsed(offset: 0);
    });
  }

  void _handleBackspaceMerge(_EditorBlock block) {
    final index = _blocks.indexOf(block);
    final previous = _blocks[index - 1];
    final joinOffset = previous.controller.text.length;
    final merged = previous.controller.text + block.controller.text;
    previous.controller.text = merged;
    setState(() => _blocks.removeAt(index));
    final removedId = block.id;
    block.dispose();
    _send([
      {'type': 'update_block', 'block_id': previous.id, 'text': merged},
      {'type': 'delete_block', 'block_id': removedId},
    ]);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      previous.focus.requestFocus();
      previous.controller.selection = TextSelection.collapsed(offset: joinOffset);
    });
  }

  void _onChanged(_EditorBlock block, String value) {
    // Slash menu: typing "/" into an empty paragraph opens it; the text after
    // the slash filters; a space or non-slash text dismisses it.
    if (_slashEntry != null && _slashBlockId == block.id) {
      if (value.startsWith('/') && !value.contains(' ')) {
        _slashQuery = value.substring(1);
        _slashEntry!.markNeedsBuild();
        return;
      }
      _closeSlash();
    } else if (block.kind == 'paragraph' && value == '/') {
      _openSlash(block);
      return;
    }

    if (block.kind == 'paragraph') {
      final conversion = _detectMarkdown(value);
      if (conversion != null) {
        block.kind = conversion.kind;
        block.data = conversion.data;
        block.controller.value = TextEditingValue(
          text: conversion.text,
          selection: TextSelection.collapsed(offset: conversion.text.length),
        );
        setState(() {});
        _send([
          {
            'type': 'update_block',
            'block_id': block.id,
            'kind': conversion.kind,
            'text': conversion.text,
            'data': conversion.data,
          },
        ]);
        return;
      }
    }
    _scheduleSave(block);
  }

  ({String kind, String text, Map<String, dynamic> data})? _detectMarkdown(
    String value,
  ) {
    for (var level = 6; level >= 1; level--) {
      final prefix = '${'#' * level} ';
      if (value.startsWith(prefix)) {
        return (
          kind: 'heading',
          text: value.substring(prefix.length),
          data: {'level': level},
        );
      }
    }
    if (value.startsWith('- [ ] ')) {
      return (kind: 'todo', text: value.substring(6), data: {'checked': false});
    }
    if (value.startsWith('- [x] ') || value.startsWith('- [X] ')) {
      return (kind: 'todo', text: value.substring(6), data: {'checked': true});
    }
    if (value.startsWith('[] ')) {
      return (kind: 'todo', text: value.substring(3), data: {'checked': false});
    }
    if (value.startsWith('- ') || value.startsWith('* ')) {
      return (kind: 'bulleted_list', text: value.substring(2), data: {});
    }
    if (value.startsWith('> ')) {
      return (kind: 'quote', text: value.substring(2), data: {});
    }
    if (value.startsWith('```')) {
      return (kind: 'code_block', text: value.substring(3), data: {});
    }
    final numbered = RegExp(r'^(\d+)\.\s').firstMatch(value);
    if (numbered != null) {
      return (kind: 'numbered_list', text: value.substring(numbered.end), data: {});
    }
    return null;
  }

  void _scheduleSave(_EditorBlock block) {
    block.saveTimer?.cancel();
    block.saveTimer = Timer(const Duration(milliseconds: 450), () {
      _send([
        {'type': 'update_block', 'block_id': block.id, 'text': block.controller.text},
      ]);
    });
  }

  void _toggleTodo(_EditorBlock block) {
    final checked = block.data['checked'] != true;
    block.data = {...block.data, 'checked': checked};
    setState(() {});
    _send([
      {'type': 'update_block', 'block_id': block.id, 'data': block.data},
    ]);
  }

  void _changeKind(_EditorBlock block, String kind, {Map<String, dynamic>? data}) {
    block.kind = kind;
    block.data = data ?? {};
    setState(() {});
    _send([
      {'type': 'update_block', 'block_id': block.id, 'kind': kind, 'data': block.data},
    ]);
    block.focus.requestFocus();
  }

  void _deleteBlock(_EditorBlock block) {
    final index = _blocks.indexOf(block);
    setState(() => _blocks.removeAt(index));
    final removedId = block.id;
    block.dispose();
    _send([
      {'type': 'delete_block', 'block_id': removedId},
    ]);
    if (_blocks.isNotEmpty) {
      final neighbor = _blocks[index > 0 ? index - 1 : 0];
      WidgetsBinding.instance.addPostFrameCallback(
        (_) => neighbor.focus.requestFocus(),
      );
    }
  }

  void _appendBlock() {
    final block = _EditorBlock(id: _genId(), kind: 'paragraph', text: '', data: {});
    _attachKeyHandler(block);
    final index = _blocks.length;
    setState(() => _blocks.add(block));
    _send([_insertOp(block, index)]);
    WidgetsBinding.instance.addPostFrameCallback((_) => block.focus.requestFocus());
  }

  /// Tapping the empty area below the last block: focus a trailing empty
  /// paragraph if one exists, otherwise append one. No visible button.
  void _appendOrFocusLast() {
    if (_blocks.isNotEmpty) {
      final last = _blocks.last;
      if (last.kind == 'paragraph' && last.controller.text.isEmpty) {
        last.focus.requestFocus();
        return;
      }
    }
    _appendBlock();
  }

  void _insertBlockBelow(_EditorBlock block) {
    final index = _blocks.indexOf(block);
    final created = _EditorBlock(id: _genId(), kind: 'paragraph', text: '', data: {});
    _attachKeyHandler(created);
    setState(() => _blocks.insert(index + 1, created));
    _send([_insertOp(created, index + 1)]);
    WidgetsBinding.instance.addPostFrameCallback((_) => created.focus.requestFocus());
  }

  // --- Slash menu -----------------------------------------------------------

  List<_SlashOption> _filteredSlashOptions() {
    final query = _slashQuery.toLowerCase();
    if (query.isEmpty) {
      return _slashOptions;
    }
    return _slashOptions
        .where((option) => option.label.toLowerCase().contains(query))
        .toList();
  }

  void _openSlash(_EditorBlock block) {
    _slashEntry?.remove();
    _slashBlockId = block.id;
    _slashQuery = '';
    _slashEntry = OverlayEntry(builder: _buildSlashOverlay);
    Overlay.of(context).insert(_slashEntry!);
  }

  void _closeSlash() {
    _slashEntry?.remove();
    _slashEntry = null;
    _slashBlockId = null;
    _slashQuery = '';
  }

  void _applySlash(_EditorBlock block, _SlashOption option) {
    _closeSlash();
    block.kind = option.kind;
    block.data = option.data == null
        ? <String, dynamic>{}
        : Map<String, dynamic>.from(option.data!);
    block.controller.value = const TextEditingValue(
      selection: TextSelection.collapsed(offset: 0),
    );
    setState(() {});
    _send([
      {
        'type': 'update_block',
        'block_id': block.id,
        'kind': block.kind,
        'text': '',
        'data': block.data,
      },
    ]);
    WidgetsBinding.instance.addPostFrameCallback((_) => block.focus.requestFocus());
  }

  Widget _buildSlashOverlay(BuildContext context) {
    final block = _blocks
        .where((item) => item.id == _slashBlockId)
        .firstOrNull;
    if (block == null) {
      return const SizedBox.shrink();
    }
    final options = _filteredSlashOptions();

    return Stack(
      children: [
        // Tap-outside barrier.
        Positioned.fill(
          child: GestureDetector(
            behavior: HitTestBehavior.translucent,
            onTap: _closeSlash,
          ),
        ),
        CompositedTransformFollower(
          link: block.link,
          showWhenUnlinked: false,
          targetAnchor: Alignment.bottomLeft,
          followerAnchor: Alignment.topLeft,
          offset: const Offset(0, 6),
          child: Align(
            alignment: Alignment.topLeft,
            child: Material(
              elevation: 8,
              borderRadius: BorderRadius.circular(8),
              child: ConstrainedBox(
                constraints: const BoxConstraints(maxWidth: 260, maxHeight: 320),
                child: options.isEmpty
                    ? const Padding(
                        padding: EdgeInsets.all(16),
                        child: Text('No matching block'),
                      )
                    : ListView(
                        padding: const EdgeInsets.symmetric(vertical: 4),
                        shrinkWrap: true,
                        children: [
                          for (final option in options)
                            ListTile(
                              dense: true,
                              leading: Icon(option.icon, size: 20),
                              title: Text(option.label),
                              onTap: () => _applySlash(block, option),
                            ),
                        ],
                      ),
              ),
            ),
          ),
        ),
      ],
    );
  }

  Map<String, dynamic> _insertOp(
    _EditorBlock block,
    int index, {
    String text = '',
    Map<String, dynamic> data = const {},
  }) {
    return {
      'type': 'insert_block',
      'parent_id': widget.rootBlockId,
      'index': index,
      'block': {
        'id': block.id,
        'type': block.kind,
        'text': text,
        'data': data,
        'children': <String>[],
      },
    };
  }

  void _send(List<Map<String, dynamic>> operations) {
    _sendChain = _sendChain
        .then((_) => widget.onApplyOperations(operations))
        .catchError((_) {});
  }

  String _genId() => 'block_${DateTime.now().microsecondsSinceEpoch}_${_idCounter++}';

  int _ordinal(int index) {
    var ordinal = 1;
    for (var j = index - 1; j >= 0; j--) {
      if (_blocks[j].kind == 'numbered_list') {
        ordinal++;
      } else {
        break;
      }
    }
    return ordinal;
  }

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (var i = 0; i < _blocks.length; i++) _buildBlock(context, _blocks[i], i),
        // Invisible click target below the document: tapping it continues
        // writing, the way clicking under the text does in Word/Typora.
        if (widget.canEdit)
          GestureDetector(
            behavior: HitTestBehavior.translucent,
            onTap: _appendOrFocusLast,
            child: const SizedBox(height: 96, width: double.infinity),
          ),
      ],
    );
  }

  Widget _buildBlock(BuildContext context, _EditorBlock block, int index) {
    Widget content;
    switch (block.kind) {
      case 'bulleted_list':
        content = Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const Padding(
              padding: EdgeInsets.only(top: 6, right: 8),
              child: Text('•', style: TextStyle(fontSize: 16)),
            ),
            Expanded(child: _blockField(block, hint: 'List item')),
          ],
        );
      case 'numbered_list':
        content = Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Padding(
              padding: const EdgeInsets.only(top: 6, right: 8),
              child: Text('${_ordinal(index)}.', style: const TextStyle(fontSize: 14)),
            ),
            Expanded(child: _blockField(block, hint: 'List item')),
          ],
        );
      case 'todo':
        final checked = block.data['checked'] == true;
        content = Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            SizedBox(
              width: 28,
              height: 32,
              child: Checkbox(
                value: checked,
                visualDensity: VisualDensity.compact,
                onChanged: widget.canEdit ? (_) => _toggleTodo(block) : null,
              ),
            ),
            const SizedBox(width: 4),
            Expanded(child: _blockField(block, hint: 'To-do')),
          ],
        );
      case 'quote':
        content = Container(
          decoration: const BoxDecoration(
            border: Border(left: BorderSide(color: Color(0xFFCBD5E1), width: 3)),
          ),
          padding: const EdgeInsets.only(left: 12),
          child: _blockField(block, hint: 'Quote'),
        );
      case 'code_block':
        content = Container(
          decoration: BoxDecoration(
            color: const Color(0xFFF1F5F9),
            borderRadius: BorderRadius.circular(6),
          ),
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
          child: _blockField(block, hint: 'Code'),
        );
      case 'heading':
        content = _blockField(block, hint: 'Heading');
      default:
        content = _blockField(
          block,
          hint: index == 0
              ? "Write, or press '/' for commands"
              : null,
        );
    }

    final hovered = _hoveredId == block.id;

    return MouseRegion(
      onEnter: (_) => setState(() => _hoveredId = block.id),
      onExit: (_) {
        if (_hoveredId == block.id) {
          setState(() => _hoveredId = null);
        }
      },
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: 3),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            // Reserved gutter keeps the text aligned; the handle only appears
            // on hover, so the resting page shows no block chrome.
            SizedBox(
              width: 28,
              child: widget.canEdit && hovered
                  ? Align(alignment: Alignment.topCenter, child: _blockMenu(block))
                  : null,
            ),
            Expanded(
              child: CompositedTransformTarget(link: block.link, child: content),
            ),
          ],
        ),
      ),
    );
  }

  Widget _blockField(_EditorBlock block, {String? hint}) {
    final theme = Theme.of(context);
    TextStyle? style;
    switch (block.kind) {
      case 'heading':
        final level = (block.data['level'] as num?)?.toInt() ?? 1;
        style = (level <= 1
                ? theme.textTheme.headlineSmall
                : level == 2
                ? theme.textTheme.titleLarge
                : theme.textTheme.titleMedium)
            ?.copyWith(fontWeight: FontWeight.w700);
      case 'code_block':
        style = const TextStyle(
          fontFamily: 'monospace',
          fontSize: 13.5,
          height: 1.5,
        );
      case 'quote':
        style = theme.textTheme.bodyLarge?.copyWith(
          fontStyle: FontStyle.italic,
          color: const Color(0xFF475569),
        );
      case 'todo':
        final checked = block.data['checked'] == true;
        style = theme.textTheme.bodyLarge?.copyWith(
          decoration: checked ? TextDecoration.lineThrough : null,
          color: checked ? const Color(0xFF94A3B8) : null,
        );
      default:
        style = theme.textTheme.bodyLarge?.copyWith(height: 1.5);
    }

    return TextField(
      controller: block.controller,
      focusNode: block.focus,
      readOnly: !widget.canEdit,
      style: style,
      maxLines: null,
      keyboardType: TextInputType.multiline,
      textInputAction: TextInputAction.newline,
      onChanged: (value) => _onChanged(block, value),
      decoration: InputDecoration(
        isDense: true,
        border: InputBorder.none,
        hintText: hint,
        contentPadding: const EdgeInsets.symmetric(vertical: 4),
      ),
    );
  }

  Widget _blockMenu(_EditorBlock block) {
    return PopupMenuButton<String>(
      tooltip: 'Block options',
      icon: const Icon(Icons.drag_indicator, size: 18, color: Color(0xFFB4BCC8)),
      iconSize: 18,
      padding: EdgeInsets.zero,
      constraints: const BoxConstraints(),
      splashRadius: 16,
      onSelected: (value) => _onMenu(block, value),
      itemBuilder: (_) => const [
        PopupMenuItem(value: 'add_below', child: Text('Add block below')),
        PopupMenuDivider(),
        PopupMenuItem(value: 'paragraph', child: Text('Turn into Text')),
        PopupMenuItem(value: 'heading1', child: Text('Turn into Heading 1')),
        PopupMenuItem(value: 'heading2', child: Text('Turn into Heading 2')),
        PopupMenuItem(value: 'heading3', child: Text('Turn into Heading 3')),
        PopupMenuItem(value: 'bulleted_list', child: Text('Turn into Bulleted list')),
        PopupMenuItem(value: 'numbered_list', child: Text('Turn into Numbered list')),
        PopupMenuItem(value: 'todo', child: Text('Turn into To-do')),
        PopupMenuItem(value: 'quote', child: Text('Turn into Quote')),
        PopupMenuItem(value: 'code_block', child: Text('Turn into Code')),
        PopupMenuDivider(),
        PopupMenuItem(value: 'delete', child: Text('Delete')),
      ],
    );
  }

  void _onMenu(_EditorBlock block, String value) {
    switch (value) {
      case 'add_below':
        _insertBlockBelow(block);
      case 'delete':
        _deleteBlock(block);
      case 'heading1':
        _changeKind(block, 'heading', data: {'level': 1});
      case 'heading2':
        _changeKind(block, 'heading', data: {'level': 2});
      case 'heading3':
        _changeKind(block, 'heading', data: {'level': 3});
      case 'todo':
        _changeKind(block, 'todo', data: {'checked': false});
      default:
        _changeKind(block, value);
    }
  }
}
