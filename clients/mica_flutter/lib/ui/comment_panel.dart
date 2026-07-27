import 'package:flutter/material.dart';

import '../api/models.dart';
import '../editor/render.dart' show EditorTheme;
import '../l10n/locale_controller.dart';

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
    super.key,
  });

  final List<CommentThread> threads;
  final Future<void> Function(String threadId, String body) onReply;
  final Future<void> Function(String threadId, bool resolved) onSetResolved;
  final Future<void> Function(String threadId) onDelete;

  /// Highlight this thread on the canvas. Null → threads are not focusable.
  final void Function(CommentThread thread)? onFocusThread;

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
    final threads = [...widget.threads]..sort((a, b) {
      if (a.isResolved != b.isResolved) return a.isResolved ? 1 : -1;
      final at = a.createdAt;
      final bt = b.createdAt;
      if (at == null || bt == null) return 0;
      return at.compareTo(bt);
    });

    return SizedBox(
      width: 380,
      child: threads.isEmpty
          ? Padding(
              padding: const EdgeInsets.symmetric(vertical: 24),
              child: Text(
                l10n.commentsEmpty,
                style: const TextStyle(fontSize: 13, color: EditorTheme.muted),
              ),
            )
          : ListView.separated(
              shrinkWrap: true,
              itemCount: threads.length,
              separatorBuilder: (_, _) => const Divider(height: 20),
              itemBuilder: (context, i) => _threadTile(context, threads[i]),
            ),
    );
  }

  Widget _threadTile(BuildContext context, CommentThread thread) {
    final l10n = context.l10n;
    final canDelete =
        widget.currentUserId == null || thread.createdBy == widget.currentUserId;
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
                  ? EditorTheme.codeBg
                  : EditorTheme.commentHighlight,
              borderRadius: BorderRadius.circular(4),
            ),
            child: Text(
              thread.quote.isEmpty ? '—' : thread.quote,
              maxLines: 3,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                fontSize: 12,
                color: EditorTheme.muted,
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
              style: const TextStyle(fontSize: 11, color: EditorTheme.faint),
            ),
          ),
        if (thread.isResolved)
          Padding(
            padding: const EdgeInsets.only(top: 4),
            child: Row(
              children: [
                const Icon(
                  Icons.check_circle_outline,
                  size: 13,
                  color: EditorTheme.faint,
                ),
                const SizedBox(width: 4),
                Text(
                  l10n.commentResolved,
                  style: const TextStyle(
                    fontSize: 11,
                    color: EditorTheme.faint,
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
