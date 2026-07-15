// Leaf + panel widgets for the Mica client: workspace selector, list rows,
// block row, document row, presence bar, update checker, small helpers.
// `part of main.dart`. Extracted 2026-07 for navigability.
part of '../main.dart';

/// Workspace switcher: the workspaces of the ONE world the app is connected
/// to — a server (with its account, or a sign-in row when signed out) or this
/// device. Which world that is, is chosen on the account tile at the bottom of
/// the sidebar, where `本地模式` and each server sit in one menu as the same kind
/// of choice.
///
/// Listing both worlds here at once was tried (AFFiNE does exactly that: `local`
/// is a reserved server id in one flat list) and rejected on sight: a local and
/// a cloud workspace look alike but are not — one syncs and belongs to an
/// account, the other is a file on this disk that no server has ever seen —
/// and tiling them invites treating them as interchangeable.
///
/// Row actions dispatch on the ROW's entry, and create/import ask where they
/// should land, so neither depends on this list being filtered.
class _WorkspaceSelector extends StatefulWidget {
  const _WorkspaceSelector({
    required this.entries,
    required this.activeIsLocal,
    required this.selectedRef,
    required this.cloudEmail,
    required this.onSignIn,
    required this.onSelect,
    required this.onRename,
    required this.onDelete,
    required this.onExport,
    required this.onCreate,
    required this.onImport,
    required this.onImportFilesInto,
    required this.onImportFolderInto,
    required this.onMigrate,
    required this.onDetach,
  });

  /// Every workspace of both worlds; the menu shows the connected one's.
  final List<WorkspaceEntry> entries;

  /// The connected world. Picks which of [entries] the menu lists, and which
  /// icon the collapsed button shows.
  final bool activeIsLocal;

  final WorkspaceRef? selectedRef;

  /// Signed-in account email, or null when signed out (shows the sign-in row).
  final String? cloudEmail;
  final VoidCallback? onSignIn;
  final Future<void> Function(WorkspaceEntry entry) onSelect;
  final void Function(WorkspaceEntry entry) onRename;
  final void Function(WorkspaceEntry entry) onDelete;
  final void Function(WorkspaceEntry entry) onExport;
  final VoidCallback onCreate;
  final void Function(bool notion) onImport;
  final void Function(WorkspaceEntry entry) onImportFilesInto;
  final void Function(WorkspaceEntry entry) onImportFolderInto;

  /// P3f row actions: upload a local row to the cloud / detach a cloud row to
  /// a local copy. Null hides the item.
  final void Function(WorkspaceEntry entry)? onMigrate;
  final void Function(WorkspaceEntry entry)? onDetach;

  @override
  State<_WorkspaceSelector> createState() => _WorkspaceSelectorState();
}

class _WorkspaceSelectorState extends State<_WorkspaceSelector> {
  final MenuController _menu = MenuController();

  WorkspaceEntry? get _selectedEntry {
    final ref = widget.selectedRef;
    if (ref == null) return null;
    for (final e in widget.entries) {
      if (e.ref == ref) return e;
    }
    return null;
  }

