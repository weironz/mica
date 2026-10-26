#!/usr/bin/env bash
# Run the real `scripts/release.sh` end to end inside a disposable Git fixture,
# with the toolchain, Flutter, Docker/Postgres probe and network queries replaced
# by stubs. Nothing outside the fixture is touched: the commit and the tag this
# creates exist only in that throwaway repository.
#
# The regression (#125): release.sh demands a clean tree, release-bump.sh bumped
# only the CODE version, and release-check.sh then refuses because README.md and
# docs/ops/deploy.md still pin the previous `RELEASE=`. So a clean-tree release
# N+1 could never pass its own gate — the script created the failure it reported,
# and left six modified files that made the retry refuse as "dirty".
set -euo pipefail
source_root=$(cd "$(dirname "$0")/.." && pwd)
temp_parent=$(cd "${TMPDIR:-/tmp}" && pwd -P)
fixture_root=$(mktemp -d "$temp_parent/mica-release-orchestration.XXXXXX")
# Only remove the directory this invocation just created under the resolved temp
# directory, never a repository/worktree or a path supplied by a caller.
[[ "$fixture_root" == "$temp_parent"/mica-release-orchestration.* ]] || exit 2
trap 'rm -rf -- "$fixture_root"' EXIT

old=0.13.68
new=0.13.69
stub_bin="$fixture_root/bin"
mkdir -p "$stub_bin"

# ── stubs ────────────────────────────────────────────────────────────────────
# flutter: the pin check reads `--version`; the gate runs pub get / gen-l10n /
# analyze / test, all of which are somebody else's test suite.
cat > "$stub_bin/flutter" <<'EOF'
#!/bin/sh
case "$1" in
  --version) echo 'Flutter 3.35.0 • channel stable • https://github.com/flutter/flutter.git' ;;
  test)
    # The gate counts passed tests from the JSON file reporter (lib-receipt.sh).
    for arg in "$@"; do
      case "$arg" in
        json:*)
          i=0
          while [ $i -lt 1000 ]; do
            echo '{"testID":1,"result":"success","skipped":false,"hidden":false,"type":"testDone"}'
            i=$((i + 1))
          done > "${arg#json:}" ;;
      esac
    done ;;
esac
exit 0
EOF

# rustc: the gate compares this against rust-toolchain.toml.
cat > "$stub_bin/rustc" <<'EOF'
#!/bin/sh
echo 'rustc 1.90.0 (1159e78c4 2026-01-01)'
EOF

# cargo: `check -p mica-api-server` is the bump's "refresh the lock" step, so it
# rewrites the lock the way cargo would. `test` can be made to fail on demand so
# the gate-failure path is exercised deterministically.
cat > "$stub_bin/cargo" <<'EOF'
#!/bin/sh
if [ "$1" = test ] && [ -n "${FAIL_GATE_MARKER:-}" ] && [ -f "$FAIL_GATE_MARKER" ]; then
  echo 'stub cargo test: induced failure (release gate fixture)' >&2
  exit 1
fi
# The gate counts passed tests (lib-receipt.sh), so report some.
[ "$1" = test ] && echo 'test result: ok. 600 passed; 0 failed; 0 ignored'
if [ "$1" = check ] && [ -f Cargo.toml ] && [ -f Cargo.lock ]; then
  v=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
  sed -i "/^name = \"mica-api-server\"$/{n;s/^version = \".*\"/version = \"$v\"/;}" Cargo.lock
  sed -i "/^name = \"mica-cli\"$/{n;s/^version = \".*\"/version = \"$v\"/;}" Cargo.lock
fi
exit 0
EOF

# The DB-backed tests must not SKIP, so the gate insists on a reachable
# Postgres. The stub answers the reachability probe; no database is contacted.
cat > "$stub_bin/pg_isready" <<'EOF'
#!/bin/sh
exit 0
EOF

