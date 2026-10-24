part of '../main.dart';

// Appearance tab: theme/font/page-width picks and the preference-row
// scaffolding the other preference screens borrow (_preferenceRow/-Group).
extension _SettingsAppearanceTab on _SettingsDialogState {
  void _applyAppearance() {
    widget.onAppearanceChanged(
      EditorAppearance(fontScale: _fontScale, fontFamily: _fontFamily),
      _pageWidth,
    );
  }

  Widget _preferenceRow(String label, Widget control) => LayoutBuilder(
    builder: (context, constraints) {
      final labelWidget = Text(label);
      if (constraints.maxWidth < 440) {
        return Padding(
          padding: const EdgeInsets.symmetric(vertical: 8),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [labelWidget, const SizedBox(height: 8), control],
          ),
        );
      }
      return Padding(
        padding: const EdgeInsets.symmetric(vertical: 6),
        child: Row(
          children: [
            SizedBox(width: 108, child: labelWidget),
            Expanded(child: control),
          ],
        ),
      );
    },
  );

  Widget _preferenceGroup(String label) => Padding(
    padding: const EdgeInsets.only(top: 18, bottom: 10),
    child: Text(
      label,
      style: TextStyle(
        fontSize: 14,
        fontWeight: FontWeight.w600,
        color: MicaTheme.of(context).text.primary,
      ),
    ),
  );

  /// Page width as 11 discrete stops (AppFlowy-style), plus a reset to the
  /// readable default. Fixed range (not window-relative) so the stops are stable
  /// "levels"; the editor caps the render at the window, so a wide stop on a
  /// small window just fills it. Reset → [kPageWidthDefault], NOT full width.
  Widget _pageWidthRow(BuildContext context) {
    final w = _pageWidth.clamp(kPageWidthMin, kPageWidthMax);
    final atDefault = w.round() == kPageWidthDefault.round();
    return _preferenceRow(
      context.l10n.settingsPageWidth,
      Row(
        children: [
          Expanded(
            child: Slider(
              value: w,
              min: kPageWidthMin,
              max: kPageWidthMax,
              divisions: kPageWidthDivisions,
              onChanged: (value) {
                refresh(() => _pageWidth = value);
                _applyAppearance();
              },
            ),
          ),
          Text(
            '${w.round()} px',
            style: TextStyle(
              color: MicaTheme.of(context).text.muted,
              fontSize: 13,
            ),
          ),
          IconButton(
            tooltip: context.l10n.settingsResetPageWidth,
            visualDensity: VisualDensity.compact,
            onPressed: atDefault
                ? null
                : () {
                    refresh(() => _pageWidth = kPageWidthDefault);
                    _applyAppearance();
                  },
            icon: const Icon(Icons.restart_alt, size: 18),
          ),
        ],
      ),
    );
  }

  Widget _sliderRow({
    required String label,
    required double value,
    required double min,
    required double max,
    required String display,
    required ValueChanged<double> onChanged,
    int? divisions,
  }) => _preferenceRow(
    label,
    Row(
      children: [
        Expanded(
          child: Slider(
            value: value,
            min: min,
            max: max,
            divisions: divisions,
            onChanged: onChanged,
          ),
        ),
        Text(
          display,
          style: TextStyle(
            color: MicaTheme.of(context).text.muted,
            fontSize: 13,
          ),
        ),
      ],
    ),
  );

  Widget _fontChip(String label, String? family) {
    return ChoiceChip(
      label: Text(label),
      selected: _fontFamily == family,
      onSelected: (_) {
        refresh(() => _fontFamily = family);
        _applyAppearance();
      },
    );
  }

  /// Theme chip. Writes the pref and flips [themeModeController], which rebuilds
  /// MaterialApp — this open dialog included, so the switch is visible on the
  /// very surface you flipped it from.
  Widget _themeChip(String label, MicaThemeMode mode) {
    return ChoiceChip(
      label: Text(label),
      selected: themeModeController.value == mode,
      onSelected: (_) => refresh(() {
        savePref('themeMode', themeModePref(mode));
        themeModeController.value = mode;
      }),
    );
  }

  /// UI-language chip. Writes through [setLanguage] (persists + flips
  /// localeController), which rebuilds MaterialApp — the whole app, including
  /// this open dialog, re-renders in the chosen language immediately.
  Widget _langChip(String label, String choice) {
    return ChoiceChip(
      label: Text(label),
      selected: currentLanguageChoice == choice,
      onSelected: (_) => refresh(() => setLanguage(choice)),
    );
  }

  List<Widget> _appearanceSection(BuildContext context) => [
    _preferenceGroup(context.l10n.settingsInterfaceGroup),
    _preferenceRow(
      context.l10n.languageLabel,
      Wrap(
        spacing: 8,
        children: [
          _langChip(context.l10n.languageSystem, kLangSystem),
          _langChip(context.l10n.languageChinese, kLangChinese),
          _langChip(context.l10n.languageEnglish, kLangEnglish),
        ],
      ),
    ),
    const SizedBox(height: 8),
    _preferenceRow(
      context.l10n.themeLabel,
      Wrap(
        spacing: 8,
        children: [
          _themeChip(context.l10n.themeSystem, MicaThemeMode.system),
          _themeChip(context.l10n.themeLight, MicaThemeMode.light),
          _themeChip(context.l10n.themeDark, MicaThemeMode.dark),
        ],
      ),
    ),
    const SizedBox(height: 8),
    _preferenceGroup(context.l10n.settingsEditorGroup),
    _pageWidthRow(context),
    _sliderRow(
      label: context.l10n.settingsFontSize,
      // The slider lives in the px domain — whole-px detents, labelled as a
      // size. See kFontBasePx above for why this replaced the free % slider.
      value: fontPxFromScale(_fontScale).toDouble(),
      min: kFontPxMin.toDouble(),
      max: kFontPxMax.toDouble(),
      divisions: kFontPxMax - kFontPxMin,
      display: '${fontPxFromScale(_fontScale)} px',
      onChanged: (value) {
        refresh(() => _fontScale = fontScaleFromPx(value.round()));
        _applyAppearance();
      },
    ),
    const SizedBox(height: 4),
    _preferenceRow(
      context.l10n.settingsFont,
      Wrap(
        spacing: 8,
        children: [
          _fontChip(context.l10n.settingsFontSystem, null),
          _fontChip('Serif', 'serif'),
          _fontChip('Mono', kMonoFont),
        ],
      ),
    ),
    const SizedBox(height: 8),
    SwitchListTile(
      contentPadding: EdgeInsets.zero,
      dense: true,
      value: _showFormatBar,
      title: Text(context.l10n.settingsFormatBar),
      subtitle: Text(context.l10n.settingsFormatBarSub),
      onChanged: (value) {
        refresh(() => _showFormatBar = value);
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
        refresh(() => _showPageTitle = value);
        widget.onShowPageTitleChanged(value);
      },
    ),
    const SizedBox(height: 16),
    MicaAppearancePreview(
      appearance: EditorAppearance(
        fontScale: _fontScale,
        fontFamily: _fontFamily,
      ),
      pageWidth: _pageWidth,
      title: context.l10n.settingsPreviewTitle,
      body: context.l10n.settingsPreviewBody,
      code: context.l10n.settingsPreviewCode,
      label: context.l10n.settingsPreview,
    ),
    _preferenceGroup(context.l10n.settingsPasteGroup),
    SwitchListTile(
      contentPadding: EdgeInsets.zero,
      dense: true,
      value: _reHostImages,
      title: Text(context.l10n.settingsReHostImages),
      subtitle: Text(context.l10n.settingsReHostImagesSub),
      onChanged: (value) {
        refresh(() => _reHostImages = value);
        widget.onReHostImagesChanged(value);
      },
    ),
    // Desktop only — a browser tab's close button belongs to the browser, and
    // no app code can intercept it.
    if (!kIsWeb) ...[
      const SizedBox(height: 8),
      Text(
        context.l10n.closeWindowHeader,
        style: TextStyle(
          fontWeight: FontWeight.w600,
          color: MicaTheme.of(context).text.muted,
        ),
      ),
      // RadioGroup, not per-tile groupValue/onChanged — those were deprecated
      // after Flutter 3.32 in favour of this ancestor.
      RadioGroup<String>(
        groupValue: _closeBehavior,
        onChanged: (value) {
          if (value == null) return;
          refresh(() => _closeBehavior = value);
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
}
