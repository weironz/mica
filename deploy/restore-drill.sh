#!/usr/bin/env bash
# Prove a Postgres restore point actually restores. Runs on the node.
#
# `gzip -t` proves an archive is not truncated. `zcat | grep -c "^COPY public.x"`
# proves a table is mentioned. NEITHER proves that `psql < dump` yields a working
# database — and docs/backup.md says it out loud: 「没恢复过的备份只是猜测」.
#
# That went from hygiene to load-bearing with S5 (migration 0016), the first
# irreversible migration in this schema: it dropped the op model's three tables,
# so for anything predating it the pg_dump is the ONLY way back. A restore path
# you have never exercised is not a rollback plan.
#
# What this does: restore into a throwaway database beside the live one, assert
# structure + row counts + that the read path's join really returns content, then
# drop it. The live `mica` database is never touched and no container is
# restarted. Output is counts only — no document text, no user field.
#
#   just restore-drill /data/mica/pre-0.13.4-20260730-100443.sql.gz
#   # or, on the node:
#   bash restore-drill.sh /data/mica/<dump>.sql.gz
#
# Exits non-zero on: unreadable archive, any restore error, or a database that
# comes back structurally complete but empty of readable content.
#
# WHY THIS ONE STAYS SHELL (2026-08-02). The backup orchestration moved into
# `mica-cli backup` because "partial success" had no type in bash: a missing leg
# logged a WARN and still exited 0, and production went months with no off-site
# copy of its database. There is no such shape here — three assertions, each
# either true or false, with no middle state to lose.
#
# And it runs on the HOST, driving `docker exec` + `psql` against a throwaway
# database beside the live one. mica-cli lives inside the backup container, so
# folding this in would mean shipping another binary to the node to buy type
# safety over "run three queries, compare three numbers".
#
# It used to have company: node-deploy-policy.sh, the node-side deploy fence,
# stayed in shell for the same reason. That one is gone (2026-08-25 — deploying
# is ansible/deploy.yml now, and the fence it enforced was retired with it; see
# docs/cd-plan.md §4.1). This is the only script left in deploy/.
set -euo pipefail

DUMP=${1:?usage: restore-drill.sh /data/mica/<dump>.sql.gz}
DRILL=${DRILL_DB:-mica_restore_drill}
PG=${PG_CONTAINER:-mica-postgres-1}

# Every `docker exec -i` below MUST redirect stdin. Piped through `ssh host bash -s`
# the script itself arrives on stdin, and an `-i` exec with no redirect reads it,
# swallowing the rest of the script — the first run of this drill died silently
# right after CREATE DATABASE exactly that way. `-i` is kept (the restore pipeline
# needs it) and every non-piping call gets `</dev/null`.
psql_drill() { docker exec -i "$PG" psql -qU mica -d "$DRILL" -P pager=off "$@"; }

cleanup() {
  docker exec -i "$PG" psql -qU mica -d postgres \
    -c "DROP DATABASE IF EXISTS $DRILL" </dev/null >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo "== 0. the archive itself =="
ls -la "$DUMP"
gzip -t "$DUMP"
echo "   gzip -t OK"

echo "== 1. restore into a throwaway database =="
cleanup
docker exec -i "$PG" psql -qU mica -d postgres \
  -c "CREATE DATABASE $DRILL" </dev/null
# Count errors instead of aborting on the first: a dump that restores WITH errors
# is precisely what this drill exists to surface, so it has to be reported.
errs=$(zcat "$DUMP" | docker exec -i "$PG" psql -qU mica -d "$DRILL" 2>&1 \
        | grep -ciE '^(ERROR|FATAL)' || true)
echo "   restore errors: $errs"

echo "== 2. is the restored database intact and USABLE? =="
psql_drill </dev/null <<'SQL'
SELECT max(version) AS migrations_applied FROM _sqlx_migrations;

SELECT (SELECT count(*) FROM documents)         AS docs,
       (SELECT count(*) FROM document_yrs_base) AS bases,
       (SELECT count(*) FROM views)             AS views,
       (SELECT count(*) FROM users)             AS users;

-- Structure, not just rows.
SELECT count(*) FILTER (WHERE contype = 'f') AS foreign_keys,
       count(*) FILTER (WHERE contype = 'p') AS primary_keys
FROM pg_constraint c
JOIN pg_namespace n ON n.oid = c.connamespace
WHERE n.nspname = 'public';

-- Usable, not merely present: the join every read walks, over content that is
-- really there. A restore that produced empty `state` blobs would pass every
-- count above and fail this one.
SELECT count(*) AS readable_pages
FROM views v
JOIN documents d         ON d.id = v.object_id AND v.object_type = 'document'
JOIN document_yrs_base b ON b.document_id = d.id
WHERE v.is_deleted = false
  AND length(b.state) > 0
  AND coalesce(b.content_text, '') <> '';
SQL

# Hard gates. Everything above is for a human to read; these fail the run.
docs=$(psql_drill -tA -c "SELECT count(*) FROM documents" </dev/null)
readable=$(psql_drill -tA <<'SQL'
SELECT count(*) FROM views v
JOIN documents d         ON d.id = v.object_id AND v.object_type = 'document'
JOIN document_yrs_base b ON b.document_id = d.id
WHERE v.is_deleted = false AND length(b.state) > 0
  AND coalesce(b.content_text, '') <> '';
SQL
)
[ "$errs" -eq 0 ]     || { echo "FAIL: restore reported $errs error(s)"; exit 1; }
[ "$docs" -gt 0 ]     || { echo "FAIL: no documents in the restored database"; exit 1; }
[ "$readable" -gt 0 ] || { echo "FAIL: restored, but no page reads back with content"; exit 1; }

echo "== 3. object-storage side: rustic check =="
# The other half of the same gap: the OSS repository can rot silently, and prune
# is what amplifies it. Only discoverable by asking.
if docker ps --format '{{.Names}}' | grep -q "^mica-backup-1$"; then
  docker exec mica-backup-1 rustic check </dev/null 2>&1 | tail -12
else
  echo "   (mica-backup-1 not running — skipped)"
fi

echo "== PASS: $docs documents, $readable readable pages, 0 restore errors =="
