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

# User decisions (2026-10-10 ~00:40 UTC)
6. Release 0.1.22 from PR #209 (implemented: commit `0f411f5ce`): crates.io takes `yggdryl`,
   `yggdryl-market`, `yggdryl-fix`; PyPI and npm keep `yggdryl`. The user: "upgrade to 0.1.22 to
   publish before next steps with medaillon optimized working".
7. The PR's scope before that release, in this order, each a commit read green: P9 (the
   Instrument replacing the ISIN registry, `p9/`), P7 (D40), P8 (D41), P5R (D37); then the live
   AWS run on the user's machine (`.handoff/next/LIVE_AWS_TEST_PROMPT.md`); then the user's merge,
   which publishes 0.1.22. The other crates' split - M6 (S6: excel, xmla, avro, parquet, s3,
   iceberg) and B5 (S5: the market binding packages) - and S7, S8 and S9 are postponed to a later
   PR from `main` after the release. The user: "Include the instrument implementations and add
   local live aws testing prompt to finalize this pr first and publish 0.1.22", then "ask to
   finalize next implementations postponing the othe crates split", answered "All in this PR".
8. The instrument cross code (`p9/crosscode_decision.md`): a security is keyed by its real ISIN
   alone, the CFI a fact beside it; CFI class + characteristics key everything no agency numbers
   (forex, forwards, swaps, options, futures, strategies). Answered "Bare ISIN for securities
   (Recommended)"; the decision's items 2-9 stand unless the user says otherwise.
