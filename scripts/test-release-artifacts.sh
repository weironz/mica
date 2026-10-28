#!/usr/bin/env bash
# Exercise the real scripts/release-artifacts.sh and scripts/release-publish.sh
# inside disposable Git fixtures, with every external command they touch
# (builds, Docker, Inno Setup, PowerShell, gh) replaced by a recording stub.
#
# The regression: a Windows host builds the Linux CLI too, but only wrote
# sha256-windows.txt — which listed the Linux file as well. The Linux updater
# asks for sha256-linux.txt by name (crates/cli/src/update.rs), so that release
# could never be verified on Linux, and nothing before publishing noticed.
set -euo pipefail
source_root=$(cd "$(dirname "$0")/.." && pwd)
temp_parent=$(cd "${TMPDIR:-/tmp}" && pwd -P)
fixture_root=$(mktemp -d "$temp_parent/mica-release-artifacts.XXXXXX")
# Only remove the directory this invocation just created under the resolved temp
# directory, never a repository/worktree or a path supplied by a caller.
[[ "$fixture_root" == "$temp_parent"/mica-release-artifacts.* ]] || exit 2
trap 'rm -rf -- "$fixture_root"' EXIT

version=0.13.69
stub_bin="$fixture_root/bin"
repo="$fixture_root/repo"
mkdir -p "$stub_bin" "$repo/scripts" "$repo/clients/mica_flutter/lib" "$repo/clients/mica_flutter/installer"

# ── stubs ────────────────────────────────────────────────────────────────────
cat > "$stub_bin/uname" <<'EOF'
#!/bin/sh
echo 'MINGW64_NT-10.0-26100'
EOF

# Some coreutils ports (the Windows build this train runs on) append GNU's
# binary-mode marker to the filename: `<hex> *<name>`. Both release-publish and
# the in-app updater match the name exactly, so the manifests must come out
# canonical whatever flavour wrote them — 0.13.69 was refused at publish time
# for inheriting the `*`. Hash with the real tool, mark the way that port does.
real_sha256sum=$(command -v sha256sum)
cat > "$stub_bin/sha256sum" <<EOF
#!/bin/sh
"$real_sha256sum" "\$@" | awk '{ printf "%s *%s\\n", \$1, \$2 }'
EOF

cat > "$stub_bin/powershell" <<'EOF'
#!/bin/sh
exit 0
EOF

cat > "$stub_bin/flutter" <<'EOF'
#!/bin/sh
exit 0
EOF

cat > "$stub_bin/rustup" <<'EOF'
#!/bin/sh
exit 0
EOF

# `cargo build -p mica-cli --release` -> the binary build_cli_native copies.
cat > "$stub_bin/cargo" <<'EOF'
#!/bin/sh
mkdir -p target/release
printf 'windows-cli-fixture\n' > target/release/mica-cli.exe
printf 'windows-cli-fixture\n' > target/release/mica-cli
EOF

# `docker info` must answer, and the musl build must land where the script
# copies it from. Nothing here talks to a daemon.
cat > "$stub_bin/docker" <<'EOF'
#!/bin/sh
[ "$1" = info ] && exit 0
[ "$1" = run ] || exit 0
mkdir -p target/docker-musl/x86_64-unknown-linux-musl/release
printf 'linux-cli-fixture\n' > target/docker-musl/x86_64-unknown-linux-musl/release/mica-cli
EOF

# The static-link assertion reads `file`; the fixture binaries are shell
# scripts, so the stub answers the question the script asks.
cat > "$stub_bin/file" <<'EOF'
#!/bin/sh
echo "$1: ELF 64-bit LSB executable, x86-64, statically linked, stripped"
EOF

# Inno Setup writes the installer under the name the script copies from.
iscc="$fixture_root/ISCC-stub"
cat > "$iscc" <<'EOF'
#!/bin/sh
version=unknown
for arg in "$@"; do
  case "$arg" in //DAppVersion=*) version=${arg#//DAppVersion=} ;; esac
done
mkdir -p clients/mica_flutter/installer/Output
printf 'installer-fixture\n' > "clients/mica_flutter/installer/Output/Mica-Setup-$version.exe"
EOF

