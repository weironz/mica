#!/usr/bin/env bash
# Exercise real download/check/execute gates with disposable release fixtures.
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
mkdir -p "$fixture/bin" "$fixture/assets"
printf '#!/usr/bin/env bash\nprintf "mica-cli %%s\\n" "${CANARY_VERSION:-1.2.3}"\n' > "$fixture/assets/mica-cli-1.2.3-linux-x64"
chmod +x "$fixture/assets/mica-cli-1.2.3-linux-x64"
(cd "$fixture/assets" && sha256sum mica-cli-1.2.3-linux-x64 > sha256-linux.txt)
cat > "$fixture/bin/gh" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
case "$*" in
  'auth status') exit 0 ;;
  *'--json isDraft'*) echo "${CANARY_DRAFT:-false}" ;;
  *'--json assets'*) if [ "${CANARY_WINDOWS:-}" = 1 ]; then printf '%s\n' Mica-Setup-1.2.3.exe mica-cli-1.2.3-windows-x64.exe; else echo 'mica-cli-1.2.3-linux-x64'; fi ;;
  *'release download'*)
    while [ "$#" -gt 0 ]; do
      if [ "$1" = --dir ]; then out=$2; break; fi
      shift
    done
    cp "$CANARY_FIXTURE/assets/"* "$out/"
    ;;
  *) exit 3 ;;
esac
STUB
cat > "$fixture/bin/git" <<'STUB'
#!/usr/bin/env bash
case "$1" in rev-parse) exit 0 ;; cat-file) [ "${OLD_CANARY_CONTRACT:-0}" != 1 ] ;; *) exit 3 ;; esac
STUB
chmod +x "$fixture/bin/gh" "$fixture/bin/git"
export CANARY_FIXTURE="$fixture" PATH="$fixture/bin:$PATH"
check() {
  local expected=$1 label=$2 rc=0
  shift 2
  env "$@" bash "$repo/scripts/release-canary.sh" 1.2.3 > "$fixture/output" 2>&1 || rc=$?
  if [ "$expected" = pass ]; then
    [ "$rc" -eq 0 ] && grep -q 'Published artifact canary: ALL GREEN' "$fixture/output" || { cat "$fixture/output"; exit 1; }
  else
    [ "$rc" -ne 0 ] || { echo "FAIL: $label accepted"; exit 1; }
  fi
  echo "PASS: $label"
}
check pass 'downloaded binary is executed and matches the release version'
check fail 'wrong delivered binary version' CANARY_VERSION=9.9.9
check fail 'draft release is refused' CANARY_DRAFT=true
check fail 'old desktop contract is refused' OLD_CANARY_CONTRACT=1
printf '%064d  mica-cli-1.2.3-linux-x64\n' 0 > "$fixture/assets/sha256-linux.txt"
check fail 'corrupt delivered checksum'
printf '%064d  wrong-asset\n' 0 > "$fixture/assets/sha256-linux.txt"
check fail 'manifest must cover the actual executable'

# Independent hosted Windows producers: installer and CLI own different manifests.
cp "$fixture/assets/mica-cli-1.2.3-linux-x64" "$fixture/assets/mica-cli-1.2.3-windows-x64.exe"
printf 'installer fixture\n' > "$fixture/assets/Mica-Setup-1.2.3.exe"
(cd "$fixture/assets" && sha256sum mica-cli-1.2.3-windows-x64.exe > sha256-windows.txt && sha256sum Mica-Setup-1.2.3.exe > sha256-windows-installer.txt)
cat > "$fixture/bin/uname" <<'STUB'
#!/usr/bin/env bash
if [ "${CANARY_WINDOWS:-}" = 1 ]; then echo MINGW64_NT; else /usr/bin/uname "$@"; fi
STUB
cat > "$fixture/bin/cygpath" <<'STUB'
#!/usr/bin/env bash
echo "$2"
STUB
cat > "$fixture/bin/powershell" <<'STUB'
#!/usr/bin/env bash
case "$*" in *'-ExpectedVersion 1.2.3'*) exit 0 ;; *) exit 19 ;; esac
STUB
chmod +x "$fixture/bin/"*
check fail 'Windows installer is not run on a developer host' CANARY_WINDOWS=1 GITHUB_ACTIONS=false RUNNER_ENVIRONMENT=local
check pass 'independent Windows installer and CLI producer manifests' CANARY_WINDOWS=1 GITHUB_ACTIONS=true RUNNER_ENVIRONMENT=github-hosted
rm "$fixture/assets/sha256-windows-installer.txt"
check fail 'missing installer manifest is refused despite a valid CLI manifest' CANARY_WINDOWS=1 GITHUB_ACTIONS=true RUNNER_ENVIRONMENT=github-hosted
