part of '../main.dart';

// AI tab: provider list/detail/add forms, model fetch and commit-on-blur
// saves. Everything here sits behind a non-null onLoadAiSettings.
extension _SettingsAiTab on _SettingsDialogState {
  /// Fingerprint of what is in the form, INCLUDING which provider it belongs
  /// to. Without the id, switching between two providers that happen to hold
  /// identical values would look unchanged and skip the write that makes the
  /// new one active.
  String get _aiNow =>
      '$_fieldsProviderId|$_protocol|${_baseUrl.text.trim()}|'
      '${_model.text.trim()}|${_apiKey.text.trim()}';

  Future<void> _load() async {
    // 本地模式: no AI provider to load, and no AI tab to load it into. initState
    // calls this unconditionally, so the absence has to be handled here.
    final load = widget.onLoadAiSettings;
    if (load == null) return;
    setUi(() {
      _loading = true;
      _aiLoadFailed = false;
      _error = null;
    });
    try {
      final settings = await load();
      if (!mounted) return;
      setUi(() => _loading = false);
      // Same population path a provider switch takes, so a reload and a switch
      // cannot end up showing different things.
      _adoptSettings(settings);
    } catch (error) {
      if (!mounted) return;
      setUi(() {
        _error = error.toString();
        _aiLoadFailed = true;
        _loading = false;
      });
    }
  }

  /// Ask the provider for its models, using the values currently in the form so
  /// a provider can be evaluated before it is saved. A failure is reported and
  /// nothing else changes — the model field stays a text box, which is the same
  /// affordance as before this button existed.
  Future<void> _fetchModels() async {
    final fetch = widget.onListAiModels;
    if (fetch == null || _fetchingModels) return;
    setUi(() {
      _fetchingModels = true;
      _modelsError = null;
    });
    final provider = _fieldsProviderId;
    try {
      final result = await fetch(
        provider: _protocol,
        providerId: provider,
        baseUrl: _baseUrl.text.trim(),
        // Only send a key the user just typed; an empty one tells the server to
        // use the stored key, which is the common case on a configured server.
        apiKey: _apiKey.text.trim().isEmpty ? null : _apiKey.text.trim(),
      );
      if (!mounted) return;
      final models = (result['models'] as List?)?.cast<String>() ?? const [];
      // File the answer under the provider it was ASKED for, not whoever is
      // selected when it lands — a switch mid-flight would otherwise hand one
      // vendor's catalogue to another.
      _modelsByProvider[provider] = models;
      setUi(() {
        _fetchingModels = false;
        if (_fieldsProviderId == provider) {
          _models = models;
          _modelsError = models.isEmpty ? context.l10n.aiModelsEmpty : null;
        }
      });
    } catch (error) {
      if (!mounted) return;
      setUi(() {
        _fetchingModels = false;
        // Same filing rule as success: not the vendor now selected.
        if (_fieldsProviderId == provider) {
          _modelsError = _modelsFailure(context, error);
        }
      });
    }
  }

  /// Turn a fetch failure into something the reader can act on. By status
  /// code, not message text — the message is English server prose a reworded
  /// sentence would break. 404 means this route is missing on the server being
  /// talked to (the api is older than this build), not a bare grey `not found`
  /// that reads like the button did nothing.
  String _modelsFailure(BuildContext context, Object error) {
    final status = error is ApiException ? error.statusCode : null;
    return switch (status) {
      404 => context.l10n.aiModelsRouteMissing,
      401 || 403 => context.l10n.aiModelsRejected,
      _ => error is ApiException ? error.message : error.toString(),
    };
  }

  /// What to call a stored provider on screen.
  ///
  /// The built-ins get the preset table's label; anything else IS its id — the
  /// operator typed that name into the add form and it is the primary key the
  /// config lives under, so inventing a prettier one would just be a second
  /// name to keep in step. `custom` is excluded from the table on purpose: it
  /// is the "roll your own" ENTRY, never a vendor.
  String _providerLabel(String id) {
    for (final preset in _AiPreset.values) {
      if (preset != _AiPreset.custom && preset.id == id) return preset.label;
    }
    return id;
  }

  /// The built-in endpoint for a provider id, or empty for one we do not know.
  /// Used only as a floor under the base-URL box — see [_adoptSettings].
  String _presetBaseUrl(String id) {
    for (final preset in _AiPreset.values) {
      if (preset != _AiPreset.custom && preset.id == id) return preset.baseUrl;
    }
    return '';
  }

