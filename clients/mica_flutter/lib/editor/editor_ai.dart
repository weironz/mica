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

extension _SelectionAiActions on _MicaEditorState {
  /// Rewrite the SELECTED text with AI.
  ///
  /// Nothing reaches the document until the user picks 「replace」 or 「insert
  /// below」 — closing the dialog, an error mid-stream, or a model that answers
  /// with prose all leave the page exactly as it was. That staging is what all
  /// three references do (Notion previews, AFFiNE panels, AppFlowy writes into
  /// its own block); none of them types into the document as it streams,
  /// because an accepted rewrite has to be ONE undoable step and a stream is
  /// hundreds of them.
  Future<void> _runSelectionAi() async {
    final stream = widget.onAiStream;
    if (stream == null || _controller.selection?.isCollapsed != false) return;
    final source = _controller.selectionText(imageUrls: _imageUrlCache);
    if (source.trim().isEmpty) return;
    final l10n = context.l10n;

    final result = await showDialog<AiInsertResult>(
      context: context,
      builder: (context) => InlineAiDialog(
        onStream: stream,
        system: kAiRewriteSystem,
        contextText: source,
        allowReplace: true,
        presets: [
          (label: l10n.aiPresetImprove, prompt: l10n.aiPresetImprovePrompt),
          (label: l10n.aiPresetFix, prompt: l10n.aiPresetFixPrompt),
          (label: l10n.aiPresetShorter, prompt: l10n.aiPresetShorterPrompt),
          (label: l10n.aiPresetLonger, prompt: l10n.aiPresetLongerPrompt),
        ],
      ),
    );
    if (!mounted || !widget.canEdit || result == null) return;
    if (result.markdown.trim().isEmpty) return;

    final sel = _controller.selection;
    if (sel == null || sel.isCollapsed) return;
    final specs = _parseOrNull(result.markdown);
    if (specs == null || specs.isEmpty) return;
    if (result.replace) {
      _controller.insertBlocksReplacingSelection(specs);
    } else {
      // A backwards drag has its focus at the start; insert after the live end.
      _controller.collapseTo(sel.end);
      _controller.insertBlocksAfterFocus(specs);
    }
    _rehostExternalImages();
    _syncImeFromSelection(force: true);
  }
}
