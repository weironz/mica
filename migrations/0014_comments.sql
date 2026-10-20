-- Comments (docs/comments-plan.md, Phase 1).
--
-- The anchor is a yrs STICKY INDEX, not a character offset: an offset is only
-- correct against one version, and any concurrent insert before it silently
-- shifts the highlight onto the wrong text. `StickyIndex::encode_v1` bytes
-- survive concurrent edits for free — the same mechanism Yjs documents as the
-- one for "comments and cursors", and the same thing BlockSuite/AFFiNE anchors
-- its comments with.
--
-- These live in Postgres, NOT inside the yrs document, and that is the load-
-- bearing decision: the document's Markdown stays byte-for-byte untouched, so
-- the round-trip invariant (CLAUDE.md red line) needs zero changes and a comment
-- can never leak into an export. Research found no product that puts comment
-- markers into an authoritative Markdown body; every one of them keeps comments
-- out of the exportable text instead.
--
-- Table names are plural to match the rest of the schema (documents, users,
-- files); the design doc sketched them singular.
CREATE TABLE comment_threads (
  id                  uuid PRIMARY KEY,
  document_id         uuid NOT NULL REFERENCES documents(id) ON DELETE CASCADE,

  -- Anchor: one (block id, sticky index) pair per end, so a thread can span
  -- blocks. Assoc on write is After for the start and Before for the end, which
  -- makes the highlight hug the selected text: typing immediately outside it is
  -- not swallowed into the comment.
  anchor_start_block  text NOT NULL,
  anchor_start_sticky bytea NOT NULL,
  anchor_end_block    text NOT NULL,
  anchor_end_sticky   bytea NOT NULL,

  -- The anchored text as it read when the thread was created. Two jobs: the
  -- comment list can preview a thread without decoding the CRDT at all, and an
  -- orphaned thread (its text deleted) still shows what was being discussed
  -- instead of vanishing. BlockSuite keeps the same snapshot for the same reason.
  quote               text NOT NULL,

  -- 'open' → anchor resolves, highlight drawn. 'resolved' → kept WITH its anchor
  -- (so "show resolved" can highlight it again), never deleted. 'orphaned' → an
  -- anchor end no longer resolves because the text is gone; the discussion is
  -- preserved and shown against `quote` rather than hard-deleted.
  status              text NOT NULL DEFAULT 'open'
                        CHECK (status IN ('open', 'resolved', 'orphaned')),

  -- A deleted account takes its comments with it (matches account deletion's
  -- "remove my data" contract, and avoids a nullable author every reader would
  -- have to render as a tombstone).
  created_by          uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_at          timestamptz NOT NULL DEFAULT now(),
  resolved_by         uuid REFERENCES users(id) ON DELETE SET NULL,
  resolved_at         timestamptz
);

CREATE TABLE comments (
  id         uuid PRIMARY KEY,
  thread_id  uuid NOT NULL REFERENCES comment_threads(id) ON DELETE CASCADE,
  author_id  uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  -- Markdown, like the document body — rendered by the same inline pipeline.
  body       text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  edited_at  timestamptz
);

-- Listing a document's threads and a thread's replies are the only hot reads.
CREATE INDEX comment_threads_document_id_idx ON comment_threads (document_id);
CREATE INDEX comments_thread_id_created_at_idx ON comments (thread_id, created_at);
