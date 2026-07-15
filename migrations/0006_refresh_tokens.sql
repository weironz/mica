-- Refresh tokens: keep a signed-in client signed in without a 24h re-login.
--
-- Access tokens are short-lived JWTs and carry no server state, so they cannot
-- be revoked; that is exactly why they stay short-lived. The refresh token is
-- the opposite — opaque, stateful, revocable — and it is the only thing that
-- can mint new access tokens. Like a PAT (0004) the plaintext is returned once
-- and only its SHA-256 hash is stored here.
--
-- `family_id` is one sign-in. Rotation issues a new row in the same family and
-- stamps `used_at` on the old one, so a refresh token is single-use. A request
-- presenting an ALREADY-USED token is a replay: either the client raced itself,
-- or the token leaked and someone else is spending it. We cannot tell which, so
-- the whole family is revoked and the user signs in again — the standard OAuth 2
-- reuse-detection trade (RFC 6819 §5.2.2.3).
CREATE TABLE refresh_tokens (
  id         uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
  user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  token_hash text NOT NULL UNIQUE,
  family_id  uuid NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  expires_at timestamptz NOT NULL,
  -- Set when this token is spent on a rotation. Non-null + presented again =
  -- replay.
  used_at    timestamptz,
  revoked_at timestamptz
);

CREATE INDEX refresh_tokens_user_id_idx ON refresh_tokens (user_id);
-- Reuse detection revokes by family, so that lookup has to be cheap.
CREATE INDEX refresh_tokens_family_id_idx ON refresh_tokens (family_id);
