-- Page version history, yrs-native (the op-model document_versions table froze
-- with P4①b — see docs/version-history-plan.md). One row = one full-state yrs
-- snapshot of a document at a point in time. Auto rows (label IS NULL) are
-- captured on a cadence by sync::push_update and expire by `expires_at`; named
-- rows (label set) are user checkpoints and never expire (expires_at NULL).
--
-- Full state, not a delta: each row is independently restorable and cheap to
-- prune. Mirrors AFFiNE's snapshotHistory (Mica's base+append-log storage is
-- byte-for-byte isomorphic to AFFiNE's).
CREATE TABLE document_yrs_versions (
  id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  document_id uuid NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
  rid         bigint,                         -- workspace_updates rid at capture (ordering hint)
  label       text,                           -- NULL = auto snapshot; set = named version
  created_by  uuid,                           -- user id; NULL for system/auto
  created_at  timestamptz NOT NULL DEFAULT now(),
  expires_at  timestamptz,                    -- auto rows expire; named rows are NULL (kept forever)
  state       bytea NOT NULL                  -- yrs encode_state_as_update_v1 (full state)
);

-- Timeline reads (newest first) and the per-doc cadence/retention checks.
CREATE INDEX document_yrs_versions_doc_created_idx
  ON document_yrs_versions (document_id, created_at DESC);
