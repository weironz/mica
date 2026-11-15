import 'typography.dart';
import 'package:flutter/material.dart';

import 'theme_tokens.dart';

/// What the editor pane shows when no page is open: a short list of the three
/// things you came to do — write something new, find something, or go back to
/// what you were last writing.
///
/// This replaced the home dashboard (create card + "recently edited" grid +
/// "directories" list, reached from a 首页 row at the top of the sidebar). In
/// use nobody opened it: a notes app is entered by writing or by searching, and
/// startup already reopens the last page (`lastViewId`, plus the restored tab
/// strip), so the dashboard only ever appeared when every page had been closed.
/// Its two lists live on where they were actually used — recents in the search
/// dialog's pre-typing list, directories in the folder overview the tree opens.
///
/// Deliberately dumb: it owns no data and talks to no service. The recent page
/// arrives pre-formatted because the caller holds the locale and the clock.
class HomeScreen extends StatelessWidget {
  const HomeScreen({
    required this.strings,
    required this.onCreatePage,
    required this.onSearch,
    required this.onOpen,
    this.recent,
    super.key,
  });

  final HomeStrings strings;

  /// Null in a read-only workspace: offering "new page" there would fail.
  final VoidCallback? onCreatePage;

  final VoidCallback? onSearch;

  /// Opens the [recent] page by id.
  final void Function(String id) onOpen;

  /// The most recently edited page, if there is one.
  final HomeDocEntry? recent;

  @override
  Widget build(BuildContext context) {
    final recent = this.recent;
    return Center(
      child: SingleChildScrollView(
        padding: const EdgeInsets.symmetric(horizontal: 32, vertical: 48),
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 440),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            mainAxisSize: MainAxisSize.min,
            children: [
              if (onCreatePage case final create?)
                _ActionRow(
                  icon: Icons.note_add_outlined,
                  label: strings.createTitle,
                  hint: strings.createHint,
                  onTap: create,
                ),
              if (onSearch case final search?)
                _ActionRow(
                  icon: Icons.search,
                  label: strings.searchTitle,
                  hint: strings.searchHint,
                  onTap: search,
                ),
              if (recent != null) ...[
                const SizedBox(height: 12),
                Divider(height: 1, color: MicaTheme.of(context).border.subtle),
                const SizedBox(height: 12),
                _ActionRow(
                  icon: Icons.history,
                  label: '${strings.recentLabel}：${recent.name}',
                  hint: recent.meta,
                  onTap: () => onOpen(recent.id),
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

/// One page's worth of already-presentable text. A record, not an API model:
/// this screen must not depend on the wire format, and `meta` ("2 hours ago")
/// is the caller's to phrase. Also the search dialog's recent-row shape.
typedef HomeDocEntry = ({
  String id,
  String? icon,
  String name,
  String workspaceName,
  String meta,
});

/// Every user-facing string on this screen, injected. The l10n layer is the
/// caller's; this widget stays translatable without importing it.
@immutable
class HomeStrings {
  const HomeStrings({
    required this.createTitle,
    required this.createHint,
    required this.searchTitle,
    required this.searchHint,
    required this.recentLabel,
  });

  final String createTitle;

  /// The platform's shortcut glyphs, e.g. `Ctrl+N` / `⌘N`.
  final String createHint;
  final String searchTitle;
  final String searchHint;
  final String recentLabel;
}

class _ActionRow extends StatelessWidget {
  const _ActionRow({
    required this.icon,
    required this.label,
    required this.hint,
    required this.onTap,
  });

  final IconData icon;
  final String label;
  final String hint;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final tokens = MicaTheme.of(context);
    return InkWell(
      onTap: onTap,
      borderRadius: BorderRadius.circular(8),
      hoverColor: tokens.surface.hover,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 12),
        child: Row(
          children: [
            Icon(icon, size: 18, color: tokens.text.muted),
            const SizedBox(width: 12),
            Expanded(
              child: Text(
                label,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: MicaTypography.body.copyWith(color: tokens.text.primary),
              ),
            ),
            const SizedBox(width: 12),
            Text(
              hint,
              style: MicaTypography.caption.copyWith(color: tokens.text.faint),
            ),
          ],
        ),
      ),
    );
  }
}
