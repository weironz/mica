-- Slow-query observability: the per-statement statistics view.
--
-- The extension only COLLECTS when the server preloads it
-- (shared_preload_libraries, set in deploy/docker-compose*.yml); creating it
-- without that is harmless — the view just refuses to be read until the next
-- restart picks the library up.
--
-- Best effort, never a migration failure: a self-hosted database whose role may
-- not create extensions, or whose build has no contrib modules, still starts.
DO $$
BEGIN
  CREATE EXTENSION IF NOT EXISTS pg_stat_statements;
EXCEPTION WHEN OTHERS THEN
  RAISE NOTICE 'pg_stat_statements not installed (%): slow-query statistics unavailable', SQLERRM;
END
$$;
