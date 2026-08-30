#!/usr/bin/env bash
# Everything docs/release.md says to check before tagging — as a command that
# REFUSES, not a list somebody has to remember.
#
# The rule that forced this into existence: the Postgres-backed tests SKIP when
# DATABASE_URL is unset, and a skipped test reports as PASSED. So "I ran the
# tests" and "the tests ran" were different facts, and the only thing standing
# between them was a human remembering the difference. That is not a control.
#
# Lives in scripts/ rather than as a `just` shebang recipe on purpose: a shebang
# recipe is executed by `just` itself, which needs `cygpath` to translate the
# interpreter path on Windows — and cygpath is absent unless the caller already
# fixed their PATH. Recipes that call `bash scripts/…` run under the justfile's
# `windows-shell` (Git Bash) instead, where the whole toolchain is present.
set -euo pipefail

fail() { printf '\n  REFUSED: %s\n' "$*" >&2; exit 1; }

# 1. The dev database must be UP — not "warn and continue" the way `just test`
#    does. At release time a silent skip IS the failure being prevented.
docker exec mica-postgres pg_isready -U mica -d mica >/dev/null 2>&1 \
  || fail "dev Postgres is down, so the DB-backed tests would SKIP and still report passed. Run 'just dev' first."
echo "==> DB-backed tests: ON"

# 2. The three version numbers must already agree. The reading lives in
#    scripts/manifest-version.sh because release.yml needs the same answer to
#    check the TAG against them, and two copies of three greps is the drift this
#    repo keeps paying for. It prints its own REFUSED and exits non-zero, which
#    `set -e` turns into ours.
version=$(bash scripts/manifest-version.sh)
echo "==> version: $version (three places agree)"

# 2b. The TOOLCHAINS must be the pinned ones, on this machine too.
#
#     Everything used to float on "stable": CI resolved one Flutter, the laptop
#     had another, and the shipped Docker image a third Rust — with no record of
#     any of it. That is how Flutter 3.47 turned Windows over to Impeller and
#     shipped a new renderer to users without anyone deciding, while this machine
#     stayed on 3.44 and debugged a different engine than the one it releases.
#
#     Checked HERE because CI cannot see a laptop. The pins themselves live in
#     .fvmrc (read by every flutter-action step) and rust-toolchain.toml (read by
#     rustup); this only asserts the machine agrees with them.
want_flutter=$(sed -n 's/.*"flutter"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' .fvmrc)
[ -n "$want_flutter" ] || fail ".fvmrc has no \"flutter\" version — that file is the pin CI reads"
have_flutter=$("${FLUTTER:-flutter}" --version | sed -n 's/^Flutter \([0-9][^ ]*\).*/\1/p' | head -1)
[ "$have_flutter" = "$want_flutter" ] || fail \
  "local Flutter is $have_flutter, the pin is $want_flutter (.fvmrc) — CI builds what you are about to ship with the PIN, so testing on anything else tests a different engine. Run 'flutter upgrade', or check out $want_flutter."
echo "==> Flutter: $have_flutter (matches .fvmrc)"

want_rust=$(sed -n 's/^channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' rust-toolchain.toml)
have_rust=$(rustc --version | awk '{print $2}')
[ "$have_rust" = "$want_rust" ] || fail \
  "local rustc is $have_rust, rust-toolchain.toml pins $want_rust — rustup honours that file automatically, so this usually means the pin was just changed and the toolchain is not installed: rustup toolchain install $want_rust"
#     And the images that actually ship must be built with that same Rust. Three
#     files, one number: drift here is invisible until a release.
for f in deploy/Dockerfile.api deploy/Dockerfile.cli docker-compose.yml; do
  grep -q "rust:${want_rust%.*}-slim-bookworm" "$f" || fail \
    "$f does not build on rust:${want_rust%.*}-slim-bookworm, but rust-toolchain.toml pins $want_rust. Bump them together."
done
echo "==> Rust: $have_rust (toolchain + all three images agree)"

# 3. Run the gates the SAME WAY CI runs them. `cargo check` does not run clippy,
#    and clippy without `-D warnings` exits 0 while printing the very thing that
#    fails CI — both have turned a pipeline red here before.
DATABASE_URL=postgres://mica:mica@127.0.0.1:5432/mica cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
#    The Flutter half is spelled EXACTLY as ci.yml spells it, flags included.
#    It was `analyze lib` with no flags, which is stricter than CI — and with
#    ~130 long-standing `info` lints in the package, `flutter analyze` exits 1
#    and the gate could NEVER pass. A gate that always refuses does not stop bad
#    releases; it gets bypassed, and then nothing is checked at all.
#
#    `--no-fatal-warnings` is NOT here, and that is the deliberate half.
#    Warnings are fatal in both this gate and CI as of 2026-08-25, because an
#    `invalid_return_type_for_catch_error` warning sat unread long enough to
#    ship a real defect: a `.catchError` handler returning the wrong type, so
#    the error path — whose entire job was to fail quietly — threw a TypeError
#    instead. Two warnings existed when that was found; both are gone, and the
#    bar is met today, which is the only condition under which raising it is
#    honest. The one genuine false positive is suppressed at its own line
#    (`// ignore:` in main.dart, where the analyzer and the CFE disagree).
#
#    Infos stay non-fatal: ~130 of them, long-standing, style-level. Making them
#    fatal would be a repo-wide sweep nobody asked for.
(cd clients/mica_flutter \
  && "${FLUTTER:-flutter}" analyze --no-fatal-infos \
  && "${FLUTTER:-flutter}" test)

