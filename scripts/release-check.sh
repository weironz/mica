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

# 2. The three version numbers must already agree.
pub=$(grep -m1 '^version:' clients/mica_flutter/pubspec.yaml | tr -d ' ' | cut -d: -f2)
dart=$(grep -m1 'kAppVersion' clients/mica_flutter/lib/main.dart | sed "s/.*'\(.*\)'.*/\1/")
rust=$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')
[ "$pub" = "$dart" ] && [ "$dart" = "$rust" ] \
  || fail "version numbers disagree: pubspec=$pub kAppVersion=$dart Cargo=$rust"
echo "==> version: $pub (three places agree)"

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

# 4. Cargo.lock must already carry the version, or the release commit ships a
#    lock that disagrees with its own manifest.
git diff --quiet Cargo.lock \
  || fail "Cargo.lock is dirty — run 'cargo check' and include it in the release commit"

printf '\n  release-check passed for %s\n' "$pub"
