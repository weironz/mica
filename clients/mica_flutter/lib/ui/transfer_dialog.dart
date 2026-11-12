// Move / copy pages to another workspace: destination picker (searchable,
// gh#165), optional folder, dry-run preview, then commit.
// Its own library since 2026-11-08 (was a part of main.dart).
library;

import 'typography.dart';

import 'package:flutter/material.dart';

import '../api/models.dart';
import '../l10n/locale_controller.dart';
import 'theme_tokens.dart';

/// Move / copy a page-or-folder subtree into another cloud workspace. Picking a
/// destination runs a server dry-run (report only, no mutation) so the user
/// sees the counts and any breaking links BEFORE committing. Confirm runs the
/// real transfer and pops with the report + destination name for the caller to
/// refresh + snackbar.
///
/// The destination can be a FOLDER in the target workspace, not only its root:
/// picking a workspace loads its folders and offers them as a second dropdown.
/// Landing everything at the root was the v1 behaviour and is still the default
/// choice, so the extra control costs nothing to ignore.
class TransferDialog extends StatefulWidget {
  const TransferDialog({
    super.key,
    required this.copy,
    required this.destinations,
    required this.onTransfer,
    required this.onLoadFolders,
    this.itemCount = 1,
  });

  /// true = copy (source kept); false = move (source soft-deleted after copy).
  final bool copy;

  /// Cloud workspaces the subtree can go to — the source is already excluded.
  final List<({String id, String name})> destinations;

  /// How many roots are being transferred. Only affects wording; the caller has
  /// already bound the actual ids into [onTransfer].
  final int itemCount;

  /// Runs one transfer against the picked destination. [dryRun] true = preview
  /// (no mutation); false = commit. Source workspace + views + move/copy are
  /// bound by the caller.
  final Future<TransferReport> Function({
    required String destWorkspaceId,
    required String? parentViewId,
    required bool dryRun,
  })
  onTransfer;

  /// Folders of [destWorkspaceId], flattened depth-first for a dropdown.
  ///
  /// Loaded per destination rather than up front: there can be many workspaces
  /// and the user picks one, so fetching every tree to populate a control they
  /// may never open would be the expensive half of this dialog.
  final Future<List<({String id, String name, int depth})>> Function(
    String destWorkspaceId,
  )
  onLoadFolders;

  @override
  State<TransferDialog> createState() => _TransferDialogState();
}

class _TransferDialogState extends State<TransferDialog> {
  String? _destId;
  TransferReport? _preview; // dry-run result for the current destination
  bool _loadingPreview = false;
  bool _submitting = false;
  String? _error;

  /// Folders of the picked destination; null = not loaded (or still loading).
  List<({String id, String name, int depth})>? _folders;

  /// Chosen destination folder; null = the destination's root.
  String? _parentViewId;

  ({String id, String name})? get _dest {
    final id = _destId;
    if (id == null) return null;
    return widget.destinations.where((d) => d.id == id).firstOrNull;
  }

  Future<void> _selectDest(String? id) async {
    if (id == null || id == _destId) return;
    setState(() {
      _destId = id;
      _preview = null;
      _error = null;
      _folders = null;
      // A folder id belongs to the workspace it came from; carrying it across
      // would send the server a parent it will (correctly) refuse.
      _parentViewId = null;
      _loadingPreview = true;
    });
    // Folders are a convenience — failing to list them must not block the
    // transfer, which works perfectly well into the root.
    widget.onLoadFolders(id).then((folders) {
      if (!mounted || _destId != id) return;
      setState(() => _folders = folders);
    }, onError: (_) {});
    await _runPreview(id, null);
  }

  Future<void> _selectParent(String? parentViewId) async {
    final id = _destId;
    if (id == null || parentViewId == _parentViewId) return;
    setState(() {
      _parentViewId = parentViewId;
      _preview = null;
      _error = null;
      _loadingPreview = true;
    });
    await _runPreview(id, parentViewId);
  }

  Future<void> _runPreview(String destId, String? parentViewId) async {
    try {
      final report = await widget.onTransfer(
        destWorkspaceId: destId,
        parentViewId: parentViewId,
        dryRun: true,
      );
      // Superseded by a newer pick — of EITHER control.
      if (!mounted || _destId != destId || _parentViewId != parentViewId) {
        return;
      }
      setState(() {
        _preview = report;
        _loadingPreview = false;
      });
    } catch (error) {
      if (!mounted || _destId != destId || _parentViewId != parentViewId) {
        return;
      }
      setState(() {
        _error = context.l10n.transferFailed(error.toString());
        _loadingPreview = false;
      });
    }
  }

