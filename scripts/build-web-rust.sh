#!/usr/bin/env bash
# Build Mica's pure-Rust WebAssembly client (`clients/mica_rust`) + lazy
# `mica_preview/{math,mermaid}` WASM modules into deploy/web/, optimize with
# `wasm-opt -Oz`, and enforce the Brotli-11 initial-bundle size gate.
set -euo pipefail
cd "$(dirname "$0")/.."

BUDGET_BYTES="${MICA_WEB_BUDGET_BYTES:-4194304}" # 4.0 MiB Brotli-11 hard gate (W5)

echo "==> Building mica-rust for wasm32-unknown-unknown (--profile web)..."
cargo build \
  --manifest-path clients/mica_rust/Cargo.toml \
  --lib \
  --target wasm32-unknown-unknown \
  --profile web \
  --locked

target_dir=$(cargo metadata --manifest-path clients/mica_rust/Cargo.toml --format-version 1 --no-deps --locked | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])' 2>/dev/null || echo "clients/mica_rust/target")
wasm_path="$target_dir/wasm32-unknown-unknown/web/mica_rust.wasm"
if [[ ! -f "$wasm_path" ]]; then
  wasm_path="clients/mica_rust/target/wasm32-unknown-unknown/web/mica_rust.wasm"
fi
if [[ ! -f "$wasm_path" ]]; then
  echo "FAIL: expected $wasm_path not found" >&2
  exit 1
fi

# Locate nsh.js shipped inside the locked nsh-platform-web crate.
nsh_web_manifest=$(cargo metadata --manifest-path clients/mica_rust/Cargo.toml --format-version 1 --locked | python3 -c '
import json, sys
meta = json.load(sys.stdin)
for p in meta["packages"]:
    if p["name"] == "nsh-platform-web":
        print(p["manifest_path"])
        break
' 2>/dev/null || true)

nsh_js=""
if [[ -n "$nsh_web_manifest" ]]; then
  cand="$(dirname "$nsh_web_manifest")/web/nsh.js"
  [[ -f "$cand" ]] && nsh_js="$cand"
fi
if [[ -z "$nsh_js" ]]; then
  cargo_home="${CARGO_HOME:-$HOME/.cargo}"
  nsh_js=$(find "$cargo_home/registry/src" -path "*/nsh-platform-web-0.6.1/web/nsh.js" 2>/dev/null | head -n 1 || true)
fi
if [[ -z "$nsh_js" || ! -f "$nsh_js" ]]; then
  echo "FAIL: could not locate nsh-platform-web web/nsh.js" >&2
  exit 1
fi

if ! command -v wasm-bindgen >/dev/null 2>&1; then
  echo "FAIL: wasm-bindgen CLI not found on PATH (install wasm-bindgen-cli matching clients/mica_rust/Cargo.lock)" >&2
  exit 1
fi

out="deploy/web"
mkdir -p "$out"
# Clear contents in place rather than deleting `$out` so Docker bind mounts stay attached.
find "$out" -mindepth 1 -delete
mkdir -p "$out/mica_preview"
cp clients/mica_rust/web/index.html "$out/index.html"
cp clients/mica_rust/web/favicon.svg "$out/favicon.svg"
cp clients/mica_rust/web/sw.js "$out/sw.js"
cp "$nsh_js" "$out/nsh.js"

# Generate wasm-bindgen glue for transitive crates and adapt to NSH's glue hook contract.
wasm-bindgen \
  --target web \
  --no-typescript \
  --remove-name-section \
  --remove-producers-section \
  --out-dir "$out" \
  --out-name mica_rust \
  "$wasm_path"

python3 - "$out/mica_rust.js" <<'PY'
import re, sys
path = sys.argv[1]
src = open(path, "r", encoding="utf-8").read()
src = re.sub(r'^import \* as import\d+ from "(?:nsh|env)";?\r?\n', '', src, flags=re.M)
src = re.sub(r'^\s*"(?:nsh|env)": import\d+,?\r?\n', '', src, flags=re.M)
if '__nsh_imports' not in src:
    src += (
        "\nexport function __nsh_imports() { return __wbg_get_imports(); }\n"
        "export function __nsh_finalize(instance, module) { return __wbg_finalize_init(instance, module); }\n"
    )
open(path, "w", encoding="utf-8", newline="\n").write(src)
PY

# W2: Optimize main WASM with binaryen `wasm-opt -Oz` (after wasm-bindgen so
# wasm-bindgen reads `target_features` before final size reduction).
WASM_OPT_FLAGS=(
  -Oz
  --strip-debug
  --strip-producers
  --enable-bulk-memory
  --enable-reference-types
  --enable-multivalue
  --enable-sign-ext
  --enable-nontrapping-float-to-int
)
if command -v wasm-opt >/dev/null 2>&1; then
  echo "==> Running wasm-opt -Oz on mica_rust_bg.wasm..."
  before_opt=$(wc -c < "$out/mica_rust_bg.wasm")
  wasm-opt "${WASM_OPT_FLAGS[@]}" "$out/mica_rust_bg.wasm" -o "$out/mica_rust_bg.opt.wasm"
  mv "$out/mica_rust_bg.opt.wasm" "$out/mica_rust_bg.wasm"
  after_opt=$(wc -c < "$out/mica_rust_bg.wasm")
  echo "    wasm-opt -Oz: ${before_opt} B -> ${after_opt} B"
else
  echo "FAIL: wasm-opt (binaryen >= 118) not found on PATH; required for W2 size optimization" >&2
  exit 1
fi

# W4: Build lazy opt-in Math (ratex) and Mermaid (merman) preview WASM modules.
root_target_dir=$(cargo metadata --format-version 1 --no-deps --locked | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])' 2>/dev/null || echo "target")
for kind in math mermaid; do
  echo "==> Building lazy mica_preview/${kind}.wasm..."
  CARGO_PROFILE_RELEASE_LTO=true \
  CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 \
  CARGO_PROFILE_RELEASE_PANIC=abort \
  CARGO_PROFILE_RELEASE_OPT_LEVEL=z \
    cargo build --locked -p mica-md-wasm --lib --release \
      --target wasm32-unknown-unknown \
      --no-default-features \
      --features "preview-$kind"
  wasm-bindgen \
    --target web \
    --no-typescript \
    --remove-name-section \
    --remove-producers-section \
    --out-dir "$out/mica_preview" \
    --out-name "$kind" \
    "$root_target_dir/wasm32-unknown-unknown/release/mica_md_wasm.wasm"
  wasm-opt "${WASM_OPT_FLAGS[@]}" "$out/mica_preview/${kind}_bg.wasm" -o "$out/mica_preview/${kind}_bg.opt.wasm"
  mv "$out/mica_preview/${kind}_bg.opt.wasm" "$out/mica_preview/${kind}_bg.wasm"
  p_wasm=$(gzip -n -9 -c "$out/mica_preview/${kind}_bg.wasm" | wc -c)
  p_js=$(gzip -n -9 -c "$out/mica_preview/${kind}.js" | wc -c)
  p_total=$((p_wasm + p_js))
  case "$kind" in
    math) p_budget=1048576 ;;
    mermaid) p_budget=2097152 ;;
  esac
  if [[ "$p_total" -gt "$p_budget" ]]; then
    echo "FAIL: $kind preview gzip-9 is ${p_total} B > budget ${p_budget} B" >&2
    exit 1
  fi
  echo "    mica_preview/${kind} gzip-9: wasm=${p_wasm} B js=${p_js} B total=${p_total} B (budget ${p_budget} B)"
