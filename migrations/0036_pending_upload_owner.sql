-- Keep pending-upload ownership after its workspace has been deleted.
-- No FK: both the workspace and user may disappear before orphan collection.
ALTER TABLE pending_file_uploads ADD COLUMN owner_id uuid;

UPDATE pending_file_uploads p
SET owner_id = w.owner_id
FROM workspaces w
WHERE w.id = p.workspace_id;

CREATE INDEX pending_file_uploads_owner ON pending_file_uploads (owner_id);

-- Attribute every producer, including deletion transactions and older servers
-- during rollout. Existing orphan rows whose workspace is already gone cannot
-- recover their historical owner; they remain tracked for normal orphan GC.
CREATE FUNCTION attribute_pending_upload_owner() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.owner_id IS NULL THEN
    SELECT owner_id INTO NEW.owner_id FROM workspaces WHERE id = NEW.workspace_id;
  END IF;
  RETURN NEW;
END;
$$;

CREATE TRIGGER pending_upload_owner
BEFORE INSERT OR UPDATE ON pending_file_uploads
FOR EACH ROW EXECUTE FUNCTION attribute_pending_upload_owner();