chmod +x "$stub_bin"/* "$iscc"

# ── fixture repo ─────────────────────────────────────────────────────────────
cp "$source_root/scripts/release-artifacts.sh" "$source_root/scripts/lib-local.sh" \
   "$source_root/scripts/manifest-version.sh" "$source_root/scripts/release-publish.sh" \
   "$repo/scripts/"
printf 'channel = "1.90.0"\n' > "$repo/rust-toolchain.toml"
printf '[package]\nname = "mica-cli"\nversion = "%s"\n' "$version" > "$repo/Cargo.toml"
printf 'version: %s\n' "$version" > "$repo/clients/mica_flutter/pubspec.yaml"
printf "const kAppVersion = '%s';\n" "$version" > "$repo/clients/mica_flutter/lib/main.dart"
printf '/target/\nclients/mica_flutter/installer/Output/\ndist/\n' > "$repo/.gitignore"

git -C "$repo" init -q
git -C "$repo" config user.name 'Release artifact fixture'
git -C "$repo" config user.email 'release-artifacts@example.invalid'
git -C "$repo" add -A
git -C "$repo" commit -qm fixture
git -C "$repo" tag "v$version"

out="$repo/dist/v$version"

# ── slice 1: one manifest per platform, each listing only its own files ──────
(
  cd "$repo"
  PATH="$stub_bin:$PATH" ISCC="$iscc" bash scripts/release-artifacts.sh "$version" --no-smoke
)
manifest_names() { sed 's/^[0-9a-f]*[[:space:]][[:space:]]//' "$1" | sort; }

[ -f "$out/sha256-windows.txt" ] || {
  echo 'FAIL: no sha256-windows.txt after a Windows-host release' >&2
  ls -l "$out" >&2
  exit 1
}
[ -f "$out/sha256-linux.txt" ] || {
  echo 'FAIL: the Windows host cross-built the Linux CLI but wrote no sha256-linux.txt — the Linux updater asks for that file by name' >&2
  ls -l "$out" >&2
  exit 1
}
[ ! -f "$out/sha256-macos.txt" ] || {
  echo 'FAIL: a manifest was written for a platform this run built nothing for' >&2
  exit 1
}

expected_windows=$'Mica-Setup-'"$version"$'.exe\nmica-cli-'"$version"'-windows-x64.exe'
expected_linux="mica-cli-$version-linux-x64"
[ "$(manifest_names "$out/sha256-windows.txt")" = "$expected_windows" ] || {
  echo 'FAIL: sha256-windows.txt must list the Windows assets and nothing else' >&2
  cat "$out/sha256-windows.txt" >&2
  exit 1
}
[ "$(manifest_names "$out/sha256-linux.txt")" = "$expected_linux" ] || {
  echo 'FAIL: sha256-linux.txt must list the Linux asset and nothing else' >&2
  cat "$out/sha256-linux.txt" >&2
  exit 1
}
# The stub `sha256sum` writes `<hex> *<name>`. A manifest that keeps that
# marker is unverifiable end to end, so assert the canonical shape directly
# rather than trusting manifest_names' stripping.
for m in "$out"/sha256-*.txt; do
  grep -q '\*' "$m" && {
    echo "FAIL: $m carries coreutils' binary-mode marker; publish and the updater" \
      'match the asset name exactly' >&2
    cat "$m" >&2
    exit 1
  }
done
# Recompute one hash the way the updater does, rather than trusting the file the
# script just wrote about itself.
[ "$(cut -d' ' -f1 "$out/sha256-linux.txt")" = "$(sha256sum "$out/mica-cli-$version-linux-x64" | cut -d' ' -f1)" ] || {
  echo 'FAIL: sha256-linux.txt does not carry the Linux binary hash' >&2
  exit 1
}
for m in "$out"/sha256-*.txt; do
  grep -q 'sha256-' "$m" && { echo "FAIL: $m lists a manifest as an asset" >&2; exit 1; }
done
[ -z "$(git -C "$repo" status --porcelain)" ] || {
  echo 'FAIL: the artifact run dirtied the tagged tree' >&2
  git -C "$repo" status --short >&2
  exit 1
}

# ── slice 1b: a failed build must not leave a stale manifest behind ──────────
printf 'deadbeef  mica-cli-%s-linux-x64\n' "$version" > "$out/sha256-linux.txt"
fail_iscc="$fixture_root/ISCC-fail"
printf '#!/bin/sh\nexit 1\n' > "$fail_iscc"; chmod +x "$fail_iscc"
if (cd "$repo" && PATH="$stub_bin:$PATH" ISCC="$fail_iscc" \
    bash scripts/release-artifacts.sh "$version" --no-smoke) >/dev/null 2>&1; then
  echo 'FAIL: a build whose installer compiler fails must not succeed' >&2
  exit 1
fi
[ ! -f "$out/sha256-linux.txt" ] || {
  echo 'FAIL: a failed run left a stale sha256-linux.txt behind' >&2
  exit 1
}

# ── slice 2: publishing verifies each asset against its manifest AND digest ──
gh_stub="$stub_bin/gh"
cat > "$gh_stub" <<'EOF'
#!/bin/sh
case "$*" in
  'auth status') exit 0 ;;
  *'--json assets'*)
    [ -n "${GH_STUB_ASSETS_TSV:-}" ] || exit 1
    cat "$GH_STUB_ASSETS_TSV"
    ;;
  *'release download'*)
    pattern=""; dir=""; prev=""
    for arg in "$@"; do
      case "$prev" in --pattern) pattern=$arg ;; --dir) dir=$arg ;; esac
      prev=$arg
    done
    mkdir -p "$dir"
    cp "${GH_STUB_FILES_DIR:?}/$pattern" "$dir/$pattern"
    ;;
  *'release edit'*)
    printf '%s\n' "$*" >> "${GH_STUB_LOG:-/dev/null}"
    ;;
  *'api '*)
    printf 'v%s\n' "${GH_STUB_VERSION:?}"
    ;;
esac
exit 0
EOF
chmod +x "$gh_stub"

assets_tsv="$fixture_root/assets.tsv"
files_dir="$fixture_root/files"
mkdir -p "$files_dir"
h_setup=1111111111111111111111111111111111111111111111111111111111111111
h_win=2222222222222222222222222222222222222222222222222222222222222222
h_lin=3333333333333333333333333333333333333333333333333333333333333333
h_mac=4444444444444444444444444444444444444444444444444444444444444444

printf '%s  %s\n' "$h_setup" "Mica-Setup-$version.exe" \
                   "$h_win" "mica-cli-$version-windows-x64.exe" \
  > "$files_dir/sha256-windows.txt"
printf '%s  %s\n' "$h_lin" "mica-cli-$version-linux-x64" > "$files_dir/sha256-linux.txt"
printf '%s  %s\n' "$h_mac" "mica-cli-$version-macos-arm64" > "$files_dir/sha256-macos.txt"

write_tsv() { # args are name:digest pairs; digest may be empty
  : > "$assets_tsv"
  for pair in "$@"; do
    printf '%s\t%s\n' "${pair%%:*}" "${pair#*:}"
  done >> "$assets_tsv"
}

publish() { # the real script, with the fixture's gh double and asset list
  (cd "$repo" && PATH="$stub_bin:$PATH" GH_STUB_ASSETS_TSV="$assets_tsv" \
     GH_STUB_FILES_DIR="$files_dir" GH_STUB_LOG="$fixture_root/gh.log" \
     GH_STUB_VERSION="$version" bash scripts/release-publish.sh "$version")
}

expect_publish_refusal() { # $1 = needle
  rm -f "$fixture_root/gh.log"
  local out status=0
  out=$(publish 2>&1) || status=$?
  [ "$status" != 0 ] || { echo "FAIL: publish accepted a release it should refuse ($1)" >&2; exit 1; }
  case "$out" in
    *"$1"*) ;;
    *) echo "FAIL: the refusal does not mention '$1': $out" >&2; exit 1 ;;
  esac
  [ ! -s "$fixture_root/gh.log" ] || {
    echo "FAIL: publish edited the release despite the refusal ($1)" >&2
    exit 1
  }
}

# Assets present, no manifests attached: refuse before any digest comparison.
write_tsv "Mica-Setup-$version.exe:$h_setup" "mica-cli-$version-windows-x64.exe:$h_win" \
          "mica-cli-$version-linux-x64:$h_lin"
expect_publish_refusal 'sha256-windows.txt'

# Complete and correct: publish. gh reports digests with the algorithm prefix
# (`sha256:<hex>`); comparing that verbatim against a bare-hex manifest refused
# a release whose two sides were identical (v0.13.69).
write_tsv "Mica-Setup-$version.exe:sha256:$h_setup" \
          "mica-cli-$version-windows-x64.exe:sha256:$h_win" \
          "mica-cli-$version-linux-x64:sha256:$h_lin" \
          "sha256-windows.txt:" "sha256-linux.txt:"
publish
[ -s "$fixture_root/gh.log" ] || { echo 'FAIL: publish did not edit a complete release' >&2; exit 1; }

# Manifest attached but its hash disagrees with the asset GitHub holds.
write_tsv "Mica-Setup-$version.exe:$h_setup" "mica-cli-$version-windows-x64.exe:$h_win" \
          "mica-cli-$version-linux-x64:9999999999999999999999999999999999999999999999999999999999999999" \
          "sha256-windows.txt:" "sha256-linux.txt:"
expect_publish_refusal 'stale'

# An attached macOS CLI is verifiable too: without its manifest, refuse.
write_tsv "Mica-Setup-$version.exe:$h_setup" "mica-cli-$version-windows-x64.exe:$h_win" \
          "mica-cli-$version-linux-x64:$h_lin" "mica-cli-$version-macos-arm64:$h_mac" \
          "sha256-windows.txt:" "sha256-linux.txt:"
expect_publish_refusal 'sha256-macos.txt'

# No server-side digest: the script must hash the attached asset itself.
printf 'linux-cli-fixture\n' > "$files_dir/mica-cli-$version-linux-x64"
actual_lin=$(sha256sum "$files_dir/mica-cli-$version-linux-x64" | cut -d' ' -f1)
printf '%s  %s\n' "$actual_lin" "mica-cli-$version-linux-x64" > "$files_dir/sha256-linux.txt"
write_tsv "Mica-Setup-$version.exe:$h_setup" "mica-cli-$version-windows-x64.exe:$h_win" \
          "mica-cli-$version-linux-x64:" "sha256-windows.txt:" "sha256-linux.txt:"
publish
[ -s "$fixture_root/gh.log" ] || { echo 'FAIL: publish did not verify a digest-less asset by hashing it' >&2; exit 1; }

echo "PASS: per-platform manifests for a cross-built CLI; publish verifies asset digests and refuses gaps"
