-- Blob GC: when did this file stop being referenced by any live document?
--
-- NULL = referenced right now (or never yet judged). Non-NULL = the sweep found
-- no document pointing at it, at that instant. The column is recomputed from
-- scratch on every sweep, so a wrong value heals itself on the next run — the
-- reason this is a projection and not a refcount, whose drift would be
-- permanent and invisible.
--
-- Deletion is gated on `now() - unreferenced_since`, NOT on the file's age.
-- AFFiNE gates on object age, which reads as a generous 30-day margin but
-- actually gives ~1 day between "removed from the page" and "gone forever" —
-- the age of an old image says nothing about when it stopped being used.
ALTER TABLE files ADD COLUMN unreferenced_since timestamptz;

-- The sweep pages through unreferenced rows; nulls are the overwhelming
-- majority and are never scanned.
CREATE INDEX idx_files_unreferenced_since ON files (unreferenced_since)
  WHERE unreferenced_since IS NOT NULL;
