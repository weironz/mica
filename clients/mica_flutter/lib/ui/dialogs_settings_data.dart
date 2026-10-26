part of '../main.dart';

// Data tab: export stats, mirror cache, import history.
extension _SettingsDataTab on _SettingsDialogState {
  /// Drop the mirrored half of the on-device store, after saying what that costs.
  Future<void> _clearMirrorCache() async {
    final l10n = context.l10n;
    final before = _cacheStats;
    final ok = await showDestructiveConfirm(
      context,
      title: l10n.cacheClearConfirmTitle,
      body: l10n.cacheClearConfirmBody,
      confirmLabel: l10n.cacheClearConfirm,
      cancelLabel: l10n.commonCancel,
      // Recoverable: reconnecting re-caches all of it. See destructive_confirm.
      destructive: false,
    );
    if (!ok || !mounted) return;
    setUi(() => _cacheBusy = true);
    try {
      final result = await widget.onClearMirrorCache!();
      if (!mounted) return;
      final freed = (before?.mirroredBytes ?? 0) - result.stats.mirroredBytes;
      final parts = <String>[l10n.cacheCleared(formatBytes(freed))];
      if (result.retainedDocs > 0) {
        parts.add(l10n.cacheRetainedDocs(result.retainedDocs));
      }
      if (result.quarantinedOrigins.isNotEmpty) {
        parts.add(l10n.cacheQuarantinedBody(result.quarantinedOrigins.length));
      }
      setUi(() {
        _cacheStats = result.stats;
        // The freed figure is measured, not assumed: a blob that could not be
        // unlinked is still counted in `after`, so this never overstates.
        _cacheMsg = parts.join('\n');
      });
      _loadQuarantine(force: true);
    } catch (error) {
      if (mounted) setUi(() => _cacheMsg = error.toString());
    } finally {
      if (mounted) setUi(() => _cacheBusy = false);
    }
  }

  /// Load the quarantined legacy list once (and after a clear).
  void _loadQuarantine({bool force = false}) {
    final load = widget.onLoadQuarantinedCaches;
    if (load == null) return;
    if (!force && _quarantineAsked) return;
    _quarantineAsked = true;
    load()
        .then((q) {
          if (mounted) setUi(() => _quarantine = q);
        })
        .catchError((_) {});
  }
  /// Export one quarantined legacy workspace as a Markdown ZIP. No name is
  /// displayed or used as the file name — only the page count, so the current
  /// account never learns another account's titles.
  Future<void> _exportQuarantine(
    ({String origin, String workspaceId, int pages}) item,
  ) async {
    final export = widget.onExportQuarantinedCache;
    if (export == null) return;
    final l10n = context.l10n;
    try {
      final bytes = await export(item.origin, item.workspaceId);
      if (bytes == null || bytes.isEmpty) {
        throw StateError(l10n.exportEmptyContent);
      }
      downloadImage(bytes, 'legacy-cache-${item.workspaceId}.zip', 'application/zip');
      if (mounted) setUi(() => _cacheMsg = l10n.cacheQuarantinedExport);
    } catch (error) {
      if (mounted) setUi(() => _cacheMsg = error.toString());
    }
  }

  /// Ask for the on-device numbers once.
  void _ensureCacheStats() {
    final load = widget.onLoadCacheStats;
    if (load == null || _cacheStatsAsked) return;
    _cacheStatsAsked = true;
    load()
        .then((st) {
          if (mounted) setUi(() => _cacheStats = st);
        })
        .catchError((_) {});
    _loadQuarantine();
  }

  /// Ask for the export numbers once, the first time this tab is built. Failure
  /// leaves the meta line absent rather than surfacing an error — a count nobody
  /// can act on is not worth interrupting the screen for.
  void _ensureExportStats() {
    final load = widget.onLoadExportStats;
    if (load == null || _exportStatsAsked) return;
    _exportStatsAsked = true;
    load()
        .then((st) {
          if (mounted) setUi(() => _exportStats = st);
        })
        .catchError((_) {});
  }

  /// Past imports, read from the server.
  ///
  /// It lives in Settings for the reason Notion, Outline and Slack put theirs
  /// there: a long import outlives the screen that started it, so the progress
  /// has to be somewhere the user can find AGAIN — after closing the tab, after
  /// a restart, a week later.
  ///
  /// Loaded on first build rather than with the dialog: most visits to Settings
  /// are not about imports, and this is a round trip.
  List<Widget> _importHistorySection(BuildContext context) {
    final load = widget.onLoadImportHistory;
    if (load == null) return const [];
    if (_importHistory == null && !_importHistoryLoading) {
      _importHistoryLoading = true;
      unawaited(_loadImportHistory(load));
    }
    final entries = _importHistory;
    if (entries == null || entries.isEmpty) return const [];
    return [
      const SizedBox(height: 18),
      MicaEyebrow(context.l10n.importHistoryTitle),
      const SizedBox(height: 10),
      for (final e in entries.take(8)) _importHistoryRow(context, e),
    ];
  }

