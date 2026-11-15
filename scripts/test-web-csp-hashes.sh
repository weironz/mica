#!/usr/bin/env bash
# The report-only CSP in deploy/nginx.conf and deploy/nginx.dev.conf allows
# index.html's inline scripts by hash. Edit one of those scripts without
# updating the hashes and every page load reports a violation, and the policy
# can never be switched to enforcing. Nothing else would notice: the policy is
# report-only today. This keeps the two in step, both ways (no stale extras).
set -euo pipefail
cd "$(dirname "$0")/.."
python3 - <<'PY'
import base64, hashlib, re
from pathlib import Path

html = Path('clients/mica_flutter/web/index.html').read_text(encoding='utf-8')
inline = re.findall(r'<script>(.*?)</script>', html, re.S)
assert inline, 'index.html has no inline scripts to check'
want = {
    "'sha256-%s'" % base64.b64encode(hashlib.sha256(s.encode()).digest()).decode()
    for s in inline
}
for conf in ('deploy/nginx.conf', 'deploy/nginx.dev.conf'):
    text = Path(conf).read_text(encoding='utf-8')
    m = re.search(r"script-src ('self'[^;]*);", text)
    assert m, f'{conf}: no script-src in the CSP'
    have = set(re.findall(r"'sha256-[A-Za-z0-9+/=]+'", m.group(1)))
    missing, stale = want - have, have - want
    assert not missing, f'{conf}: script-src lacks {sorted(missing)} (an index.html inline script changed)'
    assert not stale, f'{conf}: script-src still allows {sorted(stale)}, which matches no inline script'
print(f'web CSP hashes: {len(want)} inline scripts allowed in both nginx configs')
PY
