#!/usr/bin/env bash
# Guard the pure-Rust Windows release installer smoke test's isolation (#179).
set -euo pipefail
cd "$(dirname "$0")/.."
python3 - <<'PY'
from pathlib import Path
import re

smoke = Path('scripts/smoke-installer.ps1').read_text(encoding='utf-8')
canary = Path('scripts/release-canary.sh').read_text(encoding='utf-8')
iss = Path('clients/mica_rust/installer/mica.iss').read_text(encoding='utf-8')

def section(text: str, name: str) -> str:
    m = re.search(rf'(?ms)^\[{name}\]\n(.*?)(?=^\[|\Z)', text)
    assert m, f'mica.iss has no [{name}] section'
    return m.group(1)

def entries(body: str) -> list[str]:
    return [l for l in body.splitlines() if l.strip() and not l.lstrip().startswith(';')]

assert re.search(r"function IsSmokeTest\(\): Boolean;\s*begin\s*Result := CmdLineParamExists\('/MICASMOKE'\);",
                 iss), 'IsSmokeTest must read /MICASMOKE'
assert re.search(r'(?m)^CreateUninstallRegKey=not IsSmokeTest\s*$', section(iss, 'Setup')), \
    'smoke install must not write uninstall registry entry'
icons = entries(section(iss, 'Icons'))
assert icons, 'mica.iss has no shortcuts to check'
for line in icons:
    assert re.search(r'Check: not IsSmokeTest\b', line), f'shortcut created by smoke install: {line}'
for line in entries(section(iss, 'Run')):
    if '{#AppExe}' in line:
        assert 'not IsSmokeTest' in line and "not CmdLineParamExists('/SKIPRUN')" in line, \
            f'installer must not auto-launch during smoke install: {line}'
assert 'mica-rust.exe' in smoke, 'smoke-installer.ps1 must check mica-rust.exe'
assert '$UpgradeFrom' in smoke and '$UpgradeFromVersion' in smoke, \
    'smoke-installer.ps1 must support in-place upgrade smoke parameters'
assert 'upgrade-sentinel.txt' in smoke and 'Remove-Item -LiteralPath $ProbePath' in smoke, \
    'smoke-installer.ps1 must reset probe and verify profile data survives in-place upgrade'
assert 'find_previous_windows_release' in canary and '-UpgradeFrom' in canary, \
    'release-canary.sh must run in-place upgrade smoke from the previous published Windows release'
print('smoke-installer isolation & in-place upgrade contract (pure Rust): passed')
PY
