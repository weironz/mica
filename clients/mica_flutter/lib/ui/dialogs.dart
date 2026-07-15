// Modal dialogs for the Mica client (export / search / settings / AI /
// recycle bin). `part of main.dart` — same library, so they keep using its
// imports and private helpers. Extracted 2026-07 for navigability.
part of '../main.dart';

/// Shows exported Markdown in a selectable box with a one-tap copy.
class _ExportDialog extends StatelessWidget {
  const _ExportDialog({required this.title, required this.markdown});

  final String title;
  final String markdown;

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: Text(title),
      content: SizedBox(
        width: 560,
        height: 460,
        child: Container(
          width: double.infinity,
          padding: const EdgeInsets.all(12),
          decoration: BoxDecoration(
            color: const Color(0xFFF8FAFC),
            borderRadius: BorderRadius.circular(8),
            border: Border.all(color: const Color(0xFFE2E8F0)),
          ),
          child: SingleChildScrollView(
            child: SelectableText(
              markdown.isEmpty ? '(empty)' : markdown,
              style: const TextStyle(
                fontFamily: kMonoFont,
                fontSize: 13,
                height: 1.5,
              ),
            ),
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Close'),
        ),
        FilledButton.icon(
          onPressed: () async {
            await copyTextToClipboard(markdown);
            if (context.mounted) {
              ScaffoldMessenger.of(context).showSnackBar(
                const SnackBar(content: Text('Copied to clipboard')),
              );
            }
          },
          icon: const Icon(Icons.copy, size: 18),
          label: const Text('Copy'),
        ),
      ],
    );
  }
}

/// Workspace search: type to find pages by title or body text; click to open.
class _SearchDialog extends StatefulWidget {
  const _SearchDialog({required this.onSearch, required this.onOpen});

  final Future<List<SearchResult>> Function(String query) onSearch;
  final void Function(String viewId) onOpen;

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

  @override
  void dispose() {
    _debounce?.cancel();
    _query.dispose();
    super.dispose();
  }

  void _onChanged(String value) {
    _debounce?.cancel();
    _debounce = Timer(const Duration(milliseconds: 280), () => _run(value));
  }

