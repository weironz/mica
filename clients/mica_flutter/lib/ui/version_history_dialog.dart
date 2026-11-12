import '../l10n/locale_controller.dart';
import 'dart:async';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import '../api/models.dart';
import '../editor/editor.dart';
import '../editor/version_diff.dart';
import '../l10n/app_localizations.dart';
import 'theme_tokens.dart';
import 'version_data.dart';

class VersionHistoryDialog extends StatefulWidget {
  const VersionHistoryDialog({
    super.key,
    required this.onList,
    required this.onCreate,
    required this.onRestore,
    required this.onLoadContent,
    this.onLoadImageBytes,
    this.onResolveImageUrls,
    this.authorNames = const {},
  });

  /// User id → display name, for [DocVersion.createdBy].
  ///
  /// Empty is the normal case and means "don't show an author column": local
  /// history has no user ids at all, and in a one-person workspace a column
  /// repeating your own name down every row is noise, not information.
  final Map<String, String> authorNames;

  final Future<List<DocVersion>> Function() onList;
  final Future<void> Function(String name) onCreate;
  final Future<void> Function(String versionId) onRestore;

  /// A version's blocks (tree order) + root id, for the read-only preview.
  final Future<({String rootBlockId, List<Map<String, dynamic>> blocks})>
  Function(String versionId)
  onLoadContent;

  /// Image handlers passed straight to the preview editor so pictures render.
  final Future<Uint8List?> Function(String fileId)? onLoadImageBytes;
  final Future<Map<String, String>> Function(List<String> fileIds)?
  onResolveImageUrls;

  @override
  State<VersionHistoryDialog> createState() => _VersionHistoryDialogState();
}

class _VersionHistoryDialogState extends State<VersionHistoryDialog> {
  bool _loading = true;
  bool _busy = false; // a create/restore is in flight
  String? _error;
  List<DocVersion> _versions = const [];

