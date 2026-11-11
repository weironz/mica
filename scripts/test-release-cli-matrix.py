#!/usr/bin/env python3
"""No macOS job is scheduled by default; explicit opt-in adds exactly one."""
import json
import os
from pathlib import Path
import subprocess
import sys
script = Path(__file__).with_name('release-cli-matrix.py')
for value in (None, '', 'false', 'true', 'invalid'):
    env = dict(os.environ)
    env.pop('BUILD_MACOS_CLI', None)
    if value is not None:
        env['BUILD_MACOS_CLI'] = value
    result = subprocess.run([sys.executable, str(script)], env=env, capture_output=True, text=True)
    if value == 'invalid':
        assert result.returncode != 0
        continue
    assert result.returncode == 0, result.stderr
    rows = json.loads(result.stdout.removeprefix('matrix='))['include']
    assert len(rows) == (3 if value == 'true' else 2)
    assert any(row.get('target') == 'x86_64-unknown-linux-musl' for row in rows)
    assert [row['osname'] for row in rows].count('macos') == (1 if value == 'true' else 0)
print('CLI matrix: unset/empty/false omit macOS, true includes it, invalid values fail')