  /// Fire-and-forget: the section renders immediately and fills in when the
  /// round trip lands.
  ///
  /// try/catch rather than `.catchError`, which is what this was. `catchError`
  /// takes an untyped `Function`, so nothing checked that the handler returned
  /// the right thing — and it did not: it returned `<ImportHistoryEntry>[]` to
  /// a `Future<Null>`. On the ERROR path, the path whose entire job is to fail
  /// quietly, that throws a TypeError as an unhandled async error. The failure
  /// mode was the exact opposite of the intent written beside it.
  ///
  /// The analyzer did say so (`invalid_return_type_for_catch_error`), for long
  /// enough that nobody remembers. That is the argument for warnings being
  /// fatal in the release gate, not the argument for reading them more
  /// carefully — see scripts/release-check.sh.
  Future<void> _loadImportHistory(
    Future<List<ImportHistoryEntry>> Function() load,
  ) async {
    try {
      final entries = await load();
      if (!mounted) return;
      setUi(() {
        _importHistory = entries;
        _importHistoryLoading = false;
      });
    } catch (_) {
      // A history that cannot be read is not worth an error banner over an
      // import button that still works. It shows as empty.
      if (!mounted) return;
      setUi(() {
        _importHistory = const [];
        _importHistoryLoading = false;
      });
    }
  }

