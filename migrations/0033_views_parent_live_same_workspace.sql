-- A LIVE view's parent must be a live folder in the same workspace.
--
-- 0011 made "only folders contain" a trigger, but the trigger checked only the
-- parent's type. The API guard (`ensure_parent_accepts_children`) also checks
-- `is_deleted = false` and the workspace — once, at the START of a request.
-- An import runs for minutes after that check (2026-10-21 review, C2): trash
-- the target folder in that window and every later page still lands under it.
-- Those pages are live but invisible (their parent is in the bin), and emptying
-- the bin deletes them for real through `parent_view_id ON DELETE CASCADE`,
-- leaving their `documents` and share rows behind. Nothing else checks at the
-- moment of the write, so the trigger has to.
--
-- Two halves, in the 0011 order: repair what is stored, then enforce.

-- 1. REPAIR. A live view under a trashed parent, or under a parent in another
--    workspace, is lifted to its own workspace's top level — exactly what
--    restoring a page whose folder is still in the bin already does. Nothing is
--    deleted, nothing moves into the bin: the user never deleted these pages.
UPDATE views AS c
   SET parent_view_id = NULL, updated_at = now()
  FROM views AS p
 WHERE c.parent_view_id = p.id
   AND c.is_deleted = false
   AND (p.is_deleted OR p.workspace_id <> c.workspace_id);

-- 2. ENFORCE. Same trigger (0011's `BEFORE INSERT OR UPDATE OF parent_view_id`,
--    left as it is), stricter function:
--    - the parent must be a folder (unchanged);
--    - in the child's workspace;
--    - and not in the bin — when the child itself is live. A trashed child
--      under a trashed folder is the bin's own shape (trashing a folder
--      produces it), so it stays writable.
--    The trigger still fires only on writes of `parent_view_id`, never on
--    `is_deleted` flips: trashing and restoring are untouched, and restoring a
--    page whose folder is still in the bin keeps working because the restore
--    paths lift it to the top level (parent NULL, which the trigger's WHEN
--    skips) in the same transaction.
--
--    `FOR SHARE` on the parent row: a trash of that folder that has already
--    written the row but not yet committed makes this insert WAIT for it and
--    then see `is_deleted = true`, instead of reading the old live row and
--    slipping a child in under a folder that is about to be in the bin.
--
--    `CONSTRAINT` names the rule in the error, so the API can answer 409
--    with a readable reason (`mica_infra::ApiError`) instead of a 500 carrying
--    Postgres text.
CREATE OR REPLACE FUNCTION views_parent_must_be_folder() RETURNS trigger AS $$
DECLARE
  parent_type object_type;
  parent_workspace uuid;
  parent_deleted boolean;
BEGIN
  SELECT object_type, workspace_id, is_deleted
    INTO parent_type, parent_workspace, parent_deleted
    FROM views
   WHERE id = NEW.parent_view_id
     FOR SHARE;
  IF NOT FOUND THEN
    -- A missing parent is the foreign key's to report.
    RETURN NEW;
  END IF;
  IF parent_type <> 'folder' THEN
    RAISE EXCEPTION
      'view % cannot nest under non-folder view % (pages are leaves)',
      NEW.id, NEW.parent_view_id
      USING ERRCODE = 'check_violation', CONSTRAINT = 'views_parent_must_be_folder';
  END IF;
  IF parent_workspace <> NEW.workspace_id THEN
    RAISE EXCEPTION
      'view % cannot nest under view % of another workspace',
      NEW.id, NEW.parent_view_id
      USING ERRCODE = 'check_violation', CONSTRAINT = 'views_parent_must_be_folder';
  END IF;
  IF parent_deleted AND NOT NEW.is_deleted THEN
    RAISE EXCEPTION
      'live view % cannot nest under trashed folder %',
      NEW.id, NEW.parent_view_id
      USING ERRCODE = 'check_violation', CONSTRAINT = 'views_parent_must_be_folder';
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;
