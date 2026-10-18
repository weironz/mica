-- The workspace owner's membership row must keep role 'owner'.
--
-- Every owner check (rename, delete, manage members, write) reads
-- `workspace_members.role`, not `workspaces.owner_id`. `update_member` and
-- `remove_member` refused to touch the owner, but `add_member` is an upsert
-- (`ON CONFLICT … SET role = EXCLUDED.role`) and did not: an admin "re-adding"
-- the owner's email as viewer demoted the owner, who then could not undo it.
-- The API now refuses that with a 403; this trigger is the backstop for the
-- paths nobody remembered — the same reasoning as 0011.
--
-- DELETE is deliberately not guarded: deleting a workspace or an account
-- cascades through this table, and those are legitimate.
CREATE FUNCTION workspace_owner_keeps_owner_role() RETURNS trigger AS $$
BEGIN
  IF EXISTS (
    SELECT 1 FROM workspaces
    WHERE id = NEW.workspace_id AND owner_id = NEW.user_id
  ) THEN
    RAISE EXCEPTION
      'the owner of workspace % must keep the owner role', NEW.workspace_id
      USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER workspace_owner_keeps_owner_role
  BEFORE INSERT OR UPDATE OF role ON workspace_members
  FOR EACH ROW
  WHEN (NEW.role <> 'owner')
  EXECUTE FUNCTION workspace_owner_keeps_owner_role();
