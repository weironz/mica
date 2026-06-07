-- P2-M4: yrs CRDT sync layer, added ALONGSIDE the op model (parallel, not a
-- cutover — the op path keeps working until the client fully moves over).
--
-- `workspace_updates` is a per-workspace monotonic update stream: `rid` is a
-- global bigserial (cheap, gives total order); a client keeps one per-workspace
-- high-water mark and catches up with `WHERE workspace_id = ? AND rid > ?`.
--
-- `document_yrs_base` is the folded yrs state per document (refreshed as updates
-- land) — used to validate incoming updates and, later, to fast-bootstrap a
-- fresh client without replaying the whole stream. `base_rid` is the highest
-- stream rid already folded into `state`.

CREATE TABLE workspace_updates (
    rid          bigserial PRIMARY KEY,
    workspace_id uuid NOT NULL REFERENCES workspaces ON DELETE CASCADE,
    document_id  uuid NOT NULL REFERENCES documents ON DELETE CASCADE,
    actor_id     uuid NOT NULL REFERENCES users,
    payload      bytea NOT NULL,
    created_at   timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX idx_workspace_updates_ws_rid ON workspace_updates(workspace_id, rid);
CREATE INDEX idx_workspace_updates_doc_rid ON workspace_updates(document_id, rid);

CREATE TABLE document_yrs_base (
    document_id  uuid PRIMARY KEY REFERENCES documents ON DELETE CASCADE,
    state        bytea NOT NULL,
    state_vector bytea NOT NULL,
    base_rid     bigint NOT NULL DEFAULT 0,
    updated_at   timestamptz NOT NULL DEFAULT now()
);
