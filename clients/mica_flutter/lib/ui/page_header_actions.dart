// The cluster at the right end of the page header row: when the page was last
// edited, share, copy link, favorite, and the page menu — Notion's order.
//
// A library of its own rather than more of `main_workspace_editor_pane.dart`:
// that file is a part of main.dart, whose line budget only goes down, and none
// of this needs the shell's private state — it takes plain values and
// callbacks, which is also what lets it be widget-tested without the
// ninety-parameter WorkspaceView around it.

import 'dart:async';

import 'package:flutter/material.dart';

import '../l10n/app_localizations.dart';
import '../l10n/locale_controller.dart';
import 'favorites.dart';
import 'theme_tokens.dart';
import 'typography.dart';

/// The internal link for a page — the same `mica://page/<viewId>` form the
/// editor writes for a page link, so pasting it into a page links back here.
String pageLinkFor(String viewId) => 'mica://page/$viewId';

/// The label's time half: 刚刚 / N 分钟前 / 11月11日 (with the year once it is
/// not this year's).
///
/// Three forms only, by request. There is deliberately no "N hours ago": past
/// the hour the date is what people use to place an edit.
String lastEditedWhen(DateTime when, AppLocalizations l10n, {DateTime? now}) {
  final ref = now ?? DateTime.now();
  final local = when.toLocal();
  final diff = ref.difference(local);
  if (diff.isNegative || diff.inMinutes < 1) return l10n.homeJustNow;
  if (diff.inMinutes < 60) return l10n.homeMinutesAgo(diff.inMinutes);
  if (local.year == ref.year) {
    return l10n.pageEditedDate(local.month, local.day);
  }
  return l10n.pageEditedDateYear(local.year, local.month, local.day);
}

/// The latest of the timestamps that can say a page changed, or null when none
/// can.
///
/// More than one because none of them is the whole story: the view's
/// `updated_at` moves on rename, the document's on a body write, and an edit
/// made in this window is known here before any server round trip.
DateTime? latestEdit(Iterable<DateTime?> stamps) {
  DateTime? best;
  for (final s in stamps) {
    if (s != null && (best == null || s.isAfter(best))) best = s;
  }
  return best;
}

/// Edits made in this window, per page, so the label can say 刚刚 the moment
/// you type rather than after the next reload.
///
/// Notifies at most once per [_quiet] per page: the label's resolution is a
/// minute, and a notify per keystroke would rebuild the header for nothing.
class LocalEditClock extends ChangeNotifier {
  LocalEditClock({DateTime Function()? clock}) : _clock = clock ?? DateTime.now;

  final DateTime Function() _clock;
  final Map<String, DateTime> _at = {};
  final Map<String, DateTime> _notifiedAt = {};
  static const _quiet = Duration(seconds: 20);

  DateTime? at(String viewId) => _at[viewId];

  void mark(String viewId) {
    final now = _clock();
    final prev = _notifiedAt[viewId];
    _at[viewId] = now;
    if (prev == null || now.difference(prev) >= _quiet) {
      _notifiedAt[viewId] = now;
      notifyListeners();
    }
  }
}

/// Values of the page menu's entries, shared with the handler that acts on
/// them.
abstract final class PageMenuAction {
  static const copyMarkdown = 'copy-md';
  static const exportZip = 'export-zip';
  static const exportHtml = 'export-html';
  static const exportPdf = 'export-pdf';
  static const importMarkdown = 'import-md';
  static const versionHistory = 'version-history';
  static const restoreCheckpoint = 'restore-checkpoint';
}

class PageHeaderActions extends StatelessWidget {
  const PageHeaderActions({
    super.key,
    required this.viewId,
    required this.workspaceId,
    required this.favorites,
    required this.editClock,
    required this.serverStamps,
    required this.onCopyLink,
    required this.onMenu,
    this.onShare,
    this.hasVersionHistory = false,
    this.hasRestoreCheckpoint = false,
    this.compact = false,
  });

  final String viewId;
  final String workspaceId;
  final FavoritePages favorites;
  final LocalEditClock editClock;

  /// What the server last said about this page's edit time — see
  /// [latestEdit].
  final List<DateTime?> serverStamps;

  /// Null where a page cannot be shared (the local world).
  final Future<void> Function()? onShare;
  final Future<bool> Function(String link) onCopyLink;
  final void Function(String action) onMenu;
  final bool hasVersionHistory;
  final bool hasRestoreCheckpoint;

