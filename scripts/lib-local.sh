# Shared helpers for local build / release scripts. Sourced, not run.

fail() { printf '\n  REFUSED: %s\n' "$*" >&2; exit 1; }
note() { printf '==> %s\n' "$*"; }

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

check_version_arg() {
  [[ "${1:-}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] \
    || fail "want X.Y.Z with no leading v (got: ${1:-<nothing>})"
}

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

GH_REPO=${GH_REPO:-weironz/mica}
require_gh() {
  command -v gh >/dev/null 2>&1 || fail "the GitHub CLI 'gh' is not installed (https://cli.github.com)"
  gh auth status >/dev/null 2>&1 || fail "gh is not logged in — run 'gh auth login'"
}

require_gh_api() {
  command -v gh >/dev/null 2>&1 || fail "the GitHub CLI 'gh' is not installed (https://cli.github.com)"
  gh api "repos/$GH_REPO" --jq .full_name >/dev/null 2>&1 \
    || fail "gh cannot call the GitHub API for $GH_REPO from this shell."
}

find_iscc() {
  local iscc=${ISCC:-"/c/Program Files (x86)/Inno Setup 6/ISCC.exe"}
  if [ -z "${ISCC:-}" ] && [ ! -x "$iscc" ] && [ -n "${LOCALAPPDATA:-}" ]; then
    iscc="$(cygpath -u "$LOCALAPPDATA")/Programs/Inno Setup 6/ISCC.exe"
  fi
  [ -x "$iscc" ] && printf '%s\n' "$iscc"
}

dist_dir() { echo "dist/v$1"; }

manifest_for_asset() {
  case "${1:-}" in
    Mica-*-linux-*.deb) echo sha256-linux-desktop.txt ;;
    *-linux-*) echo sha256-linux.txt ;;
    *-macos-*) echo sha256-macos.txt ;;
    Mica-Setup-*) echo sha256-windows-installer.txt ;;
    *-windows-*) echo sha256-windows.txt ;;
    *) return 1 ;;
  esac
}
