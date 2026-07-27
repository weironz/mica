// Modal dialogs for the Mica client (search / settings / AI /
// recycle bin). `part of main.dart` — same library, so they keep using its
// imports and private helpers. Extracted 2026-07 for navigability.
part of '../main.dart';

/// Workspace search: type to find pages by title or body text; click to open.
/// Fixed row height in the search result list.
///
/// Fixed so the keyboard selection can be scrolled into view by arithmetic
/// (`index * extent`) rather than hanging a GlobalKey off every row.
const double _searchRowHeight = 64;

/// Move the search result selection by [delta] rows.
///
/// A real Intent rather than a `CallbackShortcuts` binding, because of where the
/// key events go. `CallbackShortcuts` is a `Focus.onKeyEvent`, and those run from
/// the INNERMOST focused node outwards — the focused node here is inside the
/// TextField, whose `DefaultTextEditingShortcuts` map ArrowUp/ArrowDown to
/// move-caret-to-start/end and consume them. `Shortcuts` placed BETWEEN the
/// TextField and the app is nearer than the default map, so it wins the lookup;
/// the intent then resolves against the [Actions] wrapper below.
class _SearchMoveIntent extends Intent {
  const _SearchMoveIntent(this.delta);

  final int delta;
}

class _SearchDialog extends StatefulWidget {
  const _SearchDialog({
    required this.onSearch,
    required this.onOpen,
    this.initialQuery,
  });

  /// Null when the active world has no workspace search at all (本地模式), which
  /// is not the same thing as a search that finds nothing. It used to be wired
  /// to a stub returning `const []`, so every local query answered 「没有找到与
  /// 「x」匹配的内容」 — the app stating as fact that the page isn't there.
  final Future<List<SearchResult>> Function(String query)? onSearch;
  final void Function(String viewId) onOpen;

  /// Pre-filled query, run immediately on open — e.g. clicking a page-property
  /// tag opens search already looking for that tag.
  final String? initialQuery;

  @override
  State<_SearchDialog> createState() => _SearchDialogState();
}

class _SearchDialogState extends State<_SearchDialog> {
  final _query = TextEditingController();
  Timer? _debounce;
  bool _loading = false;
  bool _failed = false;
  List<SearchResult> _results = const [];
  String _lastQuery = '';

  /// Keyboard-highlighted row, -1 when none. Separate from "hovered": the whole
  /// point is to be able to pick a result without touching the mouse.
  int _selected = -1;
  final _listScroll = ScrollController();

  @override
  void initState() {
    super.initState();
    final q = widget.initialQuery?.trim();
    if (q != null && q.isNotEmpty) {
      _query.text = q;
      // Run after first frame so the initial setState in _run is safe.
      WidgetsBinding.instance.addPostFrameCallback((_) => _run(q));
    }
  }

  @override
  void dispose() {
    _debounce?.cancel();
    _query.dispose();
    _listScroll.dispose();
    super.dispose();
  }

  void _onChanged(String value) {
    _debounce?.cancel();
    _debounce = Timer(const Duration(milliseconds: 280), () => _run(value));
  }

