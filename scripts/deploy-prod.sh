#!/usr/bin/env bash
set -euo pipefail
tag="v"$1""
# The node's compose is NOT a git checkout — it was hand-placed and drifted:
# prod sat on a hardcoded willdockerhub/... image while the repo had already
# moved to ACR, so a "deploy" silently kept pulling from the old registry.
# Ship the repo's copy every time (backing up first) — repo is the truth.
# NB the node's file is docker-compose.yaml; keep that name, or compose
# would find two files and pick the other one.
# (No backticks anywhere in a recipe body: just runs them as commands,
# even inside a # comment.)
# Take the compose from the TAG, not the working tree. `scp deploy/...`
# shipped whatever happened to be checked out: deploying 0.12.6 from a
# branch that had already moved on sent prod a compose from a DIFFERENT
# version, and a dirty tree sent uncommitted edits. Nothing announced it —
# the deploy just succeeded with a file nobody reviewed as part of that
# release. `git show <tag>:<path>` makes the pairing exact and reproducible
# from any checkout state.
if ! git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
  echo "==> tag $tag not local, fetching"
  git fetch -q origin "refs/tags/$tag:refs/tags/$tag"
fi
echo "==> syncing compose (from $tag) + pinning MICA_VERSION=$tag"
git show "$tag:deploy/docker-compose.yml" \
  | ssh "${NODE}" "cat > "${NODE_DIR}"/docker-compose.yaml.new"
# NOTE if you ever add a service to the compose: this recipe ships ONLY
# docker-compose.yml, and brings up ONLY the services it names below. A file
# the compose bind-mounts but this does not send is not an error docker
# reports — it silently creates a DIRECTORY at the mount point and the
# container dies on "is a directory". Both bit the 0.13.12 prometheus
# service, which is why monitoring now lives in its own stack
# (deploy/monitoring/) instead of in Mica's deployment.
ssh "${NODE}" "cd "${NODE_DIR}" \
  && cp docker-compose.yaml docker-compose.yaml.bak-\$(date +%Y%m%d-%H%M%S) \
  && mv docker-compose.yaml.new docker-compose.yaml \
  && sed -i -E 's|^MICA_VERSION=.*|MICA_VERSION=$tag|' .env \
  && grep -E '^MICA_(VERSION|REGISTRY)=' .env"
# Self-heal the deploy policy the same way as compose: install the tag's
# node-deploy-policy.sh so the node script never drifts from the repo (the gh-Deploy
# path can only WARN on drift by design — the restricted key must not install
# the policy that restricts it; this root path is the blessed out-of-band
# installer, so doing it here means a full deploy always clears any drift).
# `bash -n` syntax-checks before the atomic mv, so a broken script can't land.
echo "==> syncing deploy policy (node-deploy-policy.sh from $tag)"
# Tags cut before the 2026-07-31 rename carry this as deploy/mica-deploy.sh.
# Fall back instead of failing: THIS recipe is the fallback deploy path, and
# it has to keep working for versions that are already published.
policy=deploy/node-deploy-policy.sh
git cat-file -e "$tag:$policy" 2>/dev/null || policy=deploy/mica-deploy.sh
git show "$tag:$policy" \
  | ssh "${NODE}" "cat > /usr/local/sbin/mica-deploy.new \
      && chown root:root /usr/local/sbin/mica-deploy.new \
      && chmod 0755 /usr/local/sbin/mica-deploy.new \
      && bash -n /usr/local/sbin/mica-deploy.new \
      && mv /usr/local/sbin/mica-deploy.new /usr/local/sbin/mica-deploy \
      && sha256sum /usr/local/sbin/mica-deploy"
echo "==> pulling + recreating api + web"
ssh "${NODE}" "cd "${NODE_DIR}" && docker compose pull api web && docker compose up -d --no-deps api web"
# The backup sidecar (mica-cli: exports the workspace + external rustic
# snapshots it) is keyed to the SAME MICA_VERSION, so a version roll must
# move it too or it drifts. It lives behind the `backup` compose profile;
# only refresh it where it is ALREADY present (ps -aq returns a container),
# so a deploy never switches backup ON on a node that runs without it.
echo "==> refreshing backup sidecar (only if this node runs it)"
ssh "${NODE}" "cd "${NODE_DIR}" \
  && if [ -n \"\$(docker compose --profile backup ps -aq backup 2>/dev/null)\" ]; then \
       docker compose --profile backup pull backup \
       && docker compose --profile backup up -d --no-deps backup \
       && echo 'backup refreshed'; \
     else echo 'backup profile not active on this node — skipped'; fi"
# Go-template braces vs just: four open-braces escape a literal two, but a
# closing pair is ALREADY literal outside an interpolation — writing four
# closers emitted two extra, so the format returned "healthy}}" and the
# compare never matched. It cried "NOT healthy" over a healthy deploy.
echo "==> waiting for api health"
ssh "${NODE}" 'for i in $(seq 1 60); do s=$(docker inspect --format "{{{{.State.Health.Status}}" mica-api-1 2>/dev/null || true); [ "$s" = healthy ] && { echo "api healthy"; exit 0; }; sleep 4; done; echo "api NOT healthy (last state: ${s:-unknown})"; exit 1'
just verify-prod "$1"
echo "==> prod now on $tag from the registry in "${NODE_DIR}"/.env"
