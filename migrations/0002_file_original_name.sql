-- Preserve the original upload filename so exports can restore human-readable
-- image names (object keys are content-addressed by sha256 and not meaningful).
ALTER TABLE files ADD COLUMN original_name text NOT NULL DEFAULT '';