  Future<void> _run(String value) async {
    final search = widget.onSearch;
    if (search == null) return; // no search in this world; see [_buildResults]
    final query = value.trim();
    if (query.isEmpty) {
      setState(() {
        _results = const [];
        _loading = false;
        _selected = -1;
      });
      return;
    }
    setState(() => _loading = true);
    try {
      final results = await search(query);
      if (!mounted || _query.text.trim() != query) return;
      setState(() {
        _results = results;
        _lastQuery = query;
        _loading = false;
        _failed = false;
        // Preselect the top hit: ↵ should open the obvious answer without an
        // extra ↓ first. -1 when the query found nothing.
        _selected = results.isEmpty ? -1 : 0;
      });
    } catch (_) {
      // Surface the failure — a swallowed error reads as "no results" and
      // hides real breakage (this dialog masked a 404 for a while).
      if (mounted) {
        setState(() {
          _loading = false;
          _failed = true;
          _results = const [];
          _lastQuery = query;
          _selected = -1;
        });
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: Text(context.l10n.searchTitle),
      content: SizedBox(
        width: 480,
        height: 420,
        child: Column(
          children: [
            Shortcuts(
              shortcuts: const {
                SingleActivator(LogicalKeyboardKey.arrowDown):
                    _SearchMoveIntent(1),
                SingleActivator(LogicalKeyboardKey.arrowUp): _SearchMoveIntent(
                  -1,
                ),
              },
              child: Actions(
                actions: {
                  _SearchMoveIntent: CallbackAction<_SearchMoveIntent>(
                    onInvoke: (intent) {
                      _move(intent.delta);
                      return null;
                    },
                  ),
                },
                child: TextField(
                  controller: _query,
                  // Nothing to type into when there is no search behind it: an
                  // enabled field that eats your keystrokes and answers "no results"
                  // is worse than one that is visibly unavailable.
                  enabled: widget.onSearch != null,
                  autofocus: widget.onSearch != null,
                  onChanged: _onChanged,
                  // ↵ opens the highlighted row. Not a Shortcuts binding: Enter in a
                  // single-line field already means "submit", and hijacking it higher
                  // up would also swallow IME confirmation.
                  onSubmitted: (_) => _openSelected(),
                  decoration: InputDecoration(
                    hintText: context.l10n.searchHint,
                    prefixIcon: const Icon(Icons.search),
                    suffixIcon: _loading
                        ? const Padding(
                            padding: EdgeInsets.all(12),
                            child: SizedBox(
                              width: 16,
                              height: 16,
                              child: CircularProgressIndicator(strokeWidth: 2),
                            ),
                          )
                        : null,
                    border: const OutlineInputBorder(),
                  ),
                ),
              ),
            ),
            const SizedBox(height: 12),
            Expanded(child: _buildResults(context)),
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(context.l10n.commonClose),
        ),
      ],
    );
  }

  /// Move the keyboard selection and keep it on screen.
  void _move(int delta) {
    final next = moveSelection(
      current: _selected,
      count: _results.length,
      delta: delta,
    );
    if (next == _selected) return;
    setState(() => _selected = next);
    if (next < 0 || !_listScroll.hasClients) return;
    // Rows are a fixed height here, so the offset is computable — no need for a
    // per-row GlobalKey just to scroll.
    const rowExtent = _searchRowHeight;
    final target = next * rowExtent;
    final top = _listScroll.offset;
    final bottom = top + _listScroll.position.viewportDimension - rowExtent;
    if (target < top) {
      _listScroll.jumpTo(target);
    } else if (target > bottom) {
      _listScroll.jumpTo(
        (target - _listScroll.position.viewportDimension + rowExtent).clamp(
          0.0,
          _listScroll.position.maxScrollExtent,
        ),
      );
    }
  }

  /// Open the keyboard-selected result, if there is one.
  void _openSelected() {
    if (_selected < 0 || _selected >= _results.length) return;
    widget.onOpen(_results[_selected].viewId);
  }

  /// The snippet with query matches tinted.
  ///
  /// Case-insensitive to match the server's `ILIKE` — see [highlightRuns]; a
  /// case-sensitive version would return real hits with nothing marked in them.
  Widget _snippet(BuildContext context, String text) {
    final runs = highlightRuns(text, _lastQuery);
    return Text.rich(
      TextSpan(
        children: [
          for (final r in runs)
            TextSpan(
              text: r.text,
              style: r.hit
                  ? const TextStyle(backgroundColor: Color(0xFFFEF08A))
                  : null,
            ),
        ],
      ),
      maxLines: 2,
      overflow: TextOverflow.ellipsis,
    );
  }

  Widget _buildResults(BuildContext context) {
    // Say the true thing: this world has no workspace search yet, and point at
    // the tool that does work here (in-page find). The old stub claimed instead
    // that the workspace contains no such page.
    if (widget.onSearch == null) {
      return EmptyState(
        icon: Icons.search_off,
        title: context.l10n.searchLocalUnsupportedTitle,
        detail: context.l10n.searchLocalUnsupportedDetail,
      );
    }
    if (_query.text.trim().isEmpty) {
      return EmptyState(
        icon: Icons.search,
        title: context.l10n.searchEmptyTitle,
        detail: context.l10n.searchEmptyDetail,
      );
    }
    if (!_loading && _results.isEmpty) {
      // A failure is not an empty result set. Both used to render under the
      // title 「无匹配结果」, which told the user their workspace has no such
      // page when in fact the request never reached the server — and it hid
      // the one useful next step (retry). Separate icon, title and action.
      if (_failed) {
        return EmptyState(
          icon: Icons.cloud_off,
          title: context.l10n.searchFailedTitle,
          detail: context.l10n.searchFailed,
          actionLabel: context.l10n.commonRetry,
          onAction: () => _run(_query.text),
        );
      }
      return EmptyState(
        icon: Icons.search_off,
        title: context.l10n.searchNoMatches,
        detail: context.l10n.searchNothingFound(_lastQuery),
      );
    }
    return Column(
      children: [
        // How many hits, so a long list doesn't have to be counted by eye.
        Align(
          alignment: Alignment.centerLeft,
          child: Padding(
            padding: const EdgeInsets.only(left: 4, bottom: 6),
            child: Text(
              context.l10n.searchResultCount(_results.length),
              style: const TextStyle(
                fontSize: 11,
                fontWeight: FontWeight.w600,
                letterSpacing: 0.6,
                color: EditorTheme.faint,
              ),
            ),
          ),
        ),
        Expanded(
          child: ListView.builder(
            controller: _listScroll,
            // Fixed extent so [_move] can scroll the selection into view by
            // arithmetic instead of hanging a GlobalKey off every row.
            itemExtent: _searchRowHeight,
            itemCount: _results.length,
            itemBuilder: (context, i) {
              final result = _results[i];
              return Container(
                color: i == _selected ? const Color(0xFFEFF6FF) : null,
                child: ListTile(
                  contentPadding: const EdgeInsets.symmetric(horizontal: 4),
                  leading: const Icon(Icons.description_outlined, size: 18),
                  title: Text(result.name, overflow: TextOverflow.ellipsis),
                  subtitle: result.snippet.isEmpty
                      ? (result.titleMatch
                            ? Text(context.l10n.searchTitleMatch)
                            : null)
                      : _snippet(context, result.snippet),
                  onTap: () => widget.onOpen(result.viewId),
                ),
              );
            },
          ),
        ),
      ],
    );
  }
}

/// System prompt for AI that should produce a whole document with a title line.
const String kAiDocSystem =
    'You are a writing assistant inside a Markdown document editor. Respond with '
    'clean GitHub-Flavored Markdown only — no preamble or commentary. Begin the '
    'document with a single level-1 heading (a "# Title" line).';

enum _AiTarget { newPage, currentPage, newWorkspace }

/// Preset AI providers. Each maps to a backend provider dialect (openai/anthropic)
/// plus default base URL + model; "Local / Custom" lets the user point at any
/// OpenAI-compatible server (Ollama, LM Studio, vLLM, …).
enum _AiPreset { deepseek, openai, anthropic, custom }

extension _AiPresetInfo on _AiPreset {
  String get label => switch (this) {
    _AiPreset.deepseek => 'DeepSeek',
    _AiPreset.openai => 'OpenAI',
    _AiPreset.anthropic => 'Anthropic (Claude)',
    _AiPreset.custom => 'Local / Custom',
  };

  String get provider => this == _AiPreset.anthropic ? 'anthropic' : 'openai';

  String get baseUrl => switch (this) {
    _AiPreset.deepseek => 'https://api.deepseek.com',
    _AiPreset.openai => 'https://api.openai.com/v1',
    _AiPreset.anthropic => 'https://api.anthropic.com',
    _AiPreset.custom => 'http://localhost:11434/v1',
  };

  String get model => switch (this) {
    _AiPreset.deepseek => 'deepseek-chat',
    _AiPreset.openai => 'gpt-4o-mini',
    _AiPreset.anthropic => 'claude-sonnet-4-6',
    _AiPreset.custom => '',
  };
}

/// Settings dialog. Currently hosts the AI provider configuration; appearance and
/// account sections will slot in alongside it.
class _SettingsDialog extends StatefulWidget {
  const _SettingsDialog({
    required this.onLoadAiSettings,
    required this.onSaveAiSettings,
    this.onLoadTokens,
    this.onCreateToken,
    this.onRevokeToken,
    required this.userName,
    required this.userEmail,
    required this.onUpdateProfile,
    required this.onChangePassword,
    required this.onDeleteAccount,
    required this.appearance,
    required this.pageWidth,
    required this.reHostImages,
    required this.onReHostImagesChanged,
    required this.showFormatBar,
    required this.onShowFormatBarChanged,
    required this.showPageTitle,
    required this.onShowPageTitleChanged,
    required this.aiEnabled,
    required this.onAiEnabledChanged,
    required this.onAppearanceChanged,
    required this.onImportWorkspace,
    this.onExportAllWorkspaces,
  });

  final String userName;
  final String userEmail;

  /// Null in 本地模式 — there is no account to edit, so the Account tab is not
  /// offered at all. Null, not a do-nothing function: the same distinction
  /// [onLoadTokens] already draws. A no-op here rendered a whole page of live
  /// controls — a Display name you could type into, a Save button, a password
  /// change — that silently did nothing.
  final Future<void> Function(String displayName)? onUpdateProfile;
  final Future<void> Function(String current, String next)? onChangePassword;

  /// Null in 本地模式 — no account to delete. Deletes the cloud account and
  /// every workspace it owns (server cascade); the caller signs out on success.
  final Future<void> Function(String password)? onDeleteAccount;

  /// Null in 本地模式 — AI settings live on the server, so there is nothing to
  /// configure and the tab is absent. Null, not a no-op: same rule as
  /// [onUpdateProfile] and [onLoadTokens]. These two were the stragglers, and
  /// a no-op here meant a whole provider form — base URL, model, API key —
  /// that took your typing and dropped it.
  final Future<Map<String, dynamic>> Function()? onLoadAiSettings;
  final Future<void> Function({
    required String provider,
    required String baseUrl,
    required String model,
    String? apiKey,
  })?
  onSaveAiSettings;
  final Future<List<Map<String, dynamic>>> Function()? onLoadTokens;
  final Future<Map<String, dynamic>> Function(
    String name,
    List<String> scopes,
    int? expiresInDays,
  )?
  onCreateToken;
  final Future<void> Function(String id)? onRevokeToken;
  final EditorAppearance appearance;
  final double pageWidth;
  final bool reHostImages;
  final void Function(bool value) onReHostImagesChanged;
  final bool showFormatBar;
  final void Function(bool value) onShowFormatBarChanged;
  final bool showPageTitle;
  final void Function(bool value) onShowPageTitleChanged;
  final bool aiEnabled;
  final void Function(bool value) onAiEnabledChanged;
  final void Function(EditorAppearance appearance, double pageWidth)
  onAppearanceChanged;
  final Future<void> Function() onImportWorkspace;

  /// Null in 本地模式 — "export all workspaces" is a cloud endpoint
  /// (`GET /api/workspaces/export.zip`, every workspace this account belongs
  /// to). Null hides the button rather than offering a dead control.
  final Future<void> Function()? onExportAllWorkspaces;

  @override
  State<_SettingsDialog> createState() => _SettingsDialogState();
}

class _SettingsDialogState extends State<_SettingsDialog> {
  final _baseUrl = TextEditingController();
  final _model = TextEditingController();
  final _apiKey = TextEditingController();
  _AiPreset _preset = _AiPreset.deepseek;

  /// The AI fields commit when they lose focus — settings here apply as you
  /// touch them, and a text field's equivalent of a toggle flipping is you
  /// finishing with it. Not on every keystroke: each save is a network call,
  /// and half an API key is not a value anyone wants stored.
  late final List<FocusNode> _aiFocus = [
    for (var i = 0; i < 3; i++) FocusNode()..addListener(_saveAiOnBlur),
  ];

  /// What the server already has, so blurring an untouched field is silent.
  String _aiSaved = '';

  String get _aiNow =>
      '${_preset.provider}|${_baseUrl.text.trim()}|${_model.text.trim()}|'
      '${_apiKey.text.trim()}';

  /// Index into the built tab list. Not persisted, so reordering the list is
  /// safe — 0 is whatever comes first (外观).
  int _tab = 0;

  /// Gates the WHOLE dialog (build: `_loading ? spinner : the tabs`), but the
  /// only thing it ever waits for is [_load]'s AI-settings fetch. So it starts
  /// true only when there is a fetch: in 本地模式 [onLoadAiSettings] is null,
  /// _load returns straight away, and every line that clears this sits after
  /// that return — Settings was a spinner that never resolved.
  late bool _loading = widget.onLoadAiSettings != null;
  bool _saving = false;
  bool _hasKey = false;
  // API Tokens tab state.
  List<Map<String, dynamic>>? _tokens;
  bool _tokensLoaded = false;
  bool _tokenBusy = false;
  bool _tokenWrite = false;
  final _tokenName = TextEditingController();

  /// Chosen token lifetime in days; null = never expires.
  ///
  /// Was a free-text numeric field parsed with `int.tryParse`, so any
  /// non-number silently became null — you typed a lifetime, agreed to it,
  /// and were handed a token that never expires. A closed set of choices
  /// removes the failure instead of validating against it.
  int? _tokenExpiryDays;
  String? _tokensError;
  String? _error;

  late final _name = TextEditingController(text: widget.userName);
  final _curPass = TextEditingController();
  final _newPass = TextEditingController();
  bool _accountBusy = false;
  String? _accountMsg;

  late double _fontScale = widget.appearance.fontScale;
  late String? _fontFamily = widget.appearance.fontFamily;
  late double _pageWidth = widget.pageWidth;
  late bool _reHostImages = widget.reHostImages;

  /// Read straight from prefs rather than passed in: nothing else in the app
  /// reacts to it, so threading it through the widget tree would be ceremony.
  late bool _diagnostics = diagnosticsOn;
  late bool _showFormatBar = widget.showFormatBar;
  // Read straight from prefs rather than threaded through widget params: the
  // window layer owns this one, and Settings is its only editor.
  late String _closeBehavior = loadCloseBehavior();
  late bool _showPageTitle = widget.showPageTitle;
  late bool _aiEnabled = widget.aiEnabled;

  @override
  void initState() {
    super.initState();
    _load();
  }

  void _applyAppearance() {
    widget.onAppearanceChanged(
      EditorAppearance(fontScale: _fontScale, fontFamily: _fontFamily),
      _pageWidth,
    );
  }

  /// Page width as 11 discrete stops (AppFlowy-style), plus a reset to the
  /// readable default. Fixed range (not window-relative) so the stops are stable
  /// "levels"; the editor caps the render at the window, so a wide stop on a
  /// small window just fills it. Reset → [kPageWidthDefault], NOT full width.
  Widget _pageWidthRow(BuildContext context) {
    final w = _pageWidth.clamp(kPageWidthMin, kPageWidthMax);
    final atDefault = w.round() == kPageWidthDefault.round();
    return Row(
      children: [
        SizedBox(width: 90, child: Text(context.l10n.settingsPageWidth)),
        Expanded(
          child: Slider(
            value: w,
            min: kPageWidthMin,
            max: kPageWidthMax,
            divisions: kPageWidthDivisions,
            onChanged: (value) {
              setState(() => _pageWidth = value);
              _applyAppearance();
            },
          ),
        ),
        SizedBox(
          width: 52,
          child: Text(
            '${w.round()} px',
            textAlign: TextAlign.right,
            style: const TextStyle(color: EditorTheme.muted, fontSize: 13),
          ),
        ),
        IconButton(
          tooltip: context.l10n.settingsResetPageWidth,
          visualDensity: VisualDensity.compact,
          onPressed: atDefault
              ? null
              : () {
                  setState(() => _pageWidth = kPageWidthDefault);
                  _applyAppearance();
                },
          icon: const Icon(Icons.restart_alt, size: 18),
        ),
      ],
    );
  }

  Widget _sliderRow({
    required String label,
    required double value,
    required double min,
    required double max,
    required String display,
    required ValueChanged<double> onChanged,
  }) {
    return Row(
      children: [
        SizedBox(width: 90, child: Text(label)),
        Expanded(
          child: Slider(value: value, min: min, max: max, onChanged: onChanged),
        ),
        SizedBox(
          width: 56,
          child: Text(
            display,
            textAlign: TextAlign.right,
            style: const TextStyle(color: EditorTheme.muted, fontSize: 13),
          ),
        ),
      ],
    );
  }

  Widget _fontChip(String label, String? family) {
    return ChoiceChip(
      label: Text(label),
      selected: _fontFamily == family,
      onSelected: (_) {
        setState(() => _fontFamily = family);
        _applyAppearance();
      },
    );
  }

  /// UI-language chip. Writes through [setLanguage] (persists + flips
  /// localeController), which rebuilds MaterialApp — the whole app, including
  /// this open dialog, re-renders in the chosen language immediately.
  Widget _langChip(String label, String choice) {
    return ChoiceChip(
      label: Text(label),
      selected: currentLanguageChoice == choice,
      onSelected: (_) => setState(() => setLanguage(choice)),
    );
  }

  @override
  void dispose() {
    _baseUrl.dispose();
    _model.dispose();
    _apiKey.dispose();
    for (final f in _aiFocus) {
      f.dispose();
    }
    _name.dispose();
    _curPass.dispose();
    _newPass.dispose();
    _tokenName.dispose();
    super.dispose();
  }

  Future<void> _saveProfile() async {
    final l10n = context.l10n;
    setState(() {
      _accountBusy = true;
      _accountMsg = null;
    });
    try {
      await widget.onUpdateProfile!(_name.text.trim());
      // Stay open: saving is not leaving. ("like server config" — the thing it
      // copied — no longer closes either; nothing in Settings does.)
      if (mounted) setState(() => _accountMsg = l10n.accountSaved);
    } catch (error) {
      if (mounted) setState(() => _accountMsg = error.toString());
    } finally {
      if (mounted) setState(() => _accountBusy = false);
    }
  }

  Future<void> _changeAccountPassword() async {
    final l10n = context.l10n;
    if (_newPass.text.length < 8) {
      setState(() => _accountMsg = l10n.accountPasswordTooShort);
      return;
    }
    setState(() {
      _accountBusy = true;
      _accountMsg = null;
    });
    try {
      await widget.onChangePassword!(_curPass.text, _newPass.text);
      if (!mounted) return;
      // Stay open, and clear the fields — leaving a password sitting in a live
      // text box is the reason closing felt like the tidy option.
      _curPass.clear();
      _newPass.clear();
      // Accurate, not reassuring: change_password revokes EVERY family of this
      // user (auth.rs `revoke_user_sessions`) — this device included. Saying
      // "other devices" would be a lie, and the surprise would land later, when
      // this session quietly fails to renew.
      setState(() => _accountMsg = l10n.accountPasswordChanged);
    } catch (error) {
      if (mounted) setState(() => _accountMsg = error.toString());
    } finally {
      if (mounted) setState(() => _accountBusy = false);
    }
  }

  Future<void> _deleteAccount() async {
    // Password re-entry lives in the confirm dialog, not the Account form: it is
    // asked for once, right next to the irreversible warning, and never sits in
    // a live field afterwards. A local controller keeps it out of _SettingsDialog
    // state entirely — it dies with the dialog.
    final passCtl = TextEditingController();
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (dialogContext) {
        final l10n = dialogContext.l10n;
        return AlertDialog(
          title: Text(l10n.accountDeleteTitle),
          content: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                l10n.accountDeleteWarning,
                style: const TextStyle(color: EditorTheme.muted, fontSize: 13),
              ),
              const SizedBox(height: 16),
              TextField(
                controller: passCtl,
                obscureText: true,
                autofocus: true,
                decoration: InputDecoration(
                  labelText: l10n.accountDeletePasswordLabel,
                  border: const OutlineInputBorder(),
                ),
              ),
            ],
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.of(dialogContext).pop(false),
              child: Text(l10n.commonCancel),
            ),
            FilledButton(
              style: FilledButton.styleFrom(
                backgroundColor: const Color(0xFFDC2626),
              ),
              onPressed: () => Navigator.of(dialogContext).pop(true),
              child: Text(l10n.accountDeleteConfirm),
            ),
          ],
        );
      },
    );
    if (confirmed != true) {
      passCtl.dispose();
      return;
    }
    final password = passCtl.text;
    passCtl.dispose();
    setState(() {
      _accountBusy = true;
      _accountMsg = null;
    });
    try {
      await widget.onDeleteAccount!(password);
      // Success tears down the session (the account is gone) — the parent's
      // callback signs out, which pops Settings with it. Nothing to show here.
    } catch (error) {
      if (mounted) {
        setState(() {
          _accountMsg = error.toString();
          _accountBusy = false;
        });
      }
    }
  }

  Future<void> _load() async {
    // 本地模式: no AI provider to load, and no AI tab to load it into. initState
    // calls this unconditionally, so the absence has to be handled here.
    final load = widget.onLoadAiSettings;
    if (load == null) return;
    try {
      final settings = await load();
      if (!mounted) return;
      final provider = settings['provider'] as String? ?? 'openai';
      final base = settings['base_url'] as String? ?? '';
      final model = settings['model'] as String? ?? '';
      setState(() {
        _preset = _presetFor(provider, base);
        _baseUrl.text = base.isEmpty ? _preset.baseUrl : base;
        _model.text = model.isEmpty ? _preset.model : model;
        _hasKey = settings['has_key'] == true;
        _loading = false;
      });
      // Baseline: what the server already has. Without it, merely tabbing
      // through the AI fields would look like a change and save on blur.
      _aiSaved = _aiNow;
    } catch (error) {
      if (!mounted) return;
      setState(() {
        _error = error.toString();
        _loading = false;
      });
    }
  }

  _AiPreset _presetFor(String provider, String base) {
    if (provider == 'anthropic') return _AiPreset.anthropic;
    if (base.contains('deepseek')) return _AiPreset.deepseek;
    if (base.contains('openai.com')) return _AiPreset.openai;
    return base.isEmpty ? _AiPreset.deepseek : _AiPreset.custom;
  }

  void _applyPreset(_AiPreset preset) {
    setState(() {
      _preset = preset;
      if (preset != _AiPreset.custom) {
        _baseUrl.text = preset.baseUrl;
        _model.text = preset.model;
      }
    });
    // Commit it: this rewrites the provider, url and model, and it blurs
    // nothing — so nothing else would ever carry the change to the server.
    // An empty key field means "leave the stored key alone", so switching
    // preset can't cost you the key you already saved.
    if (_aiNow != _aiSaved) unawaited(_saveAi());
  }

  /// The built-in About popup, marking the current app version. Opened from the
  /// Settings nav's "About" item (stacks over the Settings dialog).
  void _showAboutDialog(BuildContext context) {
    showAboutDialog(
      context: context,
      applicationName: 'Mica',
      applicationVersion: 'v$kAppVersion',
      applicationIcon: const MicaLogo(size: 40),
      applicationLegalese: context.l10n.aboutLegalese,
      children: [
        const SizedBox(height: 16),
        // AGPL-3.0-or-later §13: anyone interacting with Mica over a network must
        // be prominently offered the Corresponding Source. Link the public repo;
        // the version shown above tells them which tag to check out.
        Align(
          alignment: Alignment.centerLeft,
          child: TextButton.icon(
            onPressed: () => openUrl('https://github.com/weironz/mica'),
            icon: const Icon(Icons.code, size: 16),
            label: Text(context.l10n.aboutSourceCode),
          ),
        ),
        // Self-update lives here on desktop; hidden where it can't apply (web, and
        // platforms with no packaged installer).
        if (updateSupported) ...const [SizedBox(height: 8), UpdateChecker()],
      ],
    );
  }

  Future<void> _showTokenSecret(Map<String, dynamic> created) {
    final token = created['token'] as String? ?? '';
    return showDialog<void>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text(context.l10n.tokenCreated),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(context.l10n.tokenCopyNow),
            const SizedBox(height: 12),
            Container(
              width: double.infinity,
              padding: const EdgeInsets.all(10),
              decoration: BoxDecoration(
                color: const Color(0xFFF1F5F9),
                borderRadius: BorderRadius.circular(6),
              ),
              child: SelectableText(
                token,
                style: const TextStyle(fontSize: 13),
              ),
            ),
          ],
        ),
        actions: [
          TextButton.icon(
            icon: const Icon(Icons.copy, size: 18),
            label: Text(context.l10n.commonCopy),
            onPressed: () => Clipboard.setData(ClipboardData(text: token)),
          ),
          FilledButton(
            onPressed: () => Navigator.of(ctx).pop(),
            child: Text(context.l10n.commonDone),
          ),
        ],
      ),
    );
  }

  /// One settings nav row. Active = accent wash + accent ink + 600, rather than
  /// Material's `selected` (a tinted title only), which at 180px read as barely
  /// distinguishable from its neighbours.
  Widget _navRow(BuildContext context, int i, String title, IconData icon) {
    final active = _tab == i;
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 1),
      child: Material(
        color: active ? const Color(0xFFEFF6FF) : Colors.transparent,
        borderRadius: BorderRadius.circular(8),
        child: InkWell(
          borderRadius: BorderRadius.circular(8),
          onTap: () => setState(() => _tab = i),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 9),
            child: Row(
              children: [
                Icon(
                  icon,
                  size: 18,
                  color: active ? const Color(0xFF2563EB) : EditorTheme.muted,
                ),
                const SizedBox(width: 10),
                Expanded(
                  child: Text(
                    title,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      fontSize: 13,
                      fontWeight: active ? FontWeight.w600 : FontWeight.w400,
                      color: active
                          ? const Color(0xFF2563EB)
                          : EditorTheme.text,
                    ),
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  /// Whether this token's `expires_at` is already in the past.
  ///
  /// An unparseable or absent value reads as NOT expired: guessing "expired"
  /// would dim a live token and invite someone to revoke a key their backups
  /// still depend on.
  bool _tokenExpired(dynamic expiresAt) {
    if (expiresAt == null) return false;
    final at = DateTime.tryParse(expiresAt.toString());
    if (at == null) return false;
    return at.isBefore(DateTime.now());
  }

  /// Read-only vs read-write, as a badge.
  ///
  /// The meta line used to print the server's own scope list — `read` /
  /// `read, write` — straight at the user: English, comma-joined, and in a
  /// Chinese UI. What a person needs from it is one bit, and write access is the
  /// half worth colouring.
  Widget _scopeBadge(BuildContext context, dynamic scopes) {
    final list =
        (scopes as List<dynamic>?)?.map((e) => '$e').toList() ?? const [];
    final canWrite = list.contains('write');
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 7, vertical: 1),
      decoration: BoxDecoration(
        color: canWrite ? const Color(0xFFFEF3C7) : const Color(0xFFF4F4F6),
        borderRadius: BorderRadius.circular(5),
      ),
      child: Text(
        canWrite ? context.l10n.tokenScopeWrite : context.l10n.tokenScopeRead,
        style: TextStyle(
          fontSize: 11,
          fontWeight: FontWeight.w500,
          color: canWrite ? const Color(0xFFB45309) : EditorTheme.muted,
        ),
      ),
    );
  }

  /// Not static: a null timestamp means "no expiry", and that word has to be
  /// localized — it shipped as the English literal `never`, so the Chinese UI
  /// read 「过期 never」 while `tokenNever` sat unused behind the input hint.
  String _shortTime(dynamic value) {
    if (value == null) return context.l10n.tokenNever;
    final s = value.toString();
    return s.length >= 10 ? s.substring(0, 10) : s;
  }

  /// Revocation takes effect on the server the instant it returns: every cron
  /// job, script and CLI holding this token starts getting 401s. It used to
  /// fire straight off the icon button, so one mis-click silently broke someone
  /// else's backup with no way to put the secret back.
  Future<bool> _confirmRevoke(String label) {
    final l10n = context.l10n;
    return showDestructiveConfirm(
      context,
      title: l10n.tokenRevokeConfirmTitle(label),
      body: l10n.tokenRevokeConfirmBody,
      confirmLabel: l10n.tokenRevoke,
      cancelLabel: l10n.commonCancel,
    );
  }

  List<Widget> _tokensSection(BuildContext context) {
    final onLoad = widget.onLoadTokens;
    final onCreate = widget.onCreateToken;
    final onRevoke = widget.onRevokeToken;
    if (onLoad == null || onCreate == null || onRevoke == null) {
      return const [];
    }

    // Lazy-load the list the first time the tab is shown.
    if (!_tokensLoaded) {
      _tokensLoaded = true;
      onLoad()
          .then((list) {
            if (mounted) setState(() => _tokens = list);
          })
          .catchError((Object e) {
            if (mounted) setState(() => _tokensError = e.toString());
          });
    }

    Future<void> refresh() async {
      try {
        final list = await onLoad();
        if (mounted) {
          setState(() {
            _tokens = list;
            _tokensError = null;
          });
        }
      } catch (e) {
        if (mounted) setState(() => _tokensError = e.toString());
      }
    }

    return [
      _sectionTitle(
        context,
        Icons.key_outlined,
        context.l10n.tokenTitle,
        const Color(0xFF2563EB),
      ),
      const SizedBox(height: 4),
      Text(
        context.l10n.tokenDescription,
        style: Theme.of(
          context,
        ).textTheme.bodySmall?.copyWith(color: EditorTheme.muted),
      ),
      const SizedBox(height: 16),
      TextField(
        controller: _tokenName,
        decoration: InputDecoration(
          labelText: context.l10n.tokenName,
          hintText: context.l10n.tokenNameHint,
          border: const OutlineInputBorder(),
          isDense: true,
        ),
      ),
      const SizedBox(height: 10),
      Row(
        children: [
          Switch(
            value: _tokenWrite,
            onChanged: _tokenBusy
                ? null
                : (v) => setState(() => _tokenWrite = v),
          ),
          Text(context.l10n.tokenWriteAccess),
          const Spacer(),
          SizedBox(
            width: 160,
            child: DropdownButtonFormField<int?>(
              initialValue: _tokenExpiryDays,
              isDense: true,
              decoration: InputDecoration(
                labelText: context.l10n.tokenExpires,
                border: const OutlineInputBorder(),
                isDense: true,
              ),
              // `never` stays first and stays the default: it is what shipped,
              // and quietly starting to expire tokens people already treat as
              // permanent would break their cron jobs on a timer.
              items: [
                DropdownMenuItem(
                  value: null,
                  child: Text(context.l10n.tokenNever),
                ),
                for (final d in const [30, 90, 365])
                  DropdownMenuItem(
                    value: d,
                    child: Text(context.l10n.tokenExpiryDays(d)),
                  ),
              ],
              onChanged: _tokenBusy
                  ? null
                  : (v) => setState(() => _tokenExpiryDays = v),
            ),
          ),
        ],
      ),
      const SizedBox(height: 10),
      Align(
        alignment: Alignment.centerLeft,
        child: FilledButton.icon(
          icon: const Icon(Icons.add, size: 18),
          label: Text(context.l10n.tokenCreate),
          onPressed: _tokenBusy
              ? null
              : () async {
                  final name = _tokenName.text.trim();
                  if (name.isEmpty) {
                    setState(
                      () => _tokensError = context.l10n.tokenNameRequired,
                    );
                    return;
                  }
                  setState(() {
                    _tokenBusy = true;
                    _tokensError = null;
                  });
                  try {
                    final scopes = _tokenWrite
                        ? <String>['read', 'write']
                        : <String>['read'];
                    final days = _tokenExpiryDays;
                    final created = await onCreate(name, scopes, days);
                    _tokenName.clear();

                    if (mounted) {
                      setState(() {
                        _tokenWrite = false;
                        _tokenBusy = false;
                      });
                      await _showTokenSecret(created);
                    }
                    await refresh();
                  } catch (e) {
                    if (mounted) {
                      setState(() {
                        _tokensError = e.toString();
                        _tokenBusy = false;
                      });
                    }
                  }
                },
        ),
      ),
      if (_tokensError != null) ...[
        const SizedBox(height: 12),
        ErrorBanner(_tokensError!),
      ],
      const SizedBox(height: 18),
      const Divider(height: 1),
      const SizedBox(height: 8),
      if (_tokens == null)
        const Padding(
          padding: EdgeInsets.all(16),
          child: Center(child: CircularProgressIndicator()),
        )
      else if (_tokens!.isEmpty)
        // A title with no body and no next step was the shipped state. No action
        // button: the next step is the form directly above this, so a button
        // would just point at itself.
        EmptyState(
          icon: Icons.key_outlined,
          title: context.l10n.tokenNone,
          detail: context.l10n.tokenEmptyBody,
        )
      else
        for (final t in _tokens!)
          Opacity(
            // An expired token still lists (you may want to revoke it), but it
            // can't do anything — reading as live is the misleading part.
            opacity: _tokenExpired(t['expires_at']) ? 0.62 : 1,
            child: ListTile(
              contentPadding: EdgeInsets.zero,
              dense: true,
              title: Row(
                children: [
                  Flexible(
                    child: Text(
                      t['name'] as String? ?? context.l10n.tokenUnnamed,
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
                  const SizedBox(width: 8),
                  _scopeBadge(context, t['scopes']),
                ],
              ),
              subtitle: Text(
                context.l10n.tokenMeta(
                  _shortTime(t['last_used_at']),
                  _shortTime(t['expires_at']),
                ),
              ),
              trailing: IconButton(
                icon: const Icon(Icons.delete_outline, size: 20),
                tooltip: context.l10n.tokenRevoke,
                onPressed: _tokenBusy
                    ? null
                    : () async {
                        final label =
                            t['name'] as String? ?? context.l10n.tokenUnnamed;
                        if (!await _confirmRevoke(label)) return;
                        setState(() => _tokenBusy = true);
                        try {
                          await onRevoke(t['id'] as String);
                          await refresh();
                        } catch (e) {
                          if (mounted)
                            setState(() => _tokensError = e.toString());
                        } finally {
                          if (mounted) setState(() => _tokenBusy = false);
                        }
                      },
              ),
            ),
          ),
    ];
  }

  /// Commit the AI fields when one loses focus — and only if something actually
  /// changed, so tabbing through untouched fields is silent.
  void _saveAiOnBlur() {
    if (_loading || _aiFocus.any((f) => f.hasFocus)) return;
    if (_aiNow == _aiSaved) return;
    unawaited(_saveAi());
  }

  /// Persist the AI provider settings. No longer behind a Save button: every
  /// other setting here applies as you touch it (toggles, sliders), and this was
  /// the only holdout — the button existed for these three text fields and then
  /// sat under every page, saving AI settings no matter what you were looking at.
  Future<void> _saveAi() async {
    // Unreachable in 本地模式 (the blur listeners belong to fields that only the
    // AI tab builds), but this is what makes that a fact rather than a hope.
    final save = widget.onSaveAiSettings;
    if (save == null) return;
    final attempt = _aiNow;
    setState(() {
      _saving = true;
      _error = null;
    });
    try {
      await save(
        provider: _preset.provider,
        baseUrl: _baseUrl.text.trim(),
        model: _model.text.trim(),
        apiKey: _apiKey.text.trim().isEmpty ? null : _apiKey.text.trim(),
      );
      if (!mounted) return;
      // Stay open. Saving is not leaving — the dialog closes when the user says
      // so, which is the whole point of settings that apply as you go.
      setState(() {
        _saving = false;
        _aiSaved = attempt;
      });
    } catch (error) {
      if (!mounted) return;
      setState(() {
        _saving = false;
        _error = error.toString();
      });
    }
  }

  Widget _sectionTitle(
    BuildContext context,
    IconData icon,
    String label,
    Color color,
  ) {
    return Row(
      children: [
        Icon(icon, size: 18, color: color),
        const SizedBox(width: 8),
        Text(label, style: Theme.of(context).textTheme.titleMedium),
      ],
    );
  }

  List<Widget> _appearanceSection(BuildContext context) => [
    _sectionTitle(
      context,
      Icons.tune,
      context.l10n.settingsAppearance,
      const Color(0xFF2563EB),
    ),
    const SizedBox(height: 12),
    Row(
      children: [
        SizedBox(width: 90, child: Text(context.l10n.languageLabel)),
        Expanded(
          child: Wrap(
            spacing: 8,
            children: [
              _langChip(context.l10n.languageSystem, kLangSystem),
              _langChip(context.l10n.languageChinese, kLangChinese),
              _langChip(context.l10n.languageEnglish, kLangEnglish),
            ],
          ),
        ),
      ],
    ),
    const SizedBox(height: 8),
    _pageWidthRow(context),
    _sliderRow(
      label: context.l10n.settingsFontSize,
      value: _fontScale,
      min: 0.85,
      max: 1.4,
      display: '${(_fontScale * 100).round()}%',
      onChanged: (value) {
        setState(() => _fontScale = value);
        _applyAppearance();
      },
    ),
    const SizedBox(height: 4),
    Row(
      children: [
        SizedBox(width: 90, child: Text(context.l10n.settingsFont)),
        Expanded(
          child: Wrap(
            spacing: 8,
            children: [
              _fontChip(context.l10n.settingsFontSystem, null),
              _fontChip('Serif', 'serif'),
              _fontChip('Mono', kMonoFont),
            ],
          ),
        ),
      ],
    ),
    const SizedBox(height: 8),
    SwitchListTile(
      contentPadding: EdgeInsets.zero,
      dense: true,
      value: _reHostImages,
      title: Text(context.l10n.settingsReHostImages),
      subtitle: Text(context.l10n.settingsReHostImagesSub),
      onChanged: (value) {
        setState(() => _reHostImages = value);
        widget.onReHostImagesChanged(value);
      },
    ),
    SwitchListTile(
      contentPadding: EdgeInsets.zero,
      dense: true,
      value: _showFormatBar,
      title: Text(context.l10n.settingsFormatBar),
      subtitle: Text(context.l10n.settingsFormatBarSub),
      onChanged: (value) {
        setState(() => _showFormatBar = value);
        widget.onShowFormatBarChanged(value);
      },
    ),
    SwitchListTile(
      contentPadding: EdgeInsets.zero,
      dense: true,
      value: _showPageTitle,
      title: Text(context.l10n.settingsPageTitle),
      subtitle: Text(context.l10n.settingsPageTitleSub),
      onChanged: (value) {
        setState(() => _showPageTitle = value);
        widget.onShowPageTitleChanged(value);
      },
    ),
    // Desktop only — a browser tab's close button belongs to the browser, and
    // no app code can intercept it.
    if (!kIsWeb) ...[
      const SizedBox(height: 8),
      Text(
        context.l10n.closeWindowHeader,
        style: const TextStyle(
          fontWeight: FontWeight.w600,
          color: EditorTheme.muted,
        ),
      ),
      // RadioGroup, not per-tile groupValue/onChanged — those were deprecated
      // after Flutter 3.32 in favour of this ancestor.
      RadioGroup<String>(
        groupValue: _closeBehavior,
        onChanged: (value) {
          if (value == null) return;
          setState(() => _closeBehavior = value);
          saveCloseBehavior(value);
        },
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            for (final option in _closeBehaviorOptions)
              RadioListTile<String>(
                contentPadding: EdgeInsets.zero,
                dense: true,
                value: option.$1,
                title: Text(option.$2),
                subtitle: Text(option.$3),
              ),
          ],
        ),
      ),
    ],
  ];

  /// The X-button choices. "Ask every time" is not offered as a standing
  /// setting — it is only the pre-answer default; once you have answered, an
  /// explicit choice is what you want, and the question is reachable again by
  /// picking a different option here.
  ///
  /// Tray is Windows-only for now (see `trayIsSupported`): where it is not
  /// available, offering it would promise a restore path we cannot deliver.
  List<(String, String, String)> get _closeBehaviorOptions => [
    (kCloseQuit, context.l10n.closeQuitTitle, context.l10n.closeQuitSub),
    if (trayIsSupported)
      (kCloseTray, context.l10n.closeTrayTitle, context.l10n.closeTraySub),
  ];

  List<Widget> _aiSection(BuildContext context) => [
    _sectionTitle(
      context,
      Icons.auto_awesome,
      context.l10n.settingsAiProvider,
      const Color(0xFF7C3AED),
    ),
    SwitchListTile(
      contentPadding: EdgeInsets.zero,
      dense: true,
      value: _aiEnabled,
      title: Text(context.l10n.aiEnable),
      subtitle: Text(context.l10n.aiEnableSub),
      onChanged: (value) {
        setState(() => _aiEnabled = value);
        widget.onAiEnabledChanged(value);
      },
    ),
    const SizedBox(height: 12),
    DropdownButtonFormField<_AiPreset>(
      initialValue: _preset,
      decoration: InputDecoration(
        labelText: context.l10n.aiProviderLabel,
        border: const OutlineInputBorder(),
      ),
      items: _AiPreset.values
          .map(
            (preset) =>
                DropdownMenuItem(value: preset, child: Text(preset.label)),
          )
          .toList(),
      onChanged: _saving
          ? null
          : (preset) {
              if (preset != null) _applyPreset(preset);
            },
    ),
    const SizedBox(height: 12),
    TextField(
      controller: _baseUrl,
      focusNode: _aiFocus[0],
      enabled: !_saving,
      decoration: InputDecoration(
        labelText: context.l10n.aiBaseUrl,
        hintText: 'https://api.deepseek.com',
        border: const OutlineInputBorder(),
      ),
    ),
    const SizedBox(height: 12),
    TextField(
      controller: _model,
      focusNode: _aiFocus[1],
      enabled: !_saving,
      decoration: InputDecoration(
        labelText: context.l10n.aiModel,
        hintText: 'deepseek-chat',
        border: const OutlineInputBorder(),
      ),
    ),
    const SizedBox(height: 12),
    TextField(
      controller: _apiKey,
      focusNode: _aiFocus[2],
      enabled: !_saving,
      obscureText: true,
      decoration: InputDecoration(
        labelText: context.l10n.aiApiKey,
        hintText: _hasKey
            ? context.l10n.aiApiKeyHintHasKey
            : context.l10n.aiApiKeyHintRequired,
        border: const OutlineInputBorder(),
      ),
    ),
    const SizedBox(height: 6),
    Text(
      context.l10n.aiKeyHelp,
      style: Theme.of(
        context,
      ).textTheme.bodySmall?.copyWith(color: EditorTheme.muted),
    ),
    if (_error != null) ...[const SizedBox(height: 12), ErrorBanner(_error!)],
  ];

  List<Widget> _accountSection(BuildContext context) => [
    _sectionTitle(
      context,
      Icons.person_outline,
      context.l10n.settingsAccount,
      const Color(0xFF2563EB),
    ),
    const SizedBox(height: 4),
    Text(
      widget.userEmail,
      style: Theme.of(
        context,
      ).textTheme.bodySmall?.copyWith(color: EditorTheme.muted),
    ),
    const SizedBox(height: 12),
    TextField(
      controller: _name,
      enabled: !_accountBusy,
      decoration: InputDecoration(
        labelText: context.l10n.accountDisplayName,
        border: const OutlineInputBorder(),
      ),
    ),
    const SizedBox(height: 8),
    Align(
      alignment: Alignment.centerLeft,
      child: OutlinedButton.icon(
        onPressed: _accountBusy ? null : _saveProfile,
        icon: const Icon(Icons.save, size: 16),
        label: Text(context.l10n.accountSaveName),
      ),
    ),
    const SizedBox(height: 16),
    TextField(
      controller: _curPass,
      enabled: !_accountBusy,
      obscureText: true,
      decoration: InputDecoration(
        labelText: context.l10n.accountCurrentPassword,
        border: const OutlineInputBorder(),
      ),
    ),
    const SizedBox(height: 8),
    TextField(
      controller: _newPass,
      enabled: !_accountBusy,
      obscureText: true,
      decoration: InputDecoration(
        labelText: context.l10n.accountNewPassword,
        border: const OutlineInputBorder(),
      ),
    ),
    const SizedBox(height: 8),
    Align(
      alignment: Alignment.centerLeft,
      child: OutlinedButton.icon(
        onPressed: _accountBusy ? null : _changeAccountPassword,
        icon: const Icon(Icons.lock_outline, size: 16),
        label: Text(context.l10n.accountChangePassword),
      ),
    ),
    if (_accountMsg != null) ...[
      const SizedBox(height: 10),
      Text(
        _accountMsg!,
        style: const TextStyle(color: EditorTheme.muted, fontSize: 13),
      ),
    ],
    if (widget.onDeleteAccount != null) ...[
      const Divider(height: 32),
      Align(
        alignment: Alignment.centerLeft,
        child: OutlinedButton.icon(
          onPressed: _accountBusy ? null : _deleteAccount,
          style: OutlinedButton.styleFrom(
            foregroundColor: const Color(0xFFDC2626),
            side: const BorderSide(color: Color(0xFFFCA5A5)),
          ),
          icon: const Icon(Icons.delete_forever_outlined, size: 16),
          label: Text(context.l10n.accountDelete),
        ),
      ),
    ],
  ];

  List<Widget> _dataSection(BuildContext context) => [
    _sectionTitle(
      context,
      Icons.import_export,
      context.l10n.settingsData,
      const Color(0xFF0EA5E9),
    ),
    const SizedBox(height: 12),
    Text(
      context.l10n.dataImportDescription,
      style: const TextStyle(color: EditorTheme.muted),
    ),
    const SizedBox(height: 12),
    Align(
      alignment: Alignment.centerLeft,
      child: Wrap(
        spacing: 12,
        runSpacing: 12,
        children: [
          OutlinedButton.icon(
            onPressed: () => widget.onImportWorkspace(),
            icon: const Icon(Icons.upload_file_outlined, size: 18),
            label: Text(context.l10n.dataImportButton),
          ),
          if (widget.onExportAllWorkspaces != null)
            OutlinedButton.icon(
              onPressed: () => widget.onExportAllWorkspaces!(),
              icon: const Icon(Icons.download_outlined, size: 18),
              label: Text(context.l10n.dataExportAllButton),
            ),
        ],
      ),
    ),
    const SizedBox(height: 16),
    Text(
      context.l10n.dataExportTip,
      style: Theme.of(
        context,
      ).textTheme.bodySmall?.copyWith(color: EditorTheme.faint),
    ),
  ];

  Widget _kbd(String text) => Container(
    padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
    decoration: BoxDecoration(
      color: const Color(0xFFF1F5F9),
      borderRadius: BorderRadius.circular(5),
      border: Border.all(color: const Color(0xFFCBD5E1)),
    ),
    child: Text(text, style: const TextStyle(fontSize: 12)),
  );

  /// Opt-in capture, for handing a reproducible bug over instead of describing
  /// it. Off by default and stated plainly, because it records real content.
  List<Widget> _diagnosticsSection(BuildContext context) {
    final l = context.l10n;
    return [
      SwitchListTile(
        contentPadding: EdgeInsets.zero,
        dense: true,
        value: _diagnostics,
        title: Text(l.settingsDiagnosticsToggle),
        subtitle: Text(l.settingsDiagnosticsToggleSub),
        onChanged: (value) {
          setDiagnostics(value);
          setState(() => _diagnostics = value);
        },
      ),
      const SizedBox(height: 12),
      Row(
        children: [
          OutlinedButton.icon(
            onPressed: openDiagnosticsFolder,
            icon: const Icon(Icons.folder_open_outlined, size: 18),
            label: Text(l.settingsDiagnosticsFolder),
          ),
          const SizedBox(width: 12),
          Expanded(
            child: SelectableText(
              diagnosticsDir,
              style: Theme.of(context).textTheme.bodySmall,
            ),
          ),
        ],
      ),
      const SizedBox(height: 12),
      Text(
        l.settingsDiagnosticsPrivacy,
        style: Theme.of(context).textTheme.bodySmall,
      ),
    ];
  }

  List<Widget> _shortcutsSection(BuildContext context) {
    Widget head(String t) => Padding(
      padding: const EdgeInsets.only(top: 8, bottom: 4),
      child: Text(
        t,
        style: const TextStyle(
          fontWeight: FontWeight.w600,
          color: EditorTheme.muted,
        ),
      ),
    );
    Widget row(String keys, String desc) => Padding(
      padding: const EdgeInsets.symmetric(vertical: 5),
      child: Row(
        children: [
          _kbd(keys),
          const SizedBox(width: 16),
          Expanded(child: Text(desc)),
        ],
      ),
    );
    return [
      head(context.l10n.shortcutsGroupApp),
      row('Ctrl + N', context.l10n.shortcutsNewPage),
      row('Ctrl + F', context.l10n.shortcutsFindInPage),
      row('F3 / Shift + F3', context.l10n.shortcutsFindNextPrev),
      row('Ctrl + Shift + F', context.l10n.shortcutsSearchWorkspace),
      row('Ctrl + ,', context.l10n.shortcutsOpenSettings),
      row('F2', context.l10n.shortcutsRename),
      const SizedBox(height: 8),
      head(context.l10n.shortcutsGroupFormat),
      row('Ctrl + B', context.l10n.shortcutsBold),
      row('Ctrl + I', context.l10n.shortcutsItalic),
      row('Ctrl + E', context.l10n.shortcutsInlineCode),
      row('Ctrl + K', context.l10n.shortcutsLink),
      row('Ctrl + Alt + 1…6', context.l10n.shortcutsHeadings),
      row('Ctrl + Alt + 0', context.l10n.shortcutsParagraph),
      row('Tab / Shift + Tab', context.l10n.shortcutsIndent),
      const SizedBox(height: 8),
      head(context.l10n.shortcutsGroupEdit),
      row('Ctrl + Z', context.l10n.shortcutsUndo),
      row('Ctrl + Shift + Z', context.l10n.shortcutsRedo),
      row('Ctrl + A', context.l10n.shortcutsSelectAll),
      row('Ctrl + C / X / V', context.l10n.shortcutsCopyCutPaste),
      row('Ctrl + Shift + V', context.l10n.shortcutsPastePlain),
      row('/', context.l10n.shortcutsSlashMenu),
      const SizedBox(height: 12),
      Text(
        context.l10n.shortcutsNote,
        style: Theme.of(
          context,
        ).textTheme.bodySmall?.copyWith(color: EditorTheme.faint),
      ),
    ];
  }

  @override
  Widget build(BuildContext context) {
    // These are the settings of the world you are in — WHICH world that is gets
    // picked on the account tile, not here. A null callback means the active
    // world does not have that thing, so the tab is absent rather than present
    // and inert. In 本地模式 that leaves three tabs, every one of them a
    // this-device preference, and not one ternary among them.
    //
    // No `onSave` per tab any more: there is no Save button to feed. Each
    // section commits its own changes as they are made.
    // Grouped (design 16). Seven flat rows read as one undifferentiated list;
    // the groups are what make it scannable. Order is by group so the labels can
    // be emitted inline, and a group whose every row is absent prints no label —
    // in 本地模式 the whole 账户 group disappears, which is the same
    // null-means-absent rule the tabs themselves already follow.
    final tabs =
        <({String group, String title, IconData icon, List<Widget> section})>[
          (
            group: context.l10n.settingsGroupGeneral,
            title: context.l10n.settingsAppearance,
            icon: Icons.tune,
            section: _appearanceSection(context),
          ),
          (
            group: context.l10n.settingsGroupGeneral,
            title: context.l10n.settingsShortcuts,
            icon: Icons.keyboard_outlined,
            section: _shortcutsSection(context),
          ),
          // 本地模式 has no account — same reason API Tokens below is absent there.
          if (widget.onUpdateProfile != null)
            (
              group: context.l10n.settingsGroupAccount,
              title: context.l10n.settingsAccount,
              icon: Icons.person_outline,
              section: _accountSection(context),
            ),
          if (widget.onLoadTokens != null)
            (
              group: context.l10n.settingsGroupAccount,
              title: context.l10n.tokenTitle,
              icon: Icons.key_outlined,
              section: _tokensSection(context),
            ),
          (
            group: context.l10n.settingsGroupWorkspace,
            title: context.l10n.settingsData,
            icon: Icons.import_export,
            section: _dataSection(context),
          ),
          // AI settings live on the server — 本地模式 has none to configure.
          if (widget.onLoadAiSettings != null)
            (
              group: context.l10n.settingsGroupOther,
              title: context.l10n.settingsAiProvider,
              icon: Icons.auto_awesome,
              section: _aiSection(context),
            ),
          // Web has no filesystem to capture into, so the tab is simply absent
          // there rather than offering a switch that cannot do anything.
          if (diagnosticsSupported)
            (
              group: context.l10n.settingsGroupOther,
              title: context.l10n.settingsDiagnostics,
              icon: Icons.bug_report_outlined,
              section: _diagnosticsSection(context),
            ),
        ];
    final titles = [for (final t in tabs) t.title];
    final icons = [for (final t in tabs) t.icon];
    final sections = [for (final t in tabs) t.section];
    return AlertDialog(
      title: Row(
        children: [
          const Icon(Icons.settings_outlined, size: 22),
          const SizedBox(width: 8),
          Text(context.l10n.settingsTitle),
        ],
      ),
      contentPadding: const EdgeInsets.fromLTRB(0, 12, 0, 0),
      content: SizedBox(
        width: 720,
        height: 460,
        child: _loading
            ? const Center(child: CircularProgressIndicator())
            : Row(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  SizedBox(
                    width: 180,
                    child: ListView(
                      padding: const EdgeInsets.symmetric(vertical: 4),
                      children: [
                        for (var i = 0; i < titles.length; i++) ...[
                          if (i == 0 || tabs[i].group != tabs[i - 1].group)
                            Padding(
                              padding: EdgeInsets.only(
                                left: 16,
                                right: 8,
                                top: i == 0 ? 4 : 14,
                                bottom: 4,
                              ),
                              child: Text(
                                tabs[i].group.toUpperCase(),
                                style: const TextStyle(
                                  fontSize: 11,
                                  fontWeight: FontWeight.w600,
                                  letterSpacing: 0.6,
                                  color: EditorTheme.faint,
                                ),
                              ),
                            ),
                          _navRow(context, i, titles[i], icons[i]),
                        ],
                        const Divider(height: 1),
                        // About isn't a content tab — it pops the version dialog.
                        ListTile(
                          dense: true,
                          leading: const Icon(Icons.info_outline, size: 20),
                          title: Text(context.l10n.aboutTitle),
                          subtitle: const Text('v$kAppVersion'),
                          onTap: () => _showAboutDialog(context),
                        ),
                      ],
                    ),
                  ),
                  const VerticalDivider(width: 1),
                  Expanded(
                    child: SingleChildScrollView(
                      padding: const EdgeInsets.fromLTRB(20, 16, 20, 16),
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        mainAxisSize: MainAxisSize.min,
                        children: sections[_tab],
                      ),
                    ),
                  ),
                ],
              ),
      ),
      // No Save button. Everything here applies as you touch it — toggles and
      // sliders always did, the connection switches on pick, and the AI fields
      // commit when they lose focus. AppFlowy, AFFiNE and Notion all settle in
      // the same place, and the button we had was worse than redundant: it only
      // ever saved the AI section, from under every page.
      //
      // The spinner rides here so a commit in flight is visible without a
      // button to host it.
      actions: [
        if (_saving)
          const Padding(
            padding: EdgeInsets.only(right: 12),
            child: SizedBox(
              width: 16,
              height: 16,
              child: CircularProgressIndicator(strokeWidth: 2),
            ),
          ),
        TextButton(
          onPressed: _saving ? null : () => Navigator.of(context).pop(),
          child: Text(context.l10n.commonClose),
        ),
      ],
    );
  }
}

