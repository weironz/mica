part of 'editor.dart';

/// An entry in the slash (`/`) insert menu: a label, an icon, and the block
/// kind (+ optional data) the focused block is converted to.
class _SlashOption {
  const _SlashOption(this.label, this.icon, this.kind, [this.data = const {}]);

  final String label;
  final IconData icon;
  final String kind;
  final Map<String, dynamic> data;
}

/// Special slash-menu kind that triggers the AI flow instead of a block convert.
const String _aiSlashKind = '__ai__';
const _SlashOption _aiSlashOption = _SlashOption(
  'Ask AI',
  Icons.auto_awesome,
  _aiSlashKind,
);

/// In-editor "Ask AI": streams the response live, then returns the accumulated
/// Markdown (via Navigator.pop) for the editor to insert as blocks.
/// System prompt for rewriting a selection.
///
/// The constraints are load-bearing, not politeness. A model that answers
/// 「好的,这是修改后的内容:」 turns that sentence into the user's document the
/// moment they press replace — so "no preamble" is a data-safety rule here, the
/// same one `kAiDocSystem` already states for whole-document generation.
const String kAiRewriteSystem =
    'You rewrite a fragment of a Markdown document in place. Output ONLY the '
    'rewritten Markdown — no preamble, no explanation, no code fence around the '
    'whole answer, no commentary before or after. Preserve the structure and '
    'formatting of the input (headings stay headings, list items stay list '
    'items, tables stay tables) unless the instruction asks otherwise. Keep the '
    "original language of the text unless asked to translate. If the instruction "
    'cannot be applied, return the input unchanged rather than explaining why.';

/// What the AI dialog was asked to do with its output.
///
/// Two exits, not one, because replacing the user's text and adding to it are
/// different promises — AppFlowy names them `replace` / `insertBelow` and Notion
/// 「Replace selection」 / 「Insert below」. Only ever built when the user picks
/// one: closing or cancelling returns null and the document is untouched.
class _AiResult {
  const _AiResult(this.markdown, {required this.replace});

  final String markdown;

  /// True = replace the selection, false = insert after it (original kept).
  final bool replace;
}

class _InlineAiDialog extends StatefulWidget {
  const _InlineAiDialog({
    required this.onStream,
    this.initialPrompt,
    this.autoStart = false,
    this.system,
    this.contextText,
    this.presets = const [],
    this.allowReplace = false,
  });

  final Stream<String> Function(String prompt, {String? system}) onStream;

  /// Prefill the prompt field (e.g. a code-block preset). Null = empty.
  final String? initialPrompt;

  /// Start streaming immediately on open (for one-click presets).
  final bool autoStart;

  /// System prompt. For a rewrite this is what keeps the model from wrapping
  /// its answer in 「好的,这是修改后的内容:」 — that prose would become the
  /// user's text.
  final String? system;

  /// Material the instruction applies TO — the selected Markdown for a rewrite.
  ///
  /// Kept out of the prompt FIELD on purpose: the box should hold what you are
  /// asking for, not a copy of the page you are asking about. Appended when the
  /// request is sent.
  final String? contextText;

  /// One-click instructions. AppFlowy ships six of these next to its free-form
  /// box (`AiWriterCommand`), Notion the same — the common intents are worth a
  /// button, and the box is still there for everything else.
  final List<({String label, String prompt})> presets;

  /// Offer 「replace the selection」 as well as 「insert below」.
  final bool allowReplace;

  @override
  State<_InlineAiDialog> createState() => _InlineAiDialogState();
}

class _InlineAiDialogState extends State<_InlineAiDialog> {
  late final _prompt = TextEditingController(text: widget.initialPrompt ?? '');
  final _scroll = ScrollController();
  final StringBuffer _buffer = StringBuffer();
  StreamSubscription<String>? _sub;
  bool _streaming = false;
  bool _done = false;
  String? _error;

  @override
  void initState() {
    super.initState();
    if (widget.autoStart && _prompt.text.trim().isNotEmpty) {
      WidgetsBinding.instance.addPostFrameCallback((_) => _generate());
    }
  }

  @override
  void dispose() {
    _sub?.cancel();
    _prompt.dispose();
    _scroll.dispose();
    super.dispose();
  }

  void _generate() {
    final prompt = _prompt.text.trim();
    if (prompt.isEmpty) return;
    setState(() {
      _streaming = true;
      _done = false;
      _error = null;
      _buffer.clear();
    });
    // The material rides along with the instruction, separated so the model can
    // tell "what to do" from "what to do it to".
    final ctx = widget.contextText;
    final sent = (ctx == null || ctx.trim().isEmpty)
        ? prompt
        : '$prompt\n\n---\n\n$ctx';
    _sub = widget
        .onStream(sent, system: widget.system)
        .listen(
          (delta) {
            setState(() => _buffer.write(delta));
            WidgetsBinding.instance.addPostFrameCallback((_) {
              if (_scroll.hasClients) {
                _scroll.jumpTo(_scroll.position.maxScrollExtent);
              }
            });
          },
          onError: (Object error) {
            if (mounted) {
              setState(() {
                _streaming = false;
                _error = error.toString();
              });
            }
          },
          onDone: () {
            if (mounted) {
              setState(() {
                _streaming = false;
                _done = true;
              });
            }
          },
        );
  }

