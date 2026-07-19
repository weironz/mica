-- Pages are leaves; only folders contain. This is the page-tree's core shape —
-- a folder is the "entity used solely for organizing content" (0005) and a
-- document view is a leaf. Until now the rule lived ONLY in the Flutter client
-- (`models.dart: canNestUnder`), so every server-side write path was free to
-- build trees the rest of Mica cannot represent. One did: the Notion importer
-- deliberately mapped a directory onto the same-named `.md` page, so every
-- Notion page that had subpages imported as a page WITH children.
--
-- Two halves, in order: repair what is already stored, then make it
-- unrepresentable.

-- 1. REPAIR. For each page that has children, insert a folder in its place
--    (same name, icon, parent and position — it takes over the page's slot in
--    the tree), move the children under that folder, and put the page itself
--    back as the folder's FIRST child, carrying its content along. Nothing is
--    deleted and no view id changes, so links, shares and history keep
--    resolving; the page's body just sits one level deeper. `'!' || position`
--    sorts ahead of every uuid-shaped sibling position, matching the importer's
--    convention of listing the parent's own content before its children.
--
--    The loop is required, not decorative: fixing a page hands its slot to a
--    new folder, but if that page's OWN parent was also a page, that one is
--    still violating. Each pass strictly reduces the number of pages that have
--    children (the repaired one no longer does, and a folder is not a page), so
--    it terminates. Deleted children move too — otherwise restoring one later
--    would resurrect the violation.
DO $$
DECLARE
  page RECORD;
  slot uuid;
BEGIN
  LOOP
    SELECT v.id, v.workspace_id, v.parent_view_id, v.name, v.icon, v.position, v.created_by
      INTO page
      FROM views v
     WHERE v.object_type <> 'folder'
       AND v.is_deleted = false
       AND EXISTS (
         SELECT 1 FROM views c
          WHERE c.parent_view_id = v.id AND c.is_deleted = false
       )
     LIMIT 1;
    EXIT WHEN NOT FOUND;

    INSERT INTO views (
      workspace_id, parent_view_id, object_id, object_type,
      name, icon, position, created_by
    )
    VALUES (
      page.workspace_id, page.parent_view_id, uuid_generate_v4(), 'folder',
      page.name, page.icon, page.position, page.created_by
    )
    RETURNING id INTO slot;

    UPDATE views SET parent_view_id = slot, updated_at = now()
     WHERE parent_view_id = page.id;

    UPDATE views
       SET parent_view_id = slot, position = '!' || position, updated_at = now()
     WHERE id = page.id;
  END LOOP;
END $$;

-- 2. ENFORCE. The API rejects this with a 400 and a readable message before it
--    ever gets here (`ensure_parent_accepts_children`); this trigger is the
--    backstop that covers the paths nobody remembered — the import executor,
--    MCP, a future endpoint, a hand-written UPDATE. It fires only when
--    `parent_view_id` is actually being written, so renaming or trashing a row
--    is untouched.
CREATE FUNCTION views_parent_must_be_folder() RETURNS trigger AS $$
BEGIN
  IF (SELECT object_type FROM views WHERE id = NEW.parent_view_id) <> 'folder' THEN
    RAISE EXCEPTION
      'view % cannot nest under non-folder view % (pages are leaves)',
      NEW.id, NEW.parent_view_id
      USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER views_parent_must_be_folder
  BEFORE INSERT OR UPDATE OF parent_view_id ON views
  FOR EACH ROW
  WHEN (NEW.parent_view_id IS NOT NULL)
  EXECUTE FUNCTION views_parent_must_be_folder();
