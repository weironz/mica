-- Per-user client settings, synced across devices (2026-08-28).
--
-- The appearance toggles (show page title, format bar, font, page width…)
-- lived only in each client's local storage — desktop prefs.json, web
-- localStorage — so the same account saw different behavior per device, and
-- that read as a bug ("我在桌面端设置了启用页面标题,web端依然是关着的").
--
-- One JSONB blob, not columns: which toggles exist is the CLIENT's vocabulary
-- and changes with client releases; the server only stores, timestamps, and
-- hands it back. Last write wins — settings are low-frequency and single-user,
-- so merge machinery would be cost without a conflict to resolve.
--
-- Local (offline) mode is untouched: it has no session, its prefs stay local.
CREATE TABLE user_settings (
  user_id uuid PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  settings jsonb NOT NULL DEFAULT '{}'::jsonb,
  updated_at timestamptz NOT NULL DEFAULT now()
);
