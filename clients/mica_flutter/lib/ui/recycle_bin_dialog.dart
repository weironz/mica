import 'typography.dart';
import 'destructive_confirm.dart';
import '../l10n/locale_controller.dart';
import 'dart:async';
import 'package:flutter/material.dart';
import '../api/models.dart';
import 'home_data.dart';
import 'status_kit.dart';
import 'theme_tokens.dart';
import 'trash_data.dart';

class RecycleBinDialog extends StatefulWidget {
  const RecycleBinDialog({
    super.key,
    required this.onLoad,
    required this.onRestore,
    required this.onPurge,
    required this.canEdit,
    required this.liveViews,
    required this.relativeStrings,
    this.onPurgeAll,
  });

  /// Empty the whole bin, returning how many views went. **Null in 本地模式**:
  /// the on-device store has no bulk purge, and a button that cannot work is
  /// worse than no button — same null-means-absent rule as everything else here.
  /// Local users still purge one row at a time.
  final Future<int> Function()? onPurgeAll;

  /// The workspace's live tree, used only to name where a restore will land.
  final List<DocumentView> liveViews;

  /// Localized relative-time wording for the deletion time.
  final RelativeTimeStrings relativeStrings;

  final Future<List<DocumentView>> Function() onLoad;
  final Future<void> Function(DocumentView view) onRestore;
  final Future<void> Function(DocumentView view) onPurge;

  /// Both restore and purge are gated on the editor role server-side
  /// (`ensure_workspace_editor`). The buttons were drawn regardless, so a viewer
  /// could press either and collect a 403 — the same dead-control shape the
  /// null-means-absent rule elsewhere in this file exists to prevent. The list
  /// itself stays visible: knowing what was deleted is useful read-only.
  final bool canEdit;

  @override
  State<RecycleBinDialog> createState() => _RecycleBinDialogState();
}

