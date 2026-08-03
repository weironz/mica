import 'package:flutter/material.dart';

import '../api/models.dart';
import '../l10n/locale_controller.dart';
import 'theme_tokens.dart';

/// The comment panel: every thread on the open document, with replies.
///
/// Deliberately a plain list, not a second editing surface. Two rules it exists
/// to honour (docs/comments-plan.md):
///
/// - An ORPHANED thread (its anchored text deleted) is still shown, against the
///   `quote` it saved — deleting a sentence must not silently delete the argument
///   about it — but it gets no highlight on the canvas.
/// - Nothing here ever computes a position. Anchors arrive resolved from the
///   server and are drawn by render.dart; this panel only shows text and calls
///   back.
class CommentPanel extends StatefulWidget {
  const CommentPanel({
    required this.threads,
    required this.onReply,
    required this.onSetResolved,
    required this.onDelete,
    this.onFocusThread,
    this.currentUserId,
    this.onClose,
    super.key,
  });

  final List<CommentThread> threads;
  final Future<void> Function(String threadId, String body) onReply;
  final Future<void> Function(String threadId, bool resolved) onSetResolved;
  final Future<void> Function(String threadId) onDelete;

  /// Highlight this thread on the canvas. Null → threads are not focusable.
  final void Function(CommentThread thread)? onFocusThread;

  /// Dismiss the rail. Null renders the list bare (the shape a test or a future
  /// embedded use wants); non-null adds the header with the close affordance.
  final VoidCallback? onClose;

  /// Only decides whether "delete thread" is offered — the server is the real
  /// authority (author, or write access).
  final String? currentUserId;

  @override
  State<CommentPanel> createState() => _CommentPanelState();
}

class _CommentPanelState extends State<CommentPanel> {
  /// The thread whose reply box is open, if any. One at a time keeps the panel
  /// from becoming a wall of text fields.
  String? _replyingTo;
  final _replyController = TextEditingController();
  bool _busy = false;

  @override
  void dispose() {
    _replyController.dispose();
    super.dispose();
  }