/// Global AI dialog: type an instruction and pick where the generated content
/// goes — a new page, the current page, or a brand-new workspace.
class _AiDialog extends StatefulWidget {
  const _AiDialog({
    required this.canEdit,
    required this.hasWorkspace,
    required this.onStream,
    required this.onNewPage,
    required this.onCurrentPage,
    required this.onNewWorkspace,
  });

  final bool canEdit;
  final bool hasWorkspace;
  final Stream<String> Function(String prompt, {String? system}) onStream;
  final Future<void> Function(String markdown) onNewPage;
  final Future<void> Function(String markdown)? onCurrentPage;
  final Future<void> Function(String markdown) onNewWorkspace;

  @override
  State<_AiDialog> createState() => _AiDialogState();
}

class _AiDialogState extends State<_AiDialog> {
  final _prompt = TextEditingController();
  final _scroll = ScrollController();
  late _AiTarget _target = widget.hasWorkspace
      ? _AiTarget.newPage
      : _AiTarget.newWorkspace;
  StreamSubscription<String>? _sub;
  bool _streaming = false;
  bool _applying = false;
  bool _done = false;
  final StringBuffer _buffer = StringBuffer();
  String? _error;

  bool get _busy => _streaming || _applying;

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
    // New page / new workspace want a document with a title line.
    final system = _target == _AiTarget.currentPage ? null : kAiDocSystem;
    setState(() {
      _streaming = true;
      _done = false;
      _error = null;
      _buffer.clear();
    });
    _sub = widget
        .onStream(prompt, system: system)
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
          await widget.onCurrentPage?.call(markdown);
        case _AiTarget.newWorkspace:
          await widget.onNewWorkspace(markdown);
      }
      if (mounted) Navigator.of(context).pop();
    } catch (error) {
      if (mounted) setState(() => _error = error.toString());
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
          const Icon(Icons.auto_awesome, size: 22, color: Color(0xFF7C3AED)),
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
                        ? (_) => setState(() => _target = _AiTarget.newPage)
                        : null,
                  ),
                if (widget.canEdit && canWriteCurrent)
                  ChoiceChip(
                    label: Text(context.l10n.aiTargetCurrentPage),
                    selected: _target == _AiTarget.currentPage,
                    onSelected: _busy
                        ? null
                        : (_) =>
                              setState(() => _target = _AiTarget.currentPage),
                  ),
                ChoiceChip(
                  label: Text(context.l10n.aiTargetNewWorkspace),
                  selected: _target == _AiTarget.newWorkspace,
                  onSelected: _busy
                      ? null
                      : (_) => setState(() => _target = _AiTarget.newWorkspace),
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
                  color: const Color(0xFFF8FAFC),
                  borderRadius: BorderRadius.circular(8),
                  border: Border.all(color: const Color(0xFFE2E8F0)),
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
              Row(
                children: [
                  if (_streaming) ...[
                    const SizedBox(
                      width: 14,
                      height: 14,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    ),
                    const SizedBox(width: 8),
                    Text(context.l10n.aiGenerating),
                  ] else if (_done)
                    Text(
                      context.l10n.aiDoneReview,
                      style: const TextStyle(color: EditorTheme.muted),
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
            label: Text(context.l10n.aiInsert),
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
class _ShareDialog extends StatefulWidget {
  const _ShareDialog({
    required this.onLoad,
    required this.onEnable,
    required this.onDisable,
    required this.buildUrl,
  });

  final Future<({bool shared, String? token})> Function() onLoad;
  final Future<String> Function() onEnable; // returns the token
  final Future<void> Function() onDisable;
  final String Function(String token) buildUrl;

  @override
  State<_ShareDialog> createState() => _ShareDialogState();
}

class _ShareDialogState extends State<_ShareDialog> {
  bool _loading = true;
  bool _busy = false;
  String? _error;
  bool _shared = false;
  String? _url;

  @override
  void initState() {
    super.initState();
    _refresh();
  }

  Future<void> _refresh() async {
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final status = await widget.onLoad();
      if (!mounted) return;
      setState(() {
        _shared = status.shared;
        _url = status.token == null ? null : widget.buildUrl(status.token!);
        _loading = false;
      });
    } catch (error) {
      if (!mounted) return;
      setState(() {
        _error = error.toString();
        _loading = false;
      });
    }
  }

  Future<void> _toggle(bool on) async {
    final l10n = context.l10n;
    setState(() => _busy = true);
    try {
      if (on) {
        final token = await widget.onEnable();
        if (!mounted) return;
        setState(() {
          _shared = true;
          _url = widget.buildUrl(token);
        });
      } else {
        await widget.onDisable();
        if (!mounted) return;
        setState(() {
          _shared = false;
          _url = null;
        });
      }
    } catch (error) {
      _snack(l10n.shareActionFailed(error.toString()));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  void _copy() {
    final url = _url;
    if (url == null) return;
    Clipboard.setData(ClipboardData(text: url));
    _snack(context.l10n.shareLinkCopied);
  }

  void _snack(String message) {
    if (!mounted) return;
    ScaffoldMessenger.of(
      context,
    ).showSnackBar(SnackBar(content: Text(message)));
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: Text(context.l10n.shareTitle),
      content: SizedBox(
        width: 440,
        child: _loading
            ? const SizedBox(
                height: 80,
                child: Center(child: CircularProgressIndicator()),
              )
            : _error != null
            ? SizedBox(
                height: 80,
                child: Center(
                  child: Text(context.l10n.shareLoadFailed(_error!)),
                ),
              )
            : Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  SwitchListTile(
                    contentPadding: EdgeInsets.zero,
                    value: _shared,
                    onChanged: _busy ? null : _toggle,
                    title: Text(context.l10n.sharePublicAccess),
                    subtitle: Text(context.l10n.sharePublicAccessSub),
                  ),
                  if (_shared && _url != null) ...[
                    const SizedBox(height: 12),
                    Container(
                      padding: const EdgeInsets.symmetric(
                        horizontal: 12,
                        vertical: 8,
                      ),
                      decoration: BoxDecoration(
                        color: const Color(0xFFF6F8FA),
                        borderRadius: BorderRadius.circular(6),
                      ),
                      child: Row(
                        children: [
                          Expanded(
                            child: SelectableText(
                              _url!,
                              maxLines: 1,
                              style: const TextStyle(fontSize: 13),
                            ),
                          ),
                          IconButton(
                            tooltip: context.l10n.shareCopyLink,
                            icon: const Icon(Icons.copy, size: 18),
                            onPressed: _copy,
                          ),
                        ],
                      ),
                    ),
                  ],
                ],
              ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(context.l10n.commonClose),
        ),
      ],
    );
  }
}

/// Move / copy a page-or-folder subtree into another cloud workspace. Picking a
/// destination runs a server dry-run (report only, no mutation) so the user
/// sees the counts and any breaking links BEFORE committing. Confirm runs the
/// real transfer and pops with the report + destination name for the caller to
/// refresh + snackbar.
///
/// v1 has no destination-folder picker: everything lands at the destination
/// ROOT (`parent_view_id: null`). See [_WorkspaceShellState._openTransfer].
class _TransferDialog extends StatefulWidget {
  const _TransferDialog({
    required this.copy,
    required this.destinations,
    required this.onTransfer,
  });

  /// true = copy (source kept); false = move (source soft-deleted after copy).
  final bool copy;

  /// Cloud workspaces the subtree can go to — the source is already excluded.
  final List<({String id, String name})> destinations;

  /// Runs one transfer against the picked destination. [dryRun] true = preview
  /// (no mutation); false = commit. Source workspace + view + move/copy are
  /// bound by the caller.
  final Future<TransferReport> Function({
    required String destWorkspaceId,
    required bool dryRun,
  })
  onTransfer;

  @override
  State<_TransferDialog> createState() => _TransferDialogState();
}

class _TransferDialogState extends State<_TransferDialog> {
  String? _destId;
  TransferReport? _preview; // dry-run result for the current destination
  bool _loadingPreview = false;
  bool _submitting = false;
  String? _error;

  ({String id, String name})? get _dest {
    final id = _destId;
    if (id == null) return null;
    return widget.destinations.where((d) => d.id == id).firstOrNull;
  }

  Future<void> _selectDest(String? id) async {
    if (id == null || id == _destId) return;
    setState(() {
      _destId = id;
      _preview = null;
      _error = null;
      _loadingPreview = true;
    });
    try {
      final report = await widget.onTransfer(destWorkspaceId: id, dryRun: true);
      if (!mounted || _destId != id) return; // superseded by a newer pick
      setState(() {
        _preview = report;
        _loadingPreview = false;
      });
    } catch (error) {
      if (!mounted || _destId != id) return;
      setState(() {
        _error = context.l10n.transferFailed(error.toString());
        _loadingPreview = false;
      });
    }
  }

  Future<void> _confirm() async {
    final dest = _dest;
    if (dest == null || _submitting) return;
    setState(() {
      _submitting = true;
      _error = null;
    });
    try {
      final report = await widget.onTransfer(
        destWorkspaceId: dest.id,
        dryRun: false,
      );
      if (!mounted) return;
      Navigator.of(context).pop((report: report, destName: dest.name));
    } catch (error) {
      if (!mounted) return;
      setState(() {
        _error = context.l10n.transferFailed(error.toString());
        _submitting = false;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final report = _preview;
    return AlertDialog(
      title: Text(
        widget.copy ? l10n.transferCopyTitle : l10n.transferMoveTitle,
      ),
      content: SizedBox(
        width: 440,
        child: widget.destinations.isEmpty
            ? Text(l10n.transferNoDestinations)
            : Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  DropdownButtonFormField<String>(
                    initialValue: _destId,
                    isExpanded: true,
                    decoration: InputDecoration(
                      labelText: l10n.transferPickWorkspace,
                      border: const OutlineInputBorder(),
                    ),
                    items: widget.destinations
                        .map(
                          (d) => DropdownMenuItem(
                            value: d.id,
                            child: Text(
                              d.name,
                              overflow: TextOverflow.ellipsis,
                            ),
                          ),
                        )
                        .toList(),
                    onChanged: _submitting ? null : _selectDest,
                  ),
                  const SizedBox(height: 16),
                  if (_loadingPreview)
                    const Padding(
                      padding: EdgeInsets.symmetric(vertical: 8),
                      child: Center(
                        child: SizedBox(
                          height: 20,
                          width: 20,
                          child: CircularProgressIndicator(strokeWidth: 2),
                        ),
                      ),
                    )
                  else if (report != null) ...[
                    Text(
                      // One preview string was shared by both modes, so a move
                      // confirmed itself with 「将复制 …」 — the exact opposite
                      // of what the button was about to do to the source.
                      widget.copy
                          ? l10n.transferPreview(
                              report.documents,
                              report.folders,
                              report.images,
                            )
                          : l10n.transferMovePreview(
                              report.documents,
                              report.folders,
                              report.images,
                            ),
                      style: const TextStyle(fontSize: 14),
                    ),
                    if (report.danglingLinks.isNotEmpty) ...[
                      const SizedBox(height: 10),
                      _TransferNotice(
                        icon: Icons.link_off,
                        color: const Color(0xFFB54708),
                        text: l10n.transferDanglingWarning(
                          report.danglingLinks.length,
                        ),
                      ),
                    ],
                  ],
                  // A move can't carry version history (checkpoints stay on the
                  // source; the copy starts fresh) — always warn before a move.
                  if (!widget.copy) ...[
                    const SizedBox(height: 10),
                    _TransferNotice(
                      icon: Icons.info_outline,
                      color: EditorTheme.muted,
                      text: l10n.transferVersionNotice,
                    ),
                  ],
                  if (_error != null) ...[
                    const SizedBox(height: 12),
                    Text(
                      _error!,
                      style: const TextStyle(
                        color: Color(0xFFB42318),
                        fontSize: 13,
                      ),
                    ),
                  ],
                ],
              ),
      ),
      actions: [
        TextButton(
          onPressed: _submitting ? null : () => Navigator.of(context).pop(),
          child: Text(l10n.commonCancel),
        ),
        FilledButton(
          // Enabled only once a destination is picked and no request is in
          // flight — the confirm re-runs the transfer for real (dryRun: false).
          onPressed: (_dest == null || _submitting || _loadingPreview)
              ? null
              : _confirm,
          child: _submitting
              ? const SizedBox(
                  height: 16,
                  width: 16,
                  child: CircularProgressIndicator(strokeWidth: 2),
                )
              : Text(widget.copy ? l10n.transferCopy : l10n.transferMove),
        ),
      ],
    );
  }
}

/// A small icon + text row for the transfer dialog's inline notices (broken
/// links, version-history caveat). Kept tiny and local — not worth a shared
/// widget.
class _TransferNotice extends StatelessWidget {
  const _TransferNotice({
    required this.icon,
    required this.color,
    required this.text,
  });

  final IconData icon;
  final Color color;
  final String text;

  @override
  Widget build(BuildContext context) {
    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Icon(icon, size: 16, color: color),
        const SizedBox(width: 8),
        Expanded(
          child: Text(text, style: TextStyle(fontSize: 13, color: color)),
        ),
      ],
    );
  }
}

/// Version history for the open (cloud) document: list named checkpoints, pin
/// the current state as a new one, or roll back to an old one. Restore reflects
/// live in the editor via the normal sync path (the server broadcasts it as an
/// update), so there is no manual reload here.
/// Version history as a two-pane modal (AFFiNE/Notion shape): a read-only
/// preview of the selected version on the left, the timeline on the right, a
/// restore action at the bottom. The preview reuses the SAME editor in
/// `canEdit: false` — no separate renderer, no HTML (see version-history-plan).
class _VersionHistoryDialog extends StatefulWidget {
  const _VersionHistoryDialog({
    required this.onList,
    required this.onCreate,
    required this.onRestore,
    required this.onLoadContent,
    this.onLoadImageBytes,
    this.onResolveImageUrls,
    this.authorNames = const {},
  });

  /// User id → display name, for [DocVersion.createdBy].
  ///
  /// Empty is the normal case and means "don't show an author column": local
  /// history has no user ids at all, and in a one-person workspace a column
  /// repeating your own name down every row is noise, not information.
  final Map<String, String> authorNames;

  final Future<List<DocVersion>> Function() onList;
  final Future<void> Function(String name) onCreate;
  final Future<void> Function(String versionId) onRestore;

  /// A version's blocks (tree order) + root id, for the read-only preview.
  final Future<({String rootBlockId, List<Map<String, dynamic>> blocks})>
  Function(String versionId)
  onLoadContent;

  /// Image handlers passed straight to the preview editor so pictures render.
  final Future<Uint8List?> Function(String fileId)? onLoadImageBytes;
  final Future<Map<String, String>> Function(List<String> fileIds)?
  onResolveImageUrls;

  @override
  State<_VersionHistoryDialog> createState() => _VersionHistoryDialogState();
}

class _VersionHistoryDialogState extends State<_VersionHistoryDialog> {
  bool _loading = true;
  bool _busy = false; // a create/restore is in flight
  String? _error;
  List<DocVersion> _versions = const [];

  // Preview pane state, keyed to the selected version.
  String? _selectedId;
  bool _previewLoading = false;
  String? _previewError;
  ({String rootBlockId, List<Map<String, dynamic>> blocks})? _content;
  // The previous (older) version's content, for the diff. Null = no predecessor
  // (oldest version) → the preview shows no diff tint.
  ({String rootBlockId, List<Map<String, dynamic>> blocks})? _prevContent;

  @override
  void initState() {
    super.initState();
    _refresh();
  }

  Future<void> _refresh() async {
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final vs = await widget.onList();
      if (!mounted) return;
      setState(() {
        _versions = vs;
        _loading = false;
      });
      // Auto-open the newest version so the pane is never blank.
      if (_selectedId == null && vs.isNotEmpty) {
        _select(vs.first);
      }
    } catch (error) {
      if (!mounted) return;
      setState(() {
        _error = error.toString();
        _loading = false;
      });
    }
  }

  /// Load a version's content (and its predecessor's, for the diff) into the
  /// read-only preview pane. The predecessor is the next-OLDER entry in the
  /// timeline (versions are newest-first).
  Future<void> _select(DocVersion v) async {
    setState(() {
      _selectedId = v.id;
      _previewLoading = true;
      _previewError = null;
      _content = null;
      _prevContent = null;
    });
    final idx = _versions.indexWhere((x) => x.id == v.id);
    final prev = (idx >= 0 && idx + 1 < _versions.length)
        ? _versions[idx + 1]
        : null;
    try {
      final content = await widget.onLoadContent(v.id);
      // Predecessor is best-effort: a failure just drops the diff, not the
      // preview.
      ({String rootBlockId, List<Map<String, dynamic>> blocks})? prevContent;
      if (prev != null) {
        try {
          prevContent = await widget.onLoadContent(prev.id);
        } catch (_) {}
      }
      if (!mounted || _selectedId != v.id) return;
      setState(() {
        _content = content;
        _prevContent = prevContent;
        _previewLoading = false;
      });
    } catch (error) {
      if (!mounted || _selectedId != v.id) return;
      setState(() {
        _previewError = error.toString();
        _previewLoading = false;
      });
    }
  }

  /// A block equals its predecessor if kind + text + data all match (a cheap
  /// structural compare; data is order-insensitive via jsonEncode of a sorted
  /// view is overkill here — the block data is small and written consistently).
  bool _blockEqual(Map<String, dynamic> a, Map<String, dynamic> b) {
    return a['type'] == b['type'] &&
        (a['text'] ?? '') == (b['text'] ?? '') &&
        jsonEncode(a['data']) == jsonEncode(b['data']);
  }

  /// The top-level blocks of a version, in tree order (root's children — the
  /// flat shape the editor mounts).
  List<Map<String, dynamic>> _topBlocks(
    ({String rootBlockId, List<Map<String, dynamic>> blocks}) content,
  ) {
    final byId = {for (final b in content.blocks) (b['id'] as String): b};
    final childIds =
        ((byId[content.rootBlockId]?['children'] as List?) ?? const [])
            .cast<String>();
    return [
      for (final id in childIds)
        if (byId[id] != null) byId[id]!,
    ];
  }

  EditorNode _toNode(Map<String, dynamic> b, String? diff) => EditorNode(
    id: b['id'] as String,
    kind: b['type'] as String? ?? 'paragraph',
    text: b['text'] as String? ?? '',
    data: Map<String, dynamic>.from((b['data'] as Map?) ?? const {}),
    diffStatus: diff,
  );

  /// Build read-only editor nodes for the selected version, tagged with a
  /// block-level diff vs the predecessor: added (in this version, not before),
  /// changed (same id, different content), deleted (in the predecessor, gone
  /// now — spliced back in at its old position as a struck-through ghost). No
  /// predecessor → plain nodes, no tint.
  List<EditorNode> _previewNodes(
    ({String rootBlockId, List<Map<String, dynamic>> blocks}) content,
  ) {
    final current = _topBlocks(content);
    final prev = _prevContent;
    if (prev == null) {
      return [for (final b in current) _toNode(b, null)];
    }
    final prevBlocks = _topBlocks(prev);
    final prevById = {for (final b in prevBlocks) (b['id'] as String): b};
    final currentIds = {for (final b in current) b['id'] as String};

    // Group deleted blocks (in prev, not in current) by the surviving block they
    // follow, so they render at roughly their old position ('' = before all).
    final deletedAfter = <String, List<Map<String, dynamic>>>{};
    var lastSurviving = '';
    for (final p in prevBlocks) {
      final pid = p['id'] as String;
      if (currentIds.contains(pid)) {
        lastSurviving = pid;
      } else {
        (deletedAfter[lastSurviving] ??= []).add(p);
      }
    }

    final nodes = <EditorNode>[];
    for (final d in deletedAfter[''] ?? const []) {
      nodes.add(_toNode(d, 'deleted'));
    }
    for (final b in current) {
      final id = b['id'] as String;
      final before = prevById[id];
      final status = before == null
          ? 'added'
          : (_blockEqual(b, before) ? null : 'changed');
      nodes.add(_toNode(b, status));
      for (final d in deletedAfter[id] ?? const []) {
        nodes.add(_toNode(d, 'deleted'));
      }
    }
    return nodes;
  }

  Future<void> _createCheckpoint() async {
    final l10n = context.l10n;
    final name = await _promptName();
    if (name == null || name.trim().isEmpty) return;
    setState(() => _busy = true);
    try {
      await widget.onCreate(name.trim());
      await _refresh();
      _snack(l10n.versionCheckpointSaved);
    } catch (error) {
      _snack(l10n.versionSaveFailed(error.toString()));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  /// A version's display name: named checkpoints show their label; auto
  /// snapshots (label null) read as "Auto-save" and lean on the timestamp.
  String _displayName(DocVersion v) =>
      v.isAuto ? context.l10n.versionAutoSnapshot : v.label!.trim();

  /// One timeline row, card-shaped (design 08): active = accent border + wash,
  /// which at this width reads far better than Material's tinted-title `selected`.
  Widget _versionRow(BuildContext context, DocVersion v) {
    final selected = v.id == _selectedId;
    return Padding(
      padding: const EdgeInsets.only(bottom: 6),
      child: Material(
        color: selected ? const Color(0xFFF5F9FF) : Colors.transparent,
        borderRadius: BorderRadius.circular(10),
        child: InkWell(
          borderRadius: BorderRadius.circular(10),
          onTap: _busy ? null : () => _select(v),
          child: Container(
            decoration: BoxDecoration(
              borderRadius: BorderRadius.circular(10),
              border: Border.all(
                color: selected ? const Color(0xFFBFDBFE) : Colors.transparent,
              ),
            ),
            padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 9),
            child: Row(
              children: [
                Icon(
                  v.isAuto ? Icons.schedule : Icons.bookmark,
                  size: 16,
                  color: selected ? const Color(0xFF2563EB) : EditorTheme.faint,
                ),
                const SizedBox(width: 9),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        v.isAuto ? _stamp(v) : _displayName(v),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontSize: 14,
                          fontWeight: v.isAuto
                              ? FontWeight.w400
                              : FontWeight.w600,
                          color: selected
                              ? const Color(0xFF2563EB)
                              : EditorTheme.text,
                        ),
                      ),
                      ?_rowSubtitle(v),
                    ],
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  /// A version's timestamp in the design-08 relative form: `15:08` today,
  /// 「昨天 22:03」 yesterday, a bare date further back.
  ///
  /// Falls back to the raw string when the stamp does not parse — showing the
  /// unhelpful original beats inventing a date, and beats a blank where a time
  /// belongs.
  String _stamp(DocVersion v) {
    final when = versionTime(v);
    if (when == null) return v.createdAt;
    final l10n = context.l10n;
    final s = versionStamp(when);
    return switch (s.day) {
      VersionDay.today => s.time,
      VersionDay.yesterday => l10n.versionYesterdayAt(s.time),
      VersionDay.earlier => l10n.versionDateShort(s.month, s.dayOfMonth),
    };
  }

  /// Who saved this version, or null when that can't be said usefully.
  ///
  /// Deliberately quiet: an unknown id renders nothing rather than a placeholder
  /// like 「未知用户」 — a row that admits it doesn't know who did this is worse
  /// than a row that doesn't raise the question. Ids go stale legitimately (a
  /// member who left is no longer in the map).
  String? _authorName(DocVersion v) {
    final id = v.createdBy;
    if (id == null) return null;
    final name = widget.authorNames[id]?.trim();
    return (name == null || name.isEmpty) ? null : name;
  }

  /// The second line: an author when there is one to name, the timestamp for a
  /// named checkpoint (whose title is its label, not its time), or both.
  Widget? _rowSubtitle(DocVersion v) {
    final author = _authorName(v);
    final time = v.isAuto ? null : _stamp(v);
    final parts = [if (time != null) time, if (author != null) author];
    if (parts.isEmpty) return null;
    return Text(
      parts.join('  ·  '),
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
    );
  }

  Future<void> _restore(DocVersion version) async {
    final l10n = context.l10n;
    final ok = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(context.l10n.versionRestoreConfirmTitle),
        content: Text(
          context.l10n.versionRestoreConfirmBody(_displayName(version)),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: Text(context.l10n.commonCancel),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: Text(context.l10n.versionRestore),
          ),
        ],
      ),
    );
    if (ok != true) return;
    setState(() => _busy = true);
    try {
      await widget.onRestore(version.id);
      if (!mounted) return;
      _snack(l10n.versionRestored(_displayName(version)));
      Navigator.of(context).pop();
    } catch (error) {
      _snack(l10n.versionRestoreFailed(error.toString()));
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<String?> _promptName() {
    final controller = TextEditingController();
    return showDialog<String>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(context.l10n.versionSaveCheckpoint),
        content: TextField(
          controller: controller,
          autofocus: true,
          decoration: InputDecoration(hintText: context.l10n.versionNameHint),
          onSubmitted: (v) => Navigator.pop(context, v),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: Text(context.l10n.commonCancel),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, controller.text),
            child: Text(context.l10n.commonSave),
          ),
        ],
      ),
    );
  }

  void _snack(String message) {
    if (!mounted) return;
    ScaffoldMessenger.of(
      context,
    ).showSnackBar(SnackBar(content: Text(message)));
  }

  DocVersion? get _selected =>
      _versions.where((v) => v.id == _selectedId).firstOrNull;

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    return Dialog(
      insetPadding: const EdgeInsets.symmetric(horizontal: 40, vertical: 24),
      clipBehavior: Clip.antiAlias,
      child: SizedBox(
        width: 980,
        height: 640,
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Expanded(child: _buildPreview(l10n)),
            const VerticalDivider(width: 1),
            SizedBox(width: 300, child: _buildTimeline(l10n)),
          ],
        ),
      ),
    );
  }

  Widget _buildPreview(AppLocalizations l10n) {
    if (_selectedId == null) {
      return Center(
        child: Text(l10n.versionEmpty, textAlign: TextAlign.center),
      );
    }
    if (_previewLoading) {
      return const Center(child: CircularProgressIndicator());
    }
    if (_previewError != null) {
      return Center(child: Text(l10n.versionLoadFailed(_previewError!)));
    }
    final content = _content;
    if (content == null) return const SizedBox.shrink();
    // The SAME editor in canEdit:false — reused, not re-rendered (P-A hardening
    // hides caret/IME/toolbars). Isolated: it renders the version's own blocks,
    // never the live document, so it can't affect the open page.
    return Column(
      children: [
        // Diff legend — shown only when there's a predecessor to compare against.
        if (_prevContent != null)
          Container(
            width: double.infinity,
            padding: const EdgeInsets.symmetric(horizontal: 32, vertical: 6),
            color: Theme.of(
              context,
            ).colorScheme.surfaceContainerHighest.withValues(alpha: 0.4),
            child: Row(
              children: [
                _legendDot(const Color(0xFF22C55E), l10n.versionDiffAdded),
                const SizedBox(width: 14),
                _legendDot(const Color(0xFFF59E0B), l10n.versionDiffChanged),
                const SizedBox(width: 14),
                _legendDot(const Color(0xFFEF4444), l10n.versionDiffRemoved),
              ],
            ),
          ),
        Expanded(child: _buildPreviewBody(content)),
      ],
    );
  }

  Widget _legendDot(Color color, String label) => Row(
    mainAxisSize: MainAxisSize.min,
    children: [
      Container(
        width: 10,
        height: 10,
        decoration: BoxDecoration(
          color: color,
          borderRadius: BorderRadius.circular(2),
        ),
      ),
      const SizedBox(width: 5),
      Text(label, style: const TextStyle(fontSize: 12)),
    ],
  );

  Widget _buildPreviewBody(
    ({String rootBlockId, List<Map<String, dynamic>> blocks}) content,
  ) {
    return SingleChildScrollView(
      padding: const EdgeInsets.symmetric(horizontal: 32, vertical: 24),
      child: Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 720),
          child: MicaEditor(
            key: ValueKey('version-preview-$_selectedId'),
            rootBlockId: content.rootBlockId,
            nodes: _previewNodes(content),
            version: 0,
            canEdit: false,
            onApplyOperations: (_) async {},
            onLoadImageBytes: widget.onLoadImageBytes,
            onResolveImageUrls: widget.onResolveImageUrls,
          ),
        ),
      ),
    );
  }

  Widget _buildTimeline(AppLocalizations l10n) {
    return Column(
      children: [
        // Header: title + save-checkpoint + close.
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 12, 8, 8),
          child: Row(
            children: [
              Text(
                l10n.versionHistoryTitle,
                style: const TextStyle(fontWeight: FontWeight.w600),
              ),
              const Spacer(),
              IconButton(
                tooltip: l10n.versionSaveCheckpoint,
                onPressed: _busy ? null : _createCheckpoint,
                icon: const Icon(Icons.bookmark_add_outlined, size: 20),
              ),
              IconButton(
                tooltip: l10n.commonClose,
                onPressed: () => Navigator.of(context).pop(),
                icon: const Icon(Icons.close, size: 20),
              ),
            ],
          ),
        ),
        const Divider(height: 1),
        Expanded(
          child: _loading
              ? const Center(child: CircularProgressIndicator())
              : _error != null
              ? Center(child: Text(l10n.versionLoadFailed(_error!)))
              : _versions.isEmpty
              ? Center(
                  child: Text(l10n.versionEmpty, textAlign: TextAlign.center),
                )
              : ListView(
                  padding: const EdgeInsets.symmetric(horizontal: 8),
                  children: [
                    // 今天 / 更早 (design 08). A flat list of bare timestamps
                    // gave no sense of when you were looking at.
                    for (final section in groupVersions(_versions)) ...[
                      Padding(
                        padding: const EdgeInsets.only(
                          left: 6,
                          top: 10,
                          bottom: 4,
                        ),
                        child: Text(
                          section.day == VersionDay.today
                              ? context.l10n.versionGroupToday
                              : context.l10n.versionGroupEarlier,
                          style: const TextStyle(
                            fontSize: 11,
                            fontWeight: FontWeight.w600,
                            letterSpacing: 0.6,
                            color: EditorTheme.faint,
                          ),
                        ),
                      ),
                      for (final v in section.items) _versionRow(context, v),
                    ],
                  ],
                ),
        ),
        const Divider(height: 1),
        // Restore bar — acts on the previewed version (bottom-right, AFFiNE/
        // Notion shape).
        Padding(
          padding: const EdgeInsets.all(12),
          child: Row(
            mainAxisAlignment: MainAxisAlignment.end,
            children: [
              FilledButton(
                onPressed: (_busy || _selected == null)
                    ? null
                    : () => _restore(_selected!),
                child: Text(l10n.versionRestore),
              ),
            ],
          ),
        ),
      ],
    );
  }
}

