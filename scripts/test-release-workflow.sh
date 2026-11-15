#!/usr/bin/env bash
# Evaluate the Release workflow's real canary condition. Optional package jobs
# are skipped when disabled; that must not skip artifact acceptance after a
# successful publish (the v0.14.7 release gap).
set -euo pipefail
cd "$(dirname "$0")/.."
python3 - <<'PY'
from pathlib import Path
import re

text = Path('.github/workflows/release.yml').read_text(encoding='utf-8')

def job(name: str) -> str:
    match = re.search(
        rf'(?ms)^  {re.escape(name)}:\n.*?(?=^  [a-zA-Z0-9_-]+:\n|\Z)', text
    )
    assert match, f'missing {name} job'
    return match.group(0)

canary = job('canary')
assert re.search(r'^    needs: \[draft, publish\]$', canary, re.M), \
    'canary must wait for draft and publish'
assert 'uses: ./.github/workflows/release-canary.yml' in canary, \
    'canary no longer runs the artifact acceptance workflow'
assert 'version: ${{ needs.draft.outputs.version }}' in canary, \
    'canary must accept the version validated by draft'
condition = re.search(r'^    if: \$\{\{(.*?)\}\}\s*$', canary, re.M)
assert condition, 'canary has no job condition'

# Translate only the expression forms this job is allowed to use. An unknown
# new token fails the test instead of being silently ignored.
expression = condition.group(1).strip()
for source, target in [
    ('!failure()', '(not failure)'),
    ('!cancelled()', '(not cancelled)'),
    ("needs.publish.result == 'success'", 'publish_success'),
    ('&&', 'and'),
    ('||', 'or'),
]:
    expression = expression.replace(source, target)
assert re.fullmatch(r'[a-zA-Z0-9_() \']+', expression), \
    f'unhandled condition syntax: {condition.group(1)}'

raw_condition = condition.group(1)
has_status_check = any(
    token in raw_condition for token in ('failure(', 'cancelled(', 'success(', 'always(')
)

def runs(optional_skipped: bool, failure: bool, cancelled: bool, publish: str) -> bool:
    # GitHub adds an implicit success() gate to a job condition without a
    # status-check function. A skipped optional need fails that implicit gate;
    # !failure() && !cancelled() deliberately replaces it.
    status_gate = has_status_check or not (optional_skipped or failure or cancelled)
    allowed = eval(
        expression,
        {'__builtins__': {}},
        {'failure': failure, 'cancelled': cancelled, 'publish_success': publish == 'success'},
    )
    return status_gate and bool(allowed)

cases = [
    (False, False, False, 'success', True,
     'all optional packages enabled + publish succeeded => canary runs'),
    (True, False, False, 'success', True,
     'optional packages skipped + publish succeeded => canary runs'),
    (True, False, False, 'skipped', False,
     'dry-run / publish skipped => canary stays skipped'),
    (False, False, False, 'failure', False,
     'publish failed => canary stays skipped'),
    (False, True, False, 'success', False,
     'any failed need => canary stays skipped'),
    (False, False, True, 'success', False,
     'cancelled run => canary stays skipped'),
]
for optional_skipped, failure, cancelled, publish, expected, label in cases:
    actual = runs(optional_skipped, failure, cancelled, publish)
    assert actual == expected, f'{label}: got {actual}, expected {expected}'
    print(f'PASS: {label}')
PY
