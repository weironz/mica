-- Email verification: an account proves it controls its address before it can be
-- signed in to. The point is anti-abuse — without it, "unlimited accounts" needs
-- nothing but a well-formed string, and unlimited accounts is the multiplier that
-- makes every per-workspace limit meaningless.
--
-- THE BACKFILL IS THE LOAD-BEARING LINE HERE. Every account that already exists
-- is grandfathered as verified. Without it, adding a login gate would lock out
-- every real user on the instance — including the operator, who would then have
-- no way in to fix it. There is also nothing to verify: those addresses have been
-- in use for months. `created_at`, not `now()`, so the column says when the
-- account was trusted rather than when this migration happened to run.
ALTER TABLE users ADD COLUMN email_verified_at timestamptz;
UPDATE users SET email_verified_at = created_at;

-- Shaped exactly like password_reset_tokens (0013), for the same reasons: the
-- plaintext token lives only in the emailed link and only its SHA-256 hash is
-- stored, so a database dump is not a pile of live account keys. Single-use
-- (`used_at`) and expiring (`expires_at`), because a verification link that
-- lingers is a standing key to an unclaimed account.
--
-- Longer-lived than a reset link (24h vs ~1h): a reset answers something the user
-- is doing right now, while a verification mail can sit in an inbox until
-- evening. Still bounded — an address that never confirms should not leave a
-- usable link behind forever.
CREATE TABLE email_verification_tokens (
  token_hash text PRIMARY KEY,
  user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_at timestamptz NOT NULL DEFAULT now(),
  expires_at timestamptz NOT NULL,
  used_at    timestamptz
);

-- Deleting a user cascades their tokens; "invalidate my older links when I ask
-- for a new one" filters by user_id. Both want this cheap.
CREATE INDEX email_verification_tokens_user_id_idx
  ON email_verification_tokens (user_id);