class _RecycleBinDialog extends StatefulWidget {
  const _RecycleBinDialog({
    required this.onLoad,
    required this.onRestore,
    required this.onPurge,
    required this.canEdit,
    required this.liveViews,
    required this.relativeStrings,
  });

  /// The workspace's live tree, used only to name where a restore will land.
  final List<DocumentView> liveViews;

  /// Localized relative-time wording for the deletion time.
  final RelativeTimeStrings relativeStrings;

  final Future<List<DocumentView>> Function() onLoad;
  final Future<void> Function(DocumentView view) onRestore;
  final Future<void> Function(DocumentView view) onPurge;

  /// Both restore and purge are gated on the editor role server-side
  /// (`ensure_workspace_editor`). The buttons were drawn regardless, so a viewer
  /// could press either and collect a 403 — the same dead-control shape the
  /// null-means-absent rule elsewhere in this file exists to prevent. The list
  /// itself stays visible: knowing what was deleted is useful read-only.
  final bool canEdit;

  @override
  State<_RecycleBinDialog> createState() => _RecycleBinDialogState();
}

class _RecycleBinDialogState extends State<_RecycleBinDialog> {
  bool _loading = true;
  String? _error;
  List<TrashEntry> _entries = const [];

  @override
  void initState() {
    super.initState();
    _refresh();
  }

