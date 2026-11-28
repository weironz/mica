#!/usr/bin/env bash
# Drive the real scripts/ship.sh through a disposable fixture: every stage
# script is a stub that records that it ran, `gh`/`docker`/`uname` are doubles,
# and `git push` goes to a bare repository inside the fixture.
#
# What it pins:
#   - first run executes all pure-Rust ship stages (preflight, artifacts,
#     images, upload, publish) and prints a stage timing summary;
#   - a resume reuses an artifacts build / image push already done for the same
#     commit, and REBUILD=1 forces both;
#   - a refused publish FAILS the run and the summary says which stage failed —
#     not "passed" because errexit was off inside an `if` (the shape of bug the
#     first draft of this train had);
#   - real release-preflight reports every local environment problem in one pass.
set -euo pipefail
source_root=$(cd "$(dirname "$0")/.." && pwd)
temp_parent=$(cd "${TMPDIR:-/tmp}" && pwd -P)
fixture=$(mktemp -d "$temp_parent/mica-ship.XXXXXX")
[[ "$fixture" == "$temp_parent"/mica-ship.* ]] || exit 2
trap 'rm -rf -- "$fixture"' EXIT
version=1.2.3
repo="$fixture/repo"
log="$fixture/calls.log"
mkdir -p "$repo/scripts" "$fixture/bin"
cp "$source_root/scripts/ship.sh" "$source_root/scripts/lib-local.sh" "$repo/scripts/"
for s in release-preflight release release-artifacts release-images release-upload release-publish; do
  cat > "$repo/scripts/$s.sh" <<EOF
#!/usr/bin/env bash
echo "$s \$*" >> "$log"
if [ "$s" = release-publish ] && [ "\${STUB_PUBLISH_FAIL:-}" = 1 ]; then echo 'REFUSED: stub' >&2; exit 1; fi
if [ "$s" = release-artifacts ]; then mkdir -p dist/v$version && git rev-parse HEAD > dist/v$version/.built-from; fi
if [ "$s" = release-images ]; then mkdir -p dist/v$version && git rev-parse HEAD > dist/v$version/.images-pushed; fi
exit 0
EOF
done
cat > "$fixture/bin/gh" <<'EOF'
#!/usr/bin/env bash
case "$*" in
  'auth status') exit 0 ;;
  *) exit 0 ;;
esac
EOF
cat > "$fixture/bin/docker" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
cat > "$fixture/bin/uname" <<'EOF'
#!/usr/bin/env bash
if [ "${STUB_WINDOWS:-}" = 1 ]; then echo MINGW64_NT-10.0; else echo Linux; fi
EOF
chmod +x "$fixture/bin/"* "$repo/scripts/"*.sh
printf 'dist/\n' > "$repo/.gitignore"
git init -q --bare "$fixture/origin.git"
git -C "$repo" init -q -b main
git -C "$repo" config user.name 'Ship fixture'
git -C "$repo" config user.email 'ship@example.invalid'
git -C "$repo" add -A
git -C "$repo" commit -qm fixture
git -C "$repo" tag "v$version"
git -C "$repo" remote add origin "$fixture/origin.git"

ship() { (cd "$repo" && PATH="$fixture/bin:$PATH" bash scripts/ship.sh "$version"); }
ran() { grep -q "^$1 " "$log"; }

# 1. First run from an existing tag: everything builds and publishes.
: > "$log"
ship > "$fixture/out1" 2>&1 || { cat "$fixture/out1"; echo 'FAIL: first run failed' >&2; exit 1; }
for s in release-preflight release-artifacts release-images release-upload release-publish; do
  ran "$s" || { echo "FAIL: first run skipped $s" >&2; exit 1; }
done
grep -q 'total' "$fixture/out1" || { echo 'FAIL: no stage-time summary' >&2; exit 1; }

# 2. Resume on the same commit: artifacts and images are reused, not rebuilt.
: > "$log"
ship > "$fixture/out2" 2>&1
! ran release-artifacts && ! ran release-images || { echo 'FAIL: resume rebuilt work already done for this commit' >&2; exit 1; }
grep -q 'reused' "$fixture/out2" || { echo 'FAIL: the summary does not say what was reused' >&2; exit 1; }

# 3. REBUILD=1 forces both.
: > "$log"
REBUILD=1 ship > /dev/null 2>&1
ran release-artifacts && ran release-images || { echo 'FAIL: REBUILD=1 did not rebuild' >&2; exit 1; }

# 4. A refused publish fails the run, and the summary names the stage.
status=0
STUB_PUBLISH_FAIL=1 ship > "$fixture/out4" 2>&1 || status=$?
[ "$status" != 0 ] || { echo 'FAIL: a refused publish counted as success' >&2; exit 1; }
grep -Eq 'publish +FAILED' "$fixture/out4" || { cat "$fixture/out4"; echo 'FAIL: summary does not name the failed stage' >&2; exit 1; }
! grep -q 'is published' "$fixture/out4" || { echo 'FAIL: claimed published after a refusal' >&2; exit 1; }

# 5. The real preflight reports every problem in one refusal: here gh cannot
#    reach the API, Inno Setup is missing and Docker is down, all at once.
cp "$source_root/scripts/release-preflight.sh" "$repo/scripts/release-preflight.sh"
cat > "$fixture/bin/gh" <<'EOF'
#!/usr/bin/env bash
case "$*" in 'auth status') exit 0 ;; *) exit 1 ;; esac
EOF
cat > "$fixture/bin/docker" <<'EOF'
#!/usr/bin/env bash
exit 1
EOF
status=0
(cd "$repo" && PATH="$fixture/bin:$PATH" STUB_WINDOWS=1 ISCC="$fixture/no-iscc" MICA_MIN_FREE_GB=0 \
  bash scripts/release-preflight.sh "$version") > "$fixture/out5" 2>&1 || status=$?
[ "$status" != 0 ] || { echo 'FAIL: preflight passed a broken machine' >&2; exit 1; }
for needle in 'GitHub API' 'Inno Setup' 'Docker is not running'; do
  grep -q "$needle" "$fixture/out5" || { cat "$fixture/out5"; echo "FAIL: preflight did not report '$needle' alongside the others" >&2; exit 1; }
done

echo "PASS: ship executes all stages, resumes without rebuilding, and fails loudly on a refused publish"
