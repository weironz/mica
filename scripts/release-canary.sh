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
fetch_from_release() {
  local rel_tag=$1 asset=$2 dest_dir=$3 manifest digest actual
  manifest=$(manifest_for_asset "$asset")
  mkdir -p "$dest_dir"
  gh release download "$rel_tag" --repo "$GH_REPO" --pattern "$asset" --pattern "$manifest" --dir "$dest_dir" --clobber
  digest=$(awk -v name="$asset" '$2 == name || $2 == "*"name {print $1}' "$dest_dir/$manifest")
  [[ "$digest" =~ ^[0-9a-fA-F]{64}$ ]] || fail "$asset missing from checksum manifest ($rel_tag/$manifest)"
  actual=$(sha256sum "$dest_dir/$asset" | cut -d' ' -f1)
  [ "${digest,,}" = "$actual" ] || fail "$asset checksum mismatch ($rel_tag)"
}
fetch() {
  fetch_from_release "v$version" "$1" "$work"
}
find_previous_windows_release() {
  local cur=$1
  if [ -n "${CANARY_UPGRADE_FROM:-}" ]; then
    check_version_arg "$CANARY_UPGRADE_FROM"
    printf '%s\n' "$CANARY_UPGRADE_FROM"
    return 0
  fi
  local tags cand cand_ver cand_assets
  tags=$(gh release list --repo "$GH_REPO" --exclude-drafts --exclude-pre-releases --limit 30 --json tagName --jq '.[].tagName' \
    | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' \
    | sed 's/^v//' \
    | sort -rV)
  while IFS= read -r cand_ver; do
    [ -n "$cand_ver" ] || continue
    [ "$cand_ver" != "$cur" ] || continue
    # Keep only versions strictly older than $cur (smallest sorts first under sort -V).
    if [ "$(printf '%s\n%s\n' "$cand_ver" "$cur" | sort -V | head -n1)" != "$cand_ver" ]; then
      continue
    fi
    cand_assets=$(gh release view "v$cand_ver" --repo "$GH_REPO" --json assets --jq '.assets[].name' 2>/dev/null || true)
    if grep -Fx "Mica-Setup-$cand_ver.exe" <<< "$cand_assets" >/dev/null \
      && grep -Fx "sha256-windows-installer.txt" <<< "$cand_assets" >/dev/null; then
      printf '%s\n' "$cand_ver"
      return 0
    fi
  done <<< "$tags"
  return 0
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
      app="$work/install/usr/bin/mica"
      desktop_version=$(XDG_CONFIG_HOME="$work/profile" timeout 15 "$app" --version)
      desktop_suffix=${desktop_version#"Mica $version"}
      [[ "$desktop_version" != "$desktop_suffix" && ( -z "$desktop_suffix" || "$desktop_suffix" =~ ^\+[0-9]+$ ) ]] \
        || fail "delivered desktop reports '$desktop_version', expected Mica $version with optional build number"
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
    prev_version=$(find_previous_windows_release "$version")
    if [ -n "$prev_version" ]; then
      prev_setup="Mica-Setup-$prev_version.exe"
      note "Windows in-place upgrade canary: $prev_version -> $version"
      fetch_from_release "v$prev_version" "$prev_setup" "$work/prev"
      powershell -NoProfile -ExecutionPolicy Bypass -File scripts/smoke-installer.ps1 \
        -Setup "$(cygpath -w "$work/$setup")" \
        -ExpectedVersion "$version" \
        -UpgradeFrom "$(cygpath -w "$work/prev/$prev_setup")" \
        -UpgradeFromVersion "$prev_version"
      note "Windows in-place upgrade canary ($prev_version -> $version): OK"
    else
      note "No prior published Windows installer release found before $version; skipping in-place upgrade canary"
    fi
    ;;
  *) fail "release canary supports Linux and disposable hosted Windows" ;;
esac
if [ -n "$site" ]; then
  bash scripts/release-site-canary.sh "$version" "$site"
fi
note "Published artifact canary: ALL GREEN for this platform's delivered assets"
