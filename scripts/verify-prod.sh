#!/usr/bin/env bash
# Prove prod really serves <version>.   `just verify-prod 0.13.27`
#
# SITE defaults instead of being required. It used to be `${SITE}` under
# `set -u`, supplied only by the justfile — which was fine until
# scripts/deploy-prod.sh started calling this directly, and the very first real
# deploy then died on `SITE: unbound variable` AFTER the playbook had finished
# successfully. A deploy that worked reporting as failed is the more expensive
# direction of that error: it invites a re-run, or a hunt for a problem in
# production that is not there.
#
# Still overridable, so it can be pointed at a staging host.
set -euo pipefail
SITE="${SITE:-https://mica.cloudcele.com}"

version=$1
got=$(curl -fsS "$SITE"/api/ready)
echo "$got"
echo "$got" | grep -q "\"version\":\"$version\"" \
  || { echo "VERSION MISMATCH: $SITE is not serving $version"; exit 1; }
echo "mcp:    $(curl -s -o /dev/null -w '%{http_code}' "$SITE"/mcp)"
echo "index:  $(curl -s -o /dev/null -w '%{http_code}' "$SITE"/)"
echo "bundle: $(curl -fsS "$SITE"/main.dart.js | md5sum | cut -c1-12)…"
