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
    required this.onReorder,
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

  /// Persist a new order for the connected world's workspaces (the whole list
  /// in the intended order). Reordering only ever happens within one world —
  /// cloud and local hold separate position spaces.
  final void Function(List<WorkspaceEntry> ordered) onReorder;

  @override
  State<_WorkspaceSelector> createState() => _WorkspaceSelectorState();
}

class _WorkspaceSelectorState extends State<_WorkspaceSelector> {
  final MenuController _menu = MenuController();

  /// True while a workspace row is being dragged — gates the before/after drop
  /// slots so they never intercept taps when not reordering.
  bool _dragging = false;

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
          for (final e in (widget.activeIsLocal ? locals : cloud))
            _row(e, widget.activeIsLocal ? locals : cloud),
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
                context.l10n.workspaceRowImportFromZip,
                notion: false,
              ),
              _importChoice(
                Icons.cloud_download_outlined,
                context.l10n.workspaceRowImportFromNotion,
                notion: true,
              ),
            ],
            child: Text(
              context.l10n.workspaceRowImportWorkspace,
              style: const TextStyle(
                color: Color(0xFF475569),
                fontWeight: FontWeight.w600,
              ),
            ),
          ),
        ),
      ],
      builder: (context, controller, child) {
        final label =
            _selectedEntry?.workspace.name ??
            context.l10n.workspaceRowSelectWorkspace;
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
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
          child: Row(
            children: [
              const Icon(Icons.login, size: 18, color: Color(0xFF2563EB)),
              const SizedBox(width: 10),
              Text(
                context.l10n.workspaceRowSignInCloud,
                style: const TextStyle(
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

  /// Drop [dragged] just before/after [target] within its world's list [world]
  /// and persist the whole new order. Same drag-to-reorder model as the
  /// document tree — only within the connected world (the menu lists one world).
  void _reorderWs(
    WorkspaceEntry dragged,
    WorkspaceEntry target,
    List<WorkspaceEntry> world, {
    required bool before,
  }) {
    if (dragged.ref == target.ref) return;
    final next = [
      for (final e in world)
        if (e.ref != dragged.ref) e,
    ];
    final ti = next.indexWhere((e) => e.ref == target.ref);
    if (ti < 0) return;
    next.insert(before ? ti : ti + 1, dragged);
    setState(() => _dragging = false);
    widget.onReorder(next);
  }

  /// Wrap a workspace row so it drags to reorder (mirrors the doc tree's
  /// `_draggableTreeRow`): press-and-move to drag; a motionless tap still
  /// selects. Top half = drop-before slot, bottom half = drop-after.
  Widget _wsDraggableRow(
    WorkspaceEntry entry,
    List<WorkspaceEntry> world,
    Widget row,
  ) {
    return Draggable<WorkspaceEntry>(
      data: entry,
      dragAnchorStrategy: pointerDragAnchorStrategy,
      onDragStarted: () => setState(() => _dragging = true),
      onDragEnd: (_) => setState(() => _dragging = false),
      onDraggableCanceled: (_, _) => setState(() => _dragging = false),
      onDragCompleted: () => setState(() => _dragging = false),
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
              Icon(
                entry.isLocal ? Icons.computer_outlined : Icons.cloud_outlined,
                size: 18,
                color: const Color(0xFF2563EB),
              ),
              const SizedBox(width: 8),
              Flexible(
                child: Text(
                  entry.workspace.name,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
            ],
          ),
        ),
      ),
      childWhenDragging: Opacity(opacity: 0.4, child: row),
      child: Stack(
        children: [
          row,
          if (_dragging)
            Positioned.fill(
              child: Column(
                children: [
                  Expanded(child: _wsDropSlot(entry, world, before: true)),
                  Expanded(child: _wsDropSlot(entry, world, before: false)),
                ],
              ),
            ),
        ],
      ),
    );
  }

  Widget _wsDropSlot(
    WorkspaceEntry target,
    List<WorkspaceEntry> world, {
    required bool before,
  }) {
    return DragTarget<WorkspaceEntry>(
      hitTestBehavior: HitTestBehavior.opaque,
      onWillAcceptWithDetails: (d) => d.data.ref != target.ref,
      onAcceptWithDetails: (d) =>
          _reorderWs(d.data, target, world, before: before),
      builder: (context, candidate, rejected) {
        final active = candidate.isNotEmpty;
        return Align(
          alignment: before ? Alignment.topCenter : Alignment.bottomCenter,
          child: Container(
            height: 2,
            color: active ? const Color(0xFF2563EB) : Colors.transparent,
          ),
        );
      },
    );
  }

  Widget _row(WorkspaceEntry entry, List<WorkspaceEntry> world) {
    final workspace = entry.workspace;
    final selected = entry.ref == widget.selectedRef;
    return _wsDraggableRow(
      entry,
      world,
      SizedBox(
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
                context.l10n.commonRename,
                () => widget.onRename(entry),
              ),
              _wsAction(
                Icons.folder_zip_outlined,
                context.l10n.workspaceRowExportZip,
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
                    context.l10n.workspaceRowImportFiles,
                    () => widget.onImportFilesInto(entry),
                  ),
                  _wsAction(
                    Icons.drive_folder_upload_outlined,
                    context.l10n.workspaceRowImportFolder,
                    () => widget.onImportFolderInto(entry),
                  ),
                ],
                child: Text(context.l10n.commonImport),
              ),
              if (entry.isLocal && widget.onMigrate != null)
                _wsAction(
                  Icons.cloud_upload_outlined,
                  context.l10n.workspaceRowMigrate,
                  () => widget.onMigrate!(entry),
                ),
              if (!entry.isLocal && widget.onDetach != null)
                _wsAction(
                  Icons.computer_outlined,
                  context.l10n.workspaceRowDetach,
                  () => widget.onDetach!(entry),
                ),
              _wsAction(
                Icons.delete_outline,
                context.l10n.commonDelete,
                () => widget.onDelete(entry),
                color: const Color(0xFFDC2626),
              ),
            ],
            builder: (context, controller, child) => IconButton(
              tooltip: context.l10n.workspaceRowMenu,
              icon: const Icon(Icons.more_horiz, size: 18),
              onPressed: () =>
                  controller.isOpen ? controller.close() : controller.open(),
            ),
          ),
          const SizedBox(width: 4),
        ],
      ),
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
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 12),
          child: Row(
            children: [
              const Icon(Icons.add, size: 18, color: Color(0xFF2563EB)),
              const SizedBox(width: 10),
              Text(
                context.l10n.workspaceRowNewWorkspace,
                style: const TextStyle(
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
                  decoration: InputDecoration(
                    labelText: context.l10n.widgetRoleLabel,
                    border: const OutlineInputBorder(),
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
              tooltip: context.l10n.commonRemove,
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
              tooltip: context.l10n.rowMoveUp,
              onPressed: canMoveUp ? onMoveUp : null,
              icon: const Icon(Icons.arrow_upward),
            ),
            IconButton(
              tooltip: context.l10n.rowMoveDown,
              onPressed: canMoveDown ? onMoveDown : null,
              icon: const Icon(Icons.arrow_downward),
            ),
            IconButton(
              tooltip: context.l10n.rowEdit,
              onPressed: onEdit,
              icon: const Icon(Icons.edit_outlined),
            ),
            IconButton(
              tooltip: context.l10n.commonDelete,
              onPressed: onDelete,
              icon: const Icon(Icons.delete_outline),
            ),
          ],
        ),
      ),
    );
  }

  Widget _contentFor(BuildContext context, DocumentBlockKind kind) {
    final text = block.text.isEmpty ? context.l10n.widgetEmptyBlock : block.text;
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
    this.onImportFilesIntoFolder,
    this.onImportFolderIntoFolder,
    this.onTransferMove,
    this.onTransferCopy,
    required this.onClone,
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

  /// Export this folder's subtree as a ZIP. Works in BOTH worlds now (local
  /// goes through the shared Rust builder); null only if wiring omits it.
  final VoidCallback? onExportFolder;

  /// Import loose files / a picked folder UNDER this folder (md/zip/images →
  /// pages beneath it). Works in both worlds; null hides the entries.
  final VoidCallback? onImportFilesIntoFolder;
  final VoidCallback? onImportFolderIntoFolder;

  /// Move / copy this row's subtree into another cloud workspace. Both null in
  /// a local workspace (no cross-workspace transfer there) — the pair hides
  /// together, mirroring how the whole cloud-only block gates on one callback.
  final VoidCallback? onTransferMove;
  final VoidCallback? onTransferCopy;

  /// Duplicate this row's subtree in place. Always present — clone works in both
  /// cloud and local workspaces, unlike the transfer pair above.
  final VoidCallback onClone;
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
          PopupMenuItem(
            value: 'child',
            child: _MenuRow(
              icon: Icons.add,
              label: context.l10n.rowNewChildPage,
            ),
          ),
          PopupMenuItem(
            value: 'childFolder',
            child: _MenuRow(
              icon: Icons.create_new_folder_outlined,
              label: context.l10n.rowNewChildFolder,
            ),
          ),
        ],
        PopupMenuItem(
          value: 'rename',
          child: _MenuRow(
            icon: Icons.edit_outlined,
            label: context.l10n.commonRename,
          ),
        ),
        // Duplicate in place — works for pages AND folders (a folder copies its
        // whole subtree), and in cloud AND local workspaces (unlike transfer,
        // which is cloud-only), so it's always present.
        PopupMenuItem(
          value: 'duplicate',
          child: _MenuRow(
            icon: Icons.content_copy_outlined,
            label: context.l10n.rowDuplicate,
          ),
        ),
        if (widget.hasChildren)
          PopupMenuItem(
            value: 'toggle',
            child: _MenuRow(
              icon: widget.isCollapsed ? Icons.unfold_more : Icons.unfold_less,
              label: widget.isCollapsed
                  ? context.l10n.rowExpandChildren
                  : context.l10n.rowCollapseChildren,
            ),
          ),
        // Folder subtree -> ZIP, same as a page or a workspace. Every level
        // exports the same way and carries its images; see onExportFolder.
        if (widget._isFolder && widget.onExportFolder != null)
          PopupMenuItem(
            value: 'export',
            child: _MenuRow(
              icon: Icons.folder_zip_outlined,
              label: context.l10n.rowExportZipImages,
            ),
          ),
        // Import md / images / a nested folder UNDER this folder (both worlds).
        if (widget._isFolder && widget.onImportFilesIntoFolder != null)
          PopupMenuItem(
            value: 'importFiles',
            child: _MenuRow(
              icon: Icons.upload_file_outlined,
              label: context.l10n.workspaceRowImportFiles,
            ),
          ),
        if (widget._isFolder && widget.onImportFolderIntoFolder != null)
          PopupMenuItem(
            value: 'importFolder',
            child: _MenuRow(
              icon: Icons.drive_folder_upload_outlined,
              label: context.l10n.workspaceRowImportFolder,
            ),
          ),
        // Cross-workspace move/copy — cloud-only, so both hide in a local
        // workspace. Works for pages AND folders (the folder carries its
        // subtree), matching the server endpoint's semantics.
        if (widget.onTransferMove != null) ...[
          PopupMenuItem(
            value: 'transferMove',
            child: _MenuRow(
              icon: Icons.drive_file_move_outlined,
              label: context.l10n.transferMoveTitle,
            ),
          ),
          PopupMenuItem(
            value: 'transferCopy',
            child: _MenuRow(
              icon: Icons.copy_all_outlined,
              label: context.l10n.transferCopyTitle,
            ),
          ),
        ],
        const PopupMenuDivider(),
        PopupMenuItem(
          value: 'delete',
          child: _MenuRow(
            icon: Icons.delete_outline,
            label: context.l10n.commonDelete,
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
      case 'duplicate':
        widget.onClone();
      case 'toggle':
        widget.onToggle();
      case 'export':
        widget.onExportFolder?.call();
      case 'importFiles':
        widget.onImportFilesIntoFolder?.call();
      case 'importFolder':
        widget.onImportFolderIntoFolder?.call();
      case 'transferMove':
        widget.onTransferMove?.call();
      case 'transferCopy':
        widget.onTransferCopy?.call();
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
                              tooltip: w.isCollapsed
                                  ? context.l10n.rowExpand
                                  : context.l10n.rowCollapse,
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
                                  horizontal: 0,
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
                          tooltip: context.l10n.rowMoreActions,
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
                          tooltip: context.l10n.rowNewChildPage,
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
                          tooltip: context.l10n.rowNewChildFolder,
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
          context.l10n.presenceEditing(presence.length),
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
        title: Text(context.l10n.updateDialogTitle(info.version)),
        content: Text(context.l10n.updateDialogBody),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(ctx).pop(false),
            child: Text(context.l10n.commonCancel),
          ),
          FilledButton(
            onPressed: () => Navigator.of(ctx).pop(true),
            child: Text(context.l10n.updateAndRestart),
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
            label: Text(context.l10n.updateCheck),
            onPressed: _check,
          ),
        );
      case _UpdateStage.checking:
        return Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            const SizedBox(
              width: 16,
              height: 16,
              child: CircularProgressIndicator(strokeWidth: 2),
            ),
            const SizedBox(width: 10),
            Text(context.l10n.updateChecking),
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
            Text(context.l10n.updateUpToDate(kAppVersion)),
            const Spacer(),
            TextButton(onPressed: _check, child: Text(context.l10n.updateRecheck)),
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
                  child: Text(
                    context.l10n.updateFound(info.version, kAppVersion),
                  ),
                ),
              ],
            ),
            const SizedBox(height: 10),
            Align(
              alignment: Alignment.centerLeft,
              child: FilledButton.icon(
                icon: const Icon(Icons.download, size: 18),
                label: Text(context.l10n.updateNow),
                onPressed: _update,
              ),
            ),
          ],
        );
      case _UpdateStage.downloading:
        return Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(context.l10n.updateDownloading((_progress * 100).round())),
            const SizedBox(height: 8),
            LinearProgressIndicator(value: _progress > 0 ? _progress : null),
            const SizedBox(height: 6),
            Text(
              context.l10n.updateWillRestart,
              style: const TextStyle(fontSize: 12, color: Color(0xFF64748B)),
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
                    context.l10n.updateFailed(_error ?? ''),
                    style: const TextStyle(fontSize: 13),
                  ),
                ),
              ],
            ),
            const SizedBox(height: 8),
            Align(
              alignment: Alignment.centerLeft,
              child: TextButton(onPressed: _check, child: Text(context.l10n.commonRetry)),
            ),
          ],
        );
    }
  }
}

