// Settings dialog: AI providers, appearance, account, tokens, data, misc.
// Its own library since 2026-11-01 (was a part of main.dart).
library;

import 'dart:async';
import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import '../l10n/locale_controller.dart';
import '../ui/mica_state.dart';
import '../diagnostics.dart';
import '../swallowed.dart';
import '../editor/model.dart' show kMonoFont;
import '../editor/editor.dart';
import '../editor/image_actions.dart';
import '../editor/open_url.dart';
import '../widgets/mica_logo.dart';
import '../ui/copy_button.dart';
import '../ui/destructive_confirm.dart';
import '../ui/format_bytes.dart';
import '../ui/panel_kit.dart';
import '../ui/settings_shell.dart';
import '../ui/appearance_preview.dart';
import '../ui/page_width_frame.dart';
export '../ui/page_width_frame.dart';
import '../ui/status_kit.dart';
import '../ui/theme_tokens.dart';
import '../ui/user_avatar.dart';
import '../ui/update_controls.dart';
import '../local/cache_stats.dart' show ClearMirrorResult, LocalCacheStats;
import '../prefs.dart';
import 'import_email_preference.dart';
import '../theme_controller.dart';
import '../updater.dart';
import '../window_setup.dart';
import '../api/client.dart';
import '../api/models.dart';

part 'dialogs_settings_ai.dart';
part 'dialogs_settings_appearance.dart';
part 'dialogs_settings_account.dart';
part 'dialogs_settings_tokens.dart';
part 'dialogs_settings_data.dart';
part 'dialogs_settings_misc.dart';


/// Preset AI providers. Each maps to a backend provider dialect (openai/anthropic)
/// plus default base URL + model; "Local / Custom" lets the user point at any
/// OpenAI-compatible server (Ollama, LM Studio, vLLM, …).
/// Endpoint shortcuts for the providers people actually reach for. A preset
/// only fills the Base URL — nothing here is a supported-provider list, because
/// anything speaking the OpenAI or Anthropic wire format works once its URL is
/// typed in, and pretending otherwise would mean shipping a list that goes
/// stale every time a vendor appears.
///
/// Deliberately NOT here: model names. Every one of these vendors renames and
/// retires models on its own schedule, so a name baked into this build is wrong
/// soon after it ships — and wrong in the worst way, since it looks like a
/// working default. The model list is fetched from the provider instead
/// ([_fetchModels]); [model] below is only a seed for a blank form.
enum _AiPreset { deepseek, zhipu, kimi, openai, anthropic, custom }

extension _AiPresetInfo on _AiPreset {
  String get label => switch (this) {
    _AiPreset.deepseek => 'DeepSeek',
    _AiPreset.zhipu => '智谱 GLM',
    _AiPreset.kimi => '月之暗面 Kimi',
    _AiPreset.openai => 'OpenAI',
    _AiPreset.anthropic => 'Anthropic (Claude)',
    _AiPreset.custom => 'Local / Custom',
  };

  /// The wire FORMAT. Several vendors speak `openai`; this is what the request
  /// code branches on, not what the dropdown selects.
  String get provider => this == _AiPreset.anthropic ? 'anthropic' : 'openai';

  /// The VENDOR id, and the key this provider's config is stored under. Must
  /// match the ids migration 0022 attributes existing rows to.
  String get id => switch (this) {
    _AiPreset.deepseek => 'deepseek',
    _AiPreset.zhipu => 'zhipu',
    _AiPreset.kimi => 'kimi',
    _AiPreset.openai => 'openai',
    _AiPreset.anthropic => 'anthropic',
    _AiPreset.custom => 'custom',
  };

