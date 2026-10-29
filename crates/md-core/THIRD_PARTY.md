# Third-party licenses

`mica-md` itself is `MIT OR Apache-2.0`. This file lists every dependency it
can pull in, per feature set, with that dependency's declared license. Audit
date: 2026-10-29 (crates.io metadata). Direct dependencies are named; the
rest arrive through them.

## Default features (`uuid`)

All permissive (MIT / Apache-2.0 / Unlicense families):

| crate | license | via |
| --- | --- | --- |
| `serde` (serde, serde_derive, serde_core) | MIT OR Apache-2.0 | direct |
| `serde_json` (itoa, memchr, ryu) | MIT OR Apache-2.0 | direct |
| `thiserror` (thiserror-impl, proc-macro2, quote, syn, unicode-ident) | MIT OR Apache-2.0 | direct |
| `uuid` (getrandom, cfg-if) | Apache-2.0 OR MIT | direct |
| `zmij` | MIT | direct (uuid id formatting) |

No copyleft license in the default build.

## `render` feature

Adds the diagram/math renderers and their trees:

| crate | license | via |
| --- | --- | --- |
| `merman` (+ merman-core, merman-render, roughr-merman) | MIT OR Apache-2.0 | direct |
| `ratex-parser`, `ratex-layout`, `ratex-svg`, `ratex-types`, `ratex-font`, `ratex-katex-fonts`, `ratex-unicode-font`, `ratex-font-loader` | MIT | direct |

Their transitive trees (resvg, fontdb, cssparser, icu, url, chrono,
phf, tracing, … — ~200 crates) are, as of the audit date, the standard
permissive set used by the Rust graphics ecosystem (MIT / Apache-2.0 /
BSD / ISC / Unicode-3.0); no GPL/AGPL/unknown-license crate was found via
crates.io metadata. Re-audit on any `render`-feature dependency bump with:

```bash
cargo tree -p mica-md -e normal --prefix none --features render \
  | grep -oE '^[a-z0-9_-]+ v[0-9]+' | sort -u
```

and check each new crate's `license` field.

## Conclusion

Both feature sets are compatible with `MIT OR Apache-2.0` distribution; no
separate licensing note is required for the `render` feature.