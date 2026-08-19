-- Import jobs, so a restart stops erasing the record of one.
--
-- `state.import_jobs` is an in-process `RwLock<HashMap>`. Restarting the api
-- kills the running task AND forgets it ever existed, while the pages it had
-- already written stay: the workspace holds half an archive and nothing on
-- screen says so. The client polls a job id that now 404s, so the UI cannot
-- even report the failure — it just stops knowing.
--
-- Storing the job fixes three things the memory map could not: the record
-- survives a deploy, reopening the app can pick the outcome back up, and there
-- is somewhere for an import HISTORY to live (Notion, Outline and Slack all put
-- long imports under Settings → Import, because progress has to live somewhere
-- the user can find AGAIN).
--
-- What it deliberately does NOT do: resume. The work lived in a `tokio::spawn`
-- that died with the process, and these rows are a record, not a resumable
-- execution state. Resuming would need the archive bytes kept somewhere too,
-- and a half-written import that silently restarts is a worse surprise than one
-- that says it stopped.
CREATE TABLE import_jobs (
  id            uuid PRIMARY KEY,
  user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  -- Null while the import is still deciding (a new workspace is created inside
  -- the run), and cleared if that workspace is later deleted — the job record
  -- outlives it, which is the point of having a history.
  workspace_id  uuid REFERENCES workspaces(id) ON DELETE SET NULL,
  -- running | done | error | cancelled | interrupted.
  --
  -- `interrupted` is new here and exists because a restart needs a truthful
  -- answer. Leaving those rows `running` would have the UI show a progress bar
  -- for a task with nothing behind it; calling them `error` would be wrong in
  -- the way that matters, since the pages already imported are real and staying.
  status        text NOT NULL,
  total         integer NOT NULL DEFAULT 0,
  done          integer NOT NULL DEFAULT 0,
  error         text,
  -- Archive entries nothing referenced. Capped by the writer, so this stays a
  -- readable list rather than a ten-thousand-entry array.
  skipped       jsonb NOT NULL DEFAULT '[]'::jsonb,
  skipped_total integer NOT NULL DEFAULT 0,
  created_at    timestamptz NOT NULL DEFAULT now(),
  updated_at    timestamptz NOT NULL DEFAULT now()
);

-- The history query: this user's imports, newest first.
CREATE INDEX import_jobs_user_recent ON import_jobs (user_id, created_at DESC);
