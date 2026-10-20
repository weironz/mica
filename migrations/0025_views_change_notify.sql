-- Live sidebar trees: every change to a workspace's view rows emits a NOTIFY,
-- which the api-server's listener forwards to clients watching that workspace
-- over `/ws/workspaces/{id}/views`. Before this the tree only moved when the
-- user pressed refresh — a page created over MCP sat invisible until then.
--
-- A trigger, not a `notify_views_changed()` call in every mutating route,
-- because the mutation surface is wide (create/rename/move/reorder/trash/
-- restore/purge/batch-*/transfer/import — a dozen sites and counting) and one
-- forgotten site is a silently stale tree. The docs call this out as the
-- pattern to avoid (`docs/lessons.md` #2: an invariant enforced in N places is
-- enforced in N-1 of them). Every writer goes through this table, including
-- whatever route gets added next year.
--
-- Row-level on purpose, and NOT a burst risk: Postgres deduplicates identical
-- (channel, payload) notifications within one transaction, so a 300-row
-- batch-trash in one transaction delivers ONE notification, and a
-- cross-workspace transfer delivers exactly two (one per workspace payload).
--
-- The payload is only the workspace id — a "something changed" bell, not a
-- delta. The client refetches through the existing ETag-guarded listing, which
-- answers 304 when its tree is already current; shipping row deltas would be a
-- second representation of the tree to keep honest.
CREATE OR REPLACE FUNCTION views_change_notify() RETURNS trigger AS $$
BEGIN
  PERFORM pg_notify(
    'mica_views_changed',
    (COALESCE(NEW.workspace_id, OLD.workspace_id))::text
  );
  RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER views_change_notify
AFTER INSERT OR UPDATE OR DELETE ON views
FOR EACH ROW EXECUTE FUNCTION views_change_notify();
