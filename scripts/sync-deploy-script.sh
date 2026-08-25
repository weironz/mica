#!/usr/bin/env bash
set -euo pipefail
tag=""$1""
git fetch -q origin main
# Same fallback as deploy-prod: tags older than the 2026-07-31 rename carry
# it as deploy/mica-deploy.sh.
policy=deploy/node-deploy-policy.sh
git cat-file -e "$tag:$policy" 2>/dev/null || policy=deploy/mica-deploy.sh
git show "$tag:$policy" \
  | ssh "${NODE}" "cat > /usr/local/sbin/mica-deploy.new \
      && chown root:root /usr/local/sbin/mica-deploy.new \
      && chmod 0755 /usr/local/sbin/mica-deploy.new \
      && bash -n /usr/local/sbin/mica-deploy.new \
      && mv /usr/local/sbin/mica-deploy.new /usr/local/sbin/mica-deploy \
      && sha256sum /usr/local/sbin/mica-deploy"
echo "want: $(git show "$tag:$policy" | sha256sum)"
