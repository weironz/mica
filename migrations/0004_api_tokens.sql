-- Personal Access Tokens: long-lived, revocable, scoped API credentials.
-- The plaintext token (`mica_pat_<random>`) is shown once at creation; only its
-- SHA-256 hash is stored here. Scopes gate access (currently `read` / `write`,
-- write implies read). `expires_at` NULL = never expires.
CREATE TABLE api_tokens (
  id           uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
  user_id      uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name         text NOT NULL,
  token_hash   text NOT NULL UNIQUE,
  scopes       text[] NOT NULL DEFAULT '{}',
  created_at   timestamptz NOT NULL DEFAULT now(),
  last_used_at timestamptz,
  expires_at   timestamptz
);

CREATE INDEX api_tokens_user_id_idx ON api_tokens (user_id);
