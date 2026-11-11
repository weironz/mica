#!/usr/bin/env bash
# Download a PUBLISHED release into a clean directory and run the delivered files.
# Windows installer execution is restricted to a disposable hosted runner.
# --site is a separate, AFTER-DEPLOY check: publishing does not deploy production.
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh
version=${1:?usage: release-canary.sh X.Y.Z [--site https://site]}
check_version_arg "$version"
shift
site=
if [ "$#" -gt 0 ]; then
  [ "$#" -eq 2 ] && [ "$1" = --site ] || fail "want --site <origin>"
  site=$2
  [[ "$site" =~ ^https?://[^/]+(/)?$ ]] || fail "site must be an http(s) origin"
fi
require_gh
tag="v$version"
if ! git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
  git fetch origin "refs/tags/$tag:refs/tags/$tag"
fi
git cat-file -e "$tag:scripts/release-canary.sh" 2>/dev/null \
  || fail "$tag predates the release canary contract; old desktop versions are not silently accepted"
[ "$(gh release view "v$version" --repo "$GH_REPO" --json isDraft --jq .isDraft)" = false ] \
  || fail "canary requires a published release, never a draft or working tree"
work=$(mktemp -d)
pid=
cleanup() {
  if [ -n "$pid" ]; then kill -- "-$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
  rm -rf "$work"
}
trap cleanup EXIT
assets=$(gh release view "v$version" --repo "$GH_REPO" --json assets --jq '.assets[].name')
fetch() {
  local asset=$1 manifest digest actual
  manifest=$(manifest_for_asset "$asset")
  gh release download "v$version" --repo "$GH_REPO" --pattern "$asset" --pattern "$manifest" --dir "$work" --clobber
  digest=$(awk -v name="$asset" '$2 == name || $2 == "*"name {print $1}' "$work/$manifest")
  [[ "$digest" =~ ^[0-9a-fA-F]{64}$ ]] || fail "$asset missing from checksum manifest"
  actual=$(sha256sum "$work/$asset" | cut -d' ' -f1)
  [ "${digest,,}" = "$actual" ] || fail "$asset checksum mismatch"
}
assert_version() {
  local got=$1
  # Exact token, not a substring (1.2.3 must not accept 1.2.30).
  printf '%s\n' "$got" | tr ' ' '\n' | grep -Fx "$version" > /dev/null \
    || fail "delivered executable reports '$got', expected $version"
}
case "$(host_os)" in
  linux)
    cli="mica-cli-$version-linux-x64"
    fetch "$cli"
    chmod +x "$work/$cli"
    assert_version "$(XDG_CONFIG_HOME="$work/profile" timeout 15 "$work/$cli" --version)"
    deb="Mica-$version-linux-amd64.deb"
    if grep -Fx "$deb" <<< "$assets" > /dev/null; then
      fetch "$deb"
      assert_version "$(dpkg-deb -f "$work/$deb" Version)"
      mkdir -p "$work/install" "$work/profile"
      dpkg-deb -x "$work/$deb" "$work/install"
      app="$work/install/opt/mica/mica_flutter"
      assert_version "$(XDG_CONFIG_HOME="$work/profile" timeout 15 "$app" --version)"
      command -v xvfb-run >/dev/null || fail "Linux desktop canary needs xvfb-run and the .deb runtime libraries"
      receipt="$work/first-frame.marker"
      XDG_CONFIG_HOME="$work/profile" MICA_STARTUP_PROBE_FILE="$receipt" \
        setsid xvfb-run -a "$app" > "$work/desktop.log" 2>&1 &
      pid=$!
      for _ in $(seq 1 120); do
        if [ -s "$receipt" ]; then break; fi
        kill -0 "$pid" 2>/dev/null || { cat "$work/desktop.log"; fail "delivered .deb exited before first frame"; }
        sleep 0.25
      done
      [ -s "$receipt" ] || { cat "$work/desktop.log"; fail "delivered .deb produced no first frame in 30 seconds"; }
      sleep 10
      kill -0 "$pid" 2>/dev/null || { cat "$work/desktop.log"; fail "delivered .deb exited after its first frame"; }
      note "delivered .deb version, first frame and 10-second survival: OK"
    else
      note "No .deb attached: optional desktop package was not built; desktop canary not claimed"
    fi
    ;;
  windows)
    [ "${GITHUB_ACTIONS:-}" = true ] && [ "${RUNNER_ENVIRONMENT:-}" = github-hosted ] \
      || fail "Windows installation canary must run on a disposable GitHub-hosted runner"
    cli="mica-cli-$version-windows-x64.exe"
    setup="Mica-Setup-$version.exe"
    fetch "$cli"; fetch "$setup"
    assert_version "$(timeout 15 "$work/$cli" --version)"
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts/smoke-installer.ps1 -Setup "$(cygpath -w "$work/$setup")" -ExpectedVersion "$version"
    ;;
  *) fail "release canary supports Linux and disposable hosted Windows" ;;
esac
if [ -n "$site" ]; then
  bash scripts/release-site-canary.sh "$version" "$site"
fi
note "Published artifact canary: ALL GREEN for this platform's delivered assets"
