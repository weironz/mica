import '../ui/typography.dart';
// The code block's language picker, split out of the editor library (its
// size ratchet): a self-contained anchored panel with no editor state.
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../l10n/locale_controller.dart';
import '../ui/theme_tokens.dart';
import 'highlight.dart' show kCodeLanguages;

/// Code-block language picker: a small anchored panel with a search box —
/// the language list is long, so filtering beats scrolling. ↑/↓ move the
/// highlight, Enter picks, Esc dismisses; pops with the chosen language.
class LanguagePicker extends StatefulWidget {
  const LanguagePicker({
    super.key,
    required this.anchor,
    required this.current,
  });

  final Offset anchor;

  /// The block's own setting — `auto` when it has none. Not the *resolved*
  /// language: the point is to show whether the block is pinned or detecting.
  final String current;

  @override
  State<LanguagePicker> createState() => _LanguagePickerState();
}

class _LanguagePickerState extends State<LanguagePicker> {
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
                      style: MicaTypography.small,
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
                                style: MicaTypography.small.copyWith(
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
                                        style: MicaTypography.small.copyWith(
                                          fontWeight: items[i] == widget.current
                                              ? FontWeight.w500
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
