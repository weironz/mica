# 0.13.67 startup failure: SQL migration byte drift

Confirmed on 2026-10-25. Release `v0.13.67` is commit `87db5c2`.

The published API image exits with code 1 during SQLx migration validation:

```text
Error: failed to run database migrations
Caused by:
    migration 12 was previously applied but has been modified
```

The production database contains 35 successful migration records. All 35
checksums match the files committed at `v0.13.67`. However, the Windows
working tree contained CRLF bytes in migrations 0012 through 0033. Git's
text normalization hid those differences from `git diff`. The local Docker
build copied those raw working-tree bytes into the image and embedded
different SQLx checksums.

No historical migration contents or production checksums were changed.
The 22 working-tree files were backed up locally, then restored byte for
byte from the committed LF versions.

## Isolated production-host reproduction

Only schema and `_sqlx_migrations` records were copied into a disposable
PostgreSQL container. User rows, documents and attachments were not copied.
No public ports were opened. Each API used that isolated database.

| Image | Database address | Result |
| --- | --- | --- |
| 0.13.66 | Docker service alias | Ready |
| 0.13.67 | Docker service alias | Migration 12 checksum rejection |
| 0.13.67 | Current container IP | Same migration 12 rejection |

All failed probes reported `OOMKilled=false`. The production API container
kept its identity and remained healthy throughout reproduction. Private
evidence is under
`/data/mica/ops/deploy-0.13.67-diagnosis-20261025T095723Z`.
The temporary containers were removed individually after collecting logs.

This incident is distinct from the earlier Docker DNS failure. Production
currently uses `postgres`; current Compose and the running API use matching
credentials. The earlier deployment renderer's removal of a duplicate
password entry from `.env` did not lose the password from `.env.secrets`.

## Prevention and recovery

- `check-migration-bytes.sh` reads raw bytes, including on Windows where
  text-mode tools can strip CR before matching. Release checks, local CI and
  the API Docker build reject CRLF/CR migration inputs.
- A Git fixture reproduces the exact misleading state: clean normalized
  Git diff, different working-tree bytes, and a rejected build input.
- Production host variables retain the `postgres` service alias. The
  deployment preflight checks that the running database advertises it.
  Explicit IP and `auto` remain available as deliberate DNS workarounds.
- Failed application logs are retained in a private Mica operations
  directory before rollback replaces the failed containers.
- Recovery explicitly names the application services and already-present
  sidecars. It does not reconcile PostgreSQL, RustFS or other projects.
- Environment-template diffs are suppressed because a manually added
  credential in the previous `.env` must not leak through `--diff`.

Published Git and image tags remain immutable. The corrected build is
released as 0.13.68; 0.13.67 is retained as evidence rather than overwritten.
Production deployment and upgrade acceptance are recorded in
[0.13.68 validation](release-0.13.68-validation.md).
