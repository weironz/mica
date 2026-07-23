-- Password reset tokens: the self-service "forgot my password" path for a
-- public deployment. A user proves they control the account's *email* (they
-- receive the emailed link) instead of proving they know the password.
--
-- Like refresh tokens (0006) and PATs (0004), the plaintext token lives only in
-- the emailed link and only its SHA-256 hash is stored here — a database dump is
-- not a pile of live reset links. Single-use (`used_at`) and short-lived
-- (`expires_at`, ~1h): a reset link that lingers is a standing key to the
-- account. Requesting a new reset deletes the account's older unused rows, so
-- only the most recent link works.
CREATE TABLE password_reset_tokens (
  token_hash text PRIMARY KEY,
  user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_at timestamptz NOT NULL DEFAULT now(),
  expires_at timestamptz NOT NULL,
  -- Set the moment the link is spent. Non-null = already used; the reset page
  -- refuses it, so a forwarded/leaked link cannot be replayed.
  used_at    timestamptz
);

-- Deleting a user cascades their reset tokens; the "invalidate my older links on
-- a new request" delete filters by user_id — both want this cheap.
CREATE INDEX password_reset_tokens_user_id_idx ON password_reset_tokens (user_id);