  /// A narrow pane: drop the time label and the share caption so the row
  /// keeps fitting; every action stays reachable.
  final bool compact;

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        if (!compact)
          ListenableBuilder(
            listenable: editClock,
            builder: (context, _) => LastEditedLabel(
              editedAt: latestEdit([...serverStamps, editClock.at(viewId)]),
            ),
          ),
        if (onShare != null && compact)
          IconButton(
            tooltip: l10n.pageShareButton,
            visualDensity: VisualDensity.compact,
            onPressed: onShare,
            icon: const Icon(Icons.lock_outline, size: 19),
          )
        else if (onShare != null)
          TextButton.icon(
            onPressed: onShare,
            style: TextButton.styleFrom(
              visualDensity: VisualDensity.compact,
              padding: const EdgeInsets.symmetric(horizontal: 8),
              foregroundColor: MicaTheme.of(context).text.primary,
              textStyle: MicaTypography.caption,
            ),
            icon: const Icon(Icons.lock_outline, size: 17),
            label: Text(l10n.pageShareButton),
          ),
        CopyLinkButton(link: pageLinkFor(viewId), onCopy: onCopyLink),
        ListenableBuilder(
          listenable: favorites,
          builder: (context, _) {
            final on = favorites.contains(workspaceId, viewId);
            return IconButton(
              tooltip: on ? l10n.pageRemoveFavorite : l10n.pageAddFavorite,
              visualDensity: VisualDensity.compact,
              onPressed: () => favorites.toggle(workspaceId, viewId),
              icon: on
                  ? const Icon(
                      Icons.star_rounded,
                      size: 21,
                      color: kFavoriteStarColor,
                    )
                  : const Icon(Icons.star_outline_rounded, size: 21),
            );
          },
        ),
        PopupMenuButton<String>(
          tooltip: l10n.pageMenu,
          padding: EdgeInsets.zero,
          icon: const Icon(Icons.more_horiz, size: 20),
          onSelected: onMenu,
          itemBuilder: (context) => [
            // Copy gets its own group above the exports: it is the cheap,
            // frequent one, while everything below produces a file.
            _item(
              PageMenuAction.copyMarkdown,
              Icons.copy_all_outlined,
              l10n.pageCopyContent,
            ),
            const PopupMenuDivider(),
            // One export, always a ZIP: a lone .md cannot carry its images.
            _item(
              PageMenuAction.exportZip,
              Icons.folder_zip_outlined,
              l10n.rowExportZipImages,
            ),
            _item(
              PageMenuAction.exportHtml,
              Icons.html_outlined,
              l10n.rowExportHtml,
            ),
            // Desktop prints through WebView2; web hands the same HTML to the
            // browser's print dialog. Available on both.
            _item(
              PageMenuAction.exportPdf,
              Icons.picture_as_pdf_outlined,
              l10n.rowExportPdf,
            ),
            const PopupMenuDivider(),
            _item(
              PageMenuAction.importMarkdown,
              Icons.upload_file_outlined,
              l10n.pageImportMarkdown,
            ),
            if (hasVersionHistory) ...[
              const PopupMenuDivider(),
              _item(
                PageMenuAction.versionHistory,
                Icons.history,
                l10n.versionHistoryTitle,
              ),
            ],
            if (hasRestoreCheckpoint) ...[
              const PopupMenuDivider(),
              _item(
                PageMenuAction.restoreCheckpoint,
                Icons.restore_outlined,
                l10n.pageRestoreCheckpoint,
              ),
            ],
          ],
        ),
      ],
    );
  }

  static PopupMenuItem<String> _item(
    String value,
    IconData icon,
    String label,
  ) => PopupMenuItem(
    height: 30,
    value: value,
    child: ListTile(
      minTileHeight: 30,
      minVerticalPadding: 0,
      dense: true,
      contentPadding: EdgeInsets.zero,
      leading: Icon(icon),
      title: Text(label),
    ),
  );
}

/// 「上次编辑 8 分钟前」. Renders nothing while no timestamp is known — a guessed
/// "just now" would be a lie the user can check.
///
/// Re-reads the clock every half minute, so an open page ages from 刚刚 to
/// 1 分钟前 without anything else having to rebuild it.
class LastEditedLabel extends StatefulWidget {
  const LastEditedLabel({super.key, required this.editedAt, this.now});

  final DateTime? editedAt;

  /// Test seam for the clock.
  final DateTime Function()? now;

  @override
  State<LastEditedLabel> createState() => _LastEditedLabelState();
}

class _LastEditedLabelState extends State<LastEditedLabel> {
  Timer? _tick;

  @override
  void initState() {
    super.initState();
    _tick = Timer.periodic(const Duration(seconds: 30), (_) {
      if (mounted && widget.editedAt != null) setState(() {});
    });
  }

  @override
  void dispose() {
    _tick?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final at = widget.editedAt;
    if (at == null) return const SizedBox.shrink();
    final l10n = context.l10n;
    final theme = MicaTheme.of(context);
    final local = at.toLocal();
    String two(int n) => n.toString().padLeft(2, '0');
    return Tooltip(
      message:
          '${local.year}-${two(local.month)}-${two(local.day)} '
          '${two(local.hour)}:${two(local.minute)}',
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 8),
        child: Text(
          l10n.pageLastEdited(
            lastEditedWhen(at, l10n, now: widget.now?.call()),
          ),
          maxLines: 1,
          style: MicaTypography.caption.copyWith(color: theme.text.faint),
        ),
      ),
    );
  }
}

/// Link icon that turns into a check for a moment after a successful copy —
/// the same quiet confirmation the breadcrumb's copy-path uses. A failed copy
/// is reported by [onCopy] itself, in words.
class CopyLinkButton extends StatefulWidget {
  const CopyLinkButton({super.key, required this.link, required this.onCopy});

  final String link;
  final Future<bool> Function(String link) onCopy;

  @override
  State<CopyLinkButton> createState() => _CopyLinkButtonState();
}

class _CopyLinkButtonState extends State<CopyLinkButton> {
  Timer? _reset;
  bool _copied = false;

  @override
  void dispose() {
    _reset?.cancel();
    super.dispose();
  }

  Future<void> _copy() async {
    final ok = await widget.onCopy(widget.link);
    if (!mounted || !ok) return;
    setState(() => _copied = true);
    _reset?.cancel();
    _reset = Timer(const Duration(milliseconds: 1500), () {
      if (mounted) setState(() => _copied = false);
    });
  }

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    return IconButton(
      tooltip: _copied ? l10n.pageLinkCopied : l10n.pageCopyLink,
      visualDensity: VisualDensity.compact,
      onPressed: _copy,
      icon: _copied
          ? Icon(
              Icons.check,
              size: 19,
              color: MicaTheme.of(context).status.success,
            )
          : const Icon(Icons.link, size: 20),
    );
  }
}
