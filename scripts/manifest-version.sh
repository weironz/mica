#!/usr/bin/env bash
# Print the version this working tree claims to be — or refuse if the three
# places disagree.
#
#   $ bash scripts/manifest-version.sh
#   0.13.28
#
# Two callers need this answer and they used to grep for it separately, which is
# the drift this repo keeps paying for. They ask different questions of it:
#
#   scripts/release-check.sh        "do the three agree?"          (before tagging)
#   .github/workflows/release.yml   "does the TAG agree with them?" (after)
#
# The second one is the gap that made this file worth extracting. `release.yml`
# derives the version from the tag name alone, so `git tag v0.13.29` without a
# bump builds an installer NAMED 0.13.29 whose kAppVersion is still 0.13.28 —
# and the in-app updater compares exactly that number. `verify-prod` catches the
# server half at deploy time; nothing catches the desktop half, ever.
set -euo pipefail

pub=$(grep -m1 '^version:' clients/mica_flutter/pubspec.yaml | tr -d ' ' | cut -d: -f2)
dart=$(grep -m1 'kAppVersion' clients/mica_flutter/lib/main.dart | sed "s/.*'\(.*\)'.*/\1/")
rust=$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')

[ -n "$pub" ] && [ -n "$dart" ] && [ -n "$rust" ] || {
  echo "REFUSED: could not read a version (pubspec='$pub' kAppVersion='$dart' Cargo='$rust')." \
       "Run from the repo root." >&2
  exit 1
}

[ "$pub" = "$dart" ] && [ "$dart" = "$rust" ] || {
  echo "REFUSED: version numbers disagree: pubspec=$pub kAppVersion=$dart Cargo=$rust" >&2
  exit 1
}

printf '%s\n' "$pub"
