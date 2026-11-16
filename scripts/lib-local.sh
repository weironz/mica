# Shared helpers for the LOCAL build / release scripts (scripts/local-*.sh,
# scripts/release-{artifacts,upload,publish,images}.sh). Sourced, not run.
#
# Why these exist at all: GitHub Actions minutes run out, and then the whole
# release train — tests, the Windows installer, the CLI binaries, the images,
# the GitHub release — has to be drivable from a developer machine. Linux and
# Windows (Git Bash, via the justfile's `windows-shell`) both run these.

fail() { printf '\n  REFUSED: %s\n' "$*" >&2; exit 1; }
note() { printf '==> %s\n' "$*"; }

# Git Bash ships the coreutils these scripts use; put them first so a stray
# Windows `find`/`sort` on PATH cannot answer instead. Only when one does: an
# unconditional prepend also shadowed the command stubs the script self-tests
# put at the front of PATH (`uname`, `file`), so they failed on Windows hosts.
if [ -d "/c/Program Files/Git/usr/bin" ]; then
  case "$(command -v find) $(command -v sort)" in
    */[Ss]ystem32/*) PATH="/c/Program Files/Git/usr/bin:$PATH" ;;
  esac
fi

# linux | windows | macos — what this machine can build natively.
host_os() {
  case "$(uname -s)" in
    Linux) echo linux ;;
    Darwin) echo macos ;;
    MINGW* | MSYS* | CYGWIN*) echo windows ;;
    *) echo unknown ;;
  esac
}

FLUTTER=${FLUTTER:-flutter}

check_version_arg() {
  [[ "${1:-}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] \
    || fail "want X.Y.Z with no leading v (got: ${1:-<nothing>})"
}

# The artifacts must come from the tree the tag names — the local twin of the
# "version must match the tree it is built from" step in release.yml. Building
# 0.13.29 from a checkout that says 0.13.28 ships an installer the in-app
# updater either never offers or offers forever.
require_tagged_tree() {
  local version=$1 tag="v$1"
  local got
  got=$(bash scripts/manifest-version.sh)
  [ "$got" = "$version" ] || fail "asked for $version but this tree says $got — check out $tag first"
  git rev-parse -q --verify "refs/tags/$tag" >/dev/null \
    || fail "tag $tag does not exist locally (run 'just release $version' first, or 'git fetch --tags')"
  [ "$(git rev-parse HEAD)" = "$(git rev-parse "$tag^{commit}")" ] \
    || fail "HEAD is not $tag — artifacts must be built from exactly the tagged commit (git checkout $tag)"
  git diff --quiet && git diff --cached --quiet \
    || fail "the working tree is dirty — artifacts must be built from the tagged commit as committed"
}

# A DATABASE_URL whose Postgres is reachable, or empty. The DB-backed tests
# SKIP without one and a skipped test reports as passed, so callers that gate
# on tests must refuse when this comes back empty.
#   1. $DATABASE_URL if it answers;
#   2. the dev stack's container (`just dev`).
resolve_database_url() {
  if [ -n "${DATABASE_URL:-}" ]; then
    if command -v pg_isready >/dev/null 2>&1 && pg_isready -q -d "$DATABASE_URL"; then
      echo "$DATABASE_URL"; return
    fi
    if command -v psql >/dev/null 2>&1 && psql "$DATABASE_URL" -tAc 'select 1' >/dev/null 2>&1; then
      echo "$DATABASE_URL"; return
    fi
  fi
  if docker exec mica-postgres pg_isready -U mica -d mica >/dev/null 2>&1; then
    echo "postgres://mica:mica@127.0.0.1:5432/mica"; return
  fi
  echo ""
}

# The Flutter pin CI reads — .fvmrc's "flutter" version (every flutter-action
# step in the workflows resolves this file; local scripts must agree with it).
# The parsing used to live in local-ci.sh and release-check.sh separately (#88).
flutter_pin() {
  sed -n 's/.*"flutter"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' .fvmrc
}

# Refuse unless the local Flutter IS the pin. The "why" is the caller's —
# `just ci` tests the pin, release-check builds what it ships with it.
require_flutter_pin() {
  local want have
  want=$(flutter_pin)
  [ -n "$want" ] || fail ".fvmrc has no \"flutter\" version — that file is the pin CI reads"
  have=$("$FLUTTER" --version 2>/dev/null | sed -n 's/^Flutter \([0-9][^ ]*\).*/\1/p' | head -1)
  [ "$have" = "$want" ] || fail "local Flutter is '$have', the pin is $want (.fvmrc) — $*"
  note "flutter $have (matches .fvmrc)"
}

