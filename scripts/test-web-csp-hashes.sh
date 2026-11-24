#!/usr/bin/env bash
# Enforce that deploy/nginx.conf and deploy/nginx.dev.conf allow index.html's
# inline <script> blocks strictly by sha256 hash (zero 'unsafe-inline' in script-src),
# and that both configs stay in lockstep with clients/mica_rust/web/index.html.
set -euo pipefail
cd "$(dirname "$0")/.."
python3 - <<'PY'
import base64, hashlib, re
from pathlib import Path

html = Path('clients/mica_rust/web/index.html').read_text(encoding='utf-8')
inline = re.findall(r'<script\b[^>]*>(.*?)</script>', html, re.S)
assert inline, 'clients/mica_rust/web/index.html has no inline scripts to check'
want = {
    "'sha256-%s'" % base64.b64encode(hashlib.sha256(s.encode('utf-8')).digest()).decode('ascii')
    for s in inline
}
for conf in ('deploy/nginx.conf', 'deploy/nginx.dev.conf'):
    text = Path(conf).read_text(encoding='utf-8')
    m = re.search(r"script-src ('self'[^;]*);", text)
    assert m, f'{conf}: no script-src in the CSP'
    directive = m.group(1)
    assert "'unsafe-inline'" not in directive, f"{conf}: script-src must not allow 'unsafe-inline'"
    have = set(re.findall(r"'sha256-[A-Za-z0-9+/=]+'", directive))
    missing, stale = want - have, have - want
    assert not missing, f'{conf}: script-src lacks {sorted(missing)} (an index.html inline script changed)'
    assert not stale, f'{conf}: script-src still allows {sorted(stale)}, which matches no inline script'
print(f'web CSP hashes: {len(want)} inline script(s) pinned in both nginx configs ({sorted(want)[0]})')
PY
