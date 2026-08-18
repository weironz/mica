-- AI provider settings, and the admin flag that decides who may change them.
--
-- Both halves fix something that is broken today, not a hypothetical.
--
-- 1. THE SETTINGS WERE NEVER PERSISTED. `state.ai` is an in-process
--    `RwLock<Option<AiConfig>>` seeded from the environment at boot, and
--    `PATCH /api/ai/settings` only ever wrote to that lock. So a key configured
--    in the UI survived exactly until the next api restart — i.e. every deploy
--    silently un-configured AI, and the settings dialog could not honestly say
--    whether a key was set because the answer changed under it.
--
-- 2. ANY SIGNED-IN USER COULD CHANGE THEM. The config is instance-wide and
--    holds the OPERATOR's provider key; `update_settings` only required a
--    session. On a single-user instance that is invisible, and it stops being
--    invisible the day registration is opened: another account could repoint the
--    model, or spend the operator's credit. `is_admin` is that boundary.
--
-- `is_admin` rather than recomputing "the first account": auth.rs already asks
-- "is this an empty instance?" when auto-verifying the first signup, but that
-- question is answered fresh each time and its meaning drifts the moment a
-- second account exists (deleting the first account would silently promote
-- nobody, and there would be no way to grant a second admin or hand the role
-- over). A column answers all three, and today exactly one row carries it.

ALTER TABLE users ADD COLUMN is_admin boolean NOT NULL DEFAULT false;

-- Backfill: the oldest account is the one that stood the instance up. On an
-- empty instance this touches nothing and the first signup gets the flag
-- instead (routes/auth.rs).
UPDATE users
SET is_admin = true
WHERE id = (SELECT id FROM users ORDER BY created_at, id LIMIT 1);

-- One row, instance-wide — the same scope `state.ai` already had. Pinned to a
-- single row by a constant primary key so "the settings" cannot become
-- ambiguous, and so an update is one statement with no upsert-by-search.
--
-- The key lives HERE rather than in `server_secrets` even though it is a
-- secret: it has to change atomically with the provider and base URL it belongs
-- to. A key written for one provider and read alongside another provider's
-- endpoint is precisely how a key gets sent to the wrong host.
--
-- Not stored: per-user or per-workspace overrides, usage counters, quotas. A
-- paid deployment meters usage per workspace over time, which is a different
-- table with a different shape (one row per period, not one row per instance);
-- reserving columns here for it would be guessing at that shape.
CREATE TABLE ai_settings (
  id         boolean PRIMARY KEY DEFAULT true CHECK (id),
  provider   text NOT NULL,
  base_url   text NOT NULL,
  model      text NOT NULL,
  api_key    text NOT NULL DEFAULT '',
  max_tokens integer NOT NULL DEFAULT 2048,
  updated_at timestamptz NOT NULL DEFAULT now(),
  updated_by uuid REFERENCES users(id) ON DELETE SET NULL
);