  Future<void> _run(String value) async {
    final query = value.trim();
    if (query.isEmpty) {
      setState(() {
        _results = const [];
        _loading = false;
      });
      return;
    }
    setState(() => _loading = true);
    try {
      final results = await widget.onSearch(query);
      if (!mounted || _query.text.trim() != query) return;
      setState(() {
        _results = results;
        _lastQuery = query;
        _loading = false;
        _failed = false;
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
        });
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: const Text('Search pages'),
      content: SizedBox(
        width: 480,
        height: 420,
        child: Column(
          children: [
            TextField(
              controller: _query,
              autofocus: true,
              onChanged: _onChanged,
              decoration: InputDecoration(
                hintText: 'Search titles and content…',
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
            const SizedBox(height: 12),
            Expanded(child: _buildResults(context)),
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Close'),
        ),
      ],
    );
  }

  Widget _buildResults(BuildContext context) {
    if (_query.text.trim().isEmpty) {
      return const EmptyState(
        icon: Icons.search,
        title: 'Search this workspace',
        detail: 'Find pages by title or content.',
      );
    }
    if (!_loading && _results.isEmpty) {
      return EmptyState(
        icon: Icons.search_off,
        title: 'No matches',
        detail: _failed
            ? 'Search failed — check your connection and try again.'
            : 'Nothing found for "$_lastQuery".',
      );
    }
    return ListView.separated(
      itemCount: _results.length,
      separatorBuilder: (_, _) => const Divider(height: 1),
      itemBuilder: (context, i) {
        final result = _results[i];
        return ListTile(
          contentPadding: const EdgeInsets.symmetric(horizontal: 4),
          leading: const Icon(Icons.description_outlined, size: 18),
          title: Text(result.name, overflow: TextOverflow.ellipsis),
          subtitle: result.snippet.isEmpty
              ? (result.titleMatch ? const Text('Title match') : null)
              : Text(
                  result.snippet,
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                ),
          onTap: () => widget.onOpen(result.viewId),
        );
      },
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
    required this.cloudOrigin,
    required this.onConnectCloud,
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
    required this.activeIsLocal,
    required this.onSwitchWorld,
    required this.localAvailable,
    required this.maxPageWidth,
  });

  final String userName;
  final String userEmail;
  final Future<void> Function(String displayName) onUpdateProfile;
  final Future<void> Function(String current, String next) onChangePassword;
  final String cloudOrigin;
  final Future<void> Function(String url) onConnectCloud;
  final Future<Map<String, dynamic>> Function() onLoadAiSettings;
  final Future<void> Function({
    required String provider,
    required String baseUrl,
    required String model,
    String? apiKey,
  })
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

  /// Active world + its switch, surfaced in Settings → 服务器 (moved out of the
  /// workspace menu). [localAvailable] hides the switch on web (no local world).
  final bool activeIsLocal;
  final Future<void> Function(bool local) onSwitchWorld;
  final bool localAvailable;

  /// The realizable full-bleed column width, measured at the editor — the page-
  /// width slider's max, so its travel maps to widths the window can actually show.
  final double maxPageWidth;

  @override
  State<_SettingsDialog> createState() => _SettingsDialogState();
}

class _SettingsDialogState extends State<_SettingsDialog> {
  final _baseUrl = TextEditingController();
  final _model = TextEditingController();
  final _apiKey = TextEditingController();
  _AiPreset _preset = _AiPreset.deepseek;
  int _tab = 0; // 0 Appearance, 1 AI provider, 2 Account
  bool _loading = true;
  bool _saving = false;
  bool _hasKey = false;
  // API Tokens tab state.
  List<Map<String, dynamic>>? _tokens;
  bool _tokensLoaded = false;
  bool _tokenBusy = false;
  bool _tokenWrite = false;
  final _tokenName = TextEditingController();
  final _tokenExpiry = TextEditingController();
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
  late bool _showFormatBar = widget.showFormatBar;
  late bool _showPageTitle = widget.showPageTitle;
  late bool _aiEnabled = widget.aiEnabled;

  // Cloud server connection (P3c-2: no mode anymore — the local world always
  // exists; this only configures WHICH cloud server the cloud section uses).
  late final _serverUrl = TextEditingController(
    text: widget.cloudOrigin.isEmpty ? kMicaCloudUrl : widget.cloudOrigin,
  );
  bool _serverSaving = false;
  String? _serverMsg;

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
            style: const TextStyle(color: Color(0xFF64748B), fontSize: 13),
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

  @override
  void dispose() {
    _baseUrl.dispose();
    _model.dispose();
    _apiKey.dispose();
    _name.dispose();
    _curPass.dispose();
    _newPass.dispose();
    _serverUrl.dispose();
    _tokenName.dispose();
    _tokenExpiry.dispose();
    super.dispose();
  }

  Future<void> _saveProfile() async {
    setState(() {
      _accountBusy = true;
      _accountMsg = null;
    });
    try {
      await widget.onUpdateProfile(_name.text.trim());
      if (mounted)
        Navigator.of(context).pop(); // saved → close, like server config
    } catch (error) {
      if (mounted) setState(() => _accountMsg = error.toString());
    } finally {
      if (mounted) setState(() => _accountBusy = false);
    }
  }

  Future<void> _changeAccountPassword() async {
    if (_newPass.text.length < 8) {
      setState(
        () => _accountMsg = 'New password must be at least 8 characters',
      );
      return;
    }
    setState(() {
      _accountBusy = true;
      _accountMsg = null;
    });
    try {
      await widget.onChangePassword(_curPass.text, _newPass.text);
      if (mounted)
        Navigator.of(context).pop(); // changed → close, like server config
    } catch (error) {
      if (mounted) setState(() => _accountMsg = error.toString());
    } finally {
      if (mounted) setState(() => _accountBusy = false);
    }
  }

  Future<void> _load() async {
    try {
      final settings = await widget.onLoadAiSettings();
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
  }

  /// The built-in About popup, marking the current app version. Opened from the
  /// Settings nav's "About" item (stacks over the Settings dialog).
  void _showAboutDialog(BuildContext context) {
    showAboutDialog(
      context: context,
      applicationName: 'Mica',
      applicationVersion: 'v$kAppVersion',
      applicationIcon: const MicaLogo(size: 40),
      applicationLegalese: 'Cloud-first collaborative Markdown workspace.',
      // Self-update lives here on desktop; hidden where it can't apply (web, and
      // platforms with no packaged installer).
      children: updateSupported
          ? const [SizedBox(height: 20), UpdateChecker()]
          : null,
    );
  }

  Future<void> _showTokenSecret(Map<String, dynamic> created) {
    final token = created['token'] as String? ?? '';
    return showDialog<void>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: const Text('Token created'),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const Text('Copy it now — the secret is not shown again.'),
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
            label: const Text('Copy'),
            onPressed: () => Clipboard.setData(ClipboardData(text: token)),
          ),
          FilledButton(
            onPressed: () => Navigator.of(ctx).pop(),
            child: const Text('Done'),
          ),
        ],
      ),
    );
  }