  /// Corroborated against cc-switch's provider presets, which are maintained
  /// against these endpoints daily; the ones I could not corroborate are
  /// deliberately absent rather than guessed — an endpoint that 404s is worse
  /// than an empty dropdown, because it looks like the key is wrong.
  String get baseUrl => switch (this) {
    _AiPreset.deepseek => 'https://api.deepseek.com',
    _AiPreset.zhipu => 'https://open.bigmodel.cn/api/coding/paas/v4',
    _AiPreset.kimi => 'https://api.moonshot.cn/v1',
    _AiPreset.openai => 'https://api.openai.com/v1',
    _AiPreset.anthropic => 'https://api.anthropic.com',
    _AiPreset.custom => 'http://localhost:11434/v1',
  };

  /// Always blank. There is no seed because a seeded name is a claim about the
  /// vendor's catalogue that this build cannot keep.
  ///
  /// It was `deepseek-chat` here, and on 2026-08-19 a live `/v1/models` against
  /// api.deepseek.com returned exactly `deepseek-v4-flash` and
  /// `deepseek-v4-pro` — `deepseek-chat` was gone. The instance in production
  /// was still configured with it. That is the whole failure mode in one
  /// example: a retired name does not look broken, it looks like a working
  /// default, right up until a request fails for reasons that point nowhere
  /// near this line. The fetch button beside the field is the answer that
  /// cannot go stale.
  String get model => '';
}

/// The provider presets, exposed for the regression that forbids shipping model
/// NAMES (see test/ai_preset_no_baked_models_test.dart). The enum itself stays
/// private — this is a read-only window onto it, not a second way to build one.
@visibleForTesting
List<({String label, String seedModel, String baseUrlValue})>
get aiPresetsForTest => [
  for (final preset in _AiPreset.values)
    (
      label: preset.label,
      seedModel: preset.model,
      baseUrlValue: preset.baseUrl,
    ),
];

/// Whether a string may be used as a provider id.
///
/// The id is the config's primary key server-side AND a path segment of
/// `DELETE /api/ai/providers/{id}`, so it has to survive both. Deliberately
/// strict rather than escaped-and-permitted: the operator is naming a thing
/// they will read back in a list, and a name with a slash or a space in it
/// would be a source of confusion long after it stopped being a routing
/// problem. Lower-case only, because the server lower-cases what it stores —
/// letting "DeepSeek" through here would create a row the list then shows
/// under a different name than the one that was typed.
@visibleForTesting
bool isValidAiProviderId(String id) => _providerIdPattern.hasMatch(id);

final _providerIdPattern = RegExp(r'^[a-z0-9][a-z0-9._-]*$');

/// One stored provider, as `configured_providers` reports it.
///
/// [id] doubles as the display name for anything the operator added: it is a
/// free-text primary key server-side, so "nvidia-glm5.2" is a perfectly good
/// one. The built-in vendors get their label from [_AiPreset] instead — see
/// `_SettingsDialogState._providerLabel`.
typedef _AiProviderRow = ({
  String id,
  bool hasKey,
  String model,
  String baseUrl,
  String protocol,
  bool active,
});

/// Settings dialog. Currently hosts the AI provider configuration; appearance and
/// account sections will slot in alongside it.
class SettingsDialog extends StatefulWidget {
  const SettingsDialog({
    super.key,
    required this.updates,
    this.onLoadImportHistory,
    this.onLoadImportNotifications,
    this.onSaveImportNotifications,
    required this.onLoadAiSettings,
    required this.onListAiModels,
    required this.onSaveAiSettings,
    this.onDeleteAiProvider,
    this.onLoadTokens,
    this.onCreateToken,
    this.onRevokeToken,
    required this.userName,
    required this.userEmail,
    required this.onUpdateProfile,
    required this.currentAvatarUrl,
    required this.onChangeAvatar,
    required this.onRemoveAvatar,
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
    this.onLoadExportStats,
    this.onLoadCacheStats,
    this.onClearMirrorCache,
    this.onLoadQuarantinedCaches,
    this.onExportQuarantinedCache,
  });