  Future<void> _refresh() async {
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final all = await widget.onLoad();
      // Roots only, plus what each restore will actually bring back — see
      // `buildTrashEntries`.
      final entries = buildTrashEntries(deleted: all, live: widget.liveViews);
      if (!mounted) return;
      setState(() {
        _entries = entries;
        _loading = false;
      });
    } catch (error) {
      if (!mounted) return;
      setState(() {
        _error = error.toString();
        _loading = false;
      });
    }
  }

  /// A folder must LOOK like a folder. Every row used to wear the same page
  /// glyph, so a deleted folder — the one case where restore reaches past the row
  /// you clicked — was indistinguishable from a single page.
  Widget _trashLeading(DocumentView view) {
    final emoji = view.icon?.trim();
    if (emoji != null && emoji.isNotEmpty) {
      return SizedBox(
        width: 18,
        child: Center(child: Text(emoji, style: const TextStyle(fontSize: 15))),
      );
    }
    return Icon(
      view.isFolder ? Icons.folder_outlined : Icons.description_outlined,
      size: 18,
      color: EditorTheme.muted,
    );
  }

  /// Where it came from · what comes back with it · when it was deleted.
  ///
  /// Each part is dropped when it isn't known rather than filled with a
  /// placeholder: the local world carries no `icon` and no deletion timestamp
  /// (they never reach `DocumentView` through `_viewFromData`), and a top-level
  /// page has no path. An empty subtree says nothing rather than 「含 0 个页面」.
  Widget? _trashSubtitle(BuildContext context, TrashEntry entry) {
    final l10n = context.l10n;
    final counts = <String>[
      if (entry.folders > 0) l10n.folderCount(entry.folders),
      if (entry.pages > 0) l10n.pageCount(entry.pages),
    ];
    // `delete_view` stamps `updated_at = now()` when trashing, so for a deleted
    // row this IS the deletion time. Null in the local world.
    final deletedAt = entry.view.updatedAt;
    final parts = <String>[
      if (entry.path.isNotEmpty) entry.path,
      if (counts.isNotEmpty)
        l10n.recycleSubtree(counts.join(l10n.importListSeparator)),
      if (deletedAt != null) relativeMeta(deletedAt, widget.relativeStrings),
    ];
    if (parts.isEmpty) return null;
    return Text(
      parts.join('  ·  '),
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
      style: const TextStyle(fontSize: 12, color: EditorTheme.faint),
    );
  }

  /// Permanent delete is the only irreversible action in this dialog, and it
  /// reaches far wider than the row you clicked: the server drops the whole
  /// subtree's views *and* their backing `documents` rows (`purge_view_subtree`
  /// in `routes/documents.rs`), which cascades version history and comments
  /// away with them. It used to fire straight off the icon button, so one
  /// mis-click next to 「恢复」 destroyed a subtree with nothing to undo it.
  Future<bool> _confirmPurge(DocumentView view) {
    final l10n = context.l10n;
    return showDestructiveConfirm(
      context,
      title: l10n.recyclePurgeConfirmTitle(view.name),
      body: l10n.recyclePurgeConfirmBody,
      confirmLabel: l10n.recycleDeleteForever,
      cancelLabel: l10n.commonCancel,
    );
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: Row(
        children: [
          const Icon(Icons.delete_outline, size: 22),
          const SizedBox(width: 8),
          Text(context.l10n.recycleBinTitle),
          const Spacer(),
          IconButton(
            tooltip: context.l10n.recycleRefresh,
            onPressed: _loading ? null : _refresh,
            icon: const Icon(Icons.refresh, size: 20),
          ),
        ],
      ),
      content: SizedBox(width: 420, height: 360, child: _buildBody(context)),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(context.l10n.commonClose),
        ),
      ],
    );
  }

  Widget _buildBody(BuildContext context) {
    if (_loading) {
      return const Center(child: CircularProgressIndicator());
    }
    if (_error != null) {
      return Center(child: ErrorBanner(_error!));
    }
    if (_entries.isEmpty) {
      return EmptyState(
        icon: Icons.delete_outline,
        title: context.l10n.recycleEmpty,
        detail: context.l10n.recycleEmptyDetail,
      );
    }
    return ListView.separated(
      itemCount: _entries.length,
      separatorBuilder: (_, _) => const Divider(height: 1),
      itemBuilder: (context, i) {
        final entry = _entries[i];
        final view = entry.view;
        return ListTile(
          contentPadding: const EdgeInsets.symmetric(horizontal: 4),
          leading: _trashLeading(view),
          title: Text(view.name, overflow: TextOverflow.ellipsis),
          subtitle: _trashSubtitle(context, entry),
          trailing: !widget.canEdit
              ? null
              : Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    IconButton(
                      tooltip: context.l10n.recycleRestore,
                      icon: const Icon(Icons.restore, size: 20),
                      onPressed: () async {
                        await widget.onRestore(view);
                        await _refresh();
                      },
                    ),
                    IconButton(
                      tooltip: context.l10n.recycleDeleteForever,
                      color: const Color(0xFFDC2626),
                      icon: const Icon(Icons.delete_forever, size: 20),
                      onPressed: () async {
                        if (!await _confirmPurge(view)) return;
                        await widget.onPurge(view);
                        await _refresh();
                      },
                    ),
                  ],
                ),
        );
      },
    );
  }
}