/// AppFlowy-style breadcrumb: the current page's folder path, each ancestor
/// segment clickable to jump there. A [trailing] widget (the properties toggle)
/// sits at the end. `part of main.dart`, so it shares its imports / `context.l10n`.
class _PageBreadcrumb extends StatelessWidget {
  const _PageBreadcrumb({
    required this.views,
    required this.current,
    required this.onSelect,
    required this.trailing,
  });

  final List<DocumentView> views;
  final DocumentView current;
  final Future<void> Function(DocumentView view) onSelect;
  final Widget trailing;

  @override
  Widget build(BuildContext context) {
    final byId = {for (final v in views) v.id: v};
    // Walk parent links up from the current page; `seen` guards a cyclic tree.
    final chain = <DocumentView>[];
    final seen = <String>{};
    DocumentView? v = current;
    while (v != null && seen.add(v.id)) {
      chain.add(v);
      final pid = v.parentViewId;
      v = (pid == null) ? null : byId[pid];
    }
    final path = chain.reversed.toList(); // root … current

    return Row(
      children: [
        Expanded(
          child: SingleChildScrollView(
            scrollDirection: Axis.horizontal,
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                for (var i = 0; i < path.length; i++) ...[
                  if (i > 0)
                    const Padding(
                      padding: EdgeInsets.symmetric(horizontal: 3),
                      child: Icon(Icons.chevron_right,
                          size: 14, color: EditorTheme.faint),
                    ),
                  _crumb(path[i], isLast: i == path.length - 1),
                ],
              ],
            ),
          ),
        ),
        trailing,
      ],
    );
  }

  Widget _crumb(DocumentView v, {required bool isLast}) {
    final label = v.name.trim().isEmpty ? '—' : v.name;
    final text = Text(
      label,
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
      style: TextStyle(
        color: isLast ? EditorTheme.muted : EditorTheme.faint,
        fontSize: 12,
      ),
    );
    // The current page (tail) is not a link — you're already on it.
    if (isLast) {
      return Padding(
        padding: const EdgeInsets.symmetric(vertical: 2),
        child: text,
      );
    }
    return InkWell(
      onTap: () => onSelect(v),
      borderRadius: BorderRadius.circular(4),
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: 2, horizontal: 2),
        child: text,
      ),
    );
  }
}

/// AFFiNE-style info toggle: shows/hides the page-properties panel. Filled +
/// accented when the page actually has properties, so a page's metadata is
/// discoverable even while the panel is collapsed.
class _PropertiesToggle extends StatelessWidget {
  const _PropertiesToggle({
    required this.active,
    required this.hasProperties,
    required this.onTap,
  });

  final bool active;
  final bool hasProperties;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final filled = active || hasProperties;
    return IconButton(
      tooltip: context.l10n.properties,
      onPressed: onTap,
      visualDensity: VisualDensity.compact,
      constraints: const BoxConstraints(minWidth: 28, minHeight: 28),
      padding: EdgeInsets.zero,
      icon: Icon(
        filled ? Icons.info : Icons.info_outline,
        size: 16,
        color: active
            ? EditorTheme.caret
            : (hasProperties ? EditorTheme.muted : EditorTheme.faint),
      ),
    );
  }
}