  @override
  Widget build(BuildContext context) {
    final cloud = [
      for (final e in widget.entries)
        if (!e.isLocal) e,
    ];
    final locals = [
      for (final e in widget.entries)
        if (e.isLocal) e,
    ];
    return MenuAnchor(
      controller: _menu,
      style: const MenuStyle(
        minimumSize: WidgetStatePropertyAll(Size(300, 0)),
        padding: WidgetStatePropertyAll(EdgeInsets.symmetric(vertical: 6)),
      ),
      menuChildren: [
        // Only the connected world's workspaces. Listing both at once was
        // tried and rejected: a local workspace and a cloud one look alike but
        // are not — one syncs and has an account, the other is a file on this
        // disk — and tiling them invites you to treat them as interchangeable.
        // Which world you are connected to is chosen on the account tile.
        if (!widget.activeIsLocal &&
            widget.cloudEmail == null &&
            widget.onSignIn != null)
          _signInRow()
        else
          for (final e in (widget.activeIsLocal ? locals : cloud)) _row(e),
        const Divider(height: 8),
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
              _importChoice(
                Icons.folder_zip_outlined,
                'From ZIP (Mica export)',
                notion: false,
              ),
              _importChoice(
                Icons.cloud_download_outlined,
                'From Notion (Markdown & CSV ZIP)',
                notion: true,
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
        final label = _selectedEntry?.workspace.name ?? 'Select workspace';
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
                // World-aware icon so the collapsed switcher shows which world
                // you're in (cloud vs this device).
                Icon(
                  widget.activeIsLocal
                      ? Icons.computer_outlined
                      : Icons.cloud_outlined,
                  size: 20,
                  color: const Color(0xFF475569),
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

  /// Signed out: the cloud section is one sign-in row (its workspaces appear
  /// after signing in — AFFiNE semantics: signed-out hides, offline keeps).
  Widget _signInRow() {
    return SizedBox(
      width: 320,
      child: InkWell(
        onTap: () {
          _menu.close();
          widget.onSignIn?.call();
        },
        child: const Padding(
          padding: EdgeInsets.symmetric(horizontal: 12, vertical: 10),
          child: Row(
            children: [
              Icon(Icons.login, size: 18, color: Color(0xFF2563EB)),
              SizedBox(width: 10),
              Text(
                '登录云端…',
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

  Widget _row(WorkspaceEntry entry) {
    final workspace = entry.workspace;
    final selected = entry.ref == widget.selectedRef;
    return SizedBox(
      width: 320,
      child: Row(
        children: [
          Expanded(
            child: InkWell(
              onTap: () {
                _menu.close();
                widget.onSelect(entry);
              },
              child: Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: 12,
                  vertical: 10,
                ),
                child: Row(
                  children: [
                    Icon(
                      selected
                          ? Icons.check
                          : entry.isLocal
                          ? Icons.computer_outlined
                          : Icons.cloud_outlined,
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
                () => widget.onRename(entry),
              ),
              _wsAction(
                Icons.folder_zip_outlined,
                'Export (ZIP)',
                () => widget.onExport(entry),
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
                    () => widget.onImportFilesInto(entry),
                  ),
                  _wsAction(
                    Icons.drive_folder_upload_outlined,
                    'Folder',
                    () => widget.onImportFolderInto(entry),
                  ),
                ],
                child: const Text('Import'),
              ),
              if (entry.isLocal && widget.onMigrate != null)
                _wsAction(
                  Icons.cloud_upload_outlined,
                  '上云…',
                  () => widget.onMigrate!(entry),
                ),
              if (!entry.isLocal && widget.onDetach != null)
                _wsAction(
                  Icons.computer_outlined,
                  '转为本地副本…',
                  () => widget.onDetach!(entry),
                ),
              _wsAction(
                Icons.delete_outline,
                'Delete',
                () => widget.onDelete(entry),
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
      leadingIcon: Icon(
        icon,
        size: 18,
        color: color ?? const Color(0xFF475569),
      ),
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

  /// Both submenu entries share the tree-import core; the Notion one forces
  /// Notion adaptation (ID-suffix stripping, folder↔page matching).
  Widget _importChoice(IconData icon, String label, {required bool notion}) {
    return MenuItemButton(
      leadingIcon: Icon(icon, size: 18, color: const Color(0xFF475569)),
      onPressed: () {
        _menu.close();
        widget.onImport(notion);
      },
      child: Text(label, style: const TextStyle(color: Color(0xFF475569))),
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
          style: const TextStyle(fontFamily: kMonoFont),
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

class DocumentListItem extends StatefulWidget {
  const DocumentListItem({
    required this.view,
    required this.depth,
    required this.hasChildren,
    required this.revealToggle,
    required this.isCollapsed,
    required this.isSelected,
    required this.canEdit,
    required this.isRenaming,
    required this.onToggle,
    required this.onPressed,
    required this.onCreateChild,
    required this.onCreateChildFolder,
    this.onExportFolder,
    required this.onRename,
    required this.onRenameSubmit,
    required this.onRenameCancel,
    required this.onDelete,
    super.key,
  });

  final DocumentView view;
  final int depth;
  final bool hasChildren;

  /// Pointer is over the sidebar: parents' expand toggles fade in.
  final bool revealToggle;
  final bool isCollapsed;
  final bool isSelected;
  final bool canEdit;

  /// This row's name is in inline-edit mode: render a focused TextField instead
  /// of the name Text, and hide the hover actions.
  final bool isRenaming;
  final VoidCallback onToggle;
  final VoidCallback onPressed;
  final VoidCallback onCreateChild;
  final VoidCallback onCreateChildFolder;

  /// Export this folder's subtree as a ZIP. Null hides the entry — local
  /// workspaces have no server to build the archive.
  final VoidCallback? onExportFolder;
  final VoidCallback onRename;

  /// Commit the inline-edited name (Enter or blur); cancel on Esc.
  final ValueChanged<String> onRenameSubmit;
  final VoidCallback onRenameCancel;
  final VoidCallback onDelete;

  bool get _isFolder => view.objectType == 'folder';

  @override
  State<DocumentListItem> createState() => _DocumentListItemState();
}

class _DocumentListItemState extends State<DocumentListItem> {
  // Per-row hover (Notion / Feishu style): the action affordances live off the
  // row until the pointer is on THIS row, so page names get the full width by
  // default and only the row you point at compresses to show its controls.
  bool _hovered = false;

  // ── Inline name editing ─────────────────────────────────────────────────────
  // Live only while `widget.isRenaming`. Enter or blur (click-away) commits; Esc
  // cancels. `_renameHandled` makes commit/cancel fire exactly once per edit
  // (the disposal below also drops focus, which must not re-commit).
  TextEditingController? _renameCtrl;
  FocusNode? _renameFocus;
  bool _renameHandled = false;

  @override
  void initState() {
    super.initState();
    if (widget.isRenaming) _enterRename();
  }

  @override
  void didUpdateWidget(DocumentListItem old) {
    super.didUpdateWidget(old);
    if (widget.isRenaming && !old.isRenaming) {
      _enterRename();
    } else if (!widget.isRenaming && old.isRenaming) {
      _exitRename();
    }
  }

  @override
  void dispose() {
    _exitRename();
    super.dispose();
  }

  void _enterRename() {
    _renameHandled = false;
    final name = widget.view.name;
    _renameCtrl = TextEditingController(text: name)
      ..selection = TextSelection(baseOffset: 0, extentOffset: name.length);
    _renameFocus = FocusNode()..addListener(_onRenameFocusChange);
    WidgetsBinding.instance.addPostFrameCallback(
      (_) => _renameFocus?.requestFocus(),
    );
  }

  void _exitRename() {
    _renameFocus?.removeListener(_onRenameFocusChange);
    _renameFocus?.dispose();
    _renameCtrl?.dispose();
    _renameFocus = null;
    _renameCtrl = null;
  }

  void _onRenameFocusChange() {
    if (_renameFocus?.hasFocus == false) _commitRename(); // blur = commit
  }

  void _commitRename() {
    if (_renameHandled) return;
    _renameHandled = true;
    widget.onRenameSubmit(_renameCtrl?.text ?? widget.view.name);
  }

  void _cancelRename() {
    if (_renameHandled) return;
    _renameHandled = true;
    widget.onRenameCancel();
  }

  /// The row's context menu — one place for rename/delete/new-child/collapse,
  /// opened from the `⋯` button AND from a right-click anywhere on the row
  /// (parity with Feishu/Notion). Only the capabilities Mica actually has; no
  /// invented copy/move/favorite items.
  Future<void> _openMenu(BuildContext anchorContext) async {
    final box = anchorContext.findRenderObject() as RenderBox?;
    final overlay =
        Overlay.of(anchorContext).context.findRenderObject() as RenderBox?;
    if (box == null || overlay == null) return;
    final topLeft = box.localToGlobal(Offset.zero, ancestor: overlay);
    final position = RelativeRect.fromLTRB(
      topLeft.dx,
      topLeft.dy + box.size.height,
      overlay.size.width - topLeft.dx,
      0,
    );
    await _showMenuAt(position);
  }

  Future<void> _openMenuAtGlobal(Offset globalPosition) async {
    final overlay =
        Overlay.of(context).context.findRenderObject() as RenderBox?;
    if (overlay == null) return;
    final local = overlay.globalToLocal(globalPosition);
    await _showMenuAt(
      RelativeRect.fromLTRB(
        local.dx,
        local.dy,
        overlay.size.width - local.dx,
        0,
      ),
    );
  }

  Future<void> _showMenuAt(RelativeRect position) async {
    final selected = await showMenu<String>(
      context: context,
      position: position,
      items: [
        // A page is a leaf: only folders can hold children, so the two
        // "new child" entries appear on folder rows only.
        if (widget._isFolder) ...[
          const PopupMenuItem(
            value: 'child',
            child: _MenuRow(icon: Icons.add, label: '新建子页面'),
          ),
          const PopupMenuItem(
            value: 'childFolder',
            child: _MenuRow(
              icon: Icons.create_new_folder_outlined,
              label: '新建子文件夹',
            ),
          ),
        ],
        const PopupMenuItem(
          value: 'rename',
          child: _MenuRow(icon: Icons.edit_outlined, label: '重命名'),
        ),
        if (widget.hasChildren)
          PopupMenuItem(
            value: 'toggle',
            child: _MenuRow(
              icon: widget.isCollapsed ? Icons.unfold_more : Icons.unfold_less,
              label: widget.isCollapsed ? '展开子项' : '收起子项',
            ),
          ),
        // Folder subtree -> ZIP, same as a page or a workspace. Every level
        // exports the same way and carries its images; see onExportFolder.
        if (widget._isFolder && widget.onExportFolder != null)
          const PopupMenuItem(
            value: 'export',
            child: _MenuRow(
              icon: Icons.folder_zip_outlined,
              label: '导出(ZIP,含图片)',
            ),
          ),
        const PopupMenuDivider(),
        const PopupMenuItem(
          value: 'delete',
          child: _MenuRow(
            icon: Icons.delete_outline,
            label: '删除',
            danger: true,
          ),
        ),
      ],
    );
    if (!mounted) return;
    switch (selected) {
      case 'child':
        widget.onCreateChild();
      case 'childFolder':
        widget.onCreateChildFolder();
      case 'rename':
        widget.onRename();
      case 'toggle':
        widget.onToggle();
      case 'export':
        widget.onExportFolder?.call();
      case 'delete':
        widget.onDelete();
    }
  }

  @override
  Widget build(BuildContext context) {
    final w = widget;
    // Show the controls when the row is hovered; a right-click works regardless.
    final showActions = w.canEdit && _hovered;
    return MouseRegion(
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: Material(
        color: w.isSelected ? const Color(0xFFEFF6FF) : Colors.transparent,
        borderRadius: BorderRadius.circular(6),
        child: InkWell(
          borderRadius: BorderRadius.circular(6),
          // A folder has no content to open — clicking it expands/collapses in
          // place (file-manager style). Documents open in the editor. While
          // renaming, taps stay inside the inline field (click-away commits).
          onTap: w.isRenaming ? null : (w._isFolder ? w.onToggle : w.onPressed),
          onSecondaryTapDown: w.canEdit
              ? (d) => _openMenuAtGlobal(d.globalPosition)
              : null,
          child: ConstrainedBox(
            constraints: const BoxConstraints(minHeight: 38),
            child: Padding(
              padding: EdgeInsets.only(left: 2 + (w.depth * 16), right: 4),
              child: Row(
                children: [
                  // AppFlowy-style expand column: always present so every page
                  // icon shares one column; the toggle is invisible until the
                  // pointer enters the sidebar (and only parents have one).
                  SizedBox(
                    width: 18,
                    height: 30,
                    child: w.hasChildren
                        ? Opacity(
                            opacity: (w.revealToggle || _hovered) ? 1.0 : 0.0,
                            child: IconButton(
                              tooltip: w.isCollapsed ? 'Expand' : 'Collapse',
                              onPressed: w.onToggle,
                              padding: EdgeInsets.zero,
                              iconSize: 18,
                              icon: Icon(
                                w.isCollapsed
                                    ? Icons.chevron_right
                                    : Icons.expand_more,
                              ),
                            ),
                          )
                        : const SizedBox.shrink(),
                  ),
                  Icon(
                    w._isFolder
                        ? (w.isCollapsed
                              ? Icons.folder_outlined
                              : Icons.folder_open_outlined)
                        : Icons.description_outlined,
                    size: 18,
                    color: w.isSelected
                        ? const Color(0xFF2563EB)
                        : const Color(0xFF64748B),
                  ),
                  const SizedBox(width: 6),
                  Expanded(
                    child: (w.isRenaming && _renameCtrl != null)
                        ? CallbackShortcuts(
                            bindings: {
                              const SingleActivator(LogicalKeyboardKey.escape):
                                  _cancelRename,
                            },
                            child: TextField(
                              controller: _renameCtrl,
                              focusNode: _renameFocus,
                              maxLines: 1,
                              style: Theme.of(context).textTheme.bodyMedium,
                              decoration: const InputDecoration(
                                isDense: true,
                                contentPadding: EdgeInsets.symmetric(
                                  horizontal: 6,
                                  vertical: 5,
                                ),
                                border: OutlineInputBorder(),
                              ),
                              onSubmitted: (_) => _commitRename(),
                            ),
                          )
                        : Text(
                            w.view.name,
                            overflow: TextOverflow.ellipsis,
                            style: Theme.of(context).textTheme.bodyMedium
                                ?.copyWith(
                                  fontWeight: w.isSelected
                                      ? FontWeight.w600
                                      : FontWeight.w400,
                                ),
                          ),
                  ),
                  // Two compact affordances, hover-only (Feishu pattern): `⋯`
                  // opens the full menu, `+` quick-adds a child. The `+` shows
                  // only on folders — a page is a leaf (containers = folders).
                  if (showActions && !w.isRenaming) ...[
                    SizedBox(
                      width: 28,
                      height: 30,
                      child: Builder(
                        builder: (btnCtx) => IconButton(
                          tooltip: '删除、重命名等',
                          onPressed: () => _openMenu(btnCtx),
                          padding: EdgeInsets.zero,
                          iconSize: 17,
                          icon: const Icon(Icons.more_horiz),
                        ),
                      ),
                    ),
                    // Folders hold children: quick-add a child page (`+`) and a
                    // child folder (📁). New items drop straight into inline
                    // rename, so this is: click → type name → Enter.
                    if (w._isFolder) ...[
                      SizedBox(
                        width: 28,
                        height: 30,
                        child: IconButton(
                          tooltip: '新建子页面',
                          onPressed: w.onCreateChild,
                          padding: EdgeInsets.zero,
                          iconSize: 17,
                          icon: const Icon(Icons.add),
                        ),
                      ),
                      SizedBox(
                        width: 28,
                        height: 30,
                        child: IconButton(
                          tooltip: '新建子文件夹',
                          onPressed: w.onCreateChildFolder,
                          padding: EdgeInsets.zero,
                          iconSize: 17,
                          icon: const Icon(Icons.create_new_folder_outlined),
                        ),
                      ),
                    ],
                  ],
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// A leading-icon + label row for the page context menu.
class _MenuRow extends StatelessWidget {
  const _MenuRow({
    required this.icon,
    required this.label,
    this.danger = false,
  });
  final IconData icon;
  final String label;
  final bool danger;

  @override
  Widget build(BuildContext context) {
    final color = danger ? const Color(0xFFDC2626) : null;
    return Row(
      children: [
        Icon(icon, size: 18, color: color),
        const SizedBox(width: 12),
        Text(label, style: TextStyle(color: color)),
      ],
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
    // Solo → nothing (the caller also skips rendering the row); an "Only you"
    // line here just padded the title↔body gap.
    if (presence.isEmpty) {
      return const SizedBox.shrink();
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

/// About-dialog control: "check for updates", and when a newer GitHub release
/// exists, download + launch the installer (which force-closes and relaunches
/// Mica). Only shown where [updateSupported] is true (the Windows installer).
class UpdateChecker extends StatefulWidget {
  const UpdateChecker({super.key});

  @override
  State<UpdateChecker> createState() => _UpdateCheckerState();
}

enum _UpdateStage { idle, checking, upToDate, available, downloading, error }

class _UpdateCheckerState extends State<UpdateChecker> {
  _UpdateStage _stage = _UpdateStage.idle;
  UpdateInfo? _info;
  double _progress = 0;
  String? _error;

  Future<void> _check() async {
    setState(() {
      _stage = _UpdateStage.checking;
      _error = null;
    });
    try {
      final info = await checkForUpdate(kAppVersion);
      if (!mounted) return;
      setState(() {
        _info = info;
        _stage = info == null ? _UpdateStage.upToDate : _UpdateStage.available;
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _stage = _UpdateStage.error;
        _error = e.toString();
      });
    }
  }

  Future<void> _update() async {
    final info = _info;
    if (info == null) return;
    final ok = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text('更新到 v${info.version}'),
        content: const Text('将下载安装包，然后自动关闭并重启 Mica 完成更新。是否继续?'),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(ctx).pop(false),
            child: const Text('取消'),
          ),
          FilledButton(
            onPressed: () => Navigator.of(ctx).pop(true),
            child: const Text('更新并重启'),
          ),
        ],
      ),
    );
    if (ok != true || !mounted) return;
    setState(() {
      _stage = _UpdateStage.downloading;
      _progress = 0;
    });
    try {
      // On success this calls exit(0) (the installer takes over) and never returns.
      await downloadAndApplyUpdate(
        info,
        onProgress: (p) {
          if (mounted) setState(() => _progress = p);
        },
      );
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _stage = _UpdateStage.error;
        _error = e.toString();
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    switch (_stage) {
      case _UpdateStage.idle:
        return Align(
          alignment: Alignment.centerLeft,
          child: OutlinedButton.icon(
            icon: const Icon(Icons.system_update_alt, size: 18),
            label: const Text('检查更新'),
            onPressed: _check,
          ),
        );
      case _UpdateStage.checking:
        return const Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            SizedBox(
              width: 16,
              height: 16,
              child: CircularProgressIndicator(strokeWidth: 2),
            ),
            SizedBox(width: 10),
            Text('检查中…'),
          ],
        );
      case _UpdateStage.upToDate:
        return Row(
          children: [
            const Icon(
              Icons.check_circle_outline,
              size: 18,
              color: Color(0xFF16A34A),
            ),
            const SizedBox(width: 8),
            const Text('已是最新版本 (v$kAppVersion)'),
            const Spacer(),
            TextButton(onPressed: _check, child: const Text('重新检查')),
          ],
        );
      case _UpdateStage.available:
        final info = _info!;
        return Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                const Icon(
                  Icons.new_releases_outlined,
                  size: 18,
                  color: Color(0xFF2563EB),
                ),
                const SizedBox(width: 8),
                Expanded(
                  child: Text('发现新版本 v${info.version}（当前 v$kAppVersion）'),
                ),
              ],
            ),
            const SizedBox(height: 10),
            Align(
              alignment: Alignment.centerLeft,
              child: FilledButton.icon(
                icon: const Icon(Icons.download, size: 18),
                label: const Text('立即更新'),
                onPressed: _update,
              ),
            ),
          ],
        );
      case _UpdateStage.downloading:
        return Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text('下载中 ${(_progress * 100).round()}%…'),
            const SizedBox(height: 8),
            LinearProgressIndicator(value: _progress > 0 ? _progress : null),
            const SizedBox(height: 6),
            const Text(
              '完成后会自动关闭并重启 Mica。',
              style: TextStyle(fontSize: 12, color: Color(0xFF64748B)),
            ),
          ],
        );
      case _UpdateStage.error:
        return Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                const Icon(
                  Icons.error_outline,
                  size: 18,
                  color: Color(0xFFDC2626),
                ),
                const SizedBox(width: 8),
                Expanded(
                  child: Text(
                    '操作失败：${_error ?? ''}',
                    style: const TextStyle(fontSize: 13),
                  ),
                ),
              ],
            ),
            const SizedBox(height: 8),
            Align(
              alignment: Alignment.centerLeft,
              child: TextButton(onPressed: _check, child: const Text('重试')),
            ),
          ],
        );
    }
  }
}