  /// Show one provider's stored config, and make it the one in use.
  ///
  /// Selecting IS activating, because the server has no "read one provider"
  /// route — `GET /api/ai/settings` answers about the active one only, and the
  /// single call that returns another provider's row is the write that
  /// activates it. A client-side "look without switching" would have to guess
  /// the row's contents from the list payload and then quietly activate it the
  /// moment anything was edited, which is a worse lie than the honest one the
  /// help text tells.
  Future<void> _selectProvider(String id) async {
    final save = widget.onSaveAiSettings;
    if (save == null) return;
    if (id == _selectedId && !_addingProvider) return;
    // Let a blur-save that this very tap triggered finish first. Otherwise the
    // two writes race and the loser can leave the OLD provider active.
    await _aiSaveInFlight;
    if (!mounted) return;
    setUi(() {
      _addingProvider = false;
      _saving = true;
      _error = null;
      _modelsError = null;
      // Anything half-typed in the key box belongs to the provider being LEFT,
      // and a blur would otherwise hand it to the one being opened.
      _apiKey.clear();
    });
    // Note what is NOT set here: the selection. Moving it before the answer
    // arrives would, on a failed write, leave the new provider highlighted with
    // the old one's endpoint and key sitting in the fields — and the next blur
    // would commit exactly that. The spinner covers the round trip;
    // [_adoptSettings] moves the selection and the fields together or not at
    // all.
    try {
      // The id ALONE. Sending the form's base URL or protocol here would write
      // the provider being LEFT over the one being opened — and sending an
      // empty model would clear the model that one already had.
      final result = await save(providerId: id);
      if (!mounted) return;
      _adoptSettings(result);
    } catch (error) {
      if (!mounted) return;
      setUi(() {
        _error = error.toString();
        _saving = false;
      });
    }
  }

  /// Create a provider from the add form and switch to it.
  Future<void> _addProvider() async {
    final save = widget.onSaveAiSettings;
    if (save == null) return;
    final preset = _addPreset;
    final id = preset == _AiPreset.custom
        ? _addName.text.trim().toLowerCase()
        : preset.id;
    final base = _addBaseUrl.text.trim();
    final l10n = context.l10n;
    if (!isValidAiProviderId(id)) {
      setUi(() => _addError = l10n.aiProviderNameInvalid);
      return;
    }
    if (_providers.any((row) => row.id == id)) {
      setUi(() => _addError = l10n.aiProviderNameTaken(_providerLabel(id)));
      return;
    }
    if (base.isEmpty) {
      setUi(() => _addError = l10n.aiProviderBaseUrlRequired);
      return;
    }
    await _aiSaveInFlight;
    if (!mounted) return;
    setUi(() {
      _saving = true;
      _addError = null;
      _error = null;
      _modelsError = null;
      _apiKey.clear();
    });
    // Same rule as the switch above: the selection moves in [_adoptSettings],
    // once the row exists. Pointing it at an id the server rejected would leave
    // the form editing a provider that is not there.
    try {
      // No key and no model: this call exists to bring the row into being, and
      // both are entered on the detail form that opens next. `model: null`
      // rather than `''` for the usual reason — an empty string is a value the
      // server would write.
      final result = await save(
        providerId: id,
        provider: preset.provider,
        baseUrl: base,
        model: null,
      );
      if (!mounted) return;
      _addName.clear();
      _addBaseUrl.clear();
      _adoptSettings(result);
    } catch (error) {
      if (!mounted) return;
      setUi(() {
        _addError = error.toString();
        _saving = false;
      });
    }
  }

  /// Forget one provider, after asking.
  ///
  /// Deleting the one in use is allowed and leaves the instance unconfigured —
  /// the server says so and the pane renders it. Nothing here promotes a
  /// replacement: picking which provider runs is the operator's call, and they
  /// are standing right here.
  Future<void> _deleteProvider(String id) async {
    final remove = widget.onDeleteAiProvider;
    if (remove == null) return;
    final l10n = context.l10n;
    final confirmed = await showDestructiveConfirm(
      context,
      title: l10n.aiDeleteProviderTitle,
      body: l10n.aiDeleteProviderBody(_providerLabel(id)),
      confirmLabel: l10n.commonDelete,
      cancelLabel: l10n.commonCancel,
    );
    if (!confirmed || !mounted) return;
    await _aiSaveInFlight;
    if (!mounted) return;
    setUi(() {
      _saving = true;
      _error = null;
      _apiKey.clear();
    });
    try {
      final result = await remove(id);
      if (!mounted) return;
      _modelsByProvider.remove(id);
      _adoptSettings(result);
    } catch (error) {
      if (!mounted) return;
      setUi(() {
        _error = error.toString();
        _saving = false;
      });
    }
  }

