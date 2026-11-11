-- Account-owned notification preference: independent of the replace-whole
-- client settings blob, so older clients cannot silently re-enable mail.
ALTER TABLE users ADD COLUMN import_completion_email_enabled boolean NOT NULL DEFAULT true;

-- A claim is an attempt, not proof of delivery. Sending is best-effort and
-- at-most-once; a process crash can leave 'sending', a provider failure 'failed'.
ALTER TABLE import_jobs ADD COLUMN completion_email_state text
  CHECK (completion_email_state IN ('sending', 'sent', 'failed'));
