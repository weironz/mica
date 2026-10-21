-- Adding a member becomes an INVITATION the invitee accepts.
--
-- `POST /workspaces/{id}/members` used to look the email up and INSERT the
-- membership on the spot: the other person never agreed to join, and an
-- unregistered address answered 404 — any admin could ask "is this email
-- registered?". An invitation is keyed by the (lower-cased) email instead of a
-- user id, so it is created the same way whether or not an account exists yet,
-- and someone who registers later still finds it waiting.
--
-- Accepting inserts the membership and deletes this row in one transaction;
-- declining or revoking deletes it. Nothing here is ever "accepted" in place.
CREATE TABLE workspace_invitations (
  id uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
  workspace_id uuid NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  -- Normalized by the API (trim + ASCII lower-case, same as users.email); the
  -- CHECK stops a future path from storing a variant no lookup would match.
  email text NOT NULL CHECK (email = lower(email) AND email <> ''),
  -- Never 'owner': ownership is not something you can be invited into (the
  -- API refuses it too; this is the backstop, see 0011/0028).
  role workspace_role NOT NULL CHECK (role <> 'owner'),
  invited_by uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (workspace_id, email)
);

-- The invitee's inbox query ("what am I invited to?") looks up by email.
CREATE INDEX idx_workspace_invitations_email ON workspace_invitations(email);
