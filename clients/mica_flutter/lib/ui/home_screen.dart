import 'package:flutter/material.dart';

import 'theme_tokens.dart';

/// The home screen: one "create page" affordance, then the two lists that get a
/// user back into work (recently edited, directories).
///
/// It used to open with a date line and 「下午好,X」. Both are gone. Neither
/// reference product has a greeting dashboard — AppFlowy lands you on the last
/// document (sidebar spaces + favourites), AFFiNE on `all-page` / journals — and
/// the greeting cost us a real bug on the way: in 本地模式 there is no account, so
/// the world's name was passed as the user's and the app said 「下午好,本地模式」.
/// What survives is the half that carries information: this is the only
/// cross-workspace entry point (the sidebar shows the ONE workspace you are in).
///
/// Design source: `docs/design/知识库软件开发协作/03 首页+侧栏.dc.html`, the
/// `isHome` branch of the right pane. Where the mockup's legacy half-step sizes
/// (13.5 / 14.5 / 11.5) and off-ladder radii (13 / 9 / 6 / 5) conflict with the
/// design rules in that folder's CLAUDE.md, the rules win and the value is
/// snapped to the nearest ladder step — the hierarchy (hero > card title >
/// section label > meta) is what carries the design, not the exact pixel:
///
/// - 18px create-card title → 20 (ladder has no 18; 20 keeps it clearly above
///   the 14 card names and 13 section labels, which 16 would not).
/// - 13.5 → 13, 14.5 → 14, 12.5 / 11.5 / 11 → 12 (12 is the floor of the ladder).
/// - radii 13 / 9 → 14, 6 / 5 → 8.
/// - slate fills (#F1F5F9) → `surface.sunken`, the app's own neutral fill; the
///   design rules ban introducing further slate tones, and the two are visually
///   interchangeable. Same for the #F5F7FA row rule → `border.subtle` and the
///   #DBE3EF container border → `border.normal`.
///
/// Deliberately dumb: it owns no data and talks to no service. Everything
/// arrives pre-formatted (dates, relative times, greeting) because the caller is
/// the one holding the locale and the user — this file must never grow a second
/// place where dates get formatted.
class HomeScreen extends StatelessWidget {
  const HomeScreen({
    required this.strings,
    required this.recents,
    required this.directories,
    required this.onOpen,
    required this.onCreatePage,
    super.key,
  });

  final HomeStrings strings;
  final List<HomeDocEntry> recents;
  final List<HomeDocEntry> directories;

  /// Opens a document or directory by id. The screen never decides what "open"
  /// means — a directory and a page land in different places.
  final void Function(String id) onOpen;

  final VoidCallback onCreatePage;