done

# Stamp content hashes ONLY onto <link> preload tags and <meta> tags so the
# inline <script type="module"> remains byte-identical to source (CSP sha256 invariant).
wasm_hash=$(sha256sum "$out/mica_rust_bg.wasm" | cut -c1-10)
preview_hash=$(cat "$out/mica_preview/math_bg.wasm" "$out/mica_preview/mermaid_bg.wasm" | sha256sum | cut -c1-10)
python3 - "$out/index.html" "$out/sw.js" "$wasm_hash" "$preview_hash" <<'PY'
import sys
html_path, sw_path, wh, ph = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
src = open(html_path, "r", encoding="utf-8").read()
# Replace only inside <head> (never touch the inline <script type="module"> body).
head_end = src.index("</head>")
head, rest = src[:head_end], src[head_end:]
head = head.replace('href="./mica_rust_bg.wasm"', f'href="./mica_rust_bg.wasm?v={wh}"')
head = head.replace('href="./mica_rust.js"', f'href="./mica_rust.js?v={wh}"')
head = head.replace('href="./nsh.js"', f'href="./nsh.js?v={wh}"')
head = head.replace(
    '<meta name="mica-preview-version" content="">',
    f'<meta name="mica-preview-version" content="{ph}">',
)
head = head.replace(
    '<meta name="mica-sw-cache" content="">',
    f'<meta name="mica-sw-cache" content="mica-shell-{wh}-{ph}">',
)
open(html_path, "w", encoding="utf-8", newline="\n").write(head + rest)

sw_src = open(sw_path, "r", encoding="utf-8").read()
sw_src = sw_src.replace("__WASM_HASH__", wh).replace("__PREVIEW_HASH__", ph)
open(sw_path, "w", encoding="utf-8", newline="\n").write(sw_src)
PY
chmod -R a+rX "$out"

# W5: Measure Brotli-11 transfer size of the initial startup payload and enforce budget.
if ! command -v brotli >/dev/null 2>&1; then
  echo "FAIL: brotli CLI not found on PATH; required for W5 Brotli-11 size gate" >&2
  exit 1
fi
wasm_raw=$(wc -c < "$out/mica_rust_bg.wasm")
wasm_br=$(brotli --best --stdout "$out/mica_rust_bg.wasm" | wc -c)
glue_br=$(brotli --best --stdout "$out/mica_rust.js" | wc -c)
nsh_br=$(brotli --best --stdout "$out/nsh.js" | wc -c)
total_br=$((wasm_br + glue_br + nsh_br))

echo "==> deploy/web staged (pure-Rust Web client + lazy mica_preview)"
echo "    mica_rust_bg.wasm raw:    ${wasm_raw} B"
echo "    mica_rust_bg.wasm br-11:  ${wasm_br} B"
echo "    mica_rust.js      br-11:  ${glue_br} B"
echo "    nsh.js            br-11:  ${nsh_br} B"
echo "    initial total     br-11:  ${total_br} B (hard gate <= ${BUDGET_BYTES} B)"

if [[ "$total_br" -gt "$BUDGET_BYTES" ]]; then
  echo "FAIL: Web initial bundle Brotli-11 is ${total_br} B, exceeding gate ${BUDGET_BYTES} B" >&2
  exit 1
fi
