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

/// The code-block title dialog, owning its controller. Disposing a controller
/// right after showDialog returns throws in debug (`_dependents.isEmpty`): the
/// pop only starts the exit transition, which rebuilds the field and re-adds a
/// listener to the just-freed controller. State.dispose runs after the route is
/// gone, so it's the safe hook.
class _CodeTitleDialog extends StatefulWidget {
  const _CodeTitleDialog({required this.initial});

  final String initial;

  @override
  State<_CodeTitleDialog> createState() => _CodeTitleDialogState();
}

class _CodeTitleDialogState extends State<_CodeTitleDialog> {
  late final TextEditingController _ctrl = TextEditingController(
    text: widget.initial,
  );

  @override
  void dispose() {
    _ctrl.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    return AlertDialog(
      title: Text(l10n.codeMenuAddTitle),
      content: TextField(
        controller: _ctrl,
        autofocus: true,
        decoration: InputDecoration(
          hintText: l10n.codeTitleHint,
          border: const OutlineInputBorder(),
        ),
        onSubmitted: (v) => Navigator.of(context).pop(v),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l10n.commonCancel),
        ),
        FilledButton(
          onPressed: () => Navigator.of(context).pop(_ctrl.text),
          child: Text(l10n.commonSave),
        ),
      ],
    );
  }
}

/// Code-block language picker: a small anchored panel with a search box —
/// the language list is long, so filtering beats scrolling. ↑/↓ move the
/// highlight, Enter picks, Esc dismisses; pops with the chosen language.
class _LanguagePicker extends StatefulWidget {
  const _LanguagePicker({required this.anchor, required this.current});

  final Offset anchor;

  /// The block's own setting — `auto` when it has none. Not the *resolved*
  /// language: the point is to show whether the block is pinned or detecting.
  final String current;

  @override
  State<_LanguagePicker> createState() => _LanguagePickerState();
}

class _LanguagePickerState extends State<_LanguagePicker> {
  final TextEditingController _query = TextEditingController();
  final ScrollController _scroll = ScrollController();
  int _index = 0;

  List<String> get _filtered {
    final q = _query.text.trim().toLowerCase();
    // Alphabetical so the list is scannable; `auto` (content detection) stays
    // pinned on top as the smart default.
    final rest = kCodeLanguages.where((l) => l != 'auto').toList()..sort();
    final all = <String>['auto', ...rest];
    if (q.isEmpty) return all;
    return [
      for (final l in all)
        if (l.toLowerCase().contains(q)) l,
    ];
  }

  @override
  void dispose() {
    _query.dispose();
    _scroll.dispose();
    super.dispose();
  }

  void _pick(String language) => Navigator.of(context).pop(language);

  KeyEventResult _onKey(FocusNode node, KeyEvent event) {
    if (event is! KeyDownEvent) return KeyEventResult.ignored;
    final items = _filtered;
    if (event.logicalKey == LogicalKeyboardKey.escape) {
      Navigator.of(context).pop();
      return KeyEventResult.handled;
    }
    if (event.logicalKey == LogicalKeyboardKey.arrowDown && items.isNotEmpty) {
      setState(() => _index = (_index + 1) % items.length);
      _reveal(items.length);
      return KeyEventResult.handled;
    }
    if (event.logicalKey == LogicalKeyboardKey.arrowUp && items.isNotEmpty) {
      setState(() => _index = (_index - 1 + items.length) % items.length);
      _reveal(items.length);
      return KeyEventResult.handled;
    }
    if (event.logicalKey == LogicalKeyboardKey.enter && items.isNotEmpty) {
      _pick(items[_index.clamp(0, items.length - 1)]);
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  void _reveal(int count) {
    if (!_scroll.hasClients || count == 0) return;
    const itemH = 34.0;
    final target = (_index * itemH).clamp(
      0.0,
      _scroll.position.maxScrollExtent,
    );
    final top = _scroll.offset;
    final bottom = top + 240 - itemH;
    if (target < top || target > bottom) {
      _scroll.jumpTo(
        (target - 100).clamp(0.0, _scroll.position.maxScrollExtent),
      );
    }
  }

  @override
  Widget build(BuildContext context) {
    final screen = MediaQuery.of(context).size;
    const w = 240.0;
    const h = 312.0;
    final left = widget.anchor.dx.clamp(8.0, screen.width - w - 8);
    final top = widget.anchor.dy.clamp(8.0, screen.height - h - 8);
    final items = _filtered;
    final active = _index.clamp(0, items.isEmpty ? 0 : items.length - 1);

    return Stack(
      children: [
        Positioned(
          left: left,
          top: top,
          child: Focus(
            onKeyEvent: _onKey,
            child: Material(
              elevation: 8,
              borderRadius: BorderRadius.circular(8),
              child: Container(
                width: w,
                height: h,
                padding: const EdgeInsets.all(8),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    TextField(
                      controller: _query,
                      autofocus: true,
                      style: const TextStyle(fontSize: 13),
                      decoration: InputDecoration(
                        hintText: context.l10n.editorSearchLanguage,
                        prefixIcon: const Icon(Icons.search, size: 16),
                        prefixIconConstraints: const BoxConstraints(
                          minWidth: 28,
                        ),
                        isDense: true,
                        border: const OutlineInputBorder(),
                        contentPadding: const EdgeInsets.symmetric(
                          horizontal: 8,
                          vertical: 8,
                        ),
                      ),
                      onChanged: (_) => setState(() => _index = 0),
                    ),
                    const SizedBox(height: 6),
                    Expanded(
                      child: items.isEmpty
                          ? Center(
                              child: Text(
                                context.l10n.searchNoMatches,
                                style: TextStyle(
                                  fontSize: 13,
                                  color: MicaTheme.of(context).text.faint,
                                ),
                              ),
                            )
                          : ListView.builder(
                              controller: _scroll,
                              itemExtent: 34,
                              itemCount: items.length,
                              itemBuilder: (context, i) => InkWell(
                                onTap: () => _pick(items[i]),
                                child: Container(
                                  color: i == active
                                      ? MicaTheme.of(
                                          context,
                                        ).accent.primary.withValues(alpha: 0.08)
                                      : null,
                                  padding: const EdgeInsets.symmetric(
                                    horizontal: 10,
                                  ),
                                  alignment: Alignment.centerLeft,
                                  child: Row(
                                    children: [
                                      // Which one is this block actually on?
                                      // Without the tick the list gave no
                                      // feedback at all, so a block pinned to
                                      // `python` looked exactly like one on
                                      // `auto` that had detected python.
                                      SizedBox(
                                        width: 20,
                                        child: items[i] == widget.current
                                            ? Icon(
                                                Icons.check,
                                                size: 14,
                                                color: MicaTheme.of(
                                                  context,
                                                ).accent.primary,
                                              )
                                            : null,
                                      ),
                                      Text(
                                        items[i],
                                        style: TextStyle(
                                          fontSize: 13,
                                          fontWeight: items[i] == widget.current
                                              ? FontWeight.w600
                                              : FontWeight.normal,
                                        ),
                                      ),
                                    ],
                                  ),
                                ),
                              ),
                            ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ],
    );
  }
}

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
