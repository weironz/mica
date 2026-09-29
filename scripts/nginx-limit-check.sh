#!/usr/bin/env bash
# The nginx body-size rule, as a command that REFUSES rather than a comment
# somebody has to remember.
#
# The shape being prevented: `client_max_body_size` was `1g` at `http` scope,
# set for the ZIP-import route — so every unauthenticated path inherited it too.
# Nginx buffers a request body before the app sees it, so a 1 GiB allowance at
# the edge is up to 1 GiB buffered per connection BEFORE the app's own 2/8 MiB
# limits can refuse. Nothing on the anonymous paths sends more than a few
# kilobytes.
#
# Would a plain `grep` in CI do? Not quite, and the difference is the whole
# point: the failure mode is a big number appearing at http SCOPE, and a grep
# for "client_max_body_size 1g" matches the legitimate per-location one too. So
# this reads the config as blocks and asserts WHERE the big value may appear.
set -euo pipefail

CONF="${1:-deploy/nginx.conf}"
ROUTES="${2:-crates/api-server/src/routes/mod.rs}"

fail() { printf '\n  REFUSED: %s\n' "$*" >&2; exit 1; }

[ -f "$CONF" ] || fail "no nginx config at $CONF"

# The routes file is checked too: nginx refuses an oversized body before the app
# is involved, so if the two limits drift apart the symptom is a legitimate
# import failing at the edge — or the edge allowing bodies the app then rejects.
[ -f "$ROUTES" ] || fail "no routes file at $ROUTES"

python - "$CONF" "$ROUTES" <<'PY' || exit 1
import re
import sys

conf_path, routes_path = sys.argv[1], sys.argv[2]
text = open(conf_path, encoding='utf-8').read()

def die(msg):
    print(f"\n  REFUSED: {msg}", file=sys.stderr)
    sys.exit(1)

def to_bytes(spec):
    """nginx size suffix → bytes. `1m`/`1g`/plain digits."""
    spec = spec.strip().lower()
    unit = spec[-1]
    if unit.isdigit():
        return int(spec)
    n = int(spec[:-1])
    return n * {'k': 1024, 'm': 1024**2, 'g': 1024**3}[unit]

# ── 1. The default (http-scope) cap must be small ───────────────────────────
# Anchored to line start so a nested `client_max_body_size` is not mistaken for
# the http-level one: nested directives are indented in this file, and that
# indentation is the only structural signal we can rely on without an nginx
# parser. A permissive default is exactly the bug.
m = re.search(r'^  client_max_body_size\s+(\S+);', text, re.M)
if not m:
    die(f"{conf_path}: no http-scope `client_max_body_size` — the anonymous "
        "surface then inherits nginx's default (1m) only by accident; state it")
default = to_bytes(m.group(1))
ONE_MIB = 1024 * 1024
if default > ONE_MIB:
    die(f"{conf_path}: the http-scope body cap is {m.group(1)!r} ({default} bytes). "
        f"Every unauthenticated path inherits it (sign-in, password reset, public "
        f"blobs). It must be <= 1m; raise it on the ONE route that needs it.")

# ── 2. The import route must still allow the big upload ─────────────────────
# Exact match (`location =`) on the upload path, with the 1g inside it. A prefix
# match would re-open the anonymous surface: everything under /api/workspaces/
# inherits it.
loc = re.search(
    r'location\s+=\s+(/api/[^\s{]*import[^\s{]*)\s*\{(.*?)\n    \}',
    text, re.S)
if not loc:
    die(f"{conf_path}: no exact-match `location = <path>import<path>` block. Without it "
        "the ZIP import inherits the 1m default and real archives are refused "
        "at the edge.")
loc_path, body = loc.group(1), loc.group(2)
if 'client_max_body_size' not in body:
    die(f"{conf_path}: `{loc_path}` does not raise `client_max_body_size`, so it "
        "is still capped at the default and large imports fail with 413.")
big = to_bytes(re.search(r'client_max_body_size\s+(\S+);', body).group(1))
if big < 64 * 1024 * 1024:
    die(f"{conf_path}: `{loc_path}` allows only {big} bytes; a workspace ZIP "
        "archive routinely exceeds that.")

# ── 3. Both files must name the SAME path ───────────────────────────────────
# Nginx refuses the oversized body first, so a mismatch shows up as "import
# fails on big archives" with nothing in the app logs.
routes = open(routes_path, encoding='utf-8').read()
m2 = re.search(r'"(/workspaces/import)"', routes)
if not m2:
    die(f"{routes_path}: the import route path moved; {conf_path} names "
        f"`{loc_path}` and the two must agree.")
expected = f"/api{m2.group(1)}"
if loc_path != expected:
    die(f"path drift: nginx raises the cap for `{loc_path}` but the route is "
        f"`{expected}`. A big import would be refused at the edge.")

print(f"==> nginx body cap: default {m.group(1)} | {loc_path} {big // 1024**3}g"
      f" ({big} bytes), matches {routes_path}")
PY

echo "==> body-size rule OK"