  @override
  Widget build(BuildContext context) {
    return SingleChildScrollView(
      child: Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 840),
          child: Padding(
            // Was 72 at the top to sit under a date line that no longer exists.
            padding: const EdgeInsets.fromLTRB(40, 48, 40, 120),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                _CreateCard(strings: strings, onTap: onCreatePage),
                const SizedBox(height: 48),
                _SectionHeader(
                  icon: Icons.schedule,
                  label: strings.recentLabel,
                ),
                const SizedBox(height: 16),
                if (recents.isEmpty)
                  _EmptyState(
                    title: strings.recentEmptyTitle,
                    body: strings.recentEmptyBody,
                  )
                else
                  for (final doc in recents)
                    _DirectoryRow(
                      entry: doc,
                      onOpen: onOpen,
                      fallbackIcon: Icons.description_outlined,
                    ),
                const SizedBox(height: 44),
                _SectionHeader(
                  icon: Icons.folder_outlined,
                  label: strings.directoriesLabel,
                ),
                const SizedBox(height: 8),
                if (directories.isEmpty)
                  _EmptyState(
                    title: strings.directoriesEmptyTitle,
                    body: strings.directoriesEmptyBody,
                  )
                else
                  for (final dir in directories)
                    _DirectoryRow(
                      entry: dir,
                      onOpen: onOpen,
                      fallbackIcon: Icons.folder_outlined,
                    ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// One document/directory row's worth of already-presentable text. A record, not
/// an API model: the home screen must not depend on the wire format, and `meta`
/// ("2 hours ago", "3 pages") is the caller's to phrase.
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
    required this.createSubtitle,
    required this.createHint,
    required this.recentLabel,
    required this.directoriesLabel,
    required this.recentEmptyTitle,
    required this.recentEmptyBody,
    required this.directoriesEmptyTitle,
    required this.directoriesEmptyBody,
  });

  final String createTitle;
  final String createSubtitle;

  /// The keyboard-hint chip, e.g. "⌘N" / "Ctrl+N". Caller picks per platform.
  final String createHint;

  final String recentLabel;
  final String directoriesLabel;

  /// Empty-state copy. Per the design rules each must say what happened *and*
  /// give one next step, and must never say "出错了" — an empty list is not a
  /// fault.
  final String recentEmptyTitle;
  final String recentEmptyBody;
  final String directoriesEmptyTitle;
  final String directoriesEmptyBody;
}

/// The bundled Roboto Mono. `'monospace'` as a family name does not resolve on
/// web, so mono text always names the packaged family (see CLAUDE.md). Declared
/// here rather than imported because `editor/model.dart` is out of this file's
/// dependency budget — one string is cheaper than a new coupling.
const String _monoFont = 'RobotoMono';

Color _cardBorder(BuildContext context) => MicaTheme.of(context).border.subtle;
Color _containerBorder(BuildContext context) =>
    MicaTheme.of(context).border.normal;

/// Hover border: an accent tint, not a grey, so it is exempt from the
/// three-greys rule.
Color _hoverBorder(BuildContext context) =>
    MicaTheme.of(context).accent.primary.withValues(alpha: 0.35);

/// The near-black tile behind the "+".
///
/// The one surface here that does NOT invert: it is already dark, and the "+" on
/// it is white. In dark mode #0F172A would vanish into the #0F1419 page, so there
/// it lifts to the hover surface — still a dark tile, still visibly a tile.
Color _createTileBg(BuildContext context) {
  final tokens = MicaTheme.of(context);
  return tokens.dark ? tokens.surface.hover : const Color(0xFF0F172A);
}

/// The card's near-white vertical wash. Two surface steps, so it stays a wash
/// rather than a grey slab when the palette flips.
LinearGradient _createGradient(BuildContext context) => LinearGradient(
  begin: Alignment.topCenter,
  end: Alignment.bottomCenter,
  colors: [
    MicaTheme.of(context).surface.base,
    MicaTheme.of(context).surface.raised,
  ],
);

/// Shared card hover treatment (`.card:hover` in the mockup): accent border plus
/// a soft accent shadow. Negative spread matches the CSS `-12px` inset.
List<BoxShadow>? _hoverShadow(BuildContext context, bool hovered) => hovered
    ? [
        BoxShadow(
          color: MicaTheme.of(context).accent.primary.withValues(alpha: 0.28),
          blurRadius: 24,
          spreadRadius: -12,
          offset: const Offset(0, 8),
        ),
      ]
    : null;

/// Pointer-aware tap target. Plain [GestureDetector] rather than [InkWell]: the
/// design's feedback is a border/shadow change, and a Material ripple over the
/// gradient card would fight it.
class _Hoverable extends StatefulWidget {
  const _Hoverable({required this.onTap, required this.builder});

  final VoidCallback onTap;
  final Widget Function(bool hovered) builder;

  @override
  State<_Hoverable> createState() => _HoverableState();
}

class _HoverableState extends State<_Hoverable> {
  bool _hovered = false;

  @override
  Widget build(BuildContext context) {
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: widget.onTap,
        child: widget.builder(_hovered),
      ),
    );
  }
}

class _CreateCard extends StatelessWidget {
  const _CreateCard({required this.strings, required this.onTap});

  final HomeStrings strings;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return _Hoverable(
      onTap: onTap,
      builder: (hovered) => Container(
        padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 22),
        decoration: BoxDecoration(
          gradient: _createGradient(context),
          // 1.5px, thicker than the document cards': this is the one thing on
          // the page a new user is meant to hit.
          border: Border.all(
            color: hovered ? _hoverBorder(context) : _containerBorder(context),
            width: 1.5,
          ),
          borderRadius: const BorderRadius.all(Radius.circular(16)),
          boxShadow: _hoverShadow(context, hovered),
        ),
        child: Row(
          children: [
            Container(
              width: 52,
              height: 52,
              alignment: Alignment.center,
              decoration: BoxDecoration(
                color: _createTileBg(context),
                borderRadius: BorderRadius.all(Radius.circular(14)),
              ),
              child: const Icon(Icons.add, size: 24, color: Colors.white),
            ),
            const SizedBox(width: 18),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    strings.createTitle,
                    style: TextStyle(
                      fontSize: 20,
                      fontWeight: FontWeight.w700,
                      letterSpacing: -0.3,
                      color: MicaTheme.of(context).text.primary,
                    ),
                  ),
                  const SizedBox(height: 3),
                  Text(
                    strings.createSubtitle,
                    style: TextStyle(
                      fontSize: 13,
                      color: MicaTheme.of(context).text.faint,
                    ),
                  ),
                ],
              ),
            ),
            const SizedBox(width: 18),
            Container(
              padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
              decoration: BoxDecoration(
                border: Border.fromBorderSide(
                  BorderSide(color: _containerBorder(context)),
                ),
                borderRadius: BorderRadius.all(Radius.circular(8)),
              ),
              child: Text(
                strings.createHint,
                style: TextStyle(
                  fontFamily: _monoFont,
                  fontSize: 12,
                  color: MicaTheme.of(context).text.faint,
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _SectionHeader extends StatelessWidget {
  const _SectionHeader({required this.icon, required this.label});

  final IconData icon;
  final String label;

  @override
  Widget build(BuildContext context) {
    return Row(
      children: [
        Icon(icon, size: 16, color: MicaTheme.of(context).text.faint),
        const SizedBox(width: 8),
        Text(
          label,
          style: TextStyle(
            fontSize: 13,
            fontWeight: FontWeight.w600,
            color: MicaTheme.of(context).text.muted,
          ),
        ),
      ],
    );
  }
}

/// A user-chosen emoji, or a neutral kind glyph when there is none. Never
/// invents an emoji: a made-up icon reads as user data the user did not choose.
class _EntryIcon extends StatelessWidget {
  const _EntryIcon({
    required this.icon,
    required this.fallback,
    required this.size,
  });

  final String? icon;
  final IconData fallback;
  final double size;

  @override
  Widget build(BuildContext context) {
    final emoji = icon;
    if (emoji == null || emoji.isEmpty) {
      return Icon(
        fallback,
        size: size,
        color: MicaTheme.of(context).text.faint,
      );
    }
    return Text(emoji, style: TextStyle(fontSize: size));
  }
}

class _WorkspaceChip extends StatelessWidget {
  const _WorkspaceChip({required this.name});

  final String name;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 7, vertical: 2),
      decoration: BoxDecoration(
        color: MicaTheme.of(context).surface.sunken,
        borderRadius: BorderRadius.all(Radius.circular(8)),
      ),
      child: Text(
        name,
        style: TextStyle(fontSize: 12, color: MicaTheme.of(context).text.muted),
      ),
    );
  }
}

