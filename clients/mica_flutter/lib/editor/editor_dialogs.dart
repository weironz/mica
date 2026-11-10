part of 'editor.dart';

const List<_SlashOption> _slashOptions = [
  _SlashOption('Text', Icons.notes, 'paragraph'),
  _SlashOption('Heading 1', Icons.title, 'heading', {'level': 1}),
  _SlashOption('Heading 2', Icons.title, 'heading', {'level': 2}),
  _SlashOption('Heading 3', Icons.title, 'heading', {'level': 3}),
  _SlashOption('Bulleted list', Icons.format_list_bulleted, 'bulleted_list'),
  _SlashOption('Numbered list', Icons.format_list_numbered, 'numbered_list'),
  _SlashOption('To-do', Icons.check_box_outlined, 'todo', {'checked': false}),
  _SlashOption('Quote', Icons.format_quote, 'quote'),
  // Callouts (GFM alerts) reuse the quote block, tagged with `data.alert`.
  _SlashOption('Note', Icons.info_outline, 'quote', {'alert': 'note'}),
  _SlashOption('Tip', Icons.lightbulb_outline, 'quote', {'alert': 'tip'}),
  _SlashOption('Important', Icons.campaign_outlined, 'quote', {
    'alert': 'important',
  }),
  _SlashOption('Warning', Icons.warning_amber_outlined, 'quote', {
    'alert': 'warning',
  }),
  _SlashOption('Caution', Icons.report_outlined, 'quote', {'alert': 'caution'}),
  _SlashOption('Code', Icons.code, 'code_block'),
  _SlashOption('Math formula', Icons.functions, 'math_block'),
  _SlashOption('Table', Icons.grid_on, 'table'),
  _SlashOption('Divider', Icons.horizontal_rule, 'divider'),
  _SlashOption('Image', Icons.image_outlined, 'image'),
];

/// Image properties + replace. Owns its own [TextEditingController]: creating it
/// outside and disposing after `showDialog` returns is a trap — the route's exit
/// animation is still building the TextField, and it throws "used after being
/// disposed".
///
/// Pops the url to replace with; `''` when a file upload already handled the
/// replacement (or nothing was typed), null on cancel.
class _ImageEditDialog extends StatefulWidget {
  const _ImageEditDialog({
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
  State<_ImageEditDialog> createState() => _ImageEditDialogState();
}

class _ImageEditDialogState extends State<_ImageEditDialog> {
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
                style: TextStyle(
                  fontSize: 12,
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
                  // Decoded for a HUMAN: a Chinese file name is percent-escaped
                  // in the url (it must be, to fetch), so showing it raw read as
                  // `%E8%A7%A3%E9%94%81…` — the name looked lost. Copy below
                  // still yields the raw, always-valid url.
                  readableUrl(widget.link),
                  maxLines: 3,
                  style: const TextStyle(fontFamily: kMonoFont, fontSize: 12),
                ),
              ),
              Align(
                alignment: Alignment.centerRight,
                // This one used to report nothing at all — you pressed it and
                // the dialog sat there unchanged. The check is its first
                // feedback, not a replacement for a snackbar.
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
                style: const TextStyle(fontWeight: FontWeight.w600),
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
                style: TextStyle(
                  fontSize: 11,
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
