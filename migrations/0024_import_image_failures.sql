-- External images an import promised to bring in and left as links.
--
-- Mirrors the `skipped` / `skipped_total` pair added in 0023, for the same
-- reason and with the same shape: a capped list so a polling endpoint stays
-- small, plus an honest total so a truncated list never understates the damage.
--
-- Why this is worth a column rather than a log line: a re-host that failed was
-- reported nowhere the user could see. The import said "done", the blocks kept
-- pointing at somebody else's server, and deleting that source — the reasonable
-- next step after exporting from it — took the images with it.
--
-- Defaults, not NULL: every existing row is an import whose failures were never
-- recorded. `[]` and `0` say "nothing recorded" in the shape the reader already
-- handles, where NULL would make every consumer special-case it.
ALTER TABLE import_jobs
  ADD COLUMN image_failures       jsonb   NOT NULL DEFAULT '[]'::jsonb,
  ADD COLUMN image_failures_total integer NOT NULL DEFAULT 0;