  /// Populate every field from one settings payload — the single place that
  /// decides what the screen shows, so a switch, an add, a delete and a reload
  /// cannot disagree.
  void _adoptSettings(Map<String, dynamic> settings) {
    final providerId = settings['provider_id'] as String? ?? '';
    final provider = settings['provider'] as String? ?? 'openai';
    final base = settings['base_url'] as String? ?? '';
    final model = settings['model'] as String? ?? '';
    final rows = <_AiProviderRow>[
      for (final entry
          in (settings['configured_providers'] as List? ?? const []))
        if (entry is Map && (entry['provider_id'] as String? ?? '').isNotEmpty)
          (
            id: entry['provider_id'] as String,
            hasKey: entry['has_key'] == true,
            model: entry['model'] as String? ?? '',
            baseUrl: entry['base_url'] as String? ?? '',
            protocol: entry['protocol'] as String? ?? 'openai',
            active: entry['active'] == true,
          ),
    ]..sort((a, b) => a.id.compareTo(b.id));
    setUi(() {
      _providers = rows;
      _selectedId = providerId;
      _fieldsProviderId = providerId;
      _protocol = provider == 'anthropic' ? 'anthropic' : 'openai';
      _models = _modelsByProvider[providerId] ?? const [];
      // A SELECTED provider always has an endpoint in the box: what is stored,
      // and failing that the built-in one for its id. An empty box was the
      // reported "the DeepSeek URL only appears when I click in" — the box held
      // nothing, and what appeared on focus was the hint, which Material keeps
      // hidden behind the resting label until the label floats.
      _baseUrl.text = base.isNotEmpty ? base : _presetBaseUrl(providerId);
      _model.text = model;
      _apiKey.clear();
      _hasKey = settings['has_key'] == true;
      _keyHint = settings['key_hint'] as String? ?? '';
      _canEdit = settings['can_edit'] as bool? ?? true;
      _addingProvider = false;
      _addError = null;
      _saving = false;
      _error = null;
    });
    _aiSaved = _aiNow;
    // A configured provider with no model is configured-but-unusable, and the
    // user cannot pick a name they have no way to know.
    if (_hasKey && _model.text.trim().isEmpty && _selectedId.isNotEmpty) {
      unawaited(_fetchModels());
    }
  }

  /// Commit the AI fields when one loses focus — and only if something actually
  /// changed, so tabbing through untouched fields is silent.
  void _saveAiOnBlur() {
    if (_loading || _aiFocus.any((f) => f.hasFocus)) return;
    // Nothing to commit while the add form is up, and its fields belong to a
    // provider that does not exist yet.
    if (_addingProvider || _selectedId.isEmpty) return;
    if (_aiNow == _aiSaved) return;
    unawaited(_saveAi());
  }

  /// Commit the form, and publish the write so a switch/add/delete can wait for
  /// it. The handle is never cleared: awaiting an already-completed future costs
  /// a microtask, and a nullable "is one running" flag would have to be right
  /// about every early return in [_saveAiNow].
  Future<void> _saveAi() {
    final pending = _saveAiNow();
    _aiSaveInFlight = pending;
    return pending;
  }

  /// Persist the AI provider settings. No longer behind a Save button: every
  /// other setting here applies as you touch it (toggles, sliders), and this was
  /// the only holdout — the button existed for these three text fields and then
  /// sat under every page, saving AI settings no matter what you were looking at.
  Future<void> _saveAiNow() async {
    // Unreachable in 本地模式 (the blur listeners belong to fields that only the
    // AI tab builds), but this is what makes that a fact rather than a hope.
    final save = widget.onSaveAiSettings;
    if (save == null) return;
    // No provider selected means no row to write into. Reachable after
    // deleting the last one, when a stale blur can still arrive.
    if (_fieldsProviderId.isEmpty) return;
    final writing = _fieldsProviderId;
    setUi(() {
      _saving = true;
      _error = null;
    });
    try {
      final result = await save(
        // The provider these VALUES belong to, not whichever row the left
        // column highlights — a switch is async, and a blur landing after it
        // would otherwise write the old provider's key onto the new one.
        providerId: writing,
        provider: _protocol,
        baseUrl: _baseUrl.text.trim(),
        model: _model.text.trim(),
        apiKey: _apiKey.text.trim().isEmpty ? null : _apiKey.text.trim(),
      );
      if (!mounted) return;
      // Stay open. Saving is not leaving — the dialog closes when the user says
      // so, which is the whole point of settings that apply as you go.
      //
      // Adopt the ANSWER rather than just clearing the spinner: entering a key
      // has to flip the badge and put the dot on this provider's row in the
      // list, and only the server can say that a key is now stored. Safe
      // against stomping on typing — a blur save runs precisely when no AI
      // field holds focus. Skipped if a switch overtook this write; that
      // switch's own answer is the current one.
      if (_fieldsProviderId == writing) {
        _adoptSettings(result);
      } else {
        setUi(() => _saving = false);
      }
    } catch (error) {
      if (!mounted) return;
      setUi(() {
        _saving = false;
        _error = error.toString();
      });
    }
  }