  /// The app's update controller: the version shown in About and the
  /// desktop update checker.
  final DesktopUpdateController updates;
  final String userName;
  final String userEmail;

  /// Null in 本地模式 — there is no account to edit, so the Account tab is not
  /// offered at all. Null, not a do-nothing function: the same distinction
  /// [onLoadTokens] already draws. A no-op here rendered a whole page of live
  /// controls — a Display name you could type into, a Save button, a password
  /// change — that silently did nothing.
  final Future<void> Function(String displayName)? onUpdateProfile;

  /// The signed-in user's picture URL when Settings opens (null = none). Only
  /// the starting value: this dialog is a route, so it never sees the rebuild an
  /// upload causes upstream — from then on it follows what the actions report.
  final String? Function() currentAvatarUrl;

  /// Null in 本地模式 — no account, so no picture to change. Each returns the
  /// URL that is true afterwards.
  final Future<String?> Function()? onChangeAvatar;
  final Future<String?> Function()? onRemoveAvatar;
  final Future<void> Function(String current, String next)? onChangePassword;

  /// Null in 本地模式 — no account to delete. Deletes the cloud account and
  /// every workspace it owns (server cascade); the caller signs out on success.
  final Future<void> Function(String password)? onDeleteAccount;

  /// Null in 本地模式 — AI settings live on the server, so there is nothing to
  /// configure and the tab is absent. Null, not a no-op: same rule as
  /// [onUpdateProfile] and [onLoadTokens]. These two were the stragglers, and
  /// a no-op here meant a whole provider form — base URL, model, API key —
  /// that took your typing and dropped it.
  /// This account's past imports, from the SERVER. Null in 本地模式.
  ///
  /// Read on demand rather than kept in the dialog's state: an import that a
  /// deploy interrupted only becomes knowable when the server says so, and the
  /// answer changes while this screen is not looking.
  final Future<List<ImportHistoryEntry>> Function()? onLoadImportHistory;
  final Future<bool> Function()? onLoadImportNotifications;
  final Future<void> Function(bool)? onSaveImportNotifications;

  final Future<Map<String, dynamic>> Function()? onLoadAiSettings;

  /// Ask the provider what models it has. Null in 本地模式 and on a server too
  /// old to have the route — the model field stays free text either way, which
  /// is the same fallback a failed fetch lands on.
  final ListAiModels? onListAiModels;

  /// Writes one provider's config and makes it active; answers with that
  /// provider's stored state. Returns the payload so a switch can populate the
  /// form from the server rather than from a local guess.
  final Future<Map<String, dynamic>> Function({
    required String providerId,

    /// Every field but the id is optional, and an omitted one keeps what that
    /// provider already has. Switching provider is therefore a call carrying
    /// the id ALONE — passing the form's current values along with a switch is
    /// how one provider's endpoint used to land on another's row.
    String? provider,
    String? baseUrl,

    /// Null omits the field entirely — keep what is stored. An empty STRING
    /// would be sent, and would clear it.
    String? model,
    String? apiKey,
  })?
  onSaveAiSettings;

  /// Forget one provider's config. Null in 本地模式, and on a server too old to
  /// have the route — the list simply offers no delete there.
  final Future<Map<String, dynamic>> Function(String providerId)?
  onDeleteAiProvider;
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

  /// Null hides the import zone. Local mode has no archive importer, and an
  /// upload target that silently does nothing is worse than no target at all.
  final Future<void> Function()? onImportWorkspace;

  /// Null in 本地模式 — "export all workspaces" is a cloud endpoint
  /// (`GET /api/workspaces/export.zip`, every workspace this account belongs
  /// to). Null hides the button rather than offering a dead control.
  final Future<void> Function()? onExportAllWorkspaces;

  /// Counts for the whole-account export. Null where there is nothing to
  /// describe (本地模式 has no cross-workspace export), and a failure just leaves
  /// the meta line absent — a number nobody can act on is not worth an error.
  final Future<({int workspaces, int pages, int imageBytes})> Function()?
  onLoadExportStats;

