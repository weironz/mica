-- Issue #47: when a view went to the recycle bin. Every row trashed by ONE
-- action shares one stamp (the statement's now()), which is what lets a restore
-- bring back exactly what was deleted with the page — and not a child the user
-- had already trashed on its own, earlier. NULL for rows trashed before this
-- column existed: a restore treats NULLs as one batch, i.e. as it always did.
ALTER TABLE views ADD COLUMN deleted_at timestamptz;
