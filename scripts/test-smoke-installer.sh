#!/usr/bin/env bash
# Guard the release installer smoke test's isolation (#179). The local release
# train runs scripts/smoke-installer.ps1 on a machine someone uses every day,
# so the probe must leave that machine's Mica alone:
#   - the throwaway install skips the shortcuts and the uninstall registry
#     entry it would otherwise share with the real install (/MICASMOKE);
#   - the launched app gets its own APPDATA / LOCALAPPDATA / TEMP / TMP, so it
#     never opens the real store.db, and the runner's single-instance lock is
#     scoped to that directory instead of colliding with a running Mica;
#   - everything is removed at the end.
# PowerShell, Inno Setup and the Win32 runner do not run on the Linux CI host,
# so this checks the contracts in their sources; the behaviour itself was
# verified on Windows (see the commit that added this file).
set -euo pipefail
cd "$(dirname "$0")/.."
python3 - <<'PY'
from pathlib import Path
import re

smoke = Path('scripts/smoke-installer.ps1').read_text(encoding='utf-8')
iss = Path('clients/mica_flutter/installer/mica.iss').read_text(encoding='utf-8')
runner = Path('clients/mica_flutter/windows/runner/main.cpp').read_text(encoding='utf-8')

def section(text: str, name: str) -> str:
    m = re.search(rf'(?ms)^\[{name}\]\n(.*?)(?=^\[|\Z)', text)
    assert m, f'mica.iss has no [{name}] section'
    return m.group(1)

def entries(body: str) -> list[str]:
    return [l for l in body.splitlines() if l.strip() and not l.lstrip().startswith(';')]

# --- installer: a smoke install leaves no trace on the real install ---------
assert re.search(r"function IsSmokeTest\(\): Boolean;\s*begin\s*Result := CmdLineParamExists\('/MICASMOKE'\);",
                 iss), 'IsSmokeTest must read the /MICASMOKE switch'
assert re.search(r'(?m)^CreateUninstallRegKey=not IsSmokeTest\s*$', section(iss, 'Setup')), \
    'a smoke install must not write the uninstall registry entry it shares with the real install'
icons = entries(section(iss, 'Icons'))
assert icons, 'mica.iss has no shortcuts to check'
for line in icons:
    assert re.search(r'Check: not IsSmokeTest\b', line), f'shortcut created by a smoke install: {line}'
for line in entries(section(iss, 'Run')):
    if '{#AppExe}' in line:
        assert 'not IsSmokeTest' in line and "not CmdLineParamExists('/SKIPRUN')" in line, \
            f'the installer must not auto-launch during a smoke install: {line}'

# --- smoke script ------------------------------------------------------------
setup_call = re.search(r'(?s)\$inst = Start-Process -FilePath \$Setup(.*?)-PassThru', smoke)
assert setup_call, 'smoke script no longer starts the installer'
for switch in ('/MICASMOKE', '/SKIPRUN', '/VERYSILENT'):
    # /SKIPRUN stays for installers built before /MICASMOKE (release canary).
    assert f"'{switch}'" in setup_call.group(1), f'installer must be started with {switch}'
assert re.search(r'\$installDir = Join-Path \$root ', smoke), 'install must live under $root'

isolated = re.search(r'(?s)\$isolated = \[ordered\]@\{(.*?)\n  \}', smoke)
assert isolated, 'smoke script has no isolated environment table'
names = set(re.findall(r'(?m)^\s*([A-Z_]+) = ', isolated.group(1)))
required = {'APPDATA', 'LOCALAPPDATA', 'TEMP', 'TMP', 'MICA_STARTUP_PROBE_FILE'}
assert required <= names, f'isolated environment is missing {sorted(required - names)}'
for line in isolated.group(1).splitlines():
    if line.strip() and not line.strip().startswith('MICA_STARTUP_PROBE_FILE'):
        assert '$profileDir' in line, f'isolated directory outside the throwaway profile: {line.strip()}'

# The app — and only the app — starts inside the isolated environment, and the
# caller's environment is restored in a finally block.
launch = re.search(
    r'(?s)try \{\s*foreach \(\$name in \$isolated\.Keys\) \{\s*'
    r"\[Environment\]::SetEnvironmentVariable\(\$name, \$isolated\[\$name\], 'Process'\)\s*\}\s*"
    r'\$app = Start-Process -FilePath \$exe[^\n]*\n\s*\} finally \{\s*'
    r'foreach \(\$name in \$isolated\.Keys\) \{\s*'
    r"\[Environment\]::SetEnvironmentVariable\(\$name, \$previous\[\$name\], 'Process'\)",
    smoke)
assert launch, 'the app must be launched with the isolated environment and the caller environment restored'
assert smoke.index('$inst = Start-Process') < launch.start(), 'the installer must run with the caller environment'
assert "Join-Path $isolated.APPDATA 'mica'" in smoke, \
    'the smoke test must prove the app wrote its profile under the isolated APPDATA'

final = smoke[smoke.rindex('} finally {'):]
assert final.index('unins000.exe') < final.index('Remove-Item -LiteralPath $root -Recurse -Force'), \
    'the throwaway root must be removed after the uninstall'

# --- runner: the lock follows the data directory ----------------------------
assert 'GetEnvironmentVariableW(L"APPDATA"' in runner, 'lock name must follow APPDATA'
assert 'FOLDERID_RoamingAppData' in runner, \
    'the default profile must keep the historical lock name (older running builds use it)'
assert re.search(r'CreateMutexW\(nullptr, TRUE, mutex_name\.c_str\(\)\)', runner), \
    'the guard must use the data-directory-scoped name'
for literal in ('L"Local\\\\MicaSingleInstance"', 'L"Local\\\\MicaSingleInstanceDev"'):
    assert literal in runner, f'historical lock name {literal} must stay'
assert all(ord(c) < 128 for c in runner), 'runner sources are compiled warning-as-error: keep them ASCII'

print('smoke-installer isolation: ok')
PY
