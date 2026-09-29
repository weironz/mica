-- Track every browser PUT URL until it has expired and the orphan sweep has
-- checked its object. A successful complete does not remove this row: the URL
-- may still be replayed until expiry, especially after the file row is deleted.
--
-- No workspace FK on purpose. Deleting a workspace must not erase the only
-- record of an uncompleted object that still exists under its old prefix.
CREATE TABLE pending_file_uploads (
  object_key text PRIMARY KEY,
  workspace_id uuid NOT NULL,
  expires_at timestamptz NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT pending_file_uploads_workspace_key
    CHECK (object_key LIKE 'workspaces/' || workspace_id::text || '/%')
);

CREATE INDEX pending_file_uploads_expiry ON pending_file_uploads (expires_at);
