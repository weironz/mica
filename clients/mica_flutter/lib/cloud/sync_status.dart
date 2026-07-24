/// The sync state a cloud document surfaces to the UI — three states only.
///
/// Design (calibrated against AFFiNE/SiYuan/Logseq/Anytype + the user's spec):
/// a small icon near the workspace card (AFFiNE-style) or the doc top-right
/// (SiYuan-style). DELIBERATELY quiet — [synced] shows nothing; the icon only
/// appears when it matters ([syncing] transiently, [offline] persistently). NO
/// numeric "N pending" count anywhere: no comparable product surfaces one, so
/// the input is a plain boolean, not a number. Manual "sync on click" is a
/// deferred phase-2 nicety, not modelled here.
///
/// Pure enum + pure derivation — fully unit-testable with no widget or sync
/// engine; the signal plumbing and the icon live elsewhere.
enum SyncPhase {
  /// Online and nothing left to send. The UI renders NOTHING.
  synced,

  /// Online with edits still draining to the server — a transient, unobtrusive
  /// "syncing" affordance (e.g. a slow spin).
  syncing,

  /// Not connected. Edits keep persisting locally; surfaced so the user does
  /// not assume "what I see == saved to the cloud" before switching devices.
  offline,
}

/// Derive the phase from the two raw signals the sync engine already has: is the
/// socket live ([online]), and are there ANY unsynced edits ([pending], a
/// boolean — never a count). Offline dominates: a live socket is what
/// synced/syncing presuppose, so a down link is always [offline] regardless of
/// pending work. Online + pending == draining ([syncing]); online + nothing ==
/// fully caught up ([synced], and the icon disappears).
SyncPhase deriveSyncPhase({required bool online, required bool pending}) {
  if (!online) return SyncPhase.offline;
  return pending ? SyncPhase.syncing : SyncPhase.synced;
}
