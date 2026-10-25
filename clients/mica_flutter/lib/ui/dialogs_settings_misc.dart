part of '../main.dart';

// The two leaf sections that fit nowhere else — diagnostics, keyboard
// shortcuts — plus the About popup.
extension _SettingsMiscTab on _SettingsDialogState {
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
        // The link stays now that Mica is MIT/Apache-2.0 (2026-08-28) even
        // though nothing compels it any more: it went in for AGPL §13, which
        // required prominently offering the Corresponding Source to anyone
        // interacting over a network. Permissive licensing removes the
        // obligation, not the reason — the version above tells a reader which
        // tag they are looking at, and that is worth keeping on its own.
        Align(
          alignment: Alignment.centerLeft,
          child: TextButton.icon(
            onPressed: () => openUrl('https://github.com/weironz/mica'),
            icon: const Icon(Icons.code, size: 16),
            label: Text(context.l10n.aboutSourceCode),
          ),
        ),
        // Detailed status shares the sidebar updater; absent without an installer.
        if (updateSupported) ...[
          const SizedBox(height: 8),
          UpdateChecker(controller: _desktopUpdates),
        ],
      ],
    );
  }

  Widget _kbd(String text) => Container(
    padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
    decoration: BoxDecoration(
      color: MicaTheme.of(context).surface.sunken,
      borderRadius: BorderRadius.circular(5),
      border: Border.all(color: MicaTheme.of(context).border.strong),
    ),
    child: Text(text, style: const TextStyle(fontSize: 12)),
  );

  /// Opt-in capture, for handing a reproducible bug over instead of describing
  /// it. Off by default and stated plainly, because it records real content.
  List<Widget> _diagnosticsSection(BuildContext context) {
    final l = context.l10n;
    // Errors the app drops by design (an unreachable WebSocket is a state, not a
    // crash — see swallowed.dart). Shown ONLY when something has fired: a row
    // reading "none" every time trains the eye to skip it, and the absent row
    // already says as much. Not gated on the capture switch — there is nothing to
    // record, and the value is being able to look AFTER the fact, which is
    // exactly when arming a switch in advance was not an option.
    final dropped = swallowedSummary();
    return [
      SwitchListTile(
        contentPadding: EdgeInsets.zero,
        dense: true,
        value: _diagnostics,
        title: Text(l.settingsDiagnosticsToggle),
        subtitle: Text(l.settingsDiagnosticsToggleSub),
        onChanged: (value) {
          setDiagnostics(value);
          setUi(() => _diagnostics = value);
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
      if (dropped != null) ...[
        const SizedBox(height: 12),
        SelectableText(
          '${l.settingsDiagnosticsDropped}: $dropped',
          style: Theme.of(context).textTheme.bodySmall,
        ),
      ],
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
        style: TextStyle(
          fontWeight: FontWeight.w600,
          color: MicaTheme.of(context).text.muted,
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
      row('Ctrl + Click', context.l10n.shortcutsTreeToggleSelect),
      row('Shift + Click', context.l10n.shortcutsTreeRangeSelect),
      row('Esc', context.l10n.shortcutsTreeClearSelect),
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
        style: Theme.of(context).textTheme.bodySmall?.copyWith(
          color: MicaTheme.of(context).text.faint,
        ),
      ),
    ];
  }
}