  Future<void> _run(Future<void> Function() action) async {
    if (_busy) return;
    setState(() => _busy = true);
    try {
      await action();
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _submitReply(String threadId) async {
    final body = _replyController.text.trim();
    if (body.isEmpty) return;
    await _run(() => widget.onReply(threadId, body));
    if (!mounted) return;
    _replyController.clear();
    setState(() => _replyingTo = null);
  }

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    // Open first, then resolved — an unresolved discussion is the one that wants
    // attention. Orphans stay with the open ones (they still need answering).
    final threads = [...widget.threads]
      ..sort((a, b) {
        if (a.isResolved != b.isResolved) return a.isResolved ? 1 : -1;
        final at = a.createdAt;
        final bt = b.createdAt;
        if (at == null || bt == null) return 0;
        return at.compareTo(bt);
      });

    final list = threads.isEmpty
          ? Padding(
              padding: const EdgeInsets.symmetric(vertical: 24),
              child: Text(
                l10n.commentsEmpty,
                style: TextStyle(
                  fontSize: 13,
                  color: MicaTheme.of(context).text.muted,
                ),
              ),
            )
        : ListView.separated(
            padding: const EdgeInsets.symmetric(vertical: 8),
            // Kept: inside the rail's `Expanded` this still scrolls once the
            // discussion outgrows the height, and without it the bare form
            // (no `onClose`) has no bounded parent to size against.
            shrinkWrap: true,
            itemCount: threads.length,
            separatorBuilder: (_, _) => const Divider(height: 20),
            itemBuilder: (context, i) => _threadTile(context, threads[i]),
          );

    final close = widget.onClose;
    if (close == null) return SizedBox(width: 380, child: list);

    // A RAIL, not a dialog. It used to be a modal `AlertDialog`, which covered
    // the very text the comments are about — and reading the passage while
    // reading the note on it is the entire activity. AFFiNE reaches the same
    // shape (`components/comment/sidebar`), and its rail is what makes room for
    // things a dialog has nowhere to put, like collapsing long reply chains.
    return Container(
      width: 380,
      decoration: BoxDecoration(
        color: MicaTheme.of(context).surface.base,
        border: Border(
          left: BorderSide(color: MicaTheme.of(context).border.subtle),
        ),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 12, 8, 8),
            child: Row(
              children: [
                Expanded(
                  child: Text(
                    l10n.commentsTitle,
                    style: TextStyle(
                      fontSize: 13,
                      fontWeight: FontWeight.w600,
                      color: MicaTheme.of(context).text.primary,
                    ),
                  ),
                ),
                IconButton(
                  onPressed: close,
                  icon: const Icon(Icons.close, size: 16),
                  tooltip: l10n.commonClose,
                  visualDensity: VisualDensity.compact,
                  color: MicaTheme.of(context).text.faint,
                ),
              ],
            ),
          ),
          Expanded(
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 16),
              child: list,
            ),
          ),
        ],
      ),
    );
  }

  Widget _threadTile(BuildContext context, CommentThread thread) {
    final l10n = context.l10n;
    final canDelete =
        widget.currentUserId == null ||
        thread.createdBy == widget.currentUserId;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        // The quote: what was commented on. For an orphan it is all that is left.
        InkWell(
          onTap: thread.isHighlightable && widget.onFocusThread != null
              ? () => widget.onFocusThread!(thread)
              : null,
          child: Container(
            width: double.infinity,
            padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
            decoration: BoxDecoration(
              color: thread.isOrphaned
                  ? MicaTheme.of(context).surface.sunken
                  : MicaTheme.of(context).editor.commentHighlight,
              borderRadius: BorderRadius.circular(4),
            ),
            child: Text(
              thread.quote.isEmpty ? '—' : thread.quote,
              maxLines: 3,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                fontSize: 12,
                color: MicaTheme.of(context).text.muted,
                decoration: thread.isOrphaned
                    ? TextDecoration.lineThrough
                    : null,
              ),
            ),
          ),
        ),
        if (thread.isOrphaned)
          Padding(
            padding: const EdgeInsets.only(top: 4),
            child: Text(
              l10n.commentOrphaned,
              style: TextStyle(
                fontSize: 11,
                color: MicaTheme.of(context).text.faint,
              ),
            ),
          ),
        if (thread.isResolved)
          Padding(
            padding: const EdgeInsets.only(top: 4),
            child: Row(
              children: [
                Icon(
                  Icons.check_circle_outline,
                  size: 13,
                  color: MicaTheme.of(context).text.faint,
                ),
                const SizedBox(width: 4),
                Text(
                  l10n.commentResolved,
                  style: TextStyle(
                    fontSize: 11,
                    color: MicaTheme.of(context).text.faint,
                  ),
                ),
              ],
            ),
          ),
        const SizedBox(height: 6),
        for (final comment in thread.comments)
          Padding(
            padding: const EdgeInsets.only(bottom: 6),
            child: Text(comment.body, style: const TextStyle(fontSize: 13)),
          ),
        if (_replyingTo == thread.id)
          Padding(
            padding: const EdgeInsets.only(top: 2, bottom: 4),
            child: TextField(
              controller: _replyController,
              autofocus: true,
              minLines: 1,
              maxLines: 4,
              style: const TextStyle(fontSize: 13),
              decoration: InputDecoration(
                isDense: true,
                hintText: l10n.commentReplyPlaceholder,
                border: const OutlineInputBorder(),
              ),
              onSubmitted: (_) => _submitReply(thread.id),
            ),
          ),
        Row(
          children: [
            if (_replyingTo == thread.id)
              TextButton(
                onPressed: _busy ? null : () => _submitReply(thread.id),
                child: Text(l10n.commentPost),
              )
            else
              TextButton(
                onPressed: _busy
                    ? null
                    : () => setState(() {
                        _replyingTo = thread.id;
                        _replyController.clear();
                      }),
                child: Text(l10n.commentReply),
              ),
            TextButton(
              onPressed: _busy
                  ? null
                  : () => _run(
                      () => widget.onSetResolved(thread.id, !thread.isResolved),
                    ),
              child: Text(
                thread.isResolved ? l10n.commentReopen : l10n.commentResolve,
              ),
            ),
            const Spacer(),
            if (canDelete)
              IconButton(
                tooltip: l10n.commentDelete,
                visualDensity: VisualDensity.compact,
                onPressed: _busy
                    ? null
                    : () => _run(() => widget.onDelete(thread.id)),
                icon: const Icon(Icons.delete_outline, size: 16),
              ),
          ],
        ),
      ],
    );
  }
}
