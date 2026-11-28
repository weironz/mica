#!/usr/bin/env bash
# Validate structural and conditional-execution invariants across Ship, Release,
# Release dry-run, and Release canary workflows (pure-Rust fullstack pipeline).
set -euo pipefail
cd "$(dirname "$0")/.."
python3 - <<'PY'
from pathlib import Path
import re

ci_text = Path('.github/workflows/ci.yml').read_text(encoding='utf-8')
release_text = Path('.github/workflows/release.yml').read_text(encoding='utf-8')
dryrun_text = Path('.github/workflows/release-dryrun.yml').read_text(encoding='utf-8')
ship_text = Path('.github/workflows/ship.yml').read_text(encoding='utf-8')
canary_text = Path('.github/workflows/release-canary.yml').read_text(encoding='utf-8')

# 0. Validate every workflow YAML file (forbid column-0 heredoc `---`/`...` markers inside block scalars; full PyYAML parse when installed).
for wf in sorted(Path('.github/workflows').glob('*.yml')):
    raw = wf.read_text(encoding='utf-8')
    for idx, line in enumerate(raw.splitlines(), 1):
        if idx > 1 and line in ('---', '...'):
            raise AssertionError(f'{wf}:{idx}: unexpected column-0 YAML document marker `{line}`')
    try:
        import yaml  # type: ignore
    except ImportError:
        yaml = None
    if yaml is not None:
        docs = list(yaml.safe_load_all(raw))
        assert len(docs) == 1 and isinstance(docs[0], dict), f'{wf}: expected exactly 1 YAML mapping document, got {len(docs)}'
print('PASS: all workflow YAMLs pass structural and multi-doc validation')

def job(name: str, workflow: str) -> str:
    match = re.search(
        rf'(?ms)^  {re.escape(name)}:\n.*?(?=^  [a-zA-Z0-9_-]+:\n|\Z)', workflow
    )
    assert match, f'missing {name} job'
    return match.group(0)

# 0b. CI gates heavy jobs behind `structure` (which runs workflow YAML lint + rustfmt fail-fast).
ci_structure = job('structure', ci_text)
assert 'cargo fmt --all --check' in ci_structure
assert 'test-release-workflow.sh' in ci_structure
for gated_job in ('rust', 'client', 'mcp', 'server-bins'):
    jtxt = job(gated_job, ci_text)
    assert re.search(r'^    needs: \[paths, structure\]$', jtxt, re.M), f'ci::{gated_job} must depend on [paths, structure]'
print('PASS: ci gates heavy jobs behind structure preflight (YAML lint + rustfmt)')

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
