#!/usr/bin/env bash
# rustic wrapper: render the repo config from env (opendal S3 → Aliyun OSS) into
# /etc/rustic/rustic.toml, then exec `rustic` with the given args. EVERY rustic
# invocation goes through this — the scheduled backup and one-off commands alike
# (`init`, `snapshots`, `restore`, `check`) — so the config always exists even in
# a fresh one-off container:
#   docker compose --profile backup run --rm --entrypoint rustic-mica backup init
#   docker compose --profile backup run --rm --entrypoint rustic-mica backup snapshots --group-by label
# The password stays in RUSTIC_PASSWORD (env) and is never written to the file.
# enable_virtual_host_style is required for Aliyun OSS (path style 404s).
set -euo pipefail

CONF=/etc/rustic/rustic.toml
mkdir -p "$(dirname "$CONF")"
( umask 077; cat > "$CONF" <<EOF
[repository]
repository = "opendal:s3"

[repository.options]
bucket = "${OSS_BUCKET:?set OSS_BUCKET}"
endpoint = "${OSS_ENDPOINT:?set OSS_ENDPOINT}"
region = "${OSS_REGION:?set OSS_REGION}"
access_key_id = "${OSS_ACCESS_KEY_ID:?set OSS_ACCESS_KEY_ID}"
secret_access_key = "${OSS_SECRET_ACCESS_KEY:?set OSS_SECRET_ACCESS_KEY}"
root = "${OSS_ROOT:?set OSS_ROOT}"
enable_virtual_host_style = "true"
EOF
)

exec rustic "$@"
