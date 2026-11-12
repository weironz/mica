import 'typography.dart';
import '../l10n/locale_controller.dart';
import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'copy_button.dart';
import 'theme_tokens.dart';

class ShareDialog extends StatefulWidget {
  const ShareDialog({
    super.key,
    required this.onLoad,
    required this.onEnable,
    required this.onDisable,
    required this.buildUrl,
  });

  final Future<({bool shared, String? token})> Function() onLoad;
  final Future<String> Function() onEnable; // returns the token
  final Future<void> Function() onDisable;
  final String Function(String token) buildUrl;

  @override
  State<ShareDialog> createState() => _ShareDialogState();
}

class _ShareDialogState extends State<ShareDialog> {
  bool _loading = true;
  bool _busy = false;
  String? _error;
  bool _shared = false;
  String? _url;

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
      final status = await widget.onLoad();
      if (!mounted) return;
      setState(() {
        _shared = status.shared;
        _url = status.token == null ? null : widget.buildUrl(status.token!);
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

  Future<void> _toggle(bool on) async {
    final l10n = context.l10n;
    setState(() => _busy = true);
    try {
      if (on) {
        final token = await widget.onEnable();
        if (!mounted) return;
        setState(() {
          _shared = true;
          _url = widget.buildUrl(token);
        });
      } else {
        await widget.onDisable();
        if (!mounted) return;
        setState(() {
          _shared = false;
          _url = null;
        });
      }
    } catch (error) {
      _snack(l10n.shareActionFailed(error.toString()));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<bool> _copy() async {
    final url = _url;
    if (url == null) return false;
    await Clipboard.setData(ClipboardData(text: url));
    return true;
  }

  void _snack(String message) {
    if (!mounted) return;
    ScaffoldMessenger.of(
      context,
    ).showSnackBar(SnackBar(content: Text(message)));
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: Text(context.l10n.shareTitle),
      content: SizedBox(
        width: 440,
        child: _loading
            ? const SizedBox(
                height: 80,
                child: Center(child: CircularProgressIndicator()),
              )
            : _error != null
            ? SizedBox(
                height: 80,
                child: Center(
                  child: Text(context.l10n.shareLoadFailed(_error!)),
                ),
              )
            : Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  SwitchListTile(
                    contentPadding: EdgeInsets.zero,
                    value: _shared,
                    onChanged: _busy ? null : _toggle,
                    title: Text(context.l10n.sharePublicAccess),
                    subtitle: Text(context.l10n.sharePublicAccessSub),
                  ),
                  if (_shared && _url != null) ...[
                    const SizedBox(height: 12),
                    Container(
                      padding: const EdgeInsets.symmetric(
                        horizontal: 12,
                        vertical: 8,
                      ),
                      decoration: BoxDecoration(
                        color: MicaTheme.of(context).surface.raised,
                        borderRadius: BorderRadius.circular(6),
                      ),
                      child: Row(
                        children: [
                          Expanded(
                            child: SelectableText(
                              _url!,
                              maxLines: 1,
                              style: MicaTypography.small,
                            ),
                          ),
                          InlineCopyButton(
                            tooltip: context.l10n.shareCopyLink,
                            size: 18,
                            onCopy: _copy,
                          ),
                        ],
                      ),
                    ),
                  ],
                ],
              ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(context.l10n.commonClose),
        ),
      ],
    );
  }
}

/// Version history for the open (cloud) document: list named checkpoints, pin
/// the current state as a new one, or roll back to an old one. Restore reflects
/// live in the editor via the normal sync path (the server broadcasts it as an
/// update), so there is no manual reload here.
/// Version history as a two-pane modal (AFFiNE/Notion shape): a read-only
/// preview of the selected version on the left, the timeline on the right, a
/// restore action at the bottom. The preview reuses the SAME editor in
/// `canEdit: false` — no separate renderer, no HTML (see version-history-plan).