class _RecycleBinDialogState extends State<RecycleBinDialog> {
  bool _loading = true;
  String? _error;
  List<TrashEntry> _entries = const [];

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
      // Roots only, plus what each restore will actually bring back — see
      // `buildTrashEntries`.
      final entries = buildTrashEntries(deleted: all, live: widget.liveViews);
      if (!mounted) return;
      setState(() {
        _entries = entries;
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

  /// A folder must LOOK like a folder. Every row used to wear the same page
  /// glyph, so a deleted folder — the one case where restore reaches past the row
  /// you clicked — was indistinguishable from a single page.
  Widget _trashLeading(DocumentView view) {
    final emoji = view.icon?.trim();
    if (emoji != null && emoji.isNotEmpty) {
      return SizedBox(
        width: 18,
        child: Center(child: Text(emoji, style: const TextStyle(fontSize: 16))),
      );
    }
    return Icon(
      view.isFolder ? Icons.folder_outlined : Icons.description_outlined,
      size: 18,
      color: MicaTheme.of(context).text.muted,
    );
  }

  /// Where it came from · what comes back with it · when it was deleted.
  ///
  /// Each part is dropped when it isn't known rather than filled with a
  /// placeholder: the local world carries no `icon` and no deletion timestamp
  /// (they never reach `DocumentView` through `_viewFromData`), and a top-level
  /// page has no path. An empty subtree says nothing rather than 「含 0 个页面」.
  Widget? _trashSubtitle(BuildContext context, TrashEntry entry) {
    final l10n = context.l10n;
    final counts = <String>[
      if (entry.folders > 0) l10n.folderCount(entry.folders),
      if (entry.pages > 0) l10n.pageCount(entry.pages),
    ];
    // `delete_view` stamps `updated_at = now()` when trashing, so for a deleted
    // row this IS the deletion time. Null in the local world.
    final deletedAt = entry.view.updatedAt;
    final parts = <String>[
      if (entry.path.isNotEmpty) entry.path,
      if (counts.isNotEmpty)
        l10n.recycleSubtree(counts.join(l10n.importListSeparator)),
      if (deletedAt != null) relativeMeta(deletedAt, widget.relativeStrings),
    ];
    if (parts.isEmpty) return null;
    return Text(
      parts.join('  ·  '),
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
      style: MicaTypography.caption.copyWith(
        color: MicaTheme.of(context).text.faint,
      ),
    );
  }

  /// Permanent delete is the only irreversible action in this dialog, and it
  /// reaches far wider than the row you clicked: the server drops the whole
  /// subtree's views *and* their backing `documents` rows (`purge_view_subtree`
  /// in `routes/documents.rs`), which cascades version history and comments
  /// away with them. It used to fire straight off the icon button, so one
  /// mis-click next to 「恢复」 destroyed a subtree with nothing to undo it.
  Future<bool> _confirmPurge(DocumentView view) {
    final l10n = context.l10n;
    return showDestructiveConfirm(
      context,
      title: l10n.recyclePurgeConfirmTitle(view.name),
      body: l10n.recyclePurgeConfirmBody,
      confirmLabel: l10n.recycleDeleteForever,
      cancelLabel: l10n.commonCancel,
    );
  }

  /// Empty the bin, behind the same gate as a single permanent delete — this one
  /// is every row at once, so the confirmation names the count.
  Future<void> _confirmPurgeAll() async {
    final purgeAll = widget.onPurgeAll;
    if (purgeAll == null) return;
    final l10n = context.l10n;
    final ok = await showDestructiveConfirm(
      context,
      title: l10n.recycleEmptyAllConfirmTitle(_entries.length),
      body: l10n.recyclePurgeConfirmBody,
      confirmLabel: l10n.recycleEmptyAll,
      cancelLabel: l10n.commonCancel,
    );
    if (!ok) return;
    try {
      final removed = await purgeAll();
      if (!mounted) return;
      ScaffoldMessenger.maybeOf(context)?.showSnackBar(
        SnackBar(content: Text(context.l10n.recycleEmptiedAll(removed))),
      );
    } catch (error) {
      if (!mounted) return;
      setState(() => _error = error.toString());
    }
    await _refresh();
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: Row(
        children: [
          const Icon(Icons.delete_outline, size: 22),
          const SizedBox(width: 8),
          Text(context.l10n.recycleBinTitle),
          const Spacer(),
          IconButton(
            tooltip: context.l10n.recycleRefresh,
            onPressed: _loading ? null : _refresh,
            icon: const Icon(Icons.refresh, size: 20),
          ),
        ],
      ),
      content: SizedBox(width: 420, height: 360, child: _buildBody(context)),
      actions: [
        // Only with something to empty, and only where a bulk purge exists.
        if (widget.onPurgeAll != null && widget.canEdit && _entries.isNotEmpty)
          TextButton(
            style: TextButton.styleFrom(
              foregroundColor: kDestructiveRed(context),
            ),
            onPressed: _confirmPurgeAll,
            child: Text(context.l10n.recycleEmptyAll),
          ),
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(context.l10n.commonClose),
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
    if (_entries.isEmpty) {
      return EmptyState(
        icon: Icons.delete_outline,
        title: context.l10n.recycleEmpty,
        detail: context.l10n.recycleEmptyDetail,
      );
    }
    return ListView.separated(
      itemCount: _entries.length,
      separatorBuilder: (_, _) => const Divider(height: 1),
      itemBuilder: (context, i) {
        final entry = _entries[i];
        final view = entry.view;
        return ListTile(
          contentPadding: const EdgeInsets.symmetric(horizontal: 4),
          leading: _trashLeading(view),
          title: Text(view.name, overflow: TextOverflow.ellipsis),
          subtitle: _trashSubtitle(context, entry),
          trailing: !widget.canEdit
              ? null
              : Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    IconButton(
                      tooltip: context.l10n.recycleRestore,
                      icon: const Icon(Icons.restore, size: 20),
                      onPressed: () async {
                        await widget.onRestore(view);
                        await _refresh();
                      },
                    ),
                    IconButton(
                      tooltip: context.l10n.recycleDeleteForever,
                      color: MicaTheme.of(context).status.danger,
                      icon: const Icon(Icons.delete_forever, size: 20),
                      onPressed: () async {
                        if (!await _confirmPurge(view)) return;
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