# The `gh` CLI, authenticated, and the repo it should talk to.
GH_REPO=${GH_REPO:-weironz/mica}
require_gh() {
  command -v gh >/dev/null 2>&1 || fail "the GitHub CLI 'gh' is not installed (https://cli.github.com)"
  gh auth status >/dev/null 2>&1 || fail "gh is not logged in — run 'gh auth login'"
}

# `gh auth status` reads a cached login. A background shell whose keyring is
# locked still passes it and then fails the first real call — 0.14.8's release
# stalled on exactly that, deep into the train. Ask the API itself.
require_gh_api() {
  command -v gh >/dev/null 2>&1 || fail "the GitHub CLI 'gh' is not installed (https://cli.github.com)"
  gh api "repos/$GH_REPO" --jq .full_name >/dev/null 2>&1 \
    || fail "gh cannot call the GitHub API for $GH_REPO from this shell (not logged in, keyring locked in a background session, or no network). 'gh api repos/$GH_REPO' must work."
}

# Release acceptance for the browser-only Flutter suites: the commit status
# `mica/browser-tests` on the release commit, written by
# `scripts/browser-tests.sh --report-status` (the Browser tests workflow, or any
# Linux/macOS machine). Windows cannot produce it locally (#181).
BROWSER_STATUS_CONTEXT=mica/browser-tests

browser_status() { # $1 = commit sha -> success|pending|failure|error|"" (none)
  gh api "repos/$GH_REPO/commits/$1/status" \
    --jq ".statuses[] | select(.context == \"$BROWSER_STATUS_CONTEXT\") | .state" 2>/dev/null \
    | head -1
}

report_browser_status() { # $1 = commit sha, $2 = description
  gh api -X POST "repos/$GH_REPO/statuses/$1" \
    -f state=success -f context="$BROWSER_STATUS_CONTEXT" -f description="$2" >/dev/null \
    || fail "could not record $BROWSER_STATUS_CONTEXT on $1 (is the commit pushed? does the token have statuses:write?)"
  note "recorded $BROWSER_STATUS_CONTEXT = success on ${1:0:12}"
}

require_browser_status() { # $1 = commit sha, $2 = version (for the hint)
  local state
  state=$(browser_status "$1")
  [ "$state" = success ] || fail "the browser-only Flutter suites have not passed on ${1:0:12} (status $BROWSER_STATUS_CONTEXT: ${state:-none}).
  They are release acceptance (docs/ops/release.md) and Windows cannot run them (#181). Either:
    gh workflow run browser-tests.yml --repo $GH_REPO -f ref=v$2     # hosted, ~10 min
  or, on any Linux/macOS checkout of v$2:
    just browser-tests-report $2"
}

# Inno Setup's compiler, where release-artifacts looks for it. Empty if absent.
find_iscc() {
  local iscc=${ISCC:-"/c/Program Files (x86)/Inno Setup 6/ISCC.exe"}
  if [ -z "${ISCC:-}" ] && [ ! -x "$iscc" ] && [ -n "${LOCALAPPDATA:-}" ]; then
    iscc="$(cygpath -u "$LOCALAPPDATA")/Programs/Inno Setup 6/ISCC.exe"
  fi
  [ -x "$iscc" ] && printf '%s\n' "$iscc"
}

dist_dir() { echo "dist/v$1"; }

# Which checksum manifest an asset is verified against. The in-app updater
# fetches sha256-<os>.txt on its own platform and refuses a binary that file
# does not cover (crates/cli/src/update.rs), so the NAME is a compatibility
# contract: one manifest per platform, listing only that platform's files.
#
# A host can build another platform's asset (a Windows box cross-builds the
# Linux CLI in Docker), and that is exactly how the Linux half of a release
# went unverifiable: the only manifest written was the HOST's (#113).
manifest_for_asset() {
  case "${1:-}" in
    # The opt-in client packages get manifests of their own: sha256-linux.txt
    # is the CLI updater's contract, and the release workflow's deb job must
    # not write the asset the cli job writes.
    Mica-*-linux-*.deb) echo sha256-linux-desktop.txt ;;
    Mica-*-android-*.apk) echo sha256-android.txt ;;
    *-linux-*) echo sha256-linux.txt ;;
    *-macos-*) echo sha256-macos.txt ;;
    # Independent producer: the installer job must not overwrite the CLI job's manifest.
    Mica-Setup-*) echo sha256-windows-installer.txt ;;
    *-windows-*) echo sha256-windows.txt ;;
    *) return 1 ;;
  esac
}