  Future<void> _confirm() async {
    final dest = _dest;
    if (dest == null || _submitting) return;
    setState(() {
      _submitting = true;
      _error = null;
    });
    try {
      final report = await widget.onTransfer(
        destWorkspaceId: dest.id,
        parentViewId: _parentViewId,
        dryRun: false,
      );
      if (!mounted) return;
      Navigator.of(context).pop((report: report, destName: dest.name));
    } catch (error) {
      if (!mounted) return;
      setState(() {
        _error = context.l10n.transferFailed(error.toString());
        _submitting = false;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final report = _preview;
    return AlertDialog(
      title: Text(
        widget.copy ? l10n.transferCopyTitle : l10n.transferMoveTitle,
      ),
      content: SizedBox(
        width: 440,
        child: widget.destinations.isEmpty
            ? Text(l10n.transferNoDestinations)
            : Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  // Type to narrow the list (gh#165): with dozens of
                  // workspaces a plain dropdown meant scrolling for the one you
                  // already knew the name of. The field filters by name as you
                  // type; arrows + Enter or a click pick.
                  DropdownMenu<String>(
                    key: const ValueKey('transfer-destination'),
                    initialSelection: _destId,
                    enabled: !_submitting,
                    expandedInsets: EdgeInsets.zero,
                    enableFilter: true,
                    requestFocusOnTap: true,
                    menuHeight: 320,
                    label: Text(l10n.transferPickWorkspace),
                    hintText: l10n.transferSearchWorkspace,
                    dropdownMenuEntries: [
                      for (final d in widget.destinations)
                        DropdownMenuEntry(value: d.id, label: d.name),
                    ],
                    onSelected: _selectDest,
                  ),
                  // Second control: WHERE in that workspace. Only once a
                  // destination is picked and its folders arrived — an empty
                  // dropdown offering nothing but "root" is a control that
                  // says less than the label above it already does.
                  if (_folders != null && _folders!.isNotEmpty) ...[
                    const SizedBox(height: 12),
                    DropdownButtonFormField<String?>(
                      style: Theme.of(context).textTheme.bodyLarge,
                      initialValue: _parentViewId,
                      isExpanded: true,
                      decoration: InputDecoration(
                        labelText: l10n.transferPickFolder,
                        border: const OutlineInputBorder(),
                      ),
                      items: [
                        DropdownMenuItem<String?>(
                          child: Text(l10n.transferDestinationRoot),
                        ),
                        for (final f in _folders!)
                          DropdownMenuItem<String?>(
                            value: f.id,
                            child: Text(
                              // Indented with spaces rather than nested widgets:
                              // a DropdownMenuItem renders in two places (list
                              // and closed field) and padding in the second one
                              // pushes the selected name off-centre.
                              '${'　' * f.depth}${f.name}',
                              overflow: TextOverflow.ellipsis,
                            ),
                          ),
                      ],
                      onChanged: _submitting ? null : _selectParent,
                    ),
                  ],
                  const SizedBox(height: 16),
                  if (_loadingPreview)
                    const Padding(
                      padding: EdgeInsets.symmetric(vertical: 8),
                      child: Center(
                        child: SizedBox(
                          height: 20,
                          width: 20,
                          child: CircularProgressIndicator(strokeWidth: 2),
                        ),
                      ),
                    )
                  else if (report != null) ...[
                    Text(
                      // One preview string was shared by both modes, so a move
                      // confirmed itself with 「将复制 …」 — the exact opposite
                      // of what the button was about to do to the source.
                      widget.copy
                          ? l10n.transferPreview(
                              report.documents,
                              report.folders,
                              report.images,
                            )
                          : l10n.transferMovePreview(
                              report.documents,
                              report.folders,
                              report.images,
                            ),
                      style: MicaTypography.body,
                    ),
                    if (report.danglingLinks.isNotEmpty) ...[
                      const SizedBox(height: 10),
                      _TransferNotice(
                        icon: Icons.link_off,
                        color: MicaTheme.of(context).status.warning,
                        text: l10n.transferDanglingWarning(
                          report.danglingLinks.length,
                        ),
                      ),
                    ],
                  ],
                  // A move can't carry version history (checkpoints stay on the
                  // source; the copy starts fresh) — always warn before a move.
                  if (!widget.copy) ...[
                    const SizedBox(height: 10),
                    _TransferNotice(
                      icon: Icons.info_outline,
                      color: MicaTheme.of(context).text.muted,
                      text: l10n.transferVersionNotice,
                    ),
                  ],
                  if (_error != null) ...[
                    const SizedBox(height: 12),
                    Text(
                      _error!,
                      style: MicaTypography.small.copyWith(
                        color: MicaTheme.of(context).status.danger,
                      ),
                    ),
                  ],
                ],
              ),
      ),
      actions: [
        TextButton(
          onPressed: _submitting ? null : () => Navigator.of(context).pop(),
          child: Text(l10n.commonCancel),
        ),
        FilledButton(
          // Enabled only once a destination is picked and no request is in
          // flight — the confirm re-runs the transfer for real (dryRun: false).
          onPressed: (_dest == null || _submitting || _loadingPreview)
              ? null
              : _confirm,
          child: _submitting
              ? const SizedBox(
                  height: 16,
                  width: 16,
                  child: CircularProgressIndicator(strokeWidth: 2),
                )
              : Text(widget.copy ? l10n.transferCopy : l10n.transferMove),
        ),
      ],
    );
  }
}

/// A small icon + text row for the transfer dialog's inline notices (broken
/// links, version-history caveat). Kept tiny and local — not worth a shared
/// widget.
class _TransferNotice extends StatelessWidget {
  const _TransferNotice({
    required this.icon,
    required this.color,
    required this.text,
  });

  final IconData icon;
  final Color color;
  final String text;

  @override
  Widget build(BuildContext context) {
    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Icon(icon, size: 16, color: color),
        const SizedBox(width: 8),
        Expanded(
          child: Text(text, style: MicaTypography.small.copyWith(color: color)),
        ),
      ],
    );
  }
}