# 4. Cargo.lock must carry the version, or the release commit ships a lock that
#    disagrees with its own manifest.
#
#    This used to be `git diff --quiet Cargo.lock`, which is a different claim —
#    "the lock is UNMODIFIED" — and it made `just release` impossible to pass.
#    Step 1 of that flow is release-bump.sh, whose last line is `cargo check`
#    precisely to refresh the lock; step 3 is the commit. So at this point the
#    lock is dirty BY DESIGN, and the gate refused the exact state it exists to
#    require. It passed standalone (clean tree) and failed only in the flow it
#    was written for, which is why it shipped.
#
#    Second time this script has carried a gate that could never pass — the
#    other was `flutter analyze` with no flags, in a package with ~130 infos.
#    Same shape both times: the check was written against the state the author
#    had in front of them, not the state the caller creates. Anything added here
#    has to be tried through `just release`, not only on its own.
lock=$(grep -A1 -E '^name = "mica-(api-server|cli)"$' Cargo.lock \
  | grep -E '^version = ' | sed 's/.*"\(.*\)"/\1/' | sort -u)
[ "$lock" = "$version" ] \
  || fail "Cargo.lock says '$lock' for the workspace binaries, Cargo.toml says '$version' — run 'cargo check' and include the lock in the release commit"
echo "==> Cargo.lock: $lock"

# ── The Windows icon exists twice, and the copies drifted ────────────────────
# The taskbar/exe icon is windows/runner/resources/app_icon.ico; the tray icon
# has to be a bundled Flutter ASSET (tray_manager takes an asset path), so it
# cannot be the same file on disk. Two copies kept in step by memory is exactly
# how the tray spent 2026-07-20 .. 2026-08-28 showing the stock FLUTTER logo:
# the app icon was replaced, the asset copy was not, and nothing said so.
# Byte-identical is the strongest check available and costs one cmp.
cmp -s clients/mica_flutter/assets/tray_icon.ico \
       clients/mica_flutter/windows/runner/resources/app_icon.ico \
  || fail "the tray icon and the app icon differ — they are two copies of ONE mark (tray_manager needs a bundled asset, so the file cannot be shared). Regenerate both with scripts/gen-icons.py and commit them together."
echo "==> icons: tray == app"

# ── The desktop FFI crate is not in the root workspace ───────────────────────
# `clients/mica_flutter/rust` (rust_lib_mica_flutter) is its own cargo project,
# so `cargo test --workspace` above never compiles it. Change a shared crate —
# mica-interchange, mica-markdown, mica-core — and every local check can pass
# while the desktop build is broken.
#
# Found 2026-08-30 the hard way: a field added to `mica_interchange::TreeNode`
# was wired through the server's caller and not the local store's. 1481 Flutter
# tests and the whole cargo workspace went green; `flutter run -d windows` died
# on E0063. CI does catch it (flutter-integration.yml builds the crate via
# cargokit) — but only AFTER the tag is pushed, which is the wrong side of the
# gate. Seconds when warm, and it turns a failed release train into a refusal.
(cd clients/mica_flutter/rust && cargo check --quiet)   || fail "clients/mica_flutter/rust does not compile — it is NOT part of the root workspace, so nothing above would have told you. Fix it before tagging; CI would only find this after the push."
echo "==> desktop FFI crate compiles"

# ── pg_dump must not be older than the server it dumps ───────────────────────
# `pg_dump` REFUSES a server newer than itself. The backup image pins
# postgresql-client-N; the stack pins postgres:M. N < M means the off-site
# database snapshot cannot be taken — and it fails inside a daily container,
# into a log nobody reads.
#
# Found 2026-08-29 the hard way: postgres went 16 -> 18.6 on 08-28 and this
# stayed at 16, so even after fixing the two OTHER reasons backups were broken,
# pg_dump still aborted. Three same-shaped bugs in one day (MICA_TOKEN in
# compose, MICA_API_BASE_URL in the CLI, this) is what a gate is for: the
# Dockerfile comment already stated the invariant, and stating it did nothing.
client=$(grep -oE 'postgresql-client-[0-9]+' deploy/Dockerfile.cli | head -1 | sed 's/.*-//')
server=$(grep -oE 'image: postgres:[0-9]+' deploy/docker-compose.yml | head -1 | sed 's/.*://')
[ -n "$client" ] && [ -n "$server" ] \
  || fail "could not read the postgres versions (client='$client' server='$server') — this gate must not pass by failing to look"
[ "$client" -ge "$server" ] \
  || fail "deploy/Dockerfile.cli installs postgresql-client-$client but the stack runs postgres:$server — pg_dump refuses a newer server, so the off-site DB backup would silently stop. Bump the client."
echo "==> pg_dump: client $client >= server $server"

printf '\n  release-check passed for %s\n' "$version"
