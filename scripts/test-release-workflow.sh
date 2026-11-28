#!/usr/bin/env bash
# Validate structural and conditional-execution invariants across Ship, Release,
# Release dry-run, and Release canary workflows (pure-Rust fullstack pipeline).
set -euo pipefail
cd "$(dirname "$0")/.."
python3 - <<'PY'
from pathlib import Path
import re

release_text = Path('.github/workflows/release.yml').read_text(encoding='utf-8')
dryrun_text = Path('.github/workflows/release-dryrun.yml').read_text(encoding='utf-8')
ship_text = Path('.github/workflows/ship.yml').read_text(encoding='utf-8')
canary_text = Path('.github/workflows/release-canary.yml').read_text(encoding='utf-8')

def job(name: str, workflow: str) -> str:
    match = re.search(
        rf'(?ms)^  {re.escape(name)}:\n.*?(?=^  [a-zA-Z0-9_-]+:\n|\Z)', workflow
    )
    assert match, f'missing {name} job'
    return match.group(0)

# 1. Release dry-run calls release.yml with publish=false.
dry_build = job('build', dryrun_text)
assert 'uses: ./.github/workflows/release.yml' in dry_build
assert 'publish: false' in dry_build
print('PASS: release-dryrun calls release.yml with publish=false')

# 2. Release publish verifies checksums, auto-generates notes, and guards skipped optional jobs.
publish = job('publish', release_text)
assert re.search(r'^    needs: \[draft, windows, cli, images, linux-deb\]$', publish, re.M)
assert 'verify_asset' in publish and 'generate-notes' in publish
pub_cond = re.search(r'^    if: \$\{\{(.*?)\}\}\s*$', publish, re.M)
assert pub_cond, 'publish has no job condition'
assert '!failure() && !cancelled()' in pub_cond.group(1)
assert 'inputs.publish' in pub_cond.group(1)
print('PASS: release::publish checks manifests/digests, generates notes, and skips on dry-run')

# 3. Ship runs ci -> promote -> release -> parallel (deploy + canary) -> cleanup.
ship_rel = job('release', ship_text)
assert 'web_bundle_artifact: ${{ needs.ci.outputs.web_bundle_artifact }}' in ship_rel
assert 'server_bins_artifact: ${{ needs.ci.outputs.server_bins_artifact }}' in ship_rel
ship_dep = job('deploy', ship_text)
ship_can = job('canary', ship_text)
assert 'uses: ./.github/workflows/deploy.yml' in ship_dep
assert 'uses: ./.github/workflows/release-canary.yml' in ship_can
assert '!inputs.dry_run && needs.release.result == \'success\'' in ship_can
ship_clean = job('cleanup', ship_text)
assert 'needs: [prepare, release, deploy, canary]' in ship_clean
assert "needs.deploy.result == 'success' && needs.canary.result == 'success'" in ship_clean
print('PASS: ship wires parallel artifact reuse, deploy + canary, and cleanup guard')

# 4. Release canary exposes workflow_call and installs pure-Rust GTK4 deps on Linux.
assert 'workflow_call:' in canary_text
assert 'libgtk-4-1' in canary_text and 'libgtk-3' not in canary_text
print('PASS: release-canary supports workflow_call with pure-Rust GTK4 runtime')
PY