  static String _shortTime(dynamic value) {
    if (value == null) return 'never';
    final s = value.toString();
    return s.length >= 10 ? s.substring(0, 10) : s;
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
        'API Tokens',
        const Color(0xFF2563EB),
      ),
      const SizedBox(height: 4),
      Text(
        'Long-lived tokens for the API, the CLI, and scheduled backups. '
        'The secret is shown once — copy it then. Read-only by default.',
        style: Theme.of(
          context,
        ).textTheme.bodySmall?.copyWith(color: const Color(0xFF64748B)),
      ),
      const SizedBox(height: 16),
      TextField(
        controller: _tokenName,
        decoration: const InputDecoration(
          labelText: 'Name',
          hintText: 'e.g. backup',
          border: OutlineInputBorder(),
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
          const Text('Write access'),
          const Spacer(),
          SizedBox(
            width: 140,
            child: TextField(
              controller: _tokenExpiry,
              keyboardType: TextInputType.number,
              decoration: const InputDecoration(
                labelText: 'Expires (days)',
                hintText: 'never',
                border: OutlineInputBorder(),
                isDense: true,
              ),
            ),
          ),
        ],
      ),
      const SizedBox(height: 10),
      Align(
        alignment: Alignment.centerLeft,
        child: FilledButton.icon(
          icon: const Icon(Icons.add, size: 18),
          label: const Text('Create token'),
          onPressed: _tokenBusy
              ? null
              : () async {
                  final name = _tokenName.text.trim();
                  if (name.isEmpty) {
                    setState(() => _tokensError = 'Name is required');
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
                    final days = int.tryParse(_tokenExpiry.text.trim());
                    final created = await onCreate(name, scopes, days);
                    _tokenName.clear();
                    _tokenExpiry.clear();
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
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 8),
          child: Text(
            'No tokens yet.',
            style: Theme.of(
              context,
            ).textTheme.bodySmall?.copyWith(color: const Color(0xFF94A3B8)),
          ),
        )
      else
        for (final t in _tokens!)
          ListTile(
            contentPadding: EdgeInsets.zero,
            dense: true,
            title: Text(t['name'] as String? ?? '(unnamed)'),
            subtitle: Text(
              '${(t['scopes'] as List<dynamic>?)?.join(', ') ?? ''}'
              '  ·  used ${_shortTime(t['last_used_at'])}'
              '  ·  expires ${_shortTime(t['expires_at'])}',
            ),
            trailing: IconButton(
              icon: const Icon(Icons.delete_outline, size: 20),
              tooltip: 'Revoke',
              onPressed: _tokenBusy
                  ? null
                  : () async {
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
    ];
  }

  Future<void> _save() async {
    setState(() {
      _saving = true;
      _error = null;
    });
    try {
      await widget.onSaveAiSettings(
        provider: _preset.provider,
        baseUrl: _baseUrl.text.trim(),
        model: _model.text.trim(),
        apiKey: _apiKey.text.trim().isEmpty ? null : _apiKey.text.trim(),
      );
      if (mounted)
        Navigator.of(context).pop(); // saved → close, like server config
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
    _sectionTitle(context, Icons.tune, 'Appearance', const Color(0xFF2563EB)),
    const SizedBox(height: 12),
    _sliderRow(
      label: 'Page width',
      // Max = the realizable full-bleed width (measured at the editor), so the
      // slider's whole travel changes the layout instead of the upper half being
      // a no-op once the column already fills the window. The thumb is clamped
      // into range without rewriting the stored preference unless the user drags.
      value: _pageWidth.clamp(640.0, widget.maxPageWidth),
      min: 640,
      max: widget.maxPageWidth,
      display: '${_pageWidth.clamp(640.0, widget.maxPageWidth).round()} px',
      onChanged: (value) {
        setState(() => _pageWidth = value);
        _applyAppearance();
      },
    ),
    _sliderRow(
      label: 'Font size',
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
        const SizedBox(width: 90, child: Text('Font')),
        Expanded(
          child: Wrap(
            spacing: 8,
            children: [
              _fontChip('System', null),
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
      title: const Text('Re-host network images'),
      subtitle: const Text(
        'Pasted image links are saved into Mica storage. Off: keep '
        'them as standard external Markdown links.',
      ),
      onChanged: (value) {
        setState(() => _reHostImages = value);
        widget.onReHostImagesChanged(value);
      },
    ),
    SwitchListTile(
      contentPadding: EdgeInsets.zero,
      dense: true,
      value: _showFormatBar,
      title: const Text('Formatting toolbar'),
      subtitle: const Text(
        'Show a toolbar of common Markdown actions above the page.',
      ),
      onChanged: (value) {
        setState(() => _showFormatBar = value);
        widget.onShowFormatBarChanged(value);
      },
    ),
    SwitchListTile(
      contentPadding: EdgeInsets.zero,
      dense: true,
      value: _showPageTitle,
      title: const Text('Page title'),
      subtitle: const Text(
        'Show the page title at the top of the page. Hidden, pages start '
        'straight at the first line.',
      ),
      onChanged: (value) {
        setState(() => _showPageTitle = value);
        widget.onShowPageTitleChanged(value);
      },
    ),
  ];

  List<Widget> _aiSection(BuildContext context) => [
    _sectionTitle(
      context,
      Icons.auto_awesome,
      'AI provider',
      const Color(0xFF7C3AED),
    ),
    SwitchListTile(
      contentPadding: EdgeInsets.zero,
      dense: true,
      value: _aiEnabled,
      title: const Text('Enable AI features'),
      subtitle: const Text(
        'Show the Ask AI button and the /ai command. They appear only '
        'when this is on and a provider below is configured.',
      ),
      onChanged: (value) {
        setState(() => _aiEnabled = value);
        widget.onAiEnabledChanged(value);
      },
    ),
    const SizedBox(height: 12),
    DropdownButtonFormField<_AiPreset>(
      initialValue: _preset,
      decoration: const InputDecoration(
        labelText: 'Provider',
        border: OutlineInputBorder(),
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
      enabled: !_saving,
      decoration: const InputDecoration(
        labelText: 'API base URL',
        hintText: 'https://api.deepseek.com',
        border: OutlineInputBorder(),
      ),
    ),
    const SizedBox(height: 12),
    TextField(
      controller: _model,
      enabled: !_saving,
      decoration: const InputDecoration(
        labelText: 'Model',
        hintText: 'deepseek-chat',
        border: OutlineInputBorder(),
      ),
    ),
    const SizedBox(height: 12),
    TextField(
      controller: _apiKey,
      enabled: !_saving,
      obscureText: true,
      decoration: InputDecoration(
        labelText: 'API key',
        hintText: _hasKey
            ? '•••••••• (leave blank to keep)'
            : 'Required for hosted providers',
        border: const OutlineInputBorder(),
      ),
    ),
    const SizedBox(height: 6),
    Text(
      'Local models (Ollama, LM Studio) usually need no key. '
      'The key is stored on the server and never returned.',
      style: Theme.of(
        context,
      ).textTheme.bodySmall?.copyWith(color: const Color(0xFF64748B)),
    ),
    if (_error != null) ...[const SizedBox(height: 12), ErrorBanner(_error!)],
  ];

  List<Widget> _accountSection(BuildContext context) => [
    _sectionTitle(
      context,
      Icons.person_outline,
      'Account',
      const Color(0xFF2563EB),
    ),
    const SizedBox(height: 4),
    Text(
      widget.userEmail,
      style: Theme.of(
        context,
      ).textTheme.bodySmall?.copyWith(color: const Color(0xFF64748B)),
    ),
    const SizedBox(height: 12),
    TextField(
      controller: _name,
      enabled: !_accountBusy,
      decoration: const InputDecoration(
        labelText: 'Display name',
        border: OutlineInputBorder(),
      ),
    ),
    const SizedBox(height: 8),
    Align(
      alignment: Alignment.centerLeft,
      child: OutlinedButton.icon(
        onPressed: _accountBusy ? null : _saveProfile,
        icon: const Icon(Icons.save, size: 16),
        label: const Text('Save name'),
      ),
    ),
    const SizedBox(height: 16),
    TextField(
      controller: _curPass,
      enabled: !_accountBusy,
      obscureText: true,
      decoration: const InputDecoration(
        labelText: 'Current password',
        border: OutlineInputBorder(),
      ),
    ),
    const SizedBox(height: 8),
    TextField(
      controller: _newPass,
      enabled: !_accountBusy,
      obscureText: true,
      decoration: const InputDecoration(
        labelText: 'New password (min 8 chars)',
        border: OutlineInputBorder(),
      ),
    ),
    const SizedBox(height: 8),
    Align(
      alignment: Alignment.centerLeft,
      child: OutlinedButton.icon(
        onPressed: _accountBusy ? null : _changeAccountPassword,
        icon: const Icon(Icons.lock_outline, size: 16),
        label: const Text('Change password'),
      ),
    ),
    if (_accountMsg != null) ...[
      const SizedBox(height: 10),
      Text(
        _accountMsg!,
        style: const TextStyle(color: Color(0xFF64748B), fontSize: 13),
      ),
    ],
  ];

  List<Widget> _serverSection(BuildContext context) => [
    _sectionTitle(context, Icons.dns_outlined, '服务器', const Color(0xFF2563EB)),
    const SizedBox(height: 8),
    // 本地 / 云服务器 world switch — moved here from the workspace menu (the
    // switcher just lists the active world's workspaces now). Picks which world
    // the sidebar shows. Hidden on web (no local world).
    if (widget.localAvailable) ...[
      SegmentedButton<bool>(
        segments: const [
          ButtonSegment(
            value: true,
            icon: Icon(Icons.computer_outlined, size: 18),
            label: Text('本地'),
          ),
          ButtonSegment(
            value: false,
            icon: Icon(Icons.cloud_outlined, size: 18),
            label: Text('云服务器'),
          ),
        ],
        selected: {widget.activeIsLocal},
        showSelectedIcon: false,
        onSelectionChanged: (s) {
          final local = s.first;
          if (local == widget.activeIsLocal) return;
          Navigator.of(
            context,
          ).pop(); // close Settings; land in the chosen world
          widget.onSwitchWorld(local);
        },
      ),
      const SizedBox(height: 16),
    ],
    // Which cloud server the "云服务器" world talks to.
    TextField(
      controller: _serverUrl,
      enabled: !_serverSaving,
      keyboardType: TextInputType.url,
      autocorrect: false,
      decoration: const InputDecoration(
        labelText: 'Server URL',
        hintText: 'https://mica.cloudcele.com',
        prefixIcon: Icon(Icons.link),
        border: OutlineInputBorder(),
      ),
    ),
    Align(
      alignment: Alignment.centerLeft,
      child: TextButton.icon(
        onPressed: _serverSaving
            ? null
            : () => setState(() => _serverUrl.text = kMicaCloudUrl),
        icon: const Icon(Icons.cloud_outlined, size: 16),
        label: const Text('Use Mica Cloud'),
      ),
    ),
    const SizedBox(height: 10),
    Text(
      '本地工作区始终在这台设备上,与服务器无关。切换服务器会断开当前云端'
      '连接(凭证保留,切回即恢复登录);登录/登出入口在工作区切换器与账号'
      '菜单。',
      style: Theme.of(
        context,
      ).textTheme.bodySmall?.copyWith(color: const Color(0xFF64748B)),
    ),
    if (_serverMsg != null) ...[
      const SizedBox(height: 12),
      ErrorBanner(_serverMsg!),
    ],
    const SizedBox(height: 14),
    Align(
      alignment: Alignment.centerLeft,
      child: FilledButton.icon(
        onPressed: _serverSaving ? null : _saveServer,
        icon: _serverSaving
            ? const SizedBox(
                width: 16,
                height: 16,
                child: CircularProgressIndicator(strokeWidth: 2),
              )
            : const Icon(Icons.sync, size: 18),
        label: const Text('Save & reconnect'),
      ),
    ),
  ];

  Future<void> _saveServer() async {
    final url = _serverUrl.text.trim();
    final parsed = Uri.tryParse(url);
    if (url.isEmpty ||
        parsed == null ||
        !parsed.hasScheme ||
        parsed.host.isEmpty) {
      setState(
        () => _serverMsg = 'Enter a valid URL, e.g. https://mica.example.com',
      );
      return;
    }
    setState(() {
      _serverSaving = true;
      _serverMsg = null;
    });
    await widget.onConnectCloud(url);
    // The switch lands the user in the local world (or restores the new
    // origin's session); close the dialog so they see it.
    if (mounted) Navigator.of(context).pop();
  }

  List<Widget> _dataSection(BuildContext context) => [
    _sectionTitle(
      context,
      Icons.import_export,
      'Data',
      const Color(0xFF0EA5E9),
    ),
    const SizedBox(height: 12),
    const Text(
      'Import a workspace from a ZIP — a Mica export or a Notion '
      '"Markdown & CSV" export. The page tree, ordering and images are '
      'rebuilt.',
      style: TextStyle(color: Color(0xFF64748B)),
    ),
    const SizedBox(height: 12),
    Align(
      alignment: Alignment.centerLeft,
      child: OutlinedButton.icon(
        onPressed: () => widget.onImportWorkspace(),
        icon: const Icon(Icons.upload_file_outlined, size: 18),
        label: const Text('Import workspace (ZIP)'),
      ),
    ),
    const SizedBox(height: 16),
    Text(
      'Tip: export a single page or a whole workspace from the page menu (▾) '
      'or a workspace’s ⋯ menu.',
      style: Theme.of(
        context,
      ).textTheme.bodySmall?.copyWith(color: const Color(0xFF94A3B8)),
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

  List<Widget> _shortcutsSection(BuildContext context) {
    Widget head(String t) => Padding(
      padding: const EdgeInsets.only(top: 8, bottom: 4),
      child: Text(
        t,
        style: const TextStyle(
          fontWeight: FontWeight.w600,
          color: Color(0xFF64748B),
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
      head('App'),
      row('Ctrl + N', 'New page'),
      row('Ctrl + F', 'Find in page'),
      row('Ctrl + Shift + F', 'Search workspace'),
      row('Ctrl + ,', 'Open settings'),
      const SizedBox(height: 8),
      head('Editor — format'),
      row('Ctrl + B', 'Bold'),
      row('Ctrl + I', 'Italic'),
      row('Ctrl + E', 'Inline code'),
      row('Ctrl + K', 'Link'),
      row('Ctrl + Alt + 1…6', 'Heading 1–6'),
      row('Ctrl + Alt + 0', 'Turn into text (paragraph)'),
      row('Tab / Shift + Tab', 'Indent / outdent list item'),
      const SizedBox(height: 8),
      head('Editor — edit'),
      row('Ctrl + Z', 'Undo'),
      row('Ctrl + Shift + Z', 'Redo'),
      row('Ctrl + A', 'Select all'),
      row('Ctrl + C / X / V', 'Copy / Cut / Paste'),
      row('Ctrl + Shift + V', 'Paste as plain text'),
      row('/', 'Slash command menu'),
      const SizedBox(height: 12),
      Text(
        'On macOS use ⌘ in place of Ctrl. Heading shortcuts follow the '
        'Notion/Word convention (Ctrl+Alt+N) — the web build can’t see a bare '
        'Ctrl+N, which the browser owns for tab switching.\n'
        'Note: Ctrl+, can be swallowed by a Chinese IME (punctuation toggle); '
        'switch to English input if it doesn’t respond.',
        style: Theme.of(
          context,
        ).textTheme.bodySmall?.copyWith(color: const Color(0xFF94A3B8)),
      ),
    ];
  }

  @override
  Widget build(BuildContext context) {
    // Server selection is desktop-only: the web client is served by (and talks
    // same-origin to) its own backend, and Local-offline needs the native core
    // that isn't compiled for web. Hide the whole tab on web.
    final tabs = <({String title, IconData icon, List<Widget> section})>[
      (
        title: 'Appearance',
        icon: Icons.tune,
        section: _appearanceSection(context),
      ),
      (
        title: 'AI provider',
        icon: Icons.auto_awesome,
        section: _aiSection(context),
      ),
      (
        title: 'Account',
        icon: Icons.person_outline,
        section: _accountSection(context),
      ),
      if (widget.onLoadTokens != null)
        (
          title: 'API Tokens',
          icon: Icons.key_outlined,
          section: _tokensSection(context),
        ),
      if (!kIsWeb)
        (
          title: 'Server',
          icon: Icons.dns_outlined,
          section: _serverSection(context),
        ),
      (
        title: 'Data',
        icon: Icons.import_export,
        section: _dataSection(context),
      ),
      (
        title: 'Shortcuts',
        icon: Icons.keyboard_outlined,
        section: _shortcutsSection(context),
      ),
    ];
    final titles = [for (final t in tabs) t.title];
    final icons = [for (final t in tabs) t.icon];
    final sections = [for (final t in tabs) t.section];
    return AlertDialog(
      title: const Row(
        children: [
          Icon(Icons.settings_outlined, size: 22),
          SizedBox(width: 8),
          Text('Settings'),
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
                        for (var i = 0; i < titles.length; i++)
                          ListTile(
                            dense: true,
                            selected: _tab == i,
                            leading: Icon(icons[i], size: 20),
                            title: Text(titles[i]),
                            onTap: () => setState(() => _tab = i),
                          ),
                        const Divider(height: 1),
                        // About isn't a content tab — it pops the version dialog.
                        ListTile(
                          dense: true,
                          leading: const Icon(Icons.info_outline, size: 20),
                          title: const Text('About'),
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
      actions: [
        TextButton(
          onPressed: _saving ? null : () => Navigator.of(context).pop(),
          child: const Text('Close'),
        ),
        FilledButton.icon(
          onPressed: _saving || _loading ? null : _save,
          icon: _saving
              ? const SizedBox(
                  width: 16,
                  height: 16,
                  child: CircularProgressIndicator(strokeWidth: 2),
                )
              : const Icon(Icons.save, size: 18),
          label: const Text('Save'),
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
      title: const Row(
        children: [
          Icon(Icons.auto_awesome, size: 22, color: Color(0xFF7C3AED)),
          SizedBox(width: 8),
          Text('Ask AI'),
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
              decoration: const InputDecoration(
                hintText:
                    'e.g. Write a project kickoff plan with goals, milestones, and risks',
                border: OutlineInputBorder(),
              ),
            ),
            const SizedBox(height: 12),
            Wrap(
              spacing: 8,
              children: [
                if (widget.canEdit)
                  ChoiceChip(
                    label: const Text('New page'),
                    selected: _target == _AiTarget.newPage,
                    onSelected: widget.hasWorkspace && !_busy
                        ? (_) => setState(() => _target = _AiTarget.newPage)
                        : null,
                  ),
                if (widget.canEdit && canWriteCurrent)
                  ChoiceChip(
                    label: const Text('Current page'),
                    selected: _target == _AiTarget.currentPage,
                    onSelected: _busy
                        ? null
                        : (_) =>
                              setState(() => _target = _AiTarget.currentPage),
                  ),
                ChoiceChip(
                  label: const Text('New workspace'),
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
                    const Text('Generating…'),
                  ] else if (_done)
                    const Text(
                      'Done — review, then insert.',
                      style: TextStyle(color: Color(0xFF64748B)),
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
          child: const Text('Cancel'),
        ),
        if (!_done)
          FilledButton.icon(
            onPressed: _busy ? null : _generate,
            icon: const Icon(Icons.auto_awesome, size: 18),
            label: Text(hasOutput ? 'Regenerate' : 'Generate'),
          )
        else ...[
          TextButton.icon(
            onPressed: _applying ? null : _generate,
            icon: const Icon(Icons.refresh, size: 18),
            label: const Text('Regenerate'),
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
            label: const Text('Insert'),
          ),
        ],
      ],
    );
  }
}

/// Recycle bin: lists soft-deleted pages and offers restore / delete-forever.
/// Only the roots of each deleted subtree are shown; restoring a root brings its
/// whole subtree back.
class _RecycleBinDialog extends StatefulWidget {
  const _RecycleBinDialog({
    required this.onLoad,
    required this.onRestore,
    required this.onPurge,
  });

  final Future<List<DocumentView>> Function() onLoad;
  final Future<void> Function(DocumentView view) onRestore;
  final Future<void> Function(DocumentView view) onPurge;

  @override
  State<_RecycleBinDialog> createState() => _RecycleBinDialogState();
}

class _RecycleBinDialogState extends State<_RecycleBinDialog> {
  bool _loading = true;
  String? _error;
  List<DocumentView> _roots = const [];

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
      final ids = {for (final v in all) v.id};
      // Show only subtree roots: a deleted page whose parent is not itself in
      // the bin (children come back with their parent on restore).
      final roots = all
          .where((v) => v.parentViewId == null || !ids.contains(v.parentViewId))
          .toList();
      if (!mounted) return;
      setState(() {
        _roots = roots;
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

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: Row(
        children: [
          const Icon(Icons.delete_outline, size: 22),
          const SizedBox(width: 8),
          const Text('Recycle bin'),
          const Spacer(),
          IconButton(
            tooltip: 'Refresh',
            onPressed: _loading ? null : _refresh,
            icon: const Icon(Icons.refresh, size: 20),
          ),
        ],
      ),
      content: SizedBox(width: 420, height: 360, child: _buildBody(context)),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Close'),
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
    if (_roots.isEmpty) {
      return const EmptyState(
        icon: Icons.delete_outline,
        title: 'Recycle bin is empty',
        detail: 'Deleted pages appear here and can be restored.',
      );
    }
    return ListView.separated(
      itemCount: _roots.length,
      separatorBuilder: (_, _) => const Divider(height: 1),
      itemBuilder: (context, i) {
        final view = _roots[i];
        return ListTile(
          contentPadding: const EdgeInsets.symmetric(horizontal: 4),
          leading: const Icon(Icons.description_outlined, size: 18),
          title: Text(view.name, overflow: TextOverflow.ellipsis),
          trailing: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              IconButton(
                tooltip: 'Restore',
                icon: const Icon(Icons.restore, size: 20),
                onPressed: () async {
                  await widget.onRestore(view);
                  await _refresh();
                },
              ),
              IconButton(
                tooltip: 'Delete forever',
                color: const Color(0xFFDC2626),
                icon: const Icon(Icons.delete_forever, size: 20),
                onPressed: () async {
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