  @override
  Widget build(BuildContext context) {
    final hasOutput = _buffer.isNotEmpty;
    return AlertDialog(
      title: Row(
        children: [
          Icon(
            Icons.auto_awesome,
            size: 20,
            color: MicaTheme.of(context).editor.alertAccents['important']!,
          ),
          const SizedBox(width: 8),
          Text(context.l10n.aiAskTitle),
        ],
      ),
      content: SizedBox(
        width: 480,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            if (widget.presets.isNotEmpty) ...[
              Wrap(
                spacing: 6,
                runSpacing: 6,
                children: [
                  for (final p in widget.presets)
                    ActionChip(
                      label: Text(p.label),
                      // Fills the box rather than firing immediately: the
                      // preset is a starting point you can still edit, and a
                      // one-click request that spends tokens on a misread is
                      // worse than one more click.
                      onPressed: _streaming
                          ? null
                          : () => setState(() => _prompt.text = p.prompt),
                    ),
                ],
              ),
              const SizedBox(height: 10),
            ],
            TextField(
              controller: _prompt,
              autofocus: true,
              minLines: 2,
              maxLines: 4,
              enabled: !_streaming,
              decoration: InputDecoration(
                hintText: context.l10n.editorAiPromptHint,
                border: const OutlineInputBorder(),
              ),
            ),
            if (hasOutput || _streaming) ...[
              const SizedBox(height: 12),
              Container(
                height: 200,
                width: double.infinity,
                padding: const EdgeInsets.all(12),
                decoration: BoxDecoration(
                  color: MicaTheme.of(context).surface.raised,
                  borderRadius: BorderRadius.circular(8),
                  border: Border.all(
                    color: MicaTheme.of(context).border.normal,
                  ),
                ),
                child: SingleChildScrollView(
                  controller: _scroll,
                  child: Text(
                    _buffer.isEmpty ? '…' : _buffer.toString(),
                    style: const TextStyle(fontSize: 13, height: 1.5),
                  ),
                ),
              ),
              const SizedBox(height: 6),
              if (_streaming)
                Row(
                  children: [
                    const SizedBox(
                      width: 14,
                      height: 14,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    ),
                    const SizedBox(width: 8),
                    Text(context.l10n.aiGenerating),
                  ],
                ),
            ],
            if (_error != null) ...[
              const SizedBox(height: 12),
              Container(
                width: double.infinity,
                padding: const EdgeInsets.all(10),
                decoration: BoxDecoration(
                  color: MicaTheme.of(
                    context,
                  ).status.danger.withValues(alpha: 0.08),
                  borderRadius: BorderRadius.circular(6),
                ),
                child: Text(
                  _error!,
                  style: TextStyle(
                    color: MicaTheme.of(context).status.danger,
                    fontSize: 13,
                  ),
                ),
              ),
            ],
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: _streaming ? null : () => Navigator.of(context).pop(),
          child: Text(context.l10n.commonCancel),
        ),
        if (!_done)
          FilledButton.icon(
            onPressed: _streaming ? null : _generate,
            icon: const Icon(Icons.auto_awesome, size: 18),
            label: Text(
              hasOutput ? context.l10n.aiRegenerate : context.l10n.aiGenerate,
            ),
          )
        else ...[
          TextButton.icon(
            onPressed: _generate,
            icon: const Icon(Icons.refresh, size: 18),
            label: Text(context.l10n.aiRegenerate),
          ),
          // Insert-below stays a plain button even when replacing is offered:
          // keeping the original is the reversible choice, so it must not be
          // the one that looks like the default.
          TextButton.icon(
            onPressed: () => Navigator.of(
              context,
            ).pop(_AiResult(_buffer.toString(), replace: false)),
            icon: const Icon(Icons.south, size: 18),
            label: Text(
              widget.allowReplace
                  ? context.l10n.aiInsertBelow
                  : context.l10n.aiInsert,
            ),
          ),
          if (widget.allowReplace)
            FilledButton.icon(
              onPressed: () => Navigator.of(
                context,
              ).pop(_AiResult(_buffer.toString(), replace: true)),
              icon: const Icon(Icons.check, size: 18),
              label: Text(context.l10n.aiReplaceSelection),
            ),
        ],
      ],
    );
  }
}
