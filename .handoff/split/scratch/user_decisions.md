# User decisions (2026-10-09 17:05 UTC): "Do what's recommended"
Recorded by every lane manager in `.handoff/next/MARKET_SPLIT_NEXT.md`'s questions section and the
DESIGN.md rows named, in its next results commit; no lane re-asks them.
1. S6 landing shape: ONE dependency-closed commit for avro, parquet, s3 and iceberg (+s3tables),
   after excel and xmla have landed as their own commits (DESIGN.md "## S6's shape"); the reversed
   split (iceberg first) is not taken. D39 row and "S6's shape" say "decided by the user".
2. D12 npm name: `yggdryl-market` on all three registries; `yggdryl.market` is not used.
3. Handoff question 8 (D38): `sendunix` keeps the carrier-first precedence (the carrier's instant,
   else `SendingTime(52)`); the alternative is closed.
4. The CLI links `yggdryl-s3` always, as the bindings do; its `s3` feature goes (S6d).
5. Not covered - stays on the user's explicit go, by the program's hard rule against release runs:
   the manual `release.yml` rehearsal and the registry configuration a real publish needs (PyPI
   pending trusted publisher for `yggdryl-market`, the npm bootstrap publish, `CARGO_REGISTRY_TOKEN`
   with `publish-new` over `yggdryl-*`).
