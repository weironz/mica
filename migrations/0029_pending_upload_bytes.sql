-- The byte size a browser upload URL was issued for, so the workspace quota
-- can count it before `complete` ever registers a `files` row.
--
-- The quota summed `files` only. An editor could presign many distinct hashes
-- (each one passing — "used" never grew), PUT them all, and never call
-- complete: the object store filled without bound while the quota read empty.
-- The presigned PUT signs content-length, so the declared size is exactly
-- what can land. 0 for server-side PUTs, which check the quota themselves.
ALTER TABLE pending_file_uploads ADD COLUMN byte_size bigint NOT NULL DEFAULT 0;
