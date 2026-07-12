-- Folder views (pure containers, no content) — the AFFiNE-style "entity used
-- solely for organizing content". A folder is a `views` row with
-- object_type='folder': it has a name/parent/children but no backing document,
-- no snapshot, no CRDT sync, and exports to a directory (never a stray .md).
--
-- Additive: existing views stay 'document'. PG16 allows ADD VALUE inside the
-- migration transaction as long as the new value isn't USED in the same txn
-- (it isn't — only added here; runtime uses it later).
ALTER TYPE object_type ADD VALUE IF NOT EXISTS 'folder';
