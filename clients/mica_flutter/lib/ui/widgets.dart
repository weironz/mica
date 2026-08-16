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
    this.activeMeta,
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

  /// The quiet second line under the workspace name — e.g. "248 个页面 · 云端".
  /// Formatted by the host (it owns the page counts and the world labels); null
  /// renders a single-line trigger, which is what "no workspace selected" wants.
  final String? activeMeta;
  final void Function(WorkspaceEntry entry) onDelete;
  final void Function(WorkspaceEntry entry) onExport;
  final VoidCallback onCreate;
  /// Null hides the entry. Archive import is unsupported in local mode, and a
  /// menu item that does nothing is worse than no menu item.
  final void Function(bool notion)? onImport;
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
        // The whole submenu goes, not just its children: a parent left behind
        // with nothing under it is its own kind of dead end.
        if (widget.onImport != null)
          SizedBox(
            width: 320,
            child: SubmenuButton(
              leadingIcon: Icon(
                Icons.upload_file_outlined,
                size: 18,
                color: MicaTheme.of(context).text.muted,
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
                style: TextStyle(
                  color: MicaTheme.of(context).text.muted,
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
        // Two-line trigger (design 03/12): name over a quiet meta line. The old
        // single line left the switcher saying nothing about the workspace beyond
        // its name — not how big it is, not which world it lives in.
        //
        // The design puts a per-workspace EMOJI in the tile; `Workspace` has no
        // icon column (only views do), so the tile carries the world glyph
        // instead. Adding one is a schema change, not a paint job.
        return SizedBox(
          width: double.infinity,
          child: InkWell(
            borderRadius: BorderRadius.circular(8),
            onTap: () =>
                controller.isOpen ? controller.close() : controller.open(),
            child: Padding(
              padding: const EdgeInsets.all(8),
              child: Row(
                children: [
                  Container(
                    width: 30,
                    height: 30,
                    decoration: BoxDecoration(
                      color: MicaTheme.of(context).accent.wash,
                      borderRadius: BorderRadius.circular(8),
                    ),
                    child: Icon(
                      widget.activeIsLocal
                          ? Icons.computer_outlined
                          : Icons.cloud_outlined,
                      size: 17,
                      color: MicaTheme.of(context).accent.primary,
                    ),
                  ),
                  const SizedBox(width: 10),
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        Text(
                          label,
                          overflow: TextOverflow.ellipsis,
                          maxLines: 1,
                          style: TextStyle(
                            fontSize: 14,
                            fontWeight: FontWeight.w600,
                            color: MicaTheme.of(context).text.primary,
                          ),
                        ),
                        if (widget.activeMeta != null)
                          Text(
                            widget.activeMeta!,
                            overflow: TextOverflow.ellipsis,
                            maxLines: 1,
                            style: TextStyle(
                              fontSize: 12,
                              color: MicaTheme.of(context).text.faint,
                            ),
                          ),
                      ],
                    ),
                  ),
                  Icon(
                    Icons.unfold_more,
                    size: 16,
                    color: MicaTheme.of(context).text.faint,
                  ),
                ],
              ),
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
              Icon(
                Icons.login,
                size: 18,
                color: MicaTheme.of(context).accent.primary,
              ),
              const SizedBox(width: 10),
              Text(
                context.l10n.workspaceRowSignInCloud,
                style: TextStyle(
                  color: MicaTheme.of(context).accent.primary,
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
            color: MicaTheme.of(context).surface.overlay,
            borderRadius: BorderRadius.circular(6),
          ),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(
                entry.isLocal ? Icons.computer_outlined : Icons.cloud_outlined,
                size: 18,
                color: MicaTheme.of(context).accent.primary,
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
            color: active
                ? MicaTheme.of(context).accent.primary
                : Colors.transparent,
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
                            ? MicaTheme.of(context).accent.primary
                            : MicaTheme.of(context).text.faint,
                      ),
                      const SizedBox(width: 10),
                      Expanded(
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          mainAxisSize: MainAxisSize.min,
                          children: [
                            Text(
                              workspace.name,
                              overflow: TextOverflow.ellipsis,
                              style: TextStyle(
                                fontWeight: selected
                                    ? FontWeight.w600
                                    : FontWeight.w400,
                                color: MicaTheme.of(context).text.primary,
                              ),
                            ),
                            // Page count, and ONLY when the server actually sent
                            // one. `pageCount` is 0 both for "empty" and for
                            // "this list didn't carry counts" (a local workspace,
                            // an older server), and the two are indistinguishable
                            // here — so 0 prints nothing rather than claiming a
                            // workspace is empty when nobody ever counted it.
                            if (workspace.pageCount > 0)
                              Padding(
                                padding: const EdgeInsets.only(top: 2),
                                child: Text(
                                  context.l10n.pageCount(workspace.pageCount),
                                  style: TextStyle(
                                    fontSize: 12,
                                    color: MicaTheme.of(context).text.faint,
                                  ),
                                ),
                              ),
                          ],
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
                  leadingIcon: Icon(
                    Icons.download_outlined,
                    size: 18,
                    color: MicaTheme.of(context).text.muted,
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
                  color: MicaTheme.of(context).status.danger,
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
              Icon(
                Icons.add,
                size: 18,
                color: MicaTheme.of(context).accent.primary,
              ),
              const SizedBox(width: 10),
              Text(
                context.l10n.workspaceRowNewWorkspace,
                style: TextStyle(
                  color: MicaTheme.of(context).accent.primary,
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
        color: color ?? MicaTheme.of(context).text.muted,
      ),
      onPressed: () {
        _menu.close();
        onTap();
      },
      child: Text(
        label,
        style: TextStyle(color: color ?? MicaTheme.of(context).text.primary),
      ),
    );
  }

  /// Both submenu entries share the tree-import core; the Notion one forces
  /// Notion adaptation (ID-suffix stripping, folder↔page matching).
  Widget _importChoice(IconData icon, String label, {required bool notion}) {
    return MenuItemButton(
      leadingIcon: Icon(
        icon,
        size: 18,
        color: MicaTheme.of(context).text.muted,
      ),
      onPressed: () {
        _menu.close();
        widget.onImport?.call(notion);
      },
      child: Text(
        label,
        style: TextStyle(color: MicaTheme.of(context).text.muted),
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
      color: isSelected
          ? MicaTheme.of(context).accent.wash
          : MicaTheme.of(context).surface.base,
      shape: RoundedRectangleBorder(
        side: BorderSide(
          color: isSelected
              ? MicaTheme.of(context).accent.primary
              : MicaTheme.of(context).border.normal,
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
    required this.avatarUrl,
    required this.canManage,
    required this.canRemove,
    required this.onRoleChanged,
    required this.onRemove,
    super.key,
  });

  final WorkspaceMember member;

  /// Null when this member has no picture — the row falls back to their initial.
  final String? avatarUrl;

  final bool canManage;
  final bool canRemove;
  final Future<void> Function(WorkspaceRole role) onRoleChanged;
  final Future<void> Function() onRemove;

  @override
  Widget build(BuildContext context) {
    return DecoratedBox(
      decoration: BoxDecoration(
        color: MicaTheme.of(context).surface.raised,
        borderRadius: BorderRadius.circular(8),
        border: Border.all(color: MicaTheme.of(context).border.normal),
      ),
      child: Padding(
        padding: const EdgeInsets.all(12),
        child: Row(
          children: [
            UserAvatar(
              url: avatarUrl,
              radius: 20,
              // The generic person glyph read as "we know nothing about this
              // account". Their own initial is both more informative and the
              // same fallback the sidebar uses, so one person looks like one
              // person everywhere.
              fallback: member.displayName.isNotEmpty
                  ? member.displayName.substring(0, 1).toUpperCase()
                  : '?',
            ),
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
            ? MicaTheme.of(context).editor.codeBg
            : MicaTheme.of(context).surface.base,
        borderRadius: BorderRadius.circular(8),
        border: Border.all(color: MicaTheme.of(context).border.normal),
      ),
      child: Padding(
        padding: const EdgeInsets.all(12),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Icon(_iconFor(kind), color: MicaTheme.of(context).text.muted),
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
    final text = block.text.isEmpty
        ? context.l10n.widgetEmptyBlock
        : block.text;
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
          decoration: BoxDecoration(
            border: Border(
              left: BorderSide(
                color: MicaTheme.of(context).text.faint,
                width: 3,
              ),
            ),
          ),
          child: Padding(
            padding: const EdgeInsets.only(left: 12),
            child: SelectableText(
              text,
              style: Theme.of(context).textTheme.bodyLarge?.copyWith(
                color: MicaTheme.of(context).text.muted,
              ),
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
    this.onSetIcon,
    this.onOpenInNewTab,
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

  /// Opens the emoji picker for this row. Null in worlds where icons can't be
  /// persisted (the local store has no icon column), and then the menu entry is
  /// simply absent rather than present-but-dead.
  final VoidCallback? onSetIcon;

  /// Open this page in a new tab. Null on folder rows (a folder has no document
  /// to open) and in the local world, which has no tabs.
  ///
  /// NOT gated on [canEdit], unlike the rest of this menu: opening a second tab
  /// is a read. A viewer who can open the page at all can open it twice.
  final VoidCallback? onOpenInNewTab;

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
        // First, because it is the only entry that opens something rather than
        // changing it — the same slot browsers and AppFlowy put it in.
        if (widget.onOpenInNewTab != null)
          PopupMenuItem(
            value: 'openInNewTab',
            child: _MenuRow(
              icon: Icons.tab_outlined,
              label: context.l10n.rowOpenInNewTab,
            ),
          ),
        // Everything below edits. A viewer reaches this menu only for the entry
        // above, so the edit half is built only when they could act on it —
        // showing a rename that is guaranteed to 403 is worse than showing
        // nothing, the same rule the rest of this file follows.
        if (widget.canEdit) ...[
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
        // Set/clear the page emoji. Sits next to rename because it is the same
        // kind of act — naming the thing — and the picker itself carries the
        // "remove icon" affordance, so no second menu entry is needed.
        if (widget.onSetIcon != null)
          PopupMenuItem(
            value: 'setIcon',
            child: _MenuRow(
              icon: Icons.emoji_emotions_outlined,
              label: context.l10n.rowSetIcon,
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
      ],
    );
    if (!mounted) return;
    switch (selected) {
      case 'openInNewTab':
        widget.onOpenInNewTab?.call();
      case 'child':
        widget.onCreateChild();
      case 'childFolder':
        widget.onCreateChildFolder();
      case 'rename':
        widget.onRename();
      case 'setIcon':
        widget.onSetIcon?.call();
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
        color: w.isSelected
            ? MicaTheme.of(context).accent.wash
            : Colors.transparent,
        borderRadius: BorderRadius.circular(6),
        child: InkWell(
          borderRadius: BorderRadius.circular(6),
          // A folder has no content to open — clicking it expands/collapses in
          // place (file-manager style). Documents open in the editor. While
          // renaming, taps stay inside the inline field (click-away commits).
          onTap: w.isRenaming ? null : (w._isFolder ? w.onToggle : w.onPressed),
          // A viewer gets the menu too when it has something for them: every
          // other entry is an edit, but "open in new tab" is a read. Gating the
          // whole menu on canEdit hid the one entry a viewer can actually use.
          onSecondaryTapDown: (w.canEdit || w.onOpenInNewTab != null)
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
                  // A user-set emoji replaces the kind glyph; without one the
                  // glyph still says folder-vs-page, so a tree with no icons set
                  // reads exactly as before. Sized to the same 18px box so rows
                  // never shift when an icon is added or cleared.
                  if (w.view.icon != null)
                    SizedBox(
                      width: 18,
                      height: 18,
                      child: Center(
                        child: Text(
                          w.view.icon!,
                          style: const TextStyle(fontSize: 15, height: 1.1),
                        ),
                      ),
                    )
                  else
                    Icon(
                      w._isFolder
                          ? (w.isCollapsed
                                ? Icons.folder_outlined
                                : Icons.folder_open_outlined)
                          : Icons.description_outlined,
                      size: 18,
                      color: w.isSelected
                          ? MicaTheme.of(context).accent.primary
                          : MicaTheme.of(context).text.muted,
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
    final color = danger ? MicaTheme.of(context).status.danger : null;
    return Row(
      children: [
        Icon(icon, size: 18, color: color),
        const SizedBox(width: 12),
        Text(label, style: TextStyle(color: color)),
      ],
    );
  }
}

/// The app's empty state. A thin adapter over [MicaEmptyState] so all ~11 call
/// sites moved to the design-19 standard at once, without touching each one.
///
/// What changed by delegating: the old body used #64748B (a slate the design
/// system bans), a 40px bare icon and a `titleLarge` heading — visually a
/// different family from the spec's 46px tile + 14/600 title + 12.5 body. And it
/// had no way to offer an ACTION, which is half the rule: an empty state must say
/// what happened AND give one next step. Callers that have a next step can now
/// pass one; the rest render exactly as before, just on-spec.
class EmptyState extends StatelessWidget {
  const EmptyState({
    required this.icon,
    required this.title,
    required this.detail,
    this.actionLabel,
    this.onAction,
    super.key,
  });

  final IconData icon;
  final String title;
  final String detail;

  /// Both or neither — enforced by [MicaEmptyState]: a label with no callback is
  /// a dead button, a callback with no label is invisible.
  final String? actionLabel;
  final VoidCallback? onAction;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 360),
        child: MicaEmptyState(
          icon: icon,
          title: title,
          body: detail,
          actionLabel: actionLabel,
          onAction: onAction,
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
        color: MicaTheme.of(context).status.dangerWash,
        borderRadius: BorderRadius.circular(8),
        border: Border.all(
          color: MicaTheme.of(context).status.danger.withValues(alpha: 0.5),
        ),
      ),
      child: Padding(
        padding: const EdgeInsets.all(12),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Icon(
              Icons.error_outline,
              color: MicaTheme.of(context).status.danger,
            ),
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
        Icon(
          Icons.circle,
          size: 8,
          color: MicaTheme.of(context).status.success,
        ),
        const SizedBox(width: 8),
        for (var i = 0; i < presence.length && i < 5; i++)
          Padding(
            padding: const EdgeInsets.only(right: 4),
            child: Tooltip(
              message: presence[i].name,
              child: CircleAvatar(
                radius: 12,
                // The connection's own colour, so this avatar and that
                // person's remote caret flag are the same — which is what
                // `kPresencePalette`'s doc comment promises. This used to
                // index a COPY of the palette by list position, so the two
                // disagreed, and a lone collaborator was always blue here
                // while their caret was whatever the hash picked.
                backgroundColor: presence[i].color,
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
          style: Theme.of(context).textTheme.bodySmall?.copyWith(
            color: MicaTheme.of(context).status.success,
          ),
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

  /// True only when the download's size/sha256 did not match, as opposed to any
  /// other update failure. Drives the one case that gets a full failure card.
  bool _integrityFailed = false;

  Future<void> _check() async {
    setState(() {
      _stage = _UpdateStage.checking;
      _error = null;
      _integrityFailed = false;
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
        // One failure *shape*, four bodies. Only a genuine digest mismatch earns
        // the "aborted" card: a network hiccup that reads "the installer failed
        // its integrity check" is worse than a vague message.
        _integrityFailed = e is UpdateIntegrityException;
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
            Icon(
              Icons.check_circle_outline,
              size: 18,
              color: MicaTheme.of(context).status.success,
            ),
            const SizedBox(width: 8),
            Text(context.l10n.updateUpToDate(kAppVersion)),
            const Spacer(),
            TextButton(
              onPressed: _check,
              child: Text(context.l10n.updateRecheck),
            ),
          ],
        );
      case _UpdateStage.available:
        final info = _info!;
        return Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Icon(
                  Icons.new_releases_outlined,
                  size: 18,
                  color: MicaTheme.of(context).accent.primary,
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
              style: TextStyle(
                fontSize: 12,
                color: MicaTheme.of(context).text.muted,
              ),
            ),
          ],
        );
      case _UpdateStage.error:
        // The integrity gate deleted a file it refused to run. That deserves the
        // full card (design 19 「更新已中止」): what happened, what is *not*
        // affected — this install still works — and two real ways forward. Every
        // other update failure stays the plain retry row below: same shape would
        // imply the same cause.
        if (_integrityFailed) {
          return MicaFailureCard(
            icon: Icons.gpp_maybe_outlined,
            title: context.l10n.updaterAbortedTitle,
            body: context.l10n.updaterAbortedBody(kAppVersion),
            secondaryLabel: context.l10n.updaterOpenReleases,
            onSecondary: () =>
                openUrl('https://github.com/$kUpdateRepo/releases'),
            primaryLabel: context.l10n.commonRetry,
            onPrimary: _check,
          );
        }
        return Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Icon(
                  Icons.error_outline,
                  size: 18,
                  color: MicaTheme.of(context).status.danger,
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
              child: TextButton(
                onPressed: _check,
                child: Text(context.l10n.commonRetry),
              ),
            ),
          ],
        );
    }
  }
}

/// AppFlowy-style breadcrumb: the current page's folder path, each ancestor
/// segment clickable to jump there. A [trailing] widget (the properties toggle)
/// sits at the end. `part of main.dart`, so it shares its imports / `context.l10n`.
/// A copy button that answers where it was pressed: the icon becomes a green
/// check for a moment, with a "Copied" tooltip.
///
/// Replaces a snackbar. A working copy is the EXPECTED outcome, and throwing a
/// black bar across the bottom of the window to announce it is the loudest
/// possible way to say the least — it also lands far from the cursor, so the eye
/// has to travel to read that nothing went wrong. GitHub's copy affordance does
/// it this way, and the confirmation arrives where the click did.
///
/// Failure is NOT reported here: it is unexpected, it needs words, and the
/// caller still shows those. This widget only ever says "yes, done".
class PageBreadcrumb extends StatefulWidget {
  const PageBreadcrumb({
    required this.views,
    required this.current,
    required this.onSelect,
    required this.trailing,
    this.onRename,
    this.onCopyPath,
    this.workspaceName,
  });

  final List<DocumentView> views;
  final DocumentView current;
  final Future<void> Function(DocumentView view) onSelect;
  final Widget trailing;

  /// Rename the page from the breadcrumb tail (AppFlowy does this). Null when the
  /// page cannot be renamed — a viewer's read-only workspace — and then the tail
  /// stays plain text rather than offering an edit that would 403.
  final Future<void> Function(DocumentView view, String name)? onRename;

  /// Copy the page's path, from a button sitting immediately AFTER the last
  /// crumb — where GitHub puts it, next to the path rather than off in the
  /// row's trailing utilities. Null hides the button.
  /// Returns whether the copy actually happened — the button turns into a
  /// check only on true, so a refused clipboard cannot look like a success.
  final Future<bool> Function()? onCopyPath;

  /// Shown as the FIRST crumb, so what you read matches what [onCopyPath]
  /// copies — the copied path has always led with the workspace, and a
  /// breadcrumb that started one level lower made the two disagree.
  ///
  /// Not a link: there is no "open the workspace" destination here the way
  /// there is for a folder. Null hides it.
  final String? workspaceName;

  @override
  State<PageBreadcrumb> createState() => PageBreadcrumbState();
}

class PageBreadcrumbState extends State<PageBreadcrumb> {
  final MenuController _renameMenu = MenuController();
  final TextEditingController _renameField = TextEditingController();
  final FocusNode _renameFocus = FocusNode();

  @override
  void dispose() {
    _renameField.dispose();
    _renameFocus.dispose();
    super.dispose();
  }

  void _openRename() {
    // Seed with the live name every time: the page may have been renamed from the
    // sidebar since this widget was built.
    _renameField.text = widget.current.name;
    _renameField.selection = TextSelection(
      baseOffset: 0,
      extentOffset: _renameField.text.length,
    );
    _renameMenu.open();
    // The menu takes focus as it opens; asking for it on the next frame is what
    // lands the caret in the field instead of on the menu surface.
    WidgetsBinding.instance.addPostFrameCallback(
      (_) => _renameFocus.requestFocus(),
    );
  }

  Future<void> _submitRename() async {
    final rename = widget.onRename;
    // `renamedTo` decides whether there is anything to save at all: blank means
    // "changed my mind" (the server rejects empty names) and unchanged must not
    // send a write, since closing the popover commits.
    final next = renamedTo(_renameField.text, widget.current.name);
    if (_renameMenu.isOpen) _renameMenu.close();
    if (rename == null || next == null) return;
    await rename(widget.current, next);
  }

  @override
  Widget build(BuildContext context) {
    final views = widget.views;
    final byId = {for (final v in views) v.id: v};
    // Walk parent links up from the current page; `seen` guards a cyclic tree.
    final chain = <DocumentView>[];
    final seen = <String>{};
    DocumentView? v = widget.current;
    while (v != null && seen.add(v.id)) {
      chain.add(v);
      final pid = v.parentViewId;
      v = (pid == null) ? null : byId[pid];
    }
    final full = chain.reversed.toList(); // root … current

    return Row(
      children: [
        // The path measures itself against the width it actually got. It used
        // to scroll horizontally inside a fixed budget instead, which failed
        // twice on the same page: inside a scroll view the width is unbounded,
        // so `TextOverflow.ellipsis` never fires and a long title was HARD-CUT
        // with no ellipsis (reads as a rendering bug, not as "there is more"),
        // and the copy button — which scrolled with the path — was pushed out
        // of the viewport entirely.
        // Flexible, not Expanded: a short path shrink-wraps so the copy button
        // hugs the last crumb instead of floating off in the leftover budget.
        Flexible(
          child: LayoutBuilder(
            builder: (context, box) => _path(context, full, box.maxWidth),
          ),
        ),
        // PINNED, outside the flexible path: it is about the page, so it must
        // not disappear because the page's name is long. Still immediately
        // after the crumbs (the spot GitHub uses) rather than off in `trailing`
        // with the sync badge, where it would read as another page utility
        // instead of "copy THIS".
        if (widget.onCopyPath case final copy?) ...[
          const SizedBox(width: 4),
          InlineCopyButton(onCopy: copy, tooltip: context.l10n.pageCopyPath),
        ],
        widget.trailing,
      ],
    );
  }

  static const _sepWidth = 20.0; // chevron 14 + 3px either side
  static const _crumbPad = 4.0; // _crumb's horizontal padding
  static const _ellipsisWidth = 12.0;

  double _measure(String label) {
    final tp = TextPainter(
      text: TextSpan(text: label, style: const TextStyle(fontSize: 12)),
      textDirection: TextDirection.ltr,
      maxLines: 1,
    )..layout();
    final w = tp.width;
    tp.dispose();
    return w + _crumbPad;
  }

  /// The crumbs that fit in [avail], tail first.
  ///
  /// The old rule was a segment COUNT (`> 3` collapses the middle), which says
  /// nothing about whether the thing fits: `tools › 笔记软件 › mica › 单机手动
  /// 部署（IP 直连，不用 Traefik）` is only three segments deep and still ran
  /// off the end. So the decision is made in pixels now.
  ///
  /// The tail is guaranteed room before any ancestor gets any — it is the page
  /// you are looking at, and the one segment you cannot reconstruct from
  /// context. Ancestors are then added back from the RIGHT (nearest parent
  /// first) while they fit, so what survives is what you are most likely to
  /// click. A `…` marks that something was dropped; the whole path stays one
  /// click away on the copy button.
  Widget _path(BuildContext context, List<DocumentView> full, double avail) {
    final faint = MicaTheme.of(context).text.faint;
    final ws = widget.workspaceName?.trim();
    final hasWs = ws != null && ws.isNotEmpty;
    // Leading labels, in order: the workspace (plain text — `_crumb` links a
    // DocumentView and a workspace is neither) then every ancestor.
    final leading = <String>[
      if (hasWs) ws,
      for (final v in full.take(full.length - 1)) v.name,
    ];

    final tail = full.last;
    final tailNeed = _measure(tail.name.trim().isEmpty ? '—' : tail.name);
    // Never let ancestors squeeze the tail to nothing: it gets what it needs,
    // or a majority of the row, whichever is smaller.
    var used = math.min(tailNeed, math.max(avail * 0.55, 90.0));

    final keep = <int>[];
    for (var i = leading.length - 1; i >= 0; i--) {
      final w = _measure(leading[i]) + _sepWidth;
      if (used + w > avail) break;
      used += w;
      keep.insert(0, i);
    }
    // The `…` costs width too. Rather than let it push the row over, buy it
    // back from the leftmost ancestor still standing.
    while (keep.length < leading.length &&
        keep.isNotEmpty &&
        used + _ellipsisWidth + _sepWidth > avail) {
      used -= _measure(leading[keep.removeAt(0)]) + _sepWidth;
    }
    final dropped = keep.length < leading.length;

    Widget sep() => Padding(
      padding: const EdgeInsets.symmetric(horizontal: 3),
      child: Icon(Icons.chevron_right, size: 14, color: faint),
    );

    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        if (dropped) ...[
          Text('…', style: TextStyle(color: faint, fontSize: 12)),
          sep(),
        ],
        for (final i in keep) ...[
          if (i == 0 && hasWs)
            Text(
              ws,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(color: faint, fontSize: 12),
            )
          else
            _crumb(full[hasWs ? i - 1 : i], isLast: false),
          sep(),
        ],
        // Flexible, so the tail takes every pixel the ancestors left and
        // ellipsizes inside it instead of running off the edge.
        Flexible(child: _crumb(tail, isLast: true)),
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
        color: isLast
            ? MicaTheme.of(context).text.muted
            : MicaTheme.of(context).text.faint,
        fontSize: 12,
      ),
    );
    // The tail is not a link — you are already on it — but it IS where the page
    // gets renamed, the way AppFlowy does it.
    if (isLast) {
      if (widget.onRename == null) {
        return Padding(
          padding: const EdgeInsets.symmetric(vertical: 2),
          child: text,
        );
      }
      return MenuAnchor(
        controller: _renameMenu,
        style: const MenuStyle(
          padding: WidgetStatePropertyAll(EdgeInsets.all(8)),
        ),
        menuChildren: [_renamePopover(context)],
        builder: (context, controller, child) => InkWell(
          borderRadius: BorderRadius.circular(4),
          onTap: _openRename,
          child: Padding(
            padding: const EdgeInsets.symmetric(vertical: 2, horizontal: 2),
            child: text,
          ),
        ),
      );
    }
    return InkWell(
      onTap: () => widget.onSelect(v),
      borderRadius: BorderRadius.circular(4),
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: 2, horizontal: 2),
        child: text,
      ),
    );
  }
}

/// The rename popover: page glyph + a field, anchored under the breadcrumb tail.
extension _PageBreadcrumbPopover on PageBreadcrumbState {
  Widget _renamePopover(BuildContext context) {
    return SizedBox(
      width: 260,
      child: Row(
        children: [
          Container(
            width: 26,
            height: 26,
            decoration: BoxDecoration(
              color: MicaTheme.of(context).surface.sunken,
              borderRadius: BorderRadius.circular(6),
            ),
            alignment: Alignment.center,
            child: (widget.current.icon?.trim().isNotEmpty ?? false)
                ? Text(
                    widget.current.icon!.trim(),
                    style: const TextStyle(fontSize: 14),
                  )
                : Icon(
                    widget.current.objectType == 'folder'
                        ? Icons.folder_outlined
                        : Icons.description_outlined,
                    size: 15,
                    color: MicaTheme.of(context).text.muted,
                  ),
          ),
          const SizedBox(width: 8),
          Expanded(
            child: TextField(
              controller: _renameField,
              focusNode: _renameFocus,
              style: const TextStyle(fontSize: 14),
              decoration: const InputDecoration(
                isDense: true,
                border: OutlineInputBorder(),
                contentPadding: EdgeInsets.symmetric(
                  horizontal: 8,
                  vertical: 8,
                ),
              ),
              // Enter commits. Losing focus commits too, so clicking away saves
              // rather than silently discarding what was typed — the same
              // blur-commits rule the sidebar's inline rename uses.
              onSubmitted: (_) => _submitRename(),
              onTapOutside: (_) => _submitRename(),
            ),
          ),
        ],
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
            ? MicaTheme.of(context).accent.primary
            : (hasProperties
                  ? MicaTheme.of(context).text.muted
                  : MicaTheme.of(context).text.faint),
      ),
    );
  }
}
