-- Per-user workspace ordering. A member row carries a lexical `position` so each
-- user can drag-reorder their OWN workspace switcher — order is per-user, not
-- shared (two members of the same workspaces can order them differently). Views
-- already order this way within a workspace; this brings the same to the
-- top-level workspace list.
ALTER TABLE workspace_members ADD COLUMN position text NOT NULL DEFAULT '';

-- Backfill: number each user's existing memberships by join time, zero-padded
-- `n*10` (same scheme the local store uses), so lexical order == intended order.
UPDATE workspace_members wm
SET position = lpad((ordered.rn * 10)::text, 10, '0')
FROM (
  SELECT
    workspace_id,
    user_id,
    row_number() OVER (
      PARTITION BY user_id ORDER BY joined_at ASC, workspace_id ASC
    ) AS rn
  FROM workspace_members
) ordered
WHERE wm.workspace_id = ordered.workspace_id
  AND wm.user_id = ordered.user_id;
