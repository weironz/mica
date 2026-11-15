// Favorite pages: which pages are starred, per workspace, and the sidebar
// section that lists them above the page tree.
//
// Stored as a local preference (`favoritePages:<workspaceId>`, a JSON list of
// view ids in the order they were starred), not on the server. AppFlowy keeps
// favorites server-side so they follow the account; that is a schema change and
// an endpoint for a list one person curates on one machine, and the preference
// can move to the server later without changing anything that reads it here.

import 'dart:convert';

import 'package:flutter/material.dart';

import '../api/models.dart';
import '../l10n/locale_controller.dart';
import 'shell_widgets.dart' show kSidebarRowHeight, MenuRow;
import 'theme_tokens.dart';
import 'typography.dart';

/// The star's "on" colour, shared by the header toggle and the section icon.
const Color kFavoriteStarColor = Color(0xFFF5B400);

class FavoritePages extends ChangeNotifier {
  FavoritePages({required this.read, required this.write});

  final String? Function(String key) read;
  final void Function(String key, String value) write;
  final Map<String, List<String>> _ids = {};

  static String _key(String workspaceId) => 'favoritePages:$workspaceId';
  static const _collapsedKey = 'favoritesCollapsed';

  List<String> idsOf(String workspaceId) =>
      _ids[workspaceId] ??= _decode(read(_key(workspaceId)));

  bool contains(String workspaceId, String viewId) =>
      idsOf(workspaceId).contains(viewId);

  void toggle(String workspaceId, String viewId) {
    final ids = idsOf(workspaceId);
    if (!ids.remove(viewId)) ids.add(viewId);
    write(_key(workspaceId), jsonEncode(ids));
    notifyListeners();
  }

  bool get collapsed => read(_collapsedKey) == 'true';

  void toggleCollapsed() {
    write(_collapsedKey, (!collapsed).toString());
    notifyListeners();
  }

  /// A corrupt or foreign value reads as "no favorites" rather than throwing:
  /// a preference file is not worth losing the sidebar over.
  static List<String> _decode(String? raw) {
    if (raw == null || raw.isEmpty) return [];
    try {
      final list = jsonDecode(raw);
      if (list is List) return [...list.whereType<String>()];
    } on FormatException {
      // fall through
    }
    return [];
  }
}

/// The starred pages that still exist in [views], in starring order.
///
/// Ids are resolved, not removed, when a page disappears: a trashed page is
/// simply not in [views], and restoring it brings its star back with it.
List<DocumentView> favoriteViews(List<String> ids, List<DocumentView> views) {
  final byId = {for (final v in views) v.id: v};
  return [
    for (final id in ids)
      if (byId[id] case final v? when !v.isFolder) v,
  ];
}

/// 「★ 收藏 ⌄」 and its rows. Builds to nothing when the workspace has no
/// starred page that still exists, so a workspace without favorites keeps the
/// sidebar exactly as it was.
class FavoritesSection extends StatelessWidget {
  const FavoritesSection({
    super.key,
    required this.favorites,
    required this.workspaceId,
    required this.views,
    required this.selectedId,
    required this.onOpen,
  });

  final FavoritePages favorites;
  final String workspaceId;
  final List<DocumentView> views;
  final String? selectedId;
  final void Function(DocumentView view) onOpen;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: favorites,
    builder: (context, _) {
      final items = favoriteViews(favorites.idsOf(workspaceId), views);
      if (items.isEmpty) return const SizedBox.shrink();
      final theme = MicaTheme.of(context);
      final collapsed = favorites.collapsed;
      return Padding(
        padding: const EdgeInsets.only(top: 10),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          mainAxisSize: MainAxisSize.min,
          children: [
            InkWell(
              borderRadius: BorderRadius.circular(6),
              onTap: favorites.toggleCollapsed,
              child: SizedBox(
                height: 28,
                child: Row(
                  children: [
                    const SizedBox(width: 4),
                    const Icon(
                      Icons.star_rounded,
                      size: 16,
                      color: kFavoriteStarColor,
                    ),
                    const SizedBox(width: 6),
                    Text(
                      context.l10n.sidebarFavoritesLabel,
                      style: MicaTypography.caption.copyWith(
                        fontWeight: FontWeight.w500,
                        color: theme.text.faint,
                      ),
                    ),
                    const SizedBox(width: 2),
                    Icon(
                      collapsed ? Icons.chevron_right : Icons.expand_more,
                      size: 16,
                      color: theme.text.faint,
                    ),
                  ],
                ),
              ),
            ),
            if (!collapsed)
              // Capped so a long list cannot push the page tree off screen;
              // past the cap the section scrolls on its own.
              ConstrainedBox(
                constraints: const BoxConstraints(
                  maxHeight: 6 * kSidebarRowHeight,
                ),
                child: ListView(
                  shrinkWrap: true,
                  padding: EdgeInsets.zero,
                  children: [
                    for (final v in items)
                      _FavoriteRow(
                        view: v,
                        selected: v.id == selectedId,
                        onOpen: () => onOpen(v),
                        onRemove: () => favorites.toggle(workspaceId, v.id),
                      ),
                  ],
                ),
              ),
          ],
        ),
      );
    },
  );
}

class _FavoriteRow extends StatelessWidget {
  const _FavoriteRow({
    required this.view,
    required this.selected,
    required this.onOpen,
    required this.onRemove,
  });

  final DocumentView view;
  final bool selected;
  final VoidCallback onOpen;
  final VoidCallback onRemove;

  Future<void> _menu(BuildContext context, Offset global) async {
    final overlay =
        Overlay.of(context).context.findRenderObject() as RenderBox?;
    if (overlay == null) return;
    final at = overlay.globalToLocal(global);
    final picked = await showMenu<bool>(
      context: context,
      position: RelativeRect.fromLTRB(
        at.dx,
        at.dy,
        overlay.size.width - at.dx,
        0,
      ),
      items: [
        PopupMenuItem(
          height: 30,
          value: true,
          child: MenuRow(
            icon: Icons.star_outline_rounded,
            label: context.l10n.pageRemoveFavorite,
          ),
        ),
      ],
    );
    if (picked == true) onRemove();
  }

  @override
  Widget build(BuildContext context) {
    final theme = MicaTheme.of(context);
    final name = view.name.trim().isEmpty
        ? context.l10n.untitledPage
        : view.name;
    return Material(
      color: selected ? theme.surface.hover : Colors.transparent,
      borderRadius: BorderRadius.circular(6),
      child: InkWell(
        borderRadius: BorderRadius.circular(6),
        onTap: onOpen,
        onSecondaryTapDown: (d) => _menu(context, d.globalPosition),
        child: SizedBox(
          height: kSidebarRowHeight,
          child: Row(
            children: [
              // Same columns as a top-level tree row (2 + 18 toggle slot), so
              // the icons line up with the tree below.
              const SizedBox(width: 20),
              SizedBox(
                width: 18,
                height: 18,
                child: Center(
                  child: view.icon != null
                      ? Text(
                          view.icon!,
                          style: const TextStyle(fontSize: 16, height: 18 / 16),
                        )
                      : Icon(
                          Icons.description_outlined,
                          size: 18,
                          color: selected
                              ? theme.text.primary
                              : theme.text.muted,
                        ),
                ),
              ),
              const SizedBox(width: 6),
              Expanded(
                child: Text(
                  name,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: MicaTypography.body.copyWith(
                    color: theme.text.primary,
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
