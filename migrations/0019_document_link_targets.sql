-- Backlinks: a queryable list of the pages each document links TO, so the
-- backlinks panel is one indexed query instead of decoding every document's CRDT
-- base per panel open.
--
-- Measured 2026-08-05 against a restored production snapshot (798 documents /
-- 41 MB of state): the on-demand `scan_backlinks` took ~690 ms in release — one
-- DB round-trip plus one full yrs decode PER DOCUMENT, with no early stop
-- (a panel must be complete). Full-text search on the same database is 53 ms.
-- Worse, the panel loads on every page open, so that cost was paid passively.
--
-- Same shape as `content_text` (migration 0012), for the same reason:
-- `link_targets` is a PURE DERIVATION of `state`, written in the SAME statement
-- by every path that writes `document_yrs_base`, from the same folded doc whose
-- `encode_state()` becomes `state`. Not a second source of truth.
--
-- NULL, not '{}', is the "never derived" sentinel. `content_text` could use ''
-- because an empty body is rare; an empty link list is the COMMON case, so '{}'
-- as the sentinel would make the startup backfill re-decode almost every
-- document on every boot. With NULL the backfill converges permanently.
ALTER TABLE document_yrs_base ADD COLUMN link_targets uuid[];

-- `@>` containment is what the panel asks (does this document link to X?), and
-- GIN is the index type that answers it. Rows still awaiting backfill are NULL
-- and simply do not match — the same "not yet indexed" window content_text has,
-- and the backfill runs before the server accepts traffic.
CREATE INDEX document_yrs_base_link_targets_idx
  ON document_yrs_base USING gin (link_targets);