  // Preview pane state, keyed to the selected version.
  String? _selectedId;
  bool _previewLoading = false;
  String? _previewError;
  ({String rootBlockId, List<Map<String, dynamic>> blocks})? _content;
  // The previous (older) version's content, for the diff. Null = no predecessor
  // (oldest version) → the preview shows no diff tint.
  ({String rootBlockId, List<Map<String, dynamic>> blocks})? _prevContent;

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
      final vs = await widget.onList();
      if (!mounted) return;
      setState(() {
        _versions = vs;
        _loading = false;
      });
      // Auto-open the newest version so the pane is never blank.
      if (_selectedId == null && vs.isNotEmpty) {
        _select(vs.first);
      }
    } catch (error) {
      if (!mounted) return;
      setState(() {
        _error = error.toString();
        _loading = false;
      });
    }
  }

  /// Load a version's content (and its predecessor's, for the diff) into the
  /// read-only preview pane. The predecessor is the next-OLDER entry in the
  /// timeline (versions are newest-first).
  Future<void> _select(DocVersion v) async {
    setState(() {
      _selectedId = v.id;
      _previewLoading = true;
      _previewError = null;
      _content = null;
      _prevContent = null;
    });
    final idx = _versions.indexWhere((x) => x.id == v.id);
    final prev = (idx >= 0 && idx + 1 < _versions.length)
        ? _versions[idx + 1]
        : null;
    try {
      final content = await widget.onLoadContent(v.id);
      // Predecessor is best-effort: a failure just drops the diff, not the
      // preview.
      ({String rootBlockId, List<Map<String, dynamic>> blocks})? prevContent;
      if (prev != null) {
        try {
          prevContent = await widget.onLoadContent(prev.id);
        } catch (_) {}
      }
      if (!mounted || _selectedId != v.id) return;
      setState(() {
        _content = content;
        _prevContent = prevContent;
        _previewLoading = false;
      });
    } catch (error) {
      if (!mounted || _selectedId != v.id) return;
      setState(() {
        _previewError = error.toString();
        _previewLoading = false;
      });
    }
  }

  /// Read-only editor nodes for the selected version, tagged with the
  /// block-level diff vs its predecessor. The rule itself lives in
  /// `editor/version_diff.dart` so it is testable.
  List<EditorNode> _previewNodes(
    ({String rootBlockId, List<Map<String, dynamic>> blocks}) content,
  ) => versionDiffNodes(content, _prevContent);

  Future<void> _createCheckpoint() async {
    final l10n = context.l10n;
    final name = await _promptName();
    if (name == null || name.trim().isEmpty) return;
    setState(() => _busy = true);
    try {
      await widget.onCreate(name.trim());
      await _refresh();
      _snack(l10n.versionCheckpointSaved);
    } catch (error) {
      _snack(l10n.versionSaveFailed(error.toString()));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  /// A version's display name: named checkpoints show their label; auto
  /// snapshots (label null) read as "Auto-save" and lean on the timestamp.
  String _displayName(DocVersion v) =>
      v.isAuto ? context.l10n.versionAutoSnapshot : v.label!.trim();

  /// One timeline row, card-shaped (design 08): active = accent border + wash,
  /// which at this width reads far better than Material's tinted-title `selected`.
  Widget _versionRow(BuildContext context, DocVersion v) {
    final selected = v.id == _selectedId;
    return Padding(
      padding: const EdgeInsets.only(bottom: 6),
      child: Material(
        color: selected
            ? MicaTheme.of(context).accent.wash
            : Colors.transparent,
        borderRadius: BorderRadius.circular(10),
        child: InkWell(
          borderRadius: BorderRadius.circular(10),
          onTap: _busy ? null : () => _select(v),
          child: Container(
            decoration: BoxDecoration(
              borderRadius: BorderRadius.circular(10),
              border: Border.all(
                color: selected
                    ? MicaTheme.of(
                        context,
                      ).accent.primary.withValues(alpha: 0.35)
                    : Colors.transparent,
              ),
            ),
            padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 9),
            child: Row(
              children: [
                Icon(
                  v.isAuto ? Icons.schedule : Icons.bookmark,
                  size: 16,
                  color: selected
                      ? MicaTheme.of(context).accent.primary
                      : MicaTheme.of(context).text.faint,
                ),
                const SizedBox(width: 9),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        v.isAuto ? _stamp(v) : _displayName(v),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontSize: 14,
                          fontWeight: v.isAuto
                              ? FontWeight.w400
                              : FontWeight.w600,
                          color: selected
                              ? MicaTheme.of(context).accent.primary
                              : MicaTheme.of(context).text.primary,
                        ),
                      ),
                      ?_rowSubtitle(v),
                    ],
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  /// A version's timestamp in the design-08 relative form: `15:08` today,
  /// 「昨天 22:03」 yesterday, a bare date further back.
  ///
  /// Falls back to the raw string when the stamp does not parse — showing the
  /// unhelpful original beats inventing a date, and beats a blank where a time
  /// belongs.
  String _stamp(DocVersion v) {
    final when = versionTime(v);
    if (when == null) return v.createdAt;
    final l10n = context.l10n;
    final s = versionStamp(when);
    return switch (s.day) {
      VersionDay.today => s.time,
      VersionDay.yesterday => l10n.versionYesterdayAt(s.time),
      VersionDay.earlier => l10n.versionDateShort(s.month, s.dayOfMonth),
    };
  }

  /// Who saved this version, or null when that can't be said usefully.
  ///
  /// Deliberately quiet: an unknown id renders nothing rather than a placeholder
  /// like 「未知用户」 — a row that admits it doesn't know who did this is worse
  /// than a row that doesn't raise the question. Ids go stale legitimately (a
  /// member who left is no longer in the map).
  String? _authorName(DocVersion v) {
    final id = v.createdBy;
    if (id == null) return null;
    final name = widget.authorNames[id]?.trim();
    return (name == null || name.isEmpty) ? null : name;
  }

  /// The second line: an author when there is one to name, the timestamp for a
  /// named checkpoint (whose title is its label, not its time), or both.
  Widget? _rowSubtitle(DocVersion v) {
    final author = _authorName(v);
    final time = v.isAuto ? null : _stamp(v);
    final parts = [?time, ?author];
    if (parts.isEmpty) return null;
    return Text(
      parts.join('  ·  '),
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
    );
  }

  Future<void> _restore(DocVersion version) async {
    final l10n = context.l10n;
    final ok = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(context.l10n.versionRestoreConfirmTitle),
        content: Text(
          context.l10n.versionRestoreConfirmBody(_displayName(version)),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: Text(context.l10n.commonCancel),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: Text(context.l10n.versionRestore),
          ),
        ],
      ),
    );
    if (ok != true) return;
    setState(() => _busy = true);
    try {
      await widget.onRestore(version.id);
      if (!mounted) return;
      _snack(l10n.versionRestored(_displayName(version)));
      Navigator.of(context).pop();
    } catch (error) {
      _snack(l10n.versionRestoreFailed(error.toString()));
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<String?> _promptName() {
    final controller = TextEditingController();
    return showDialog<String>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(context.l10n.versionSaveCheckpoint),
        content: TextField(
          controller: controller,
          autofocus: true,
          decoration: InputDecoration(hintText: context.l10n.versionNameHint),
          onSubmitted: (v) => Navigator.pop(context, v),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: Text(context.l10n.commonCancel),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, controller.text),
            child: Text(context.l10n.commonSave),
          ),
        ],
      ),
    );
  }

  void _snack(String message) {
    if (!mounted) return;
    ScaffoldMessenger.of(
      context,
    ).showSnackBar(SnackBar(content: Text(message)));
  }

  DocVersion? get _selected =>
      _versions.where((v) => v.id == _selectedId).firstOrNull;

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    return Dialog(
      insetPadding: const EdgeInsets.symmetric(horizontal: 40, vertical: 24),
      clipBehavior: Clip.antiAlias,
      child: SizedBox(
        width: 980,
        height: 640,
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Expanded(child: _buildPreview(l10n)),
            const VerticalDivider(width: 1),
            SizedBox(width: 300, child: _buildTimeline(l10n)),
          ],
        ),
      ),
    );
  }

  Widget _buildPreview(AppLocalizations l10n) {
    if (_selectedId == null) {
      return Center(
        child: Text(l10n.versionEmpty, textAlign: TextAlign.center),
      );
    }
    if (_previewLoading) {
      return const Center(child: CircularProgressIndicator());
    }
    if (_previewError != null) {
      return Center(child: Text(l10n.versionLoadFailed(_previewError!)));
    }
    final content = _content;
    if (content == null) return const SizedBox.shrink();
    // The SAME editor in canEdit:false — reused, not re-rendered (P-A hardening
    // hides caret/IME/toolbars). Isolated: it renders the version's own blocks,
    // never the live document, so it can't affect the open page.
    // Computed ONCE and handed to both the header and the body: the legend used
    // to key off "there is a predecessor" while the tint keyed off "some block
    // actually differs", so two identical versions produced a legend over a
    // page with nothing coloured — a legend describing colours that were not
    // there. Same list, same verdict.
    final nodes = _previewNodes(content);
    final hasDiff = nodes.any((n) => n.diffStatus != null);
    return Column(
      children: [
        // No predecessor (oldest version) → no legend at all; a predecessor but
        // no differences → say so instead of promising colours.
        if (_prevContent != null && !hasDiff)
          Container(
            width: double.infinity,
            padding: const EdgeInsets.symmetric(horizontal: 32, vertical: 6),
            color: Theme.of(
              context,
            ).colorScheme.surfaceContainerHighest.withValues(alpha: 0.4),
            child: Text(
              l10n.versionDiffNone,
              style: TextStyle(
                fontSize: 12,
                color: MicaTheme.of(context).text.muted,
              ),
            ),
          ),
        if (_prevContent != null && hasDiff)
          Container(
            width: double.infinity,
            padding: const EdgeInsets.symmetric(horizontal: 32, vertical: 6),
            color: Theme.of(
              context,
            ).colorScheme.surfaceContainerHighest.withValues(alpha: 0.4),
            child: Row(
              children: [
                _legendDot(
                  MicaTheme.of(context).status.success,
                  l10n.versionDiffAdded,
                ),
                const SizedBox(width: 14),
                _legendDot(
                  MicaTheme.of(context).status.warning,
                  l10n.versionDiffChanged,
                ),
                const SizedBox(width: 14),
                _legendDot(
                  MicaTheme.of(context).status.danger,
                  l10n.versionDiffRemoved,
                ),
              ],
            ),
          ),
        Expanded(child: _buildPreviewBody(content, nodes)),
      ],
    );
  }

  Widget _legendDot(Color color, String label) => Row(
    mainAxisSize: MainAxisSize.min,
    children: [
      Container(
        width: 10,
        height: 10,
        decoration: BoxDecoration(
          color: color,
          borderRadius: BorderRadius.circular(2),
        ),
      ),
      const SizedBox(width: 5),
      Text(label, style: const TextStyle(fontSize: 12)),
    ],
  );

  Widget _buildPreviewBody(
    ({String rootBlockId, List<Map<String, dynamic>> blocks}) content,
    List<EditorNode> nodes,
  ) {
    return SingleChildScrollView(
      padding: const EdgeInsets.symmetric(horizontal: 32, vertical: 24),
      child: Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 720),
          child: MicaEditor(
            key: ValueKey('version-preview-$_selectedId'),
            appearance: EditorAppearance(tokens: MicaTheme.of(context)),
            rootBlockId: content.rootBlockId,
            nodes: nodes,
            version: 0,
            canEdit: false,
            onApplyOperations: (_) async {},
            onLoadImageBytes: widget.onLoadImageBytes,
            onResolveImageUrls: widget.onResolveImageUrls,
          ),
        ),
      ),
    );
  }

  Widget _buildTimeline(AppLocalizations l10n) {
    return Column(
      children: [
        // Header: title + save-checkpoint + close.
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 12, 8, 8),
          child: Row(
            children: [
              Text(
                l10n.versionHistoryTitle,
                style: const TextStyle(fontWeight: FontWeight.w600),
              ),
              const Spacer(),
              IconButton(
                tooltip: l10n.versionSaveCheckpoint,
                onPressed: _busy ? null : _createCheckpoint,
                icon: const Icon(Icons.bookmark_add_outlined, size: 20),
              ),
              IconButton(
                tooltip: l10n.commonClose,
                onPressed: () => Navigator.of(context).pop(),
                icon: const Icon(Icons.close, size: 20),
              ),
            ],
          ),
        ),
        const Divider(height: 1),
        Expanded(
          child: _loading
              ? const Center(child: CircularProgressIndicator())
              : _error != null
              ? Center(child: Text(l10n.versionLoadFailed(_error!)))
              : _versions.isEmpty
              ? Center(
                  child: Text(l10n.versionEmpty, textAlign: TextAlign.center),
                )
              : ListView(
                  padding: const EdgeInsets.symmetric(horizontal: 8),
                  children: [
                    // 今天 / 更早 (design 08). A flat list of bare timestamps
                    // gave no sense of when you were looking at.
                    for (final section in groupVersions(_versions)) ...[
                      Padding(
                        padding: const EdgeInsets.only(
                          left: 6,
                          top: 10,
                          bottom: 4,
                        ),
                        child: Text(
                          section.day == VersionDay.today
                              ? context.l10n.versionGroupToday
                              : context.l10n.versionGroupEarlier,
                          style: TextStyle(
                            fontSize: 11,
                            fontWeight: FontWeight.w600,
                            letterSpacing: 0.6,
                            color: MicaTheme.of(context).text.faint,
                          ),
                        ),
                      ),
                      for (final v in section.items) _versionRow(context, v),
                    ],
                  ],
                ),
        ),
        const Divider(height: 1),
        // Restore bar — acts on the previewed version (bottom-right, AFFiNE/
        // Notion shape).
        Padding(
          padding: const EdgeInsets.all(12),
          child: Row(
            mainAxisAlignment: MainAxisAlignment.end,
            children: [
              FilledButton(
                onPressed: (_busy || _selected == null)
                    ? null
                    : () => _restore(_selected!),
                child: Text(l10n.versionRestore),
              ),
            ],
          ),
        ),
      ],
    );
  }
}
