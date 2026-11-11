import 'ai_prompts.dart';
import '../l10n/locale_controller.dart';
import 'dart:async';
import 'dart:convert';
import 'package:flutter/material.dart';
import '../editor/editor.dart';
import 'status_kit.dart';
import 'theme_tokens.dart';

enum _AiTarget { newPage, currentPage, newWorkspace }


/// Global AI dialog: type an instruction and pick where the generated content
/// goes — a new page, the current page, or a brand-new workspace.
class AiDialog extends StatefulWidget {
  const AiDialog({
    super.key,
    required this.canEdit,
    required this.hasWorkspace,
    required this.onStream,
    required this.onNewPage,
    required this.onCurrentPage,
    required this.currentPage,
    required this.onNewWorkspace,
    this.onPreviewEdits,
    this.readPageForEdits,
    this.onApplyEdits,
  });

  final bool canEdit;
  final bool hasWorkspace;
  final Stream<String> Function(String prompt, {String? system}) onStream;
  final Future<void> Function(String markdown) onNewPage;
  final Future<void> Function(String markdown)? onCurrentPage;
  final ({String title, String markdown})? currentPage;
  final Future<void> Function(String markdown) onNewWorkspace;
  final Future<AiEditPreview> Function(String, EditorAiSnapshot)?
  onPreviewEdits;
  final ({String title, String markdown, EditorAiSnapshot source}) Function()?
  readPageForEdits;
  final Future<void> Function(EditorAiSnapshot, AiEditPreview)? onApplyEdits;

  @override
  State<AiDialog> createState() => _AiDialogState();
}

class _AiDialogState extends State<AiDialog> {
  final _prompt = TextEditingController();
  final _scroll = ScrollController();
  late _AiTarget _target = widget.hasWorkspace
      ? _AiTarget.newPage
      : _AiTarget.newWorkspace;
  StreamSubscription<String>? _sub;
  bool _streaming = false;
  bool _applying = false;
  bool _done = false;
  bool _previewing = false;
  EditorAiSnapshot? _source;
  AiEditPreview? _preview;
  final StringBuffer _buffer = StringBuffer();
  String? _error;

  bool get _busy => _streaming || _previewing || _applying;
  bool get _editMode =>
      _target == _AiTarget.currentPage &&
      widget.onPreviewEdits != null &&
      widget.readPageForEdits != null &&
      widget.onApplyEdits != null;

  void _selectTarget(_AiTarget target) {
    if (target == _target) return;
    setState(() {
      _target = target;
      _buffer.clear();
      _source = null;
      _preview = null;
      _done = false;
      _error = null;
    });
  }

  @override
  void dispose() {
    _sub?.cancel();
    _prompt.dispose();
    _scroll.dispose();
    super.dispose();
  }

