# Shared by build-linux-deb.sh / build-android-apk.sh: run the inner package
# build natively or inside the pinned builder image, the same way on every host.
#
#   MICA_CLIENT_BUILD=docker|native   force a mode (default: docker when the
#                                     daemon answers, else native if possible)
#   MICA_BUILDER_BASE=<image>         another source for ubuntu:22.04 when the
#                                     default mirror rate-limits you
#   MICA_BUILDER_CA_CERT=<file.crt>   extra CA(s) to trust inside the builder,
#                                     one PEM or a bundle (TLS-intercepting proxy)
#
# Docker is the reference path: Ubuntu 22.04 + the Flutter in .fvmrc + the
# Rust in rust-toolchain.toml, identical on a Windows box (Docker Desktop, Git
# Bash) and a Linux one, and old enough a glibc that the .deb installs on
# Ubuntu 22.04 / Debian 12 and newer. Native is a convenience for a host that
# already has the toolchain; its .deb is only as portable as that host.

client_build_mode() {
  local native_ok=$1 mode=${MICA_CLIENT_BUILD:-auto}
  case "$mode" in
    docker | native) echo "$mode"; return ;;
    auto) ;;
    *) fail "MICA_CLIENT_BUILD must be docker or native, not '$mode'" ;;
  esac
  if docker info >/dev/null 2>&1; then echo docker
  elif [ "$native_ok" = 1 ]; then echo native
  else fail "neither Docker nor a native toolchain is available (see docs/ops/release.md「Android / Linux 安装包」)"
  fi
}

# A Windows path Docker Desktop understands, from a Git Bash one.
docker_host_path() {
  local p; p=$(realpath -m "$1")
  case "$p" in /[a-z]/*) p="$(echo "${p:1:1}" | tr a-z A-Z):${p:2}" ;; esac
  echo "$p"
}

# Build (once per toolchain/Dockerfile change) and return the image tag for a
# Dockerfile target: `linux` or `android`.
client_builder_image() {
  local target=$1 flutter rust hash tag ca=${MICA_BUILDER_CA_CERT:-}
  flutter=$(sed -n 's/.*"flutter": *"\([^"]*\)".*/\1/p' .fvmrc)
  rust=$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml)
  [ -n "$flutter" ] && [ -n "$rust" ] || fail "cannot read the Flutter (.fvmrc) / Rust (rust-toolchain.toml) pins"
  [ -z "$ca" ] || [ -f "$ca" ] || fail "MICA_BUILDER_CA_CERT=$ca is not a file"
  hash=$(cat deploy/Dockerfile.client-build .fvmrc rust-toolchain.toml ${ca:+"$ca"} | sha256sum | cut -c1-12)
  tag="mica-client-build:$target-$hash"
  if ! docker image inspect "$tag" >/dev/null 2>&1; then
    # stderr: this function's stdout IS its result (the tag).
    note "building $tag (Flutter $flutter, Rust $rust) — first time only" >&2
    # A throwaway context: the Dockerfile plus extra-ca/, which holds the
    # MICA_BUILDER_CA_CERT certificate when set (a TLS-intercepting corporate
    # proxy) and is empty otherwise.
    local ctx base_arg=()
    ctx=$(mktemp -d)
    mkdir "$ctx/extra-ca"
    cp deploy/Dockerfile.client-build "$ctx/Dockerfile"
    # One certificate per file: update-ca-certificates reads only the first
    # of a bundle, and a proxy chain can need several roots.
    # Concatenated PEMs often glue an END line to the next BEGIN (a file
    # without a trailing newline), so break after every END first.
    [ -n "$ca" ] && sed 's/-----END CERTIFICATE-----/&\n/g' "$ca" \
      | awk -v d="$ctx/extra-ca" '/BEGIN CERTIFICATE/{n++} n && NF{print > (d "/extra-" n ".crt")}'
    [ -n "${MICA_BUILDER_BASE:-}" ] && base_arg=(--build-arg "BASE=$MICA_BUILDER_BASE")
    docker build --target "$target" --build-arg "FLUTTER_VERSION=$flutter" \
      --build-arg "RUST_VERSION=$rust" "${base_arg[@]}" -t "$tag" "$(docker_host_path "$ctx")" >&2 \
      || { rm -rf "$ctx"; fail "building $tag failed (registry rate limit? MICA_BUILDER_BASE; TLS-intercepting proxy? MICA_BUILDER_CA_CERT)"; }
    rm -rf "$ctx"
  fi
  echo "$tag"
}

# Run the inner build for <kind> in the builder image. The checkout is mounted
# read-only and copied FRESH into /work/src each run (tracked + untracked
# files, never ignored build output), so the container never writes the host's
# .dart_tool / build / target — those hold absolute paths that differ between
# the two — and a file deleted on the host cannot linger in the copy. Build
# caches persist in a named volume, linked back in at the same paths every run,
# so the second build is incremental. The old Android debug-key volume remains
# available for debug builds and recovering a previous signer. Release keys are
# passed separately read-only and are never copied into source or cache volumes.
client_build_docker() {
  local kind=$1 version=$2 out=$3 target=$4 image repo outdir
  local signing_args=()
  if [ "$kind" = apk ]; then
    source scripts/android-signing.sh
    require_android_signing
    signing_args=(
      -v "$(docker_host_path "$MICA_ANDROID_KEYSTORE_PATH"):/signing/release.keystore:ro"
      -e MICA_ANDROID_KEYSTORE_PATH=/signing/release.keystore
      -e MICA_ANDROID_KEY_ALIAS -e MICA_ANDROID_STORE_PASSWORD -e MICA_ANDROID_KEY_PASSWORD
      -e MICA_ANDROID_STORE_TYPE
    )
  fi
  # Checked, not trusted: `set -e` does not reach into $(...).
  image=$(client_builder_image "$target") && [ -n "$image" ] || exit 1
  repo=$(docker_host_path .)
  mkdir -p "$out"
  outdir=$(docker_host_path "$out")
  MSYS_NO_PATHCONV=1 docker run --rm "${signing_args[@]}" \
    -v "$repo:/src:ro" -v "$outdir:/out" \
    -v mica-client-cargo:/root/.cargo/registry \
    -v mica-client-pub:/root/.pub-cache \
    -v mica-client-gradle:/root/.gradle \
    -v mica-client-android-key:/root/.android \
    -v "mica-client-cache-$target:/cache" \
    -e "HOST_UID=$(id -u 2>/dev/null || echo 0)" -e "HOST_GID=$(id -g 2>/dev/null || echo 0)" \
    "$image" bash -ec '
      git config --global --add safe.directory "*"
      mkdir -p /work/src
      git -C /src ls-files -co --exclude-standard -z \
        | tar -C /src --null -T - -cf - | tar -C /work/src -xf -
      cd /work/src
      for d in clients/mica_flutter/build clients/mica_flutter/.dart_tool \
               clients/mica_flutter/android/.gradle target; do
        mkdir -p "/cache/$d" "$(dirname "$d")"; rm -rf "$d"; ln -s "/cache/$d" "$d"
      done
      bash scripts/client-package-inner.sh '"$kind"' '"$version"' /out
      chown -R "$HOST_UID:$HOST_GID" /out 2>/dev/null || true'
}
