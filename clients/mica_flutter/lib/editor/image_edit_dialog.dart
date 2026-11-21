// Standalone image edit / replace dialog extracted from editor_dialogs.dart
// to keep editor.dart library strictly below its file-size ratchet budget.
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../l10n/locale_controller.dart';
import '../ui/copy_button.dart';
import '../ui/theme_tokens.dart';
import '../ui/typography.dart';
import 'file_names.dart';
import 'model.dart' show kMonoFont;

/// Image properties + replace. Owns its own [TextEditingController]: creating it
/// outside and disposing after `showDialog` returns is a trap — the route's exit
/// animation is still building the TextField, and it throws "used after being
/// disposed".
///
/// Pops the url to replace with; `''` when a file upload already handled the
/// replacement (or nothing was typed), null on cancel.
class ImageEditDialog extends StatefulWidget {
  const ImageEditDialog({
    super.key,
    required this.link,
    required this.external,
    required this.reHost,
    this.onUploadReplace,
  });

  final String link;
  final bool external;

  /// Mirrors the app's "re-host pasted images" setting, so the dialog can say
  /// what a pasted link will actually do instead of leaving you to guess.
  final bool reHost;
  final Future<bool> Function()? onUploadReplace;

  @override
  State<ImageEditDialog> createState() => _ImageEditDialogState();
}

class _ImageEditDialogState extends State<ImageEditDialog> {
  final _url = TextEditingController();
  var _busy = false;

  @override
  void dispose() {
    _url.dispose();
    super.dispose();
  }

  Future<void> _upload() async {
    final pick = widget.onUploadReplace;
    if (pick == null || _busy) return;
    setState(() => _busy = true);
    final ok = await pick();
    if (!mounted) return;
    setState(() => _busy = false);
    if (ok) Navigator.pop(context, '');
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: Text(context.l10n.imageEditTitle),
      content: SizedBox(
        width: 520,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                widget.external
                    ? context.l10n.imageExternalWarning
                    : context.l10n.imageStoredPublic,
                style: MicaTypography.caption.copyWith(
                  color: MicaTheme.of(context).text.muted,
                ),
              ),
              const SizedBox(height: 6),
              Container(
                width: double.infinity,
                padding: const EdgeInsets.all(8),
                decoration: BoxDecoration(
                  color: MicaTheme.of(context).editor.codeBg,
                  borderRadius: BorderRadius.circular(6),
                ),
                child: SelectableText(
                  readableUrl(widget.link),
                  maxLines: 3,
                  style: MicaTypography.caption.copyWith(fontFamily: kMonoFont),
                ),
              ),
              Align(
                alignment: Alignment.centerRight,
                child: InlineCopyButton(
                  label: context.l10n.linkCopy,
                  tooltip: context.l10n.linkCopy,
                  size: 16,
                  onCopy: () async {
                    await Clipboard.setData(ClipboardData(text: widget.link));
                    return true;
                  },
                ),
              ),
              const Divider(),
              Text(
                context.l10n.imageReplaceHeading,
                style: const TextStyle(fontWeight: FontWeight.w500),
              ),
              const SizedBox(height: 8),
              OutlinedButton.icon(
                icon: _busy
                    ? const SizedBox(
                        width: 16,
                        height: 16,
                        child: CircularProgressIndicator(strokeWidth: 2),
                      )
                    : const Icon(Icons.upload_file, size: 18),
                label: Text(context.l10n.imageUploadLocal),
                onPressed: widget.onUploadReplace == null || _busy
                    ? null
                    : _upload,
              ),
              const SizedBox(height: 10),
              TextField(
                controller: _url,
                autofocus: true,
                decoration: InputDecoration(
                  labelText: context.l10n.imagePasteLinkLabel,
                  hintText: context.l10n.linkUrlHint,
                  isDense: true,
                ),
                onSubmitted: (v) => Navigator.pop(context, v.trim()),
              ),
              const SizedBox(height: 6),
              Text(
                widget.reHost
                    ? context.l10n.imageRehostOn
                    : context.l10n.imageRehostOff,
                style: MicaTypography.micro.copyWith(
                  color: MicaTheme.of(context).text.faint,
                ),
              ),
            ],
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: Text(context.l10n.commonCancel),
        ),
        FilledButton(
          onPressed: () => Navigator.pop(context, _url.text.trim()),
          child: Text(context.l10n.imageReplaceButton),
        ),
      ],
    );
  }
}
