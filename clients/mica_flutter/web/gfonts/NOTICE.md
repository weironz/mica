# Same-origin mirror of the Flutter web engine's runtime fonts

Maintained by `tool/gfonts/mirror.sh` — do not edit by hand. Every `.woff2`
here is a byte-exact copy of what the engine would otherwise fetch from
`https://fonts.gstatic.com/s/` at runtime (`web/flutter_bootstrap.js` points
`fontFallbackBaseUrl` at this directory). Paths mirror the gstatic layout
(`<family>/v<N>/<hash>[.<shard>].woff2`) because the engine's compiled-in
fallback tables reference them verbatim.

Contents: `roboto/` (the engine's default text font, fetched on every page
load) plus every fallback family in the engine's tables EXCEPT the four CJK
siblings `notosansjp`/`notosanskr`/`notosanstc`/`notosanshk` (~9.5 MB whose
pan-CJK repertoire the bundled Noto Sans SC + DroidSansFallback already cover)
— the exclusion list lives in `tool/gfonts/mirror.sh`.

- **Noto fonts** (`noto*/`) — © The Noto Project Authors, SIL Open Font
  License 1.1 (<https://openfontlicense.org>).
- **Roboto** (`roboto/`) — © Google, Apache License 2.0
  (<https://www.apache.org/licenses/LICENSE-2.0>).