  Widget _importHistoryRow(BuildContext context, ImportHistoryEntry entry) {
    final tokens = MicaTheme.of(context);
    final job = entry.job;
    final (IconData icon, Color color, String label) = switch (job.status) {
      'done' => (
        Icons.check_circle,
        tokens.status.success,
        context.l10n.importStatusDone,
      ),
      'error' => (
        Icons.error_outline,
        tokens.status.danger,
        context.l10n.importStatusError,
      ),
      'cancelled' => (
        Icons.cancel_outlined,
        tokens.text.muted,
        context.l10n.importStatusCancelled,
      ),
      // Its own row style on purpose: the pages that landed are real, so this
      // is not a failure — but the archive is not all in either.
      'interrupted' => (
        Icons.pause_circle_outline,
        tokens.status.warning,
        context.l10n.importStatusInterrupted,
      ),
      _ => (
        Icons.hourglass_empty,
        tokens.text.muted,
        context.l10n.importStatusRunning,
      ),
    };
    final when = entry.startedAt;
    return Padding(
      padding: const EdgeInsets.only(bottom: 8),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Icon(icon, size: 16, color: color),
          const SizedBox(width: 8),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  context.l10n.importHistoryProgress(
                    label,
                    job.done,
                    job.total,
                  ),
                  style: TextStyle(fontSize: 13, color: tokens.text.primary),
                ),
                if (when != null)
                  Text(
                    _formatStamp(when),
                    style: TextStyle(fontSize: 11.5, color: tokens.text.muted),
                  ),
                if (job.status == 'interrupted')
                  Text(
                    context.l10n.importInterruptedHelp,
                    style: TextStyle(
                      fontSize: 11.5,
                      color: tokens.status.warning,
                    ),
                  ),
                if (job.error != null && job.error!.isNotEmpty)
                  Text(
                    job.error!,
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      fontSize: 11.5,
                      color: tokens.status.danger,
                    ),
                  ),
                if (job.skippedTotal > 0)
                  Text(
                    context.l10n.importSkippedCount(job.skippedTotal),
                    style: TextStyle(fontSize: 11.5, color: tokens.text.muted),
                  ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  String _formatStamp(DateTime t) =>
      '${t.year}-${t.month.toString().padLeft(2, '0')}-${t.day.toString().padLeft(2, '0')} '
      '${t.hour.toString().padLeft(2, '0')}:${t.minute.toString().padLeft(2, '0')}';

  List<Widget> _dataSection(BuildContext context) => [
    const SizedBox(height: 12),
    Text(
      context.l10n.dataImportDescription,
      style: TextStyle(
        color: MicaTheme.of(context).text.muted,
        fontSize: 12.5,
        height: 1.6,
      ),
    ),
    const SizedBox(height: 14),
    // A CLICK zone, not a drop zone: the app accepts no drops (see MicaPickZone),
    // so the copy says 「选择…」 and there is no 「拖到这里」 anywhere.
    if (widget.onImportWorkspace case final import?)
      MicaPickZone(
        icon: Icons.upload_file_outlined,
        title: context.l10n.dataImportButton,
        subtitle: context.l10n.dataImportZoneHint,
        onTap: import,
      ),
    ..._importHistorySection(context),
    if (widget.onExportAllWorkspaces != null) ...[
      const SizedBox(height: 18),
      MicaEyebrow(context.l10n.commonExport),
      const SizedBox(height: 10),
      // One bordered row rather than a loose button, so export reads as its own
      // thing and not as a second import action.
      Container(
        decoration: BoxDecoration(
          border: Border.all(color: MicaTheme.of(context).border.subtle),
          borderRadius: BorderRadius.circular(12),
        ),
        padding: const EdgeInsets.fromLTRB(14, 10, 10, 10),
        child: Row(
          children: [
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                mainAxisSize: MainAxisSize.min,
                children: [
                  Text(
                    context.l10n.dataExportAllButton,
                    style: const TextStyle(fontSize: 14),
                  ),
                  // Shown only once the numbers are in. Images are called images
                  // rather than "download size": the archive is a zip, Markdown
                  // compresses hard and images do not, so quoting a total here
                  // would be wrong by an amount that depends on the user's own
                  // content.
                  if (_exportStats case final st?)
                    Padding(
                      padding: const EdgeInsets.only(top: 3),
                      child: Text(
                        context.l10n.dataExportAllMeta(
                          st.workspaces,
                          st.pages,
                          formatBytes(st.imageBytes),
                        ),
                        style: TextStyle(
                          fontSize: 12,
                          color: MicaTheme.of(context).text.faint,
                        ),
                      ),
                    ),
                ],
              ),
            ),
            const SizedBox(width: 10),
            OutlinedButton(
              onPressed: () => widget.onExportAllWorkspaces!(),
              child: Text(context.l10n.commonExport),
            ),
          ],
        ),
      ),
    ],
    if (_cacheStats case final c?) ...[
      const SizedBox(height: 18),
      MicaEyebrow(context.l10n.cacheTitle),
      const SizedBox(height: 10),
      // Two lines, never one total. The store holds re-downloadable mirrors AND
      // pages that exist nowhere else; a single "已缓存 X" would invite someone
      // to reclaim the only copy of something. Each line is only drawn when it
      // has content, so a cloud-only user never sees a 「本地独有 0」 row.
      if (c.mirroredPages > 0 || c.mirroredBytes > 0)
        Row(
          children: [
            Expanded(
              child: Text(
                context.l10n.cacheMirrored(
                  c.mirroredPages,
                  formatBytes(c.mirroredBytes),
                ),
                style: TextStyle(
                  fontSize: 12.5,
                  color: MicaTheme.of(context).text.muted,
                ),
              ),
            ),
            // Offered on the mirror line only. The local-only line below has no
            // button and never will: there is nowhere to re-download it from, so
            // a "reclaim" control there would be a delete button wearing the
            // wrong word.
            if (widget.onClearMirrorCache != null)
              TextButton(
                onPressed: _cacheBusy ? null : _clearMirrorCache,
                child: Text(context.l10n.cacheClearButton),
              ),
          ],
        ),
      if (c.localOnlyPages > 0 || c.localOnlyBytes > 0)
        Padding(
          padding: const EdgeInsets.only(top: 4),
          child: Text(
            context.l10n.cacheLocalOnly(
              c.localOnlyPages,
              formatBytes(c.localOnlyBytes),
            ),
            style: TextStyle(
              fontSize: 12.5,
              color: MicaTheme.of(context).text.muted,
            ),
          ),
        ),
      if (_cacheMsg case final msg?) ...[
        const SizedBox(height: 8),
        Text(
          msg,
          style: TextStyle(
            fontSize: 12.5,
            color: MicaTheme.of(context).text.muted,
          ),
        ),
      ],
    ],
    if (_quarantine.isNotEmpty) ...[
      const SizedBox(height: 18),
      MicaEyebrow(context.l10n.cacheQuarantinedTitle),
      const SizedBox(height: 8),
      Text(
        context.l10n.cacheQuarantinedBody(_quarantine.length),
        style: TextStyle(
          fontSize: 12.5,
          height: 1.6,
          color: MicaTheme.of(context).text.muted,
        ),
      ),
      const SizedBox(height: 6),
      for (final item in _quarantine)
        Row(
          children: [
            Expanded(
              child: Text(
                context.l10n.cacheQuarantinedPages(item.pages),
                style: TextStyle(
                  fontSize: 12.5,
                  color: MicaTheme.of(context).text.muted,
                ),
              ),
            ),
            if (widget.onExportQuarantinedCache != null)
              TextButton(
                onPressed: () => _exportQuarantine(item),
                child: Text(context.l10n.cacheQuarantinedExport),
              ),
          ],
        ),
    ],
    const SizedBox(height: 14),
    Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Icon(
          Icons.info_outline,
          size: 14,
          color: MicaTheme.of(context).text.faint,
        ),
        const SizedBox(width: 7),
        Expanded(
          child: Text(
            context.l10n.dataExportTip,
            style: TextStyle(
              fontSize: 12.5,
              height: 1.6,
              color: MicaTheme.of(context).text.faint,
            ),
          ),
        ),
      ],
    ),
  ];
}