printf '#!/bin/sh\nexit 0\n' > "$stub_bin/dart"
chmod +x "$stub_bin"/*

# ── fixture repository ───────────────────────────────────────────────────────
new_fixture() {
  local repo=$1
  mkdir -p "$repo/scripts" "$repo/deploy" "$repo/docs/ops" "$repo/migrations" \
    "$repo/clients/mica_flutter/lib" "$repo/clients/mica_flutter/assets" \
    "$repo/clients/mica_flutter/windows/runner/resources" \
    "$repo/clients/mica_flutter/rust" "$repo/clients/mica_flutter/rust_builder/cargokit/build_tool"
  cp "$source_root/scripts/release.sh" "$source_root/scripts/release-bump.sh" \
     "$source_root/scripts/release-check.sh" "$source_root/scripts/lib-local.sh" \
     "$source_root/scripts/lib-receipt.sh" "$source_root/scripts/check-ignored-sources.sh" \
     "$source_root/scripts/manifest-version.sh" "$source_root/scripts/check-migration-tracking.sh" \
     "$source_root/scripts/check-migration-bytes.sh" "$source_root/scripts/test-migration-tracking.sh" \
     "$repo/scripts/"

  printf '[package]\nname = "mica-cli"\nversion = "%s"\n' "$old" > "$repo/Cargo.toml"
  cat > "$repo/Cargo.lock" <<LOCK
[[package]]
name = "mica-api-server"
version = "$old"

[[package]]
name = "mica-cli"
version = "$old"
LOCK
  printf 'channel = "1.90.0"\n' > "$repo/rust-toolchain.toml"
  printf '{ "flutter": "3.35.0" }\n' > "$repo/.fvmrc"
  printf 'version: %s\n' "$old" > "$repo/clients/mica_flutter/pubspec.yaml"
  printf "const kAppVersion = '%s';\n" "$old" > "$repo/clients/mica_flutter/lib/main.dart"
  printf 'fixture\n' > "$repo/clients/mica_flutter/assets/tray_icon.ico"
  cp "$repo/clients/mica_flutter/assets/tray_icon.ico" \
     "$repo/clients/mica_flutter/windows/runner/resources/app_icon.ico"
  printf '[package]\nname = "rust_lib_mica_flutter"\n' > "$repo/clients/mica_flutter/rust/Cargo.toml"
  printf 'FROM rust:1.90-slim-bookworm\n' > "$repo/deploy/Dockerfile.api"
  printf 'FROM rust:1.90-slim-bookworm\nRUN apk add postgresql-client-17\n' > "$repo/deploy/Dockerfile.cli"
  printf 'services:\n  dev:\n    image: rust:1.90-slim-bookworm\n' > "$repo/docker-compose.yml"
  printf 'services:\n  postgres:\n    image: postgres:16-alpine\n' > "$repo/deploy/docker-compose.yml"
  printf 'SELECT 1;\n' > "$repo/migrations/0001_initial.sql"
  printf '# fixture quickstart\n\nRELEASE=%s\n' "$old" > "$repo/README.md"
  printf '# fixture deploy\n\nRELEASE=%s\n' "$old" > "$repo/docs/ops/deploy.md"
  printf '/target/\ndist/\nclients/mica_flutter/build/\n' > "$repo/.gitignore"

  git -C "$repo" init -q
  git -C "$repo" config user.name 'Release orchestration fixture'
  git -C "$repo" config user.email 'release-orchestration@example.invalid'
  git -C "$repo" add -A
  git -C "$repo" commit -qm "chore: fixture at $old"
  git -C "$repo" tag "v$old"
}

run_release() { # $1 = repo, rest = env assignments already exported by caller
  (cd "$1" && PATH="$stub_bin:$PATH" DATABASE_URL="postgres://stub@127.0.0.1/stub" \
     bash scripts/release.sh "$new")
}

committed_files() { git -C "$1" show --name-only --format= HEAD | sed '/^[[:space:]]*$/d' | sort; }

# ── slice 1: a clean-tree release N+1 carries code AND docs ─────────────────
repo="$fixture_root/repo-success"
new_fixture "$repo"
if ! log=$(run_release "$repo" 2>&1); then
  echo 'FAIL: a clean, fully pinned tree could not release N+1' >&2
  echo "$log" >&2
  exit 1
fi

[ "$(git -C "$repo" log -1 --format=%s)" = "chore(release): $new" ] || {
  echo 'FAIL: the release commit is missing or mislabelled' >&2
  git -C "$repo" log -1 --format=%s >&2
  exit 1
}
expected_files=$(printf '%s\n' Cargo.toml Cargo.lock README.md \
  clients/mica_flutter/lib/main.dart clients/mica_flutter/pubspec.yaml docs/ops/deploy.md | sort)
[ "$(committed_files "$repo")" = "$expected_files" ] || {
  echo 'FAIL: the release commit must carry exactly the bumped files' >&2
  echo 'expected:' >&2
  printf '%s\n' "$expected_files" >&2
  echo 'got:' >&2
  committed_files "$repo" >&2
  exit 1
}
[ "$(git -C "$repo" tag --list "v$new")" = "v$new" ] || {
  echo 'FAIL: no v'"$new"' tag was created' >&2
  exit 1
}
[ "$(git -C "$repo" rev-parse "v$new^{commit}")" = "$(git -C "$repo" rev-parse HEAD)" ] || {
  echo 'FAIL: the tag does not point at the release commit' >&2
  exit 1
}
(cd "$repo" && bash scripts/manifest-version.sh) > "$fixture_root/manifest" 2>&1 || {
  echo 'FAIL: the three code version numbers disagree after the release' >&2
  cat "$fixture_root/manifest" >&2
  exit 1
}
[ "$(cat "$fixture_root/manifest")" = "$new" ] || {
  echo 'FAIL: the code version is not '"$new" >&2
  cat "$fixture_root/manifest" >&2
  exit 1
}
for doc in README.md docs/ops/deploy.md; do
  grep -qF "RELEASE=$new" "$repo/$doc" || {
    echo "FAIL: $doc still pins the previous quickstart release" >&2
    grep -F 'RELEASE=' "$repo/$doc" >&2
    exit 1
  }
done
[ -z "$(git -C "$repo" status --porcelain)" ] || {
  echo 'FAIL: a successful release left changes behind' >&2
  git -C "$repo" status --short >&2
  exit 1
}

# ── slice 2: a failed gate is explained, leaves nothing committed, and the
#            next attempt is possible without hand-repairing the tree ────────
repo="$fixture_root/repo-gate-failure"
new_fixture "$repo"
marker="$fixture_root/fail-gate"
: > "$marker"

status=0
failure=$(FAIL_GATE_MARKER="$marker" run_release "$repo" 2>&1) || status=$?
[ "$status" != 0 ] || {
  echo 'FAIL: the release succeeded with the gate failing' >&2
  exit 1
}
[ -z "$(git -C "$repo" log -1 --format=%s | grep -F "chore(release):" || true)" ] || {
  echo 'FAIL: a failed gate still produced a release commit' >&2
  exit 1
}
[ -z "$(git -C "$repo" tag --list "v$new")" ] || {
  echo 'FAIL: a failed gate still produced a tag' >&2
  exit 1
}
case "$failure" in
  *"Nothing was committed or tagged"*) ;;
  *) echo "FAIL: the failure does not say what state the tree is in:
$failure" >&2; exit 1 ;;
esac
case "$failure" in
  *"git checkout --"*) ;;
  *) echo "FAIL: the failure does not say how to discard the bump and retry:
$failure" >&2; exit 1 ;;
esac
for f in Cargo.toml Cargo.lock README.md docs/ops/deploy.md \
         clients/mica_flutter/lib/main.dart clients/mica_flutter/pubspec.yaml; do
  case "$failure" in
    *"$f"*) ;;
    *) echo "FAIL: the failure does not name the bumped file $f" >&2; exit 1 ;;
  esac
done
dirty=$(git -C "$repo" status --porcelain | wc -l | tr -d ' ')
[ "$dirty" = 6 ] || {
  echo "FAIL: expected the 6 bumped files left uncommitted, found $dirty:" >&2
  git -C "$repo" status --short >&2
  exit 1
}

# The diagnostic's own instructions must actually work: discard the bump, clear
# the cause, run it again.
rm -f "$marker"
git -C "$repo" checkout -- .
[ -z "$(git -C "$repo" status --porcelain)" ]
if ! retry=$(run_release "$repo" 2>&1); then
  echo 'FAIL: retrying after discarding the failed bump did not release' >&2
  echo "$retry" >&2
  exit 1
fi
[ "$(git -C "$repo" tag --list "v$new")" = "v$new" ] || {
  echo 'FAIL: the retry produced no tag' >&2
  exit 1
}

echo "PASS: release N+1 bumps code + quickstart pins in one commit, and a failed gate is explained and retriable"
