#!/bin/sh
# Run an ansible playbook from a Windows machine, by not running it on Windows.
#
#   DR_HOST=1.2.3.4 sh scripts/ansible-in-docker.sh ansible/provision.yml -e target=mica-dr
#
# WHY THIS EXISTS: ansible supports Linux/macOS controllers only. On Windows
# `pipx install ansible-core` succeeds and puts ansible-playbook.exe on PATH —
# and then dies inside ansible's own startup with `OSError: [WinError 87]`.
# scripts/deploy-prod.sh refuses for that reason and points at CI.
#
# That is the right answer for a DEPLOY (CI can do it). It is the wrong answer
# during a RECOVERY, where you may be standing at a laptop with a broken node
# and no appetite for a round trip through GitHub. This gets you a controller in
# ~10 seconds using the Docker that is already installed.
#
# Found necessary during the 2026-08-29 DR drill (docs/dr-drill.md). WSL would
# also work; this needs nothing installed that a Windows dev box lacks.
set -eu

repo=$(cd "$(dirname "$0")/.." && pwd)
# Docker on Windows wants a Windows path for -v. MSYS/Git-Bash gives us one.
case "$repo" in
  /c/*|/d/*) drive=$(echo "$repo" | cut -c2); rest=$(echo "$repo" | cut -c3-);
             repo="$(echo "$drive" | tr a-z A-Z):$rest" ;;
esac
ssh_dir="${SSH_DIR:-$HOME/.ssh}"
case "$ssh_dir" in
  /c/*|/d/*) drive=$(echo "$ssh_dir" | cut -c2); rest=$(echo "$ssh_dir" | cut -c3-);
             ssh_dir="$(echo "$drive" | tr a-z A-Z):$rest" ;;
esac

# The inner script, piped in rather than mounted: one less path to translate.
# Keys arrive from a Windows bind mount as 0777, which ssh refuses to use, so
# they are copied and chmodded inside.
docker run --rm -i \
  -v "$repo:/work" -v "$ssh_dir:/ssh:ro" \
  -e "DR_HOST=${DR_HOST:-}" -e ANSIBLE_HOST_KEY_CHECKING=False \
  -e "TRAEFIK_BASIC_AUTH=${TRAEFIK_BASIC_AUTH:-}" \
  --entrypoint sh alpine/ansible -s -- "$@" <<'INNER'
set -e
mkdir -p /root/.ssh
for k in id_ed25519 id_rsa; do
  [ -f "/ssh/$k" ] && cp "/ssh/$k" "/root/.ssh/$k" && chmod 600 "/root/.ssh/$k"
done
printf 'Host *\n  StrictHostKeyChecking no\n  UserKnownHostsFile /dev/null\n' > /root/.ssh/config
cd /work
# Installed rather than refused, unlike deploy-prod.sh: this container is thrown
# away, so "the collection changed under me" cannot outlive the run.
ansible-galaxy collection install -r ansible/requirements.yml >/dev/null
exec ansible-playbook -i ansible/inventory.yml "$@"
INNER
