#!/usr/bin/env bash
set -euo pipefail
# Defaulted, not required. Under `set -u` an injected-only variable turns any
# direct invocation into `NODE: unbound variable` — not hypothetical:
# verify-prod.sh had exactly this shape and it failed the first real Ansible
# deploy AFTER the deploy itself had already succeeded.
NODE="${NODE:-root@mica.cloudcele.com}"

# Ship the repo's copy each run, same as deploy-prod does with the compose:
# a drill script that has drifted from the repo is one more thing to distrust.
cat deploy/restore-drill.sh | ssh "${NODE}" "cat > /tmp/mica-restore-drill.sh"
ssh "${NODE}" "bash /tmp/mica-restore-drill.sh /data/mica/$(basename "$1")"