  /// What the on-device store holds. Null on web (there is no on-device store).
  final Future<LocalCacheStats> Function()? onLoadCacheStats;

  /// Reclaim the mirrored half. Carries what was retained (un-pushed edits) and
  /// which legacy caches were quarantined, so the result can be reported
  /// honestly instead of claiming everything went.
  final Future<ClearMirrorResult> Function()? onClearMirrorCache;

  /// Legacy (pre-#111) caches, listed by id + page count only. Null when there
  /// is no on-device store.
  final Future<List<({String origin, String workspaceId, int pages})>> Function()?
  onLoadQuarantinedCaches;

  /// Export one quarantined legacy workspace as a Markdown ZIP — an explicit
  /// recovery path, with no automatic attribution to the current account.
  final Future<Uint8List?> Function(String origin, String workspaceId)?
  onExportQuarantinedCache;

  @override
  State<SettingsDialog> createState() => _SettingsDialogState();
}

/// The editor's font-size setting, in the unit people actually think in.
///
/// The control used to be a free 85%–140% slider: dragging landed on values
/// like 113%, a number nobody chooses on purpose, cannot reproduce on another
/// machine, and cannot name when reporting a bug. Every reference that offers
/// a size at all offers DISCRETE WHOLE UNITS — Obsidian and Typora take a px
/// number, browsers zoom in fixed detents; none of them hand out a continuous
/// percentage (user asked for detents/size on 2026-08-28).
///
/// Storage is untouched: the pref stays `fontScale` (px / [kFontBasePx]), so
/// every stored value keeps working and the render pipeline does not change.
/// Only the CONTROL snaps — an old 113% shows as its nearest whole px and
/// snaps the first time the slider moves.
const double kFontBasePx = 16;

/// 13px is genuinely small and 22px genuinely large at the editor's metrics;
/// the old bounds (85%–140% = 13.6–22.4px) round to the same span.
const int kFontPxMin = 13;
const int kFontPxMax = 22;

/// The scale a whole-px choice stores. Inverse of [fontPxFromScale].
double fontScaleFromPx(int px) =>
    px.clamp(kFontPxMin, kFontPxMax) / kFontBasePx;

/// The whole px a stored scale displays as. Inverse of [fontScaleFromPx] for
/// every in-range whole px; an off-detent legacy value rounds to its nearest.
int fontPxFromScale(double scale) =>
    (scale * kFontBasePx).round().clamp(kFontPxMin, kFontPxMax);

class _SettingsDialogState extends MicaState<SettingsDialog> {
  final _baseUrl = TextEditingController();

  final _model = TextEditingController();

  final _apiKey = TextEditingController();

  /// The "add a provider" form's own fields. Separate controllers, not a reuse
  /// of the three above: the detail form commits on blur, so typing a new
  /// provider's address into the field that belongs to the CURRENT one would
  /// save it there on the way out.
  final _addName = TextEditingController();

  final _addBaseUrl = TextEditingController();

  /// The AI fields commit when they lose focus — settings here apply as you
  /// touch them, and a text field's equivalent of a toggle flipping is you
  /// finishing with it. Not on every keystroke: each save is a network call,
  /// and half an API key is not a value anyone wants stored.
  late final List<FocusNode> _aiFocus = [
    for (var i = 0; i < 3; i++) FocusNode()..addListener(_saveAiOnBlur),
  ];

  /// What the server already has, so blurring an untouched field is silent.
  String _aiSaved = '';

  /// Index into the built tab list. Not persisted, so reordering the list is
  /// safe — 0 is whatever comes first (外观).
  int _tab = 0;

  // Only the AI category waits for this request. Appearance and navigation
  // remain available even when the server is slow or unreachable.
  late bool _loading = widget.onLoadAiSettings != null;

  bool _aiLoadFailed = false;

