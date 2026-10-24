-- RIDs are global, so gaps between a document's updates do not prove pruning.
-- Record the highest RID actually removed from each document's stream instead.
ALTER TABLE document_yrs_base
  ADD COLUMN stream_pruned_rid bigint NOT NULL DEFAULT 0;

-- Old deployments did not record deletions. Preserve their conservative
-- bootstrap boundary once; subsequent deletions have an exact watermark.
UPDATE document_yrs_base AS b
SET stream_pruned_rid = COALESCE(
  (SELECT MIN(u.rid) - 1 FROM workspace_updates AS u
   WHERE u.document_id = b.document_id), b.base_rid)
WHERE b.base_rid > 0;

CREATE FUNCTION record_document_stream_pruning() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  UPDATE document_yrs_base AS b
  SET stream_pruned_rid = GREATEST(b.stream_pruned_rid, removed.max_rid)
  FROM (
    SELECT document_id, MAX(rid) AS max_rid
    FROM deleted_updates GROUP BY document_id
  ) AS removed
  WHERE b.document_id = removed.document_id
    -- Cascading document/workspace deletion needs no surviving watermark.
    AND EXISTS (
      SELECT 1 FROM documents AS d JOIN workspaces AS w ON w.id = d.workspace_id
      WHERE d.id = b.document_id
    );
  RETURN NULL;
END;
$$;

-- Statement-level aggregation also covers manual pruning and older API
-- processes during a rolling deployment, without one base write per row.
CREATE TRIGGER document_stream_pruned
AFTER DELETE ON workspace_updates
REFERENCING OLD TABLE AS deleted_updates
FOR EACH STATEMENT EXECUTE FUNCTION record_document_stream_pruning();
