-- Development seed: a demo account + workspace for a fresh local database.
--
-- FOR LOCAL DEVELOPMENT ONLY. The password below is public and its hash is
-- committed on purpose — never load this against a shared or production
-- database.
--
--   demo@mica.dev / password123
--
-- Usage (after `just dev` — it starts the API, so `sqlx::migrate!` has created
-- the tables):
--
--   just seed-dev
--   -- or, directly:
--   docker exec -i mica-postgres psql -U mica -d mica < seeds/dev_seed.sql
--
-- Idempotent: re-running changes nothing, so it is safe to re-apply after every
-- `docker compose down -v`.

BEGIN;

-- Fixed UUIDs so a re-seeded database keeps the same ids — handy when a saved
-- API call or a bug report references them.
--   user      11111111-1111-4111-8111-111111111111
--   workspace 22222222-2222-4222-8222-222222222222

-- argon2id hash of 'password123', produced by the server's own `hash_password`
-- (crates/api-server/src/routes/auth.rs) so it verifies through the normal
-- login path rather than needing a special case.
INSERT INTO users (id, email, display_name, password_hash)
VALUES (
  '11111111-1111-4111-8111-111111111111',
  'demo@mica.dev',
  'Demo User',
  '$argon2id$v=19$m=19456,t=2,p=1$m1Eq47TM45lBHJX5SJH5SA$wp0OqiBOp6mVUxWsvl64RvVl8scVXH+j0pSX7Lpnkqk'
)
ON CONFLICT (email) DO NOTHING;

-- Own the workspace from whichever row currently holds that email, so the seed
-- still works when the demo user was registered through the API first and thus
-- has a different id.
INSERT INTO workspaces (id, name, owner_id)
SELECT '22222222-2222-4222-8222-222222222222', 'demo', u.id
FROM users u
WHERE u.email = 'demo@mica.dev'
ON CONFLICT (id) DO NOTHING;

-- Membership is what the permission checks actually read; without this row the
-- workspace is invisible even to its owner.
INSERT INTO workspace_members (workspace_id, user_id, role)
SELECT w.id, u.id, 'owner'
FROM workspaces w
JOIN users u ON u.email = 'demo@mica.dev'
WHERE w.id = '22222222-2222-4222-8222-222222222222'
ON CONFLICT (workspace_id, user_id) DO NOTHING;

COMMIT;

-- The workspace deliberately starts with no pages. The page-tree invariant
-- (folder is the only container, page is a leaf) is enforced server-side, so
-- pages should be created through the app or the API — not by inserting `views`
-- rows here, which is exactly how the Notion import once produced 137 pages
-- nested under pages.
