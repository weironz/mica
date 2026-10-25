# 0.13.68 deployment and acceptance

Date: 2026-10-25. Release tag `v0.13.68` points to
`a8c6b77572dc628bdd745515a08e5a62cb6100c4`.
The deployment/migration-input repair is commit `9dfdecf`.

## Root cause and upgrade proof

See [0.13.67 failure](release-0.13.67-failure.md). On the production host, a
disposable PostgreSQL instance was populated with production schema and
migration records only. User document rows were not copied.

Using the same database service alias and migration history:

- 0.13.67 exits with migration 12's "previously applied but has been modified".
- 0.13.68 becomes ready and reports version 0.13.68.

Private receipts and logs:
`/data/mica/ops/release-0.13.68-upgrade-probe-20261025T103628Z`.
No published tag or production migration checksum was overwritten.

## Gates and artifacts

- Full `just ci` passed against a dedicated, disposable validation database.
  The existing local development database retained its original checksums.
- `just release 0.13.68` passed its standard gate. Flutter had 2,052 passing
  tests and two existing skips. Toolchain pins and gate thresholds were kept.
- Database-host preflight: 12 cases passed. Recovery/log handling: four cases
  passed, including unavailable storage and a hung Docker log reader.
- A real API Docker build refused CRLF migration input before compilation.
- Windows installer, Windows CLI, static Linux CLI and SHA256 manifest were
  uploaded. Published asset digests match the local manifest. All three
  images were pushed to ACR and Docker Hub.
- Installer smoke produced a real first-frame marker and survived another
  ten seconds after the desktop user's existing Mica instance exited.

The installer check also exposed a separate race: the EXE can appear before
the installer finishes copying DLLs. Observation found four DLLs absent at
that point and none absent after completion. The shared smoke now waits for
installer exit code zero, then requires the EXE, first frame and ten-second
survival. Four PowerShell fixtures cover completion, installer error, missing
EXE and timeout. CI and local release commands use the same smoke script.
Loader dependency diagnostics are retained.

A later attempt while the user had reopened Mica was rejected by its
single-instance guard (exit zero); it is not a loader crash or a successful
second smoke. The existing user client was left running.

## Production

The normal `just deploy-prod 0.13.68 --check --diff` rehearsal passed, followed
by successful deployment and `verify-prod`.

- Public `/api/ready`: HTTP 200, version 0.13.68.
- API: healthy, restart count zero; `DATABASE_URL` uses `postgres`.
  Inside the API container, that name resolves to the current database IP.
- PostgreSQL: all 35 migration records successful; historical SQL unchanged.
- PostgreSQL and RustFS container IDs, images and start times are unchanged.
- All 42 observed non-Mica containers retain their IDs, images, start times
  and restart counts. No shared ingress or global Docker configuration changed.
- `.env.secrets` is byte-for-byte unchanged. Deployment no longer prints old
  environment content through template diffs.
- Public Web bundle SHA256 matches the built artifact:
  `d7aa7914b02fe4fef16c2bacb5c3825cc0d0a29785f91464850b1134bf567ff5`.

Private pre-deployment configuration and container baseline:
`/data/mica/ops/release-v0.13.68-before-20261025T103533Z`.
Files are private; no credentials are included in this repository.

## Acceptance

- Production login rendered on desktop and mobile viewports with 0.13.68
  visible, no uncaught browser exceptions and no failed network responses.
- Created a temporary folder, nested folder and page in the ruby workspace.
  Read-back preserved text, formatted list items and a table.
- Moved the page between folders, then moved the whole folder subtree to the
  test workspace. Destination content matched; the source search was empty.
  Test artifacts were moved to the recycle bin, without purging user data.
- Post-deployment API logs contain no database connection, migration,
  authentication or internal-database-error matches.
- Three public TLS probes, 30 seconds apart, returned HTTP 200 with successful
  certificate validation: handshake 60.2, 58.1 and 50.2 ms.

These are point-in-time acceptance checks, not a claim of long-term latency
or availability guarantees.
