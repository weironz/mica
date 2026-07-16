-- Publish-to-web: a page becomes readable at a public /s/{token} URL.
--
-- Modeled on Outline's `shares` (the most legible reference): an unguessable
-- token is the capability (NOT the raw document id — that is where AFFiNE and
-- AppFlowy are weakest, relying on the permission layer alone). Revocation is a
-- soft timestamp, not a delete, so a link can be turned off (and later a fresh
-- one issued) with an audit trail, and every public read re-checks
-- `revoked_at IS NULL` — there is no "this token is good" cache, so revoke is
-- instant.
CREATE TABLE document_shares (
  id             uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
  -- The public capability. ~128 bits, base64url — separate from `id` so the
  -- link can be rotated without touching the row, and so the token never rides
  -- along anywhere the row id is logged.
  token          text NOT NULL UNIQUE,
  workspace_id   uuid NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  document_id    uuid NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
  created_by     uuid NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
  created_at     timestamptz NOT NULL DEFAULT now(),
  revoked_at     timestamptz,
  -- SEO opt-in. Default noindex so a shared page is not silently crawlable.
  allow_indexing boolean NOT NULL DEFAULT false,
  -- Schema-ready for "share this page AND its subpages" (Notion's #1
  -- oversharing footgun, Outline's most bug-prone path). MVP ships it false and
  -- unexposed; enforcing subtree membership at the public route comes later.
  include_children boolean NOT NULL DEFAULT false
);

-- One ACTIVE share per document: re-sharing returns the existing row, and only
-- after revoke can a new token be minted. (Partial unique index instead of an
-- EXCLUDE constraint so we need no btree_gist extension.)
CREATE UNIQUE INDEX idx_document_shares_active
  ON document_shares (document_id) WHERE revoked_at IS NULL;

-- The public read path looks up by token; only active rows matter.
CREATE INDEX idx_document_shares_token
  ON document_shares (token) WHERE revoked_at IS NULL;
