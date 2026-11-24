#!/usr/bin/env python3
"""Group a wasm module's code size by crate, from `twiggy top --format csv`.

Build the web bundle WITHOUT stripping so symbol names survive:
  CARGO_PROFILE_WEB_STRIP=false cargo build --manifest-path clients/mica_rust/Cargo.toml \
    --lib --target wasm32-unknown-unknown --profile web --locked
  twiggy top -n 100000 --format csv <target>/wasm32-unknown-unknown/web/mica_rust.wasm > twiggy.csv
  python3 -I size-breakdown.py twiggy.csv
Sizes are uncompressed code bytes; use them to compare before/after, not as transfer size.
"""
import collections
import csv
import re
import sys

GROUPS = {
    "merman": "mermaid", "dugong": "mermaid", "ratex": "math", "naga": "naga (shader)",
    "core": "std/core", "alloc": "std/core", "std": "std/core", "hashbrown": "std/core",
    "serde": "serde", "serde_json": "serde", "regex": "regex", "regex_automata": "regex",
    "regex_syntax": "regex", "fancy_regex": "regex", "yrs": "yrs (CRDT)", "taffy": "nsh (layout)",
}
NON_CODE = ('"function names"', "data segment", "custom section", "type[", "elem", "import",
            "export", "table", "global", "memory")
crate_re = re.compile(r"([A-Za-z_][A-Za-z0-9_]*)(?:\[[0-9a-f]+\])?::")

totals, other = collections.Counter(), collections.Counter()
for row in csv.DictReader(open(sys.argv[1], encoding="utf-8")):
    name, size = row["Name"], int(row["ShallowSize"])
    if name.startswith(NON_CODE):
        other[name] += size
        continue
    m = crate_re.search(name)
    crate = m.group(1) if m else "unnamed"
    group = GROUPS.get(crate) or ("nsh" if crate.startswith("nsh") else crate)
    totals[group] += size
for name, size in other.most_common(4):
    print(f"{size / 1048576:7.2f} MiB  {name}")
print(f"{sum(totals.values()) / 1048576:7.2f} MiB  CODE TOTAL")
for group, size in totals.most_common(25):
    print(f"{size / 1048576:7.2f} MiB  {group}")
