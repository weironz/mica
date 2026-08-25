#!/usr/bin/env bash
set -euo pipefail
# Ship the repo's copy each run, same as deploy-prod does with node-deploy-policy.sh:
# a drill script that has drifted from the repo is one more thing to distrust.
cat deploy/restore-drill.sh | ssh "${NODE}" "cat > /tmp/mica-restore-drill.sh"
ssh "${NODE}" "bash /tmp/mica-restore-drill.sh /data/mica/$(basename "$1")"
