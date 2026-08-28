// Which client settings travel with the ACCOUNT, and how the cloud blob and
// the local prefs exchange values. Pure — the app shell wires it to the API
// and to `loadPref`/`savePref`; tests drive it directly (the shell itself is
// not constructible in a test).
//
// Reported 2026-08-28: 「我在桌面端设置了启用页面标题,web端依然是关着的」——
// every preference lived only in the device's local store. Peers split the
// same way this list does: Notion syncs appearance and language per account;
// AFFiNE syncs its editor settings (font, layout, defaults) through a per-user
// cloud KV and leaves window/menu state local; AppFlowy syncs nothing, which
// is the behavior that just got reported here as a bug.
library;

/// The prefs that sync. Everything absent is device-local on purpose:
/// servers/tokens/origins (identity, per install), window and sidebar state
/// (this display's business), pending uploads (this disk's business),
/// diagnostics (a debugging toggle).
///
/// `themeMode` syncs, siding with Notion over AFFiNE: 「跟随系统」 is the
/// default and still resolves per device, so what syncs is only an explicit
/// light/dark choice — the kind of choice a person means account-wide.
const List<String> kSyncedPrefKeys = [
  'fontScale',
  'fontFamily',
  'pageWidth',
  'reHostImages',
  'showFormatBar',
  'showPageTitle',
  'aiEnabled',
  'themeMode',
  'uiLanguage',
];

/// The synced subset of a cloud settings blob, ready to write into local
/// prefs. Unknown keys are dropped (an older client must not have a newer
/// client's vocabulary forced into its prefs file), and so are non-string
/// values (the blob is client-written, but the server stores it opaquely — a
/// hand-edited or corrupt value must degrade to "not set", not crash apply).
Map<String, String> cloudSettingsToApply(Map<String, dynamic> cloud) => {
  for (final key in kSyncedPrefKeys)
    if (cloud[key] is String) key: cloud[key] as String,
};

/// The payload to push: the current local values of every synced key. Keys the
/// device never set are omitted rather than sent as empty — an absent key lets
/// another device's value survive, an empty one would overwrite it.
Map<String, String> settingsPayload(String? Function(String key) load) => {
  for (final key in kSyncedPrefKeys)
    if (load(key) != null) key: load(key)!,
};
