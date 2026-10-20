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
# Windows `find`/`sort` on PATH cannot answer instead.
[ -d "/c/Program Files/Git/usr/bin" ] && PATH="/c/Program Files/Git/usr/bin:$PATH"

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

# The `gh` CLI, authenticated, and the repo it should talk to.
GH_REPO=${GH_REPO:-weironz/mica}
require_gh() {
  command -v gh >/dev/null 2>&1 || fail "the GitHub CLI 'gh' is not installed (https://cli.github.com)"
  gh auth status >/dev/null 2>&1 || fail "gh is not logged in — run 'gh auth login'"
}

dist_dir() { echo "dist/v$1"; }
