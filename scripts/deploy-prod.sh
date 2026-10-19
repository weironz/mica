#!/usr/bin/env bash
# Roll prod to an already-published version.   `just deploy-prod 0.13.27`
#
# This used to be ~80 lines of `ssh "cd … && sed -i … && docker compose …"`, with
# the quoting escaped through bash -> just -> ssh -> bash. Every layer got one
# wrong at least once: a mis-escaped go-template reported "api NOT healthy" over
# healthy deploys, and a `{{version}}` inside single quotes reached the node as
# literal text. The steps now live in ansible/deploy.yml, where they are data
# rather than nested strings; this script only decides WHAT to deploy.
#
# The one thing it must get right is provenance: the compose file comes from the
# TAG, never from the working tree. `scp deploy/docker-compose.yml` once shipped
# prod a compose from a DIFFERENT version because the branch had moved on, and
# nothing announced it — the deploy simply succeeded with a file nobody had
# reviewed as part of that release.
set -euo pipefail

version=$1
tag="v$version"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] \
  || { echo "REFUSED: want X.Y.Z with no leading v (got: $version)" >&2; exit 1; }

# Runs it, rather than checking that the file exists. On Windows `pipx install
# ansible-core` succeeds and puts ansible-playbook.exe on PATH, so `command -v`
# passes — and then the real invocation dies inside ansible's own startup with
# `check_blocking_io` → `OSError: [WinError 87]`. Ansible supports Linux/macOS
# controllers only. Existence was never the question; being able to run was.
# No usable ansible here (Windows: ansible does not support it as a controller)
# but Docker is → run THIS script inside a throwaway Linux controller. The
# alternative was "deploy through CI", which is no answer when CI cannot run.
# Same trick as scripts/ansible-in-docker.sh, which the DR drill proved.
if ! ansible-playbook --version >/dev/null 2>&1 \
  && [ -z "${MICA_DEPLOY_IN_DOCKER:-}" ] && docker info >/dev/null 2>&1; then
  repo=$(cd "$(dirname "$0")/.." && pwd)
  ssh_dir=${SSH_DIR:-$HOME/.ssh}
  win() { case "$1" in /[a-zA-Z]/*) echo "$(echo "${1:1:1}" | tr a-z A-Z):${1:2}" ;; *) echo "$1" ;; esac; }
  echo "==> ansible not usable here — running the deploy in a Linux controller container"
  MSYS_NO_PATHCONV=1 exec docker run --rm -i \
    -v "$(win "$repo"):/work" -v "$(win "$ssh_dir"):/ssh:ro" \
    -e MICA_DEPLOY_IN_DOCKER=1 -e ANSIBLE_HOST_KEY_CHECKING=False \
    --entrypoint sh alpine/ansible -s -- "$@" <<'INNER'
set -e
apk add --no-cache bash git curl openssh-client >/dev/null
mkdir -p /root/.ssh
for k in id_ed25519 id_rsa; do
  [ -f "/ssh/$k" ] && cp "/ssh/$k" "/root/.ssh/$k" && chmod 600 "/root/.ssh/$k"
done
printf 'Host *\n  StrictHostKeyChecking no\n  UserKnownHostsFile /dev/null\n' > /root/.ssh/config
git config --global --add safe.directory /work
cd /work
ansible-galaxy collection install -r ansible/requirements.yml >/dev/null
exec bash scripts/deploy-prod.sh "$@"
INNER
fi

ansible-playbook --version >/dev/null 2>&1 || {
  cat >&2 <<'EOF'
REFUSED: ansible-playbook is not usable here.

  This script runs on a Linux or macOS controller (a WSL distro counts).
  Install it there:  pipx install ansible-core

  With Docker running, this script re-runs itself inside a Linux controller
  container automatically. Otherwise, deploy through CI:

      gh workflow run Deploy --repo weironz/mica -f version=X.Y.Z
EOF
  exit 1
}

# Refuse rather than auto-install: a deploy is the wrong moment to pull a fresh
# collection off the internet and find out it changed.
ansible-galaxy collection list community.docker >/dev/null 2>&1 \
  || { echo "REFUSED: ansible-galaxy collection install -r ansible/requirements.yml" >&2; exit 1; }

if ! git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
  echo "==> tag $tag not local, fetching"
  git fetch -q origin "refs/tags/$tag:refs/tags/$tag"
fi

# Materialise the compose the tag was released with. mktemp, not a path inside
# the repo: a file written into the checkout looks like a tracked one and gets
# committed by accident.
compose=$(mktemp -t mica-compose.XXXXXX)
trap 'rm -f "$compose"' EXIT
git show "$tag:deploy/docker-compose.yml" > "$compose"
echo "==> compose from $tag ($(wc -l < "$compose") lines)"

# "${@:2}" so extra flags pass through — `just deploy-prod 0.13.27 --check --diff`
# rehearses the whole thing against the real node without changing it.
ansible-playbook -i ansible/inventory.yml ansible/deploy.yml \
  -e "version=$version" -e "compose_src=$compose" "${@:2}"

# Only reached on success; the playbook fails loudly (and restores MICA_VERSION)
# otherwise. Prod claiming the right version is the one check the playbook cannot
# do for itself — it asks docker, this asks the service.
#
# Skipped after a rehearsal, where prod is still on the PREVIOUS version by
# design. A verify that runs there either fails on a deploy that did not happen,
# or — worse — is loosened until it passes and then passes on the real thing too.
for a in "${@:2}"; do
  if [ "$a" = "--check" ] || [ "$a" = "-C" ]; then
    echo "==> rehearsal: prod was NOT changed, skipping verify"
    exit 0
  fi
done
bash scripts/verify-prod.sh "$version"