  // ── AI provider settings ──────────────────────────────────────────────────
  //
  // Two columns: every configured provider on the left, the selected one's form
  // on the right. It replaced a single dropdown, and the reason is that a
  // dropdown can only answer "which one is in use" — it cannot show that three
  // providers are set up, which of them still lacks a key, or offer any way to
  // add and remove them. The instance has had one row per provider since
  // migration 0022; this is the screen that finally admits it.
  List<Widget> _aiSection(BuildContext context) => [
    if (_loading) ...[
      const LinearProgressIndicator(),
      const SizedBox(height: 12),
      Text(context.l10n.settingsAiLoading),
      const SizedBox(height: 18),
    ],
    SwitchListTile(
      contentPadding: EdgeInsets.zero,
      dense: true,
      value: _aiEnabled,
      title: Text(context.l10n.aiEnable),
      subtitle: Text(context.l10n.aiEnableSub),
      onChanged: (value) {
        setUi(() => _aiEnabled = value);
        widget.onAiEnabledChanged(value);
      },
    ),
    const SizedBox(height: 8),
    if (_aiLoadFailed) ...[
      if (_error != null) ErrorBanner(_error!),
      const SizedBox(height: 8),
      Align(
        alignment: AlignmentDirectional.centerStart,
        child: OutlinedButton.icon(
          onPressed: _load,
          icon: const Icon(Icons.refresh, size: 16),
          label: Text(context.l10n.commonRetry),
        ),
      ),
    ],
    if (!_loading && !_aiLoadFailed) ...[
      // Side by side where there is room, stacked where there is not. The
      // breakpoint is about the FORM, not the window: below it the endpoint field
      // and the model row wrap into unreadable slivers, and a list stacked above
      // a full-width form is the better trade.
      LayoutBuilder(
        builder: (context, constraints) {
          final list = _aiProviderList(context);
          final detail = _aiProviderDetail(context);
          if (constraints.maxWidth < 460) {
            return Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              mainAxisSize: MainAxisSize.min,
              children: [list, const SizedBox(height: 18), detail],
            );
          }
          return Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              SizedBox(width: 168, child: list),
              const SizedBox(width: 18),
              Expanded(child: detail),
            ],
          );
        },
      ),
      if (!_canEdit) ...[
        const SizedBox(height: 12),
        Text(
          context.l10n.aiAdminOnly,
          style: Theme.of(context).textTheme.bodySmall?.copyWith(
            color: MicaTheme.of(context).text.muted,
          ),
        ),
      ],
      if (_error != null) ...[const SizedBox(height: 12), ErrorBanner(_error!)],
    ],
  ];

  /// The left column: one row per stored provider, plus the way to add one.
  Widget _aiProviderList(BuildContext context) {
    final tokens = MicaTheme.of(context);
    final canAdd = _canEdit && !_saving && widget.onSaveAiSettings != null;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisSize: MainAxisSize.min,
      children: [
        Padding(
          padding: const EdgeInsets.only(left: 8, bottom: 6),
          child: Text(
            context.l10n.aiProvidersHeading.toUpperCase(),
            style: TextStyle(
              fontSize: 11,
              fontWeight: FontWeight.w600,
              letterSpacing: 0.6,
              color: tokens.text.faint,
            ),
          ),
        ),
        if (_providers.isEmpty)
          Padding(
            padding: const EdgeInsets.fromLTRB(8, 2, 8, 6),
            child: Text(
              context.l10n.aiNoProviders,
              style: Theme.of(
                context,
              ).textTheme.bodySmall?.copyWith(color: tokens.text.muted),
            ),
          ),
        for (final row in _providers) _aiProviderTile(context, row),
        const SizedBox(height: 4),
        Align(
          alignment: AlignmentDirectional.centerStart,
          child: TextButton.icon(
            onPressed: canAdd ? _openAddProvider : null,
            icon: const Icon(Icons.add, size: 16),
            label: Text(context.l10n.aiAddProvider),
          ),
        ),
      ],
    );
  }

  /// One row in the left column.
  Widget _aiProviderTile(BuildContext context, _AiProviderRow row) {
    final tokens = MicaTheme.of(context);
    final selected = row.id == _selectedId && !_addingProvider;
    return Padding(
      padding: const EdgeInsets.only(bottom: 2),
      child: Material(
        color: selected ? tokens.accent.wash : Colors.transparent,
        borderRadius: BorderRadius.circular(6),
        child: InkWell(
          borderRadius: BorderRadius.circular(6),
          onTap: (_saving || !_canEdit) ? null : () => _selectProvider(row.id),
          child: Padding(
            padding: const EdgeInsets.fromLTRB(8, 7, 8, 7),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: [
                Row(
                  children: [
                    Expanded(
                      child: Text(
                        _providerLabel(row.id),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontSize: 13,
                          fontWeight: selected
                              ? FontWeight.w600
                              : FontWeight.w400,
                        ),
                      ),
                    ),
                    // A provider with no key cannot answer anything. Saying so
                    // in the LIST is the point of having a list — the badge on
                    // the form only ever describes the one already opened.
                    if (!row.hasKey) ...[
                      const SizedBox(width: 4),
                      Tooltip(
                        message: context.l10n.aiKeyMissing,
                        child: Icon(
                          Icons.error_outline,
                          size: 13,
                          color: tokens.status.warning,
                        ),
                      ),
                    ],
                  ],
                ),
                const SizedBox(height: 3),
                Row(
                  children: [
                    if (row.active) ...[
                      Container(
                        padding: const EdgeInsets.symmetric(
                          horizontal: 5,
                          vertical: 1,
                        ),
                        decoration: BoxDecoration(
                          color: tokens.status.successWash,
                          borderRadius: BorderRadius.circular(4),
                        ),
                        child: Text(
                          context.l10n.aiInUse,
                          style: TextStyle(
                            fontSize: 10,
                            fontWeight: FontWeight.w600,
                            color: tokens.status.success,
                          ),
                        ),
                      ),
                      const SizedBox(width: 5),
                    ],
                    Expanded(
                      child: Text(
                        _hostOf(row.baseUrl),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontSize: 11,
                          color: tokens.text.faint,
                        ),
                      ),
                    ),
                  ],
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  /// Host part of an endpoint, for the one line a list row can spare. Falls
  /// back to the whole string rather than to nothing — an address that will not
  /// parse is exactly the one worth seeing.
  String _hostOf(String url) {
    final host = Uri.tryParse(url)?.host ?? '';
    return host.isEmpty ? url : host;
  }

  /// Open the add form, defaulting to the first built-in vendor that is not
  /// already configured — the overwhelmingly likely next choice, and picking it
  /// costs nothing since every field stays editable.
  void _openAddProvider() {
    final first = _AiPreset.values.firstWhere(
      (preset) =>
          preset != _AiPreset.custom &&
          !_providers.any((row) => row.id == preset.id),
      orElse: () => _AiPreset.custom,
    );
    setUi(() {
      _addingProvider = true;
      _addError = null;
      _addPreset = first;
      _addName.clear();
      _addBaseUrl.text = first.baseUrl;
    });
  }

  /// The right pane: the add form, the selected provider's form, or — when the
  /// instance has no provider at all — the one thing there is to do about that.
  Widget _aiProviderDetail(BuildContext context) {
    if (_addingProvider) return _aiAddForm(context);
    if (_selectedId.isEmpty) {
      return Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        mainAxisSize: MainAxisSize.min,
        children: [
          Text(
            context.l10n.aiNoProviderSelected,
            style: Theme.of(context).textTheme.bodySmall?.copyWith(
              color: MicaTheme.of(context).text.muted,
            ),
          ),
          const SizedBox(height: 10),
          FilledButton.icon(
            onPressed: (_canEdit && !_saving && widget.onSaveAiSettings != null)
                ? _openAddProvider
                : null,
            icon: const Icon(Icons.add, size: 16),
            label: Text(context.l10n.aiAddProvider),
          ),
        ],
      );
    }
    return _aiProviderForm(context);
  }

  /// The add form. Deliberately small: a vendor, an address, and — only for
  /// Local / Custom — a name. The key and the model are entered on the form
  /// that opens right after, because both of them need the provider to exist
  /// first (the model list is fetched with the key, against the address).
  Widget _aiAddForm(BuildContext context) {
    final tokens = MicaTheme.of(context);
    final custom = _addPreset == _AiPreset.custom;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(
          context.l10n.aiAddProviderTitle,
          style: const TextStyle(fontSize: 14, fontWeight: FontWeight.w600),
        ),
        const SizedBox(height: 12),
        DropdownButtonFormField<_AiPreset>(
          initialValue: _addPreset,
          // The pane is narrow enough that a long label would overflow the
          // button rather than shorten itself.
          isExpanded: true,
          decoration: InputDecoration(
            labelText: context.l10n.aiProviderPreset,
            floatingLabelBehavior: FloatingLabelBehavior.always,
            border: const OutlineInputBorder(),
          ),
          items: [
            for (final preset in _AiPreset.values)
              DropdownMenuItem(
                value: preset,
                child: Text(preset.label, overflow: TextOverflow.ellipsis),
              ),
          ],
          onChanged: _saving
              ? null
              : (preset) {
                  if (preset == null) return;
                  setUi(() {
                    _addPreset = preset;
                    _addError = null;
                    // The preset FILLS the address; it does not lock it. A
                    // vendor behind a proxy is still that vendor.
                    _addBaseUrl.text = preset.baseUrl;
                  });
                },
        ),
        if (custom) ...[
          const SizedBox(height: 12),
          TextField(
            controller: _addName,
            enabled: !_saving,
            decoration: InputDecoration(
              labelText: context.l10n.aiProviderName,
              floatingLabelBehavior: FloatingLabelBehavior.always,
              helperText: context.l10n.aiProviderNameHelp,
              helperMaxLines: 3,
              hintText: 'ollama',
              border: const OutlineInputBorder(),
            ),
          ),
        ],
        const SizedBox(height: 12),
        TextField(
          controller: _addBaseUrl,
          enabled: !_saving,
          decoration: InputDecoration(
            labelText: context.l10n.aiBaseUrl,
            floatingLabelBehavior: FloatingLabelBehavior.always,
            hintText: context.l10n.aiBaseUrlHint,
            border: const OutlineInputBorder(),
          ),
        ),
        if (_addError != null) ...[
          const SizedBox(height: 8),
          Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Icon(Icons.error_outline, size: 15, color: tokens.status.danger),
              const SizedBox(width: 6),
              Expanded(
                child: Text(
                  _addError!,
                  style: Theme.of(
                    context,
                  ).textTheme.bodySmall?.copyWith(color: tokens.status.danger),
                ),
              ),
            ],
          ),
        ],
        const SizedBox(height: 14),
        Row(
          children: [
            FilledButton(
              onPressed: _saving ? null : _addProvider,
              child: Text(context.l10n.aiAddProvider),
            ),
            const SizedBox(width: 8),
            TextButton(
              onPressed: _saving
                  ? null
                  : () => setUi(() {
                      _addingProvider = false;
                      _addError = null;
                    }),
              child: Text(context.l10n.commonCancel),
            ),
          ],
        ),
      ],
    );
  }

  /// The selected provider's form.
  Widget _aiProviderForm(BuildContext context) {
    final tokens = MicaTheme.of(context);
    final label = _providerLabel(_selectedId);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        // The name is READ-ONLY, and deliberately not a disabled text box: the
        // id is the primary key the config is stored under, so "renaming" would
        // be creating a different provider. Showing the raw id under a friendly
        // label keeps both answers on screen — the one a person recognises, and
        // the one the server knows it by.
        Text(
          label,
          style: const TextStyle(fontSize: 15, fontWeight: FontWeight.w600),
        ),
        if (label != _selectedId)
          Padding(
            padding: const EdgeInsets.only(top: 1),
            child: Text(
              _selectedId,
              style: TextStyle(fontSize: 11, color: tokens.text.faint),
            ),
          ),
        const SizedBox(height: 14),
        TextField(
          controller: _baseUrl,
          focusNode: _aiFocus[0],
          enabled: !_saving && _canEdit,
          decoration: InputDecoration(
            labelText: context.l10n.aiBaseUrl,
            // Float the label always, for the same reason as the key field
            // below: with Material's default the label sits inside an empty box
            // and hides the hint, so "empty" and "filled" look alike until you
            // click in — which is exactly what "the DeepSeek URL only shows up
            // when I put the cursor in it" was. The hint is an obviously fake
            // example address rather than a real vendor's, so whatever appears
            // on focus can never be mistaken for a stored value.
            floatingLabelBehavior: FloatingLabelBehavior.always,
            hintText: context.l10n.aiBaseUrlHint,
            border: const OutlineInputBorder(),
          ),
        ),
        const SizedBox(height: 12),
        // The wire format is a property of THIS provider, not of the vendor
        // name: several vendors serve both, and a self-hosted gateway can serve
        // either. It used to be derived from the preset table, which left a
        // custom endpoint speaking Anthropic Messages with no way to say so.
        DropdownButtonFormField<String>(
          initialValue: _protocol,
          isExpanded: true,
          decoration: InputDecoration(
            labelText: context.l10n.aiProtocol,
            floatingLabelBehavior: FloatingLabelBehavior.always,
            border: const OutlineInputBorder(),
          ),
          items: [
            DropdownMenuItem(
              value: 'openai',
              child: Text(
                context.l10n.aiProtocolOpenAi,
                overflow: TextOverflow.ellipsis,
              ),
            ),
            DropdownMenuItem(
              value: 'anthropic',
              child: Text(
                context.l10n.aiProtocolAnthropic,
                overflow: TextOverflow.ellipsis,
              ),
            ),
          ],
          onChanged: (_saving || !_canEdit)
              ? null
              : (value) {
                  if (value == null || value == _protocol) return;
                  setUi(() => _protocol = value);
                  // Commit now. Saving is on BLUR, and a dropdown never blurs a
                  // text field — the same gap that made picking a model chip
                  // change the box and nothing else.
                  unawaited(_saveAi());
                },
        ),
        const SizedBox(height: 12),
        // The saved key is never returned, so the field is always empty on
        // open. It used to say so with a row of dots as the hint — which looks
        // exactly like a filled-in password field, so "did I ever set this?"
        // had no answer. A badge answers it, and the last 4 characters answer
        // the follow-up ("is it the key I think it is?") without revealing
        // anything usable.
        Row(
          children: [
            Icon(
              _hasKey ? Icons.check_circle : Icons.error_outline,
              size: 16,
              color: _hasKey ? tokens.status.success : tokens.text.muted,
            ),
            const SizedBox(width: 6),
            Expanded(
              child: Text(
                _hasKey
                    ? (_keyHint.isEmpty
                          ? context.l10n.aiKeyConfigured
                          : context.l10n.aiKeyConfiguredHint(_keyHint))
                    : context.l10n.aiKeyMissing,
                style: Theme.of(context).textTheme.bodySmall?.copyWith(
                  color: _hasKey ? tokens.status.success : tokens.text.muted,
                ),
              ),
            ),
          ],
        ),
        const SizedBox(height: 8),
        TextField(
          controller: _apiKey,
          focusNode: _aiFocus[2],
          enabled: !_saving && _canEdit,
          obscureText: true,
          decoration: InputDecoration(
            labelText: context.l10n.aiApiKey,
            // See the base URL above: without this the label sits in the empty
            // field and hides the hint, so the dots that say "a key is stored,
            // leave this blank to keep it" were never once seen.
            floatingLabelBehavior: FloatingLabelBehavior.always,
            hintText: _hasKey
                ? context.l10n.aiApiKeyHintHasKey
                : context.l10n.aiApiKeyHintRequired,
            border: const OutlineInputBorder(),
          ),
        ),
        // The dots are a PLACEHOLDER FOR A VALUE, so they carry no prose: the
        // hint used to read `•••••••• (leave blank to keep)`, which put an
        // instruction inside the shape that stands in for the secret itself.
        // The instruction is real and still needed, so it moves here, where
        // explanations belong — and only when there IS a stored key to keep.
        if (_hasKey) ...[
          const SizedBox(height: 6),
          Text(
            context.l10n.aiKeyKeepHelp,
            style: Theme.of(context).textTheme.bodySmall?.copyWith(
              color: MicaTheme.of(context).text.muted,
            ),
          ),
        ],
        const SizedBox(height: 12),
        // Model stays a TEXT FIELD with a fetch button beside it, rather than a
        // dropdown alone. A dropdown would have to be populated from somewhere,
        // and the only honest sources are the provider (which needs a network
        // round trip and a key) or a baked-in list (which is wrong within
        // weeks). So the field always accepts a typed name, and the button
        // turns it into a pick-from-list once the provider has answered.
        Row(
          children: [
            Expanded(
              child: TextField(
                controller: _model,
                focusNode: _aiFocus[1],
                enabled: !_saving && _canEdit,
                decoration: InputDecoration(
                  labelText: context.l10n.aiModel,
                  floatingLabelBehavior: FloatingLabelBehavior.always,
                  // No example name here either — a hint is still a claim, and
                  // this one named a model DeepSeek has since retired.
                  hintText: context.l10n.aiModelHint,
                  border: const OutlineInputBorder(),
                ),
              ),
            ),
            if (widget.onListAiModels != null) ...[
              const SizedBox(width: 8),
              Tooltip(
                message: context.l10n.aiFetchModels,
                child: IconButton.outlined(
                  onPressed: (_saving || !_canEdit || _fetchingModels)
                      ? null
                      : _fetchModels,
                  icon: _fetchingModels
                      ? const SizedBox(
                          width: 16,
                          height: 16,
                          child: CircularProgressIndicator(strokeWidth: 2),
                        )
                      : const Icon(Icons.cloud_download_outlined, size: 18),
                ),
              ),
            ],
          ],
        ),
        if (_models.isNotEmpty) ...[
          const SizedBox(height: 8),
          // Chips rather than a second dropdown: the list is short enough to
          // scan, and it keeps the typed field as the single source of the
          // saved value — there is no second control that could disagree.
          Wrap(
            spacing: 6,
            runSpacing: 6,
            children: [
              for (final id in _models)
                ChoiceChip(
                  label: Text(id, style: const TextStyle(fontSize: 12)),
                  selected: _model.text.trim() == id,
                  onSelected: (_saving || !_canEdit)
                      ? null
                      : (_) {
                          setUi(() => _model.text = id);
                          // Saving is on BLUR, and assigning `controller.text`
                          // never blurs anything — so picking a model used to
                          // change the box and nothing else. The row kept its
                          // empty model, the server kept reporting "not
                          // configured", and Ask AI stayed gone with the field
                          // showing the name just chosen.
                          unawaited(_saveAi());
                        },
                ),
            ],
          ),
        ],
        // Shown whenever no model is chosen — NOT only when the list is empty.
        // It used to hide the moment the list loaded, which is exactly when the
        // user has fetched, not yet picked, and is looking for why Ask AI is
        // still gone.
        if (_model.text.trim().isEmpty) ...[
          const SizedBox(height: 6),
          Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Icon(Icons.error_outline, size: 15, color: tokens.text.muted),
              const SizedBox(width: 6),
              Expanded(
                child: Text(
                  context.l10n.aiModelMissing,
                  style: Theme.of(
                    context,
                  ).textTheme.bodySmall?.copyWith(color: tokens.text.muted),
                ),
              ),
            ],
          ),
        ],
        if (_modelsError != null) ...[
          const SizedBox(height: 6),
          // Danger colour + icon: rendered in the muted grey the hints use, a
          // failure was indistinguishable from a caption and the button looked
          // inert. A failed action has to look different from an explanation.
          Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Icon(Icons.error_outline, size: 15, color: tokens.status.danger),
              const SizedBox(width: 6),
              Expanded(
                child: Text(
                  _modelsError!,
                  style: Theme.of(
                    context,
                  ).textTheme.bodySmall?.copyWith(color: tokens.status.danger),
                ),
              ),
            ],
          ),
        ],
        const SizedBox(height: 10),
        Text(
          context.l10n.aiKeyHelp,
          style: Theme.of(
            context,
          ).textTheme.bodySmall?.copyWith(color: tokens.text.muted),
        ),
        const SizedBox(height: 4),
        Text(
          context.l10n.aiProviderInUseHelp,
          style: Theme.of(
            context,
          ).textTheme.bodySmall?.copyWith(color: tokens.text.muted),
        ),
        if (widget.onDeleteAiProvider != null && _canEdit) ...[
          const SizedBox(height: 12),
          Align(
            alignment: AlignmentDirectional.centerStart,
            child: TextButton.icon(
              onPressed: _saving ? null : () => _deleteProvider(_selectedId),
              style: TextButton.styleFrom(
                foregroundColor: kDestructiveRed(context),
              ),
              icon: const Icon(Icons.delete_outline, size: 16),
              label: Text(context.l10n.aiDeleteProvider),
            ),
          ),
        ],
      ],
    );
  }
}