  bool _saving = false;

  bool _hasKey = false;

  /// Last 4 characters of the saved key, when there is one. The field itself
  /// can never show the key — the server does not return it — so without this
  /// the dialog had only a row of dots as a hint, which reads exactly like a
  /// filled-in field and left "is a key set?" unanswerable.
  String _keyHint = '';

  /// The vendor whose values are currently IN the fields. Not the same as
  /// `_preset` during a switch — and that gap is a bug, not a nicety: the
  /// fields save on blur, so a blur landing after the dropdown changed would
  /// write the OLD provider key under the NEW provider id. That is how one
  /// DeepSeek key ended up stored for both DeepSeek and Zhipu.
  String _fieldsProviderId = '';

  List<ImportHistoryEntry>? _importHistory;

  bool _importHistoryLoading = false;

  /// The wire PROTOCOL of the provider on screen — `openai` or `anthropic`.
  /// A stored property of each provider, not a lookup from a preset table: a
  /// self-hosted gateway can speak either, and the preset table cannot know.
  String _protocol = 'openai';

  /// Every provider the server has a config for. This list IS the left column;
  /// there is no separate notion of "available providers", because anything
  /// speaking one of the two protocols works once its URL is typed in.
  List<_AiProviderRow> _providers = const [];

  /// Which provider's form the right pane shows. Empty means the instance has
  /// none configured — the pane then offers the add form and nothing else,
  /// rather than an endpoint box for a provider that does not exist.
  String _selectedId = '';

  /// True while the right pane is the add form.
  bool _addingProvider = false;

  _AiPreset _addPreset = _AiPreset.deepseek;

  String? _addError;

  /// The write currently in flight, if any.
  ///
  /// Clicking another provider in the list first BLURS the field you were in,
  /// and the blur save starts before the tap handler runs. Two writes in flight
  /// at once can land in either order, and the losing order re-activates the
  /// provider you just left. Switching, adding and deleting therefore wait for
  /// this — which is also why [_fieldsProviderId] alone is not enough: it keeps
  /// the values on the right row, but says nothing about ordering.
  Future<void>? _aiSaveInFlight;

  /// Fetched model lists, per vendor. Switching away used to drop the list, so
  /// coming back showed a bare text box with no way to see that the stored
  /// model was one of the provider's real ones — the selection looked lost even
  /// though it was not.
  final Map<String, List<String>> _modelsByProvider = {};

  /// Whether this account may change the instance-wide AI settings. Instance
  /// settings carry the operator's provider key, so only an admin may.
  bool _canEdit = true;

  /// Models the provider itself reported, empty until fetched. Not seeded from
  /// any built-in list: a stale name that looks official is worse than no list.
  List<String> _models = const [];

  bool _fetchingModels = false;

  String? _modelsError;

  // API Tokens tab state.
  List<Map<String, dynamic>>? _tokens;

  bool _tokensLoaded = false;

  bool _tokenBusy = false;
  ({int workspaces, int pages, int imageBytes})? _exportStats;

  bool _exportStatsAsked = false;

  /// What the avatar circle shows right now. Seeded from the parent, then kept
  /// current by what each action reports back.
  String? _avatarUrl;

  LocalCacheStats? _cacheStats;

  bool _cacheBusy = false;

  /// Result of the last clear, shown in the Data section — NOT in `_accountMsg`,
  /// which only renders under Account. Reporting "freed 3.2 MB" into a tab the
  /// user is not looking at is the same as not reporting it: the numbers just
  /// changed and nothing said why.
  String? _cacheMsg;

  bool _cacheStatsAsked = false;

  /// Legacy caches (pre-#111) retained on disk but owned by an unverifiable
  /// account. Listed by id + page count only; exporting is explicit.
  List<({String origin, String workspaceId, int pages})> _quarantine = const [];
  bool _quarantineAsked = false;

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

