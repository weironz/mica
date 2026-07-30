-- S5: retire the op model's storage. The yrs base becomes the only place a
-- document's content lives.
--
-- S4 stopped writing all three of these tables; this removes them. What is being
-- destroyed, measured on production before the drop:
--
--   * document_snapshots — 3973 rows, 22 MB. A full jsonb copy of each document,
--     frozen at its pre-yrs seed. Redundant: the S2 backfill folded every one of
--     them into `document_yrs_base` (3735 bases for 3735 documents, missing_base
--     = 0), and that base is what every read has returned since.
--   * document_updates — 238 rows. The op-model change log. Zero readers; the
--     live stream is `workspace_updates` (note that `sync::pull_document_updates`
--     reads THAT table, not this one — same name, different thing).
--   * document_versions — 3 rows. Op-era named checkpoints, superseded by
--     `document_yrs_versions` (migration 0009). No code has read this table
--     since, so those 3 names were already invisible in the product.
--
-- ORDER MATTERS, and not for the usual cascade reason: `document_versions.
-- snapshot_id` references `document_snapshots(id)` ON DELETE RESTRICT, so
-- dropping the snapshots first fails on the dependency. Dropping that table
-- first removes it. Deliberately not `DROP ... CASCADE`: cascade would also
-- silently drop anything else that happens to reference these, and "silently" is
-- the wrong property for the most destructive migration in this schema.
--
-- Indexes go with their tables (idx_document_updates_document_seq,
-- idx_document_snapshots_document_seq, idx_document_versions_document_created).
DROP TABLE document_versions;
DROP TABLE document_updates;
DROP TABLE document_snapshots;
