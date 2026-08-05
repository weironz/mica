-- Secrets the SERVER owns, so an operator does not have to invent them.
--
-- First tenant: `jwt_secret`. It used to be mandatory in `.env.prod`, and the
-- template once shipped a working `change-me` — an operator who copied the
-- template and missed one line got a public instance whose token signing key was
-- a constant published in a public repository. The fix at the time was to REFUSE
-- to start on a weak value, which closed the hole but handed every self-hoster a
-- chore with no decision in it: generate a random string, paste it, never look
-- at it again.
--
-- The value belongs in the DATABASE, not a file, for two reasons this deployment
-- makes concrete:
--   * the api container has no volume (deploy/docker-compose.single.yml), so a
--     file would be lost on every container recreate — i.e. every upgrade would
--     silently log everyone out;
--   * two api replicas each generating their own file would sign tokens the
--     other rejects. One row is shared by definition.
--
-- Keyed by name so later server-owned secrets land here instead of growing
-- another table. `value` is text because every consumer so far wants a string.
CREATE TABLE server_secrets (
  name       text PRIMARY KEY,
  value      text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);
