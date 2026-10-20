#!/usr/bin/env bash
# The whole pre-tag half of docs/ops/release.md as ONE command.
#
#   just release 0.13.28
#
# It exists because of one specific failure mode. The Postgres-backed tests SKIP
# when DATABASE_URL is unset, and a skipped test reports as PASSED — so "I ran
# the tests" and "the tests ran" were different facts, with nothing between them
# but a human remembering the difference. That memory failed often enough to be
# written into CLAUDE.md as a reminder, which is the tell: a rule that needs a
# reminder is not a rule, it is a hope.
#
# So the gate is not "release-check exists and you should run it". The gate is
# that bumping, checking, committing and tagging are ONE step with the check in
# the middle. There is no ordering left to get wrong and nothing to remember.
# `just release-check` still exists to run on its own.
#
# ORDER MATTERS, and it is the opposite of what the old doc string said: bump
# FIRST. release-check asserts the three version numbers agree, which they only
# do after the bump — run it before and it fails on the state it is meant to
# create.
#
# Deliberately NOT gated on docs/plans/roadmap.md being touched. Not every release
# closes a roadmap item, so that gate would fire on releases where nothing is
# wrong, and an alarm that cries wolf gets silenced — the same reason the CI
# compose-drift check had to go. It stays a step in docs/ops/release.md.
set -euo pipefail
[ -d "/c/Program Files/Git/usr/bin" ] && PATH="/c/Program Files/Git/usr/bin:$PATH"

version=$1
tag="v$version"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] \
  || { echo "REFUSED: want X.Y.Z with no leading v (got: $version)" >&2; exit 1; }

# Refuse on a dirty tree BEFORE touching anything: the bump edits four files, and
# once it has, "what did I have uncommitted?" is no longer answerable.
git diff --quiet && git diff --cached --quiet \
  || { echo "REFUSED: working tree is dirty — commit or stash first" >&2; exit 1; }

git rev-parse -q --verify "refs/tags/$tag" >/dev/null \
  && { echo "REFUSED: tag $tag already exists locally" >&2; exit 1; }
git ls-remote --exit-code --tags origin "$tag" >/dev/null 2>&1 \
  && { echo "REFUSED: tag $tag exists on origin — that version is published" >&2; exit 1; }

echo "==> 1/4  bump"
bash scripts/release-bump.sh "$version"

echo "==> 2/4  gate (this is the part that used to be skippable)"
bash scripts/release-check.sh

echo "==> 3/4  commit"
# The bump's files BY NAME, not `git add -A`. The tree was clean at the start,
# but the gate takes ~15 minutes and the tree is not frozen while it runs:
# 2026-10-20, an uncommitted ansible/deploy.yml edit that was still waiting on
# the owner's decision was written during the 0.13.59 gate, `-A` swept it into
# "chore(release): 0.13.59", and it reached main unreviewed. Anything else that
# changed meanwhile now stays in the working tree, visible, for its own commit.
bump_files=(Cargo.toml Cargo.lock clients/mica_flutter/pubspec.yaml clients/mica_flutter/lib/main.dart)
git add -- "${bump_files[@]}"
git commit -q -m "chore(release): $version" -- "${bump_files[@]}"
if ! { git diff --quiet && git diff --cached --quiet; }; then
  echo "  NOTE: other files changed while the gate ran; they were left out of this commit:"
  git status --short
fi

echo "==> 4/4  tag"
git tag "$tag"

cat <<EOF

  $version is committed and tagged locally. Nothing has been pushed.

  Push starts the release train (CI builds all 7 artefacts):

      git push origin main $tag

  Then deploy — see docs/ops/release.md. Before pushing, check that this release's
  roadmap entries are struck off in docs/plans/roadmap.md: negative items ("no X yet")
  only ever go stale silently, and a release is the one moment anybody looks.
EOF
