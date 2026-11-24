-- Usage Statistics & Observability (issue #191).
--
-- 1. Additive nullable word/character count projections on `document_yrs_base`,
--    co-written in the same statement as `state` / `content_text` / `link_targets`
--    so stats never decode CRDT blobs on read and never drift from the base.
--    Existing rows start NULL and are filled by `sync::backfill_derived_columns`.
ALTER TABLE document_yrs_base
  ADD COLUMN word_count INTEGER,
  ADD COLUMN char_count INTEGER;

-- 2. UTC hourly per-document writing activity buckets.
--    Keyed by (user_id, workspace_id, bucket_hour, document_id, source) so:
--    - High-frequency debounced edits collapse into one UPSERT per doc per hour.
--    - Reading into any user timezone folds UTC hour buckets into local calendar
--      days with exact midnight boundaries and deduplicated `pages_edited`.
--    - `document_id` intentionally has NO foreign-key CASCADE so deleting a page
--      never erases historical writing activity.
CREATE TABLE writing_activity_doc_hourly (
  user_id       UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  workspace_id  UUID        NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  bucket_hour   TIMESTAMPTZ NOT NULL,
  document_id   UUID        NOT NULL,
  source        TEXT        NOT NULL DEFAULT 'manual'
                CHECK (source IN ('manual', 'import', 'transfer')),
  is_created    BOOLEAN     NOT NULL DEFAULT false,
  edit_events   INTEGER     NOT NULL DEFAULT 0,
  words_added   INTEGER     NOT NULL DEFAULT 0,
  words_deleted INTEGER     NOT NULL DEFAULT 0,
  PRIMARY KEY (user_id, workspace_id, bucket_hour, document_id, source)
);

CREATE INDEX idx_writing_activity_user_hour
  ON writing_activity_doc_hourly (user_id, bucket_hour DESC);

CREATE INDEX idx_writing_activity_ws_hour
  ON writing_activity_doc_hourly (workspace_id, bucket_hour DESC);