  void _generate() {
    final instruction = _prompt.text.trim();
    if (instruction.isEmpty) return;
    // New page / new workspace want a document with a title line.
    var current = _target == _AiTarget.currentPage ? widget.currentPage : null;
    EditorAiSnapshot? source;
    if (_editMode) {
      try {
        final page = widget.readPageForEdits!();
        current = (title: page.title, markdown: page.markdown);
        source = page.source;
      } catch (error) {
        setState(
          () => _error = error is AiEditConflict
              ? context.l10n.aiEditConflict
              : error.toString(),
        );
        return;
      }
    }
    final system = _editMode
        ? kAiPageEditSystem
        : current == null
        ? kAiDocSystem
        : kAiCurrentPageSystem;
    final prompt = current == null
        ? instruction
        : jsonEncode({
            'instruction': instruction,
            'current_page': {
              'title': current.title,
              'markdown': current.markdown,
              if (source != null) 'blocks': source.blocks,
            },
          });
    setState(() {
      _streaming = true;
      _done = false;
      _error = null;
      _buffer.clear();
      _source = source;
      _preview = null;
    });
    _sub = widget
        .onStream(prompt, system: system)
        .listen(
          (delta) {
            if (!mounted) return;
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
          onDone: _finishGeneration,
          cancelOnError: true,
        );
  }

  Future<void> _finishGeneration() async {
    if (!mounted) return;
    if (!_editMode) {
      setState(() {
        _streaming = false;
        _done = true;
      });
      return;
    }
    setState(() {
      _streaming = false;
      _previewing = true;
    });
    try {
      final preview = await widget.onPreviewEdits!(
        _buffer.toString(),
        _source!,
      );
      if (mounted) {
        setState(() {
          _preview = preview;
          _done = true;
        });
      }
    } catch (error) {
      if (mounted) setState(() => _error = error.toString());
    } finally {
      if (mounted) setState(() => _previewing = false);
    }
  }

  Future<void> _apply() async {
    final markdown = _buffer.toString().trim();
    if (markdown.isEmpty) return;
    setState(() {
      _applying = true;
      _error = null;
    });
    try {
      switch (_target) {
        case _AiTarget.newPage:
          await widget.onNewPage(markdown);
        case _AiTarget.currentPage:
          if (_editMode) {
            if (_preview == null || _source == null) return;
            await widget.onApplyEdits!(_source!, _preview!);
          } else {
            await widget.onCurrentPage?.call(markdown);
          }
        case _AiTarget.newWorkspace:
          await widget.onNewWorkspace(markdown);
      }
      if (mounted) Navigator.of(context).pop();
    } catch (error) {
      if (mounted) {
        setState(
          () => _error = error is AiEditConflict
              ? context.l10n.aiEditConflict
              : error is AiEditSaveFailure
              ? context.l10n.aiEditSaveFailure
              : error.toString(),
        );
      }
    } finally {
      if (mounted) setState(() => _applying = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final canWriteCurrent = widget.onCurrentPage != null;
    final hasOutput = _buffer.isNotEmpty;
    return AlertDialog(
      title: Row(
        children: [
          Icon(
            Icons.auto_awesome,
            size: 22,
            color: MicaTheme.of(context).editor.alertAccents['important'],
          ),
          const SizedBox(width: 8),
          Text(context.l10n.aiAskTitle),
        ],
      ),
      content: SizedBox(
        width: 520,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            TextField(
              controller: _prompt,
              autofocus: true,
              minLines: 2,
              maxLines: 5,
              enabled: !_busy,
              decoration: InputDecoration(
                hintText: context.l10n.aiPromptHint,
                border: const OutlineInputBorder(),
              ),
            ),
            const SizedBox(height: 12),
            Wrap(
              spacing: 8,
              children: [
                if (widget.canEdit)
                  ChoiceChip(
                    label: Text(context.l10n.aiTargetNewPage),
                    selected: _target == _AiTarget.newPage,
                    onSelected: widget.hasWorkspace && !_busy
                        ? (_) => _selectTarget(_AiTarget.newPage)
                        : null,
                  ),
                if (widget.canEdit && canWriteCurrent)
                  ChoiceChip(
                    label: Text(context.l10n.aiTargetCurrentPage),
                    selected: _target == _AiTarget.currentPage,
                    onSelected: _busy
                        ? null
                        : (_) => _selectTarget(_AiTarget.currentPage),
                  ),
                ChoiceChip(
                  label: Text(context.l10n.aiTargetNewWorkspace),
                  selected: _target == _AiTarget.newWorkspace,
                  onSelected: _busy
                      ? null
                      : (_) => _selectTarget(_AiTarget.newWorkspace),
                ),
              ],
            ),
            if (hasOutput || _streaming) ...[
              const SizedBox(height: 12),
              Container(
                height: 220,
                width: double.infinity,
                padding: const EdgeInsets.all(12),
                decoration: BoxDecoration(
                  color: MicaTheme.of(context).surface.raised,
                  borderRadius: BorderRadius.circular(8),
                  border: Border.all(
                    color: MicaTheme.of(context).border.normal,
                  ),
                ),
                child: _editMode && _preview == null
                    ? Center(
                        child: _busy
                            ? const CircularProgressIndicator()
                            : const SizedBox.shrink(),
                      )
                    : SingleChildScrollView(
                        controller: _scroll,
                        child: _editMode
                            ? Column(
                                crossAxisAlignment: CrossAxisAlignment.start,
                                children: [
                                  for (final edit in _preview!.edits) ...[
                                    Text(
                                      context.l10n.aiOriginal,
                                      style: const TextStyle(
                                        fontWeight: FontWeight.w600,
                                      ),
                                    ),
                                    SelectableText(
                                      edit.before,
                                      style: const TextStyle(
                                        fontSize: 13,
                                        height: 1.5,
                                      ),
                                    ),
                                    const SizedBox(height: 8),
                                    Text(
                                      context.l10n.aiModified,
                                      style: const TextStyle(
                                        fontWeight: FontWeight.w600,
                                      ),
                                    ),
                                    SelectableText(
                                      edit.after,
                                      style: const TextStyle(
                                        fontSize: 13,
                                        height: 1.5,
                                      ),
                                    ),
                                    const Divider(height: 24),
                                  ],
                                ],
                              )
                            : Text(
                                _buffer.isEmpty ? '…' : _buffer.toString(),
                                style: const TextStyle(
                                  fontSize: 13,
                                  height: 1.5,
                                ),
                              ),
                      ),
              ),
              const SizedBox(height: 6),
              Row(
                children: [
                  if (_streaming || _previewing) ...[
                    const SizedBox(
                      width: 14,
                      height: 14,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    ),
                    const SizedBox(width: 8),
                    Text(context.l10n.aiGenerating),
                  ] else if (_done)
                    Text(
                      _editMode
                          ? context.l10n.aiDoneReviewEdits
                          : context.l10n.aiDoneReview,
                      style: TextStyle(color: MicaTheme.of(context).text.muted),
                    ),
                ],
              ),
            ],
            if (_error != null) ...[
              const SizedBox(height: 12),
              ErrorBanner(_error!),
            ],
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: _busy ? null : () => Navigator.of(context).pop(),
          child: Text(context.l10n.commonCancel),
        ),
        if (!_done)
          FilledButton.icon(
            onPressed: _busy ? null : _generate,
            icon: const Icon(Icons.auto_awesome, size: 18),
            label: Text(
              hasOutput ? context.l10n.aiRegenerate : context.l10n.aiGenerate,
            ),
          )
        else ...[
          TextButton.icon(
            onPressed: _applying ? null : _generate,
            icon: const Icon(Icons.refresh, size: 18),
            label: Text(context.l10n.aiRegenerate),
          ),
          FilledButton.icon(
            onPressed: _applying ? null : _apply,
            icon: _applying
                ? const SizedBox(
                    width: 16,
                    height: 16,
                    child: CircularProgressIndicator(strokeWidth: 2),
                  )
                : const Icon(Icons.check, size: 18),
            label: Text(
              _editMode ? context.l10n.aiApplyEdits : context.l10n.aiInsert,
            ),
          ),
        ],
      ],
    );
  }
}

/// Recycle bin: lists soft-deleted pages and offers restore / delete-forever.
/// Only the roots of each deleted subtree are shown; restoring a root brings its
/// whole subtree back.
/// Publish the open (cloud) document to a public read-only link. Toggle on to
/// mint/return the link, off to revoke it (the public URL 404s at once).