  bool _accountMessageIsError = false;

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
    // Prefetch the export numbers rather than waiting for the Data tab to be
    // opened: it is one aggregate query, and asking on first paint of that tab
    // would render the export row without its meta line and then reflow under
    // the user.
    _avatarUrl = widget.currentAvatarUrl();
    _ensureExportStats();
    _ensureCacheStats();
  }

  @override
  void dispose() {
    _baseUrl.dispose();
    _model.dispose();
    _apiKey.dispose();
    _addName.dispose();
    _addBaseUrl.dispose();
    for (final f in _aiFocus) {
      f.dispose();
    }
    _name.dispose();
    _curPass.dispose();
    _newPass.dispose();
    _tokenName.dispose();
    super.dispose();
  }

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
        <
          ({
            String group,
            String title,
            String description,
            IconData icon,
            List<Widget> section,
          })
        >[
          (
            group: context.l10n.settingsGroupGeneral,
            title: context.l10n.settingsAppearance,
            description: context.l10n.settingsAppearanceDescription,
            icon: Icons.tune,
            section: _appearanceSection(context),
          ),
          (
            group: context.l10n.settingsGroupGeneral,
            title: context.l10n.settingsShortcuts,
            description: context.l10n.settingsShortcutsDescription,
            icon: Icons.keyboard_outlined,
            section: _shortcutsSection(context),
          ),
          // 本地模式 has no account — same reason API Tokens below is absent there.
          if (widget.onUpdateProfile != null)
            (
              group: context.l10n.settingsGroupAccount,
              title: context.l10n.settingsAccount,
              description: context.l10n.settingsAccountDescription,
              icon: Icons.person_outline,
              section: _accountSection(context),
            ),
          if (widget.onLoadTokens != null)
            (
              group: context.l10n.settingsGroupAccount,
              title: context.l10n.tokenTitle,
              description: context.l10n.settingsTokensDescription,
              icon: Icons.key_outlined,
              section: _tokensSection(context),
            ),
          (
            group: context.l10n.settingsGroupWorkspace,
            title: context.l10n.settingsData,
            description: context.l10n.settingsDataDescription,
            icon: Icons.import_export,
            section: _dataSection(context),
          ),
          // AI settings live on the server — 本地模式 has none to configure.
          if (widget.onLoadAiSettings != null)
            (
              group: context.l10n.settingsGroupOther,
              title: context.l10n.settingsAiProvider,
              description: context.l10n.settingsAiDescription,
              icon: Icons.auto_awesome,
              section: _aiSection(context),
            ),
          // Web has no filesystem to capture into, so the tab is simply absent
          // there rather than offering a switch that cannot do anything.
          if (diagnosticsSupported)
            (
              group: context.l10n.settingsGroupOther,
              title: context.l10n.settingsDiagnostics,
              description: context.l10n.settingsDiagnosticsDescription,
              icon: Icons.bug_report_outlined,
              section: _diagnosticsSection(context),
            ),
        ];
    return MicaSettingsShell(
      title: context.l10n.settingsTitle,
      selectedIndex: _tab,
      onSelected: (index) => setState(() => _tab = index),
      closeLabel: context.l10n.commonClose,
      backLabel: context.l10n.commonBack,
      onClose: () => Navigator.of(context).pop(),
      busy: _saving,
      destinations: [
        for (final tab in tabs)
          MicaSettingsDestination(
            group: tab.group,
            title: tab.title,
            description: tab.description,
            icon: tab.icon,
            content: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              mainAxisSize: MainAxisSize.min,
              children: tab.section,
            ),
          ),
      ],
      footer: ListTile(
        contentPadding: const EdgeInsets.symmetric(horizontal: 10),
        leading: const Icon(Icons.info_outline, size: 18),
        title: Text(context.l10n.aboutTitle),
        subtitle: Text('v${widget.updates.currentVersion}'),
        onTap: () => _showAboutDialog(context),
      ),
    );
  }
}