/// The recents grid. Collapses 3 → 2 → 1 columns so a card never gets narrower
/// than roughly 180px, which is where the two-line name plus the chip row starts
/// to wrap into mush. Thresholds are on the grid's own width (content width,
/// i.e. after the 40px page padding): 3 columns from 600px, 2 from 380px.
class _DirectoryRow extends StatelessWidget {
  const _DirectoryRow({
    required this.entry,
    required this.onOpen,
    required this.fallbackIcon,
  });

  final HomeDocEntry entry;
  final void Function(String id) onOpen;

  /// Glyph for an entry with no emoji of its own. Passed in, because
  /// `HomeDocEntry` carries no kind and the row cannot tell a page from a folder
  /// — reusing this row for the recents list without it made every icon-less
  /// PAGE look like a FOLDER. The list that owns the entries is the thing that
  /// knows which they are.
  final IconData fallbackIcon;

  @override
  Widget build(BuildContext context) {
    return _Hoverable(
      onTap: () => onOpen(entry.id),
      builder: (hovered) => Container(
        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 12),
        decoration: BoxDecoration(
          color: hovered ? MicaTheme.of(context).surface.sunken : null,
          borderRadius: const BorderRadius.all(Radius.circular(8)),
          border: Border(bottom: BorderSide(color: _cardBorder(context))),
        ),
        child: Row(
          children: [
            _EntryIcon(icon: entry.icon, fallback: fallbackIcon, size: 19),
            const SizedBox(width: 12),
            Expanded(
              child: Text(
                entry.name,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  fontSize: 14,
                  fontWeight: FontWeight.w500,
                  color: MicaTheme.of(context).text.primary,
                ),
              ),
            ),
            const SizedBox(width: 8),
            _WorkspaceChip(name: entry.workspaceName),
            const SizedBox(width: 8),
            // Fixed min width + right alignment so the metas line up down the
            // list rather than ragging with each name's length.
            SizedBox(
              width: 56,
              child: Text(
                entry.meta,
                textAlign: TextAlign.right,
                style: TextStyle(
                  fontFamily: _monoFont,
                  fontSize: 12,
                  color: MicaTheme.of(context).text.faint,
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// An empty list is not a failure, so it gets prose, not a bald grid: what the
/// list is, plus the one thing to do about it (copy comes from [HomeStrings]).
class _EmptyState extends StatelessWidget {
  const _EmptyState({required this.title, required this.body});

  final String title;
  final String body;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 26, vertical: 40),
      alignment: Alignment.center,
      child: Column(
        children: [
          Container(
            width: 46,
            height: 46,
            alignment: Alignment.center,
            decoration: BoxDecoration(
              color: MicaTheme.of(context).surface.sunken,
              borderRadius: BorderRadius.all(Radius.circular(14)),
            ),
            child: Icon(
              Icons.article_outlined,
              size: 22,
              color: MicaTheme.of(context).text.faint,
            ),
          ),
          const SizedBox(height: 11),
          Text(
            title,
            textAlign: TextAlign.center,
            style: TextStyle(
              fontSize: 14,
              fontWeight: FontWeight.w600,
              color: MicaTheme.of(context).text.primary,
            ),
          ),
          const SizedBox(height: 9),
          Text(
            body,
            textAlign: TextAlign.center,
            style: TextStyle(
              fontSize: 12,
              height: 1.75,
              color: MicaTheme.of(context).text.faint,
            ),
          ),
        ],
      ),
    );
  }
}
