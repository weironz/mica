-- The comment part of a page's export revision (manifest v3 carries comment
-- threads in each page's .mica.json). One definition, called by both the
-- workspace export inventory and the single-page export, so an incremental
-- mirror re-fetches a page when its discussion changed even though its text
-- did not. NULL for a page with no comments: its revision stays exactly what
-- it was before comments were exported, so existing mirrors are not forced to
-- re-download every page once.
CREATE FUNCTION mica_comments_revision(doc uuid) RETURNS text
LANGUAGE sql STABLE AS $$
  SELECT ':c' || md5(string_agg(
           t.id::text || '|' || t.status || '|' || coalesce(t.resolved_at::text, '') || '|' ||
           t.quote || '|' || c.id::text || '|' || c.body || '|' || coalesce(c.edited_at::text, ''),
           E'\n' ORDER BY t.created_at, t.id, c.created_at, c.id))
  FROM comment_threads t JOIN comments c ON c.thread_id = t.id
  WHERE t.document_id = doc
$$;
