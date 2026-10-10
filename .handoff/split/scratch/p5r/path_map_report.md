# P5R path map report

Source `p5_on_p4.patch`: 115 sections. Mapped 106 (`p5_on_s4p9.patch`), routed 25 hunks into 8 files (`p5_on_s4p9.routed.patch`), unrouted 1 hunks (`p5_on_s4p9.unrouted.patch`), dropped 2 sections.

Destinations are checked against `git ls-tree -r HEAD` and the working tree (P9 in flight).

## Mapped (one destination, `git apply -3`)

| Patch path | Destination | Kind | Hunks | Old side verbatim | Destination is |
| --- | --- | --- | --- | --- | --- |
| `.api-bindings.txt` | `(unchanged)` | modified | 9 | 8/9 | HEAD |
| `.api-inventory.txt` | `(unchanged)` | modified | 8 | 2/8 | HEAD |
| `AGENTS.md` | `(unchanged)` | modified | 2 | 0/2 | HEAD |
| `docs/fix/arrow.md` | `(unchanged)` | modified | 3 | 3/3 | HEAD |
| `docs/fix/capture.md` | `(unchanged)` | modified | 14 | 12/14 | HEAD |
| `docs/fix/encode.md` | `(unchanged)` | modified | 1 | 1/1 | HEAD |
| `docs/fix/index.md` | `(unchanged)` | modified | 1 | 1/1 | HEAD |
| `docs/fix/lifecycle.md` | `(unchanged)` | modified | 13 | 12/13 | HEAD |
| `docs/fix/message.md` | `(unchanged)` | modified | 24 | 23/24 | HEAD |
| `docs/fix/registry.md` | `(unchanged)` | modified | 1 | 1/1 | HEAD |
| `docs/graph/event.md` | `(unchanged)` | modified | 1 | 1/1 | HEAD |
| `docs/graph/index.md` | `(unchanged)` | modified | 1 | 1/1 | HEAD |
| `docs/graph/market-data.md` | `(unchanged)` | modified | 5 | 3/5 | HEAD |
| `docs/graph/market.md` | `(unchanged)` | modified | 1 | 1/1 | HEAD |
| `docs/graph/operation.md` | `(unchanged)` | modified | 1 | 1/1 | HEAD |
| `docs/graph/trade.md` | `(unchanged)` | modified | 1 | 1/1 | HEAD |
| `docs/types/enum/timeinforce.md` | `(unchanged)` | modified | 1 | 1/1 | HEAD |
| `node/binding.d.ts` | `(unchanged)` | modified | 4 | 4/4 | HEAD |
| `node/binding.js` | `(unchanged)` | modified | 4 | 4/4 | HEAD |
| `node/src/fix.rs` | `(unchanged)` | modified | 26 | 25/26 | HEAD |
| `node/src/graph/market_data.rs` | `(unchanged)` | modified | 8 | 7/8 | HEAD |
| `node/src/graph/message.rs` | `(unchanged)` | new | 1 | 1/1 | new (absent, as expected) |
| `node/src/graph/mod.rs` | `(unchanged)` | modified | 2 | 2/2 | HEAD |
| `node/src/isin_registry.rs` | `node/src/instrument.rs` | modified | 3 | 3/3 | P9 (working tree, not at HEAD) |
| `node/src/lib.rs` | `(unchanged)` | modified | 1 | 1/1 | HEAD |
| `node/tests/graph/index.test.js` | `(unchanged)` | modified | 2 | 2/2 | HEAD |
| `node/tests/graph/index.types.ts` | `(unchanged)` | modified | 2 | 2/2 | HEAD |
| `node/tests/graph/market_data.test.js` | `(unchanged)` | modified | 4 | 4/4 | HEAD |
| `python/benchmarks/fix.py` | `(unchanged)` | modified | 1 | 1/1 | HEAD |
| `python/src/fix.rs` | `(unchanged)` | modified | 23 | 22/23 | HEAD |
| `python/src/graph/anomaly.rs` | `(unchanged)` | new | 1 | 1/1 | new (absent, as expected) |
| `python/src/graph/market_data.rs` | `(unchanged)` | modified | 8 | 7/8 | HEAD |
| `python/src/graph/message.rs` | `(unchanged)` | new | 1 | 1/1 | new (absent, as expected) |
| `python/src/graph/mod.rs` | `(unchanged)` | modified | 11 | 8/11 | HEAD |
| `python/src/isin_registry.rs` | `python/src/instrument.rs` | modified | 2 | 2/2 | P9 (working tree, not at HEAD) |
| `python/tests/enums/test_init.py` | `(unchanged)` | modified | 1 | 1/1 | HEAD |
| `python/tests/graph/test_anomaly.py` | `(unchanged)` | new | 1 | 1/1 | new (absent, as expected) |
| `python/tests/graph/test_init.py` | `(unchanged)` | modified | 2 | 2/2 | HEAD |
| `python/tests/graph/test_market_data.py` | `(unchanged)` | modified | 3 | 3/3 | HEAD |
| `python/tests/graph/test_message.py` | `(unchanged)` | new | 1 | 1/1 | new (absent, as expected) |
| `python/tests/test_fix.py` | `(unchanged)` | modified | 16 | 16/16 | HEAD |
| `python/tests/test_logging.py` | `(unchanged)` | modified | 1 | 1/1 | HEAD |
| `python/tests/typing_bindings.py` | `(unchanged)` | modified | 3 | 3/3 | HEAD |
| `python/yggdryl/_native.pyi` | `(unchanged)` | modified | 11 | 11/11 | HEAD |
| `python/yggdryl/fix.py` | `(unchanged)` | modified | 1 | 1/1 | HEAD |
| `python/yggdryl/graph/__init__.py` | `(unchanged)` | modified | 6 | 6/6 | HEAD |
| `rust/benchmarks/fix/pipeline.rs` | `rust/fix/benchmarks/fix/pipeline.rs` | modified | 14 | 12/14 | HEAD |
| `rust/benchmarks/fix/resolve.rs` | `rust/fix/benchmarks/fix/resolve.rs` | modified | 1 | 1/1 | HEAD |
| `rust/benchmarks/fix/ulbridge.rs` | `rust/fix/benchmarks/fix/ulbridge.rs` | modified | 4 | 4/4 | HEAD |
| `rust/benchmarks/graph.rs` | `rust/market/benchmarks/graph.rs` | modified | 1 | 0/1 | HEAD |
| `rust/benchmarks/graph/message.rs` | `rust/market/benchmarks/graph/message.rs` | new | 1 | 1/1 | new (absent, as expected) |
| `rust/benchmarks/graph/mod.rs` | `rust/market/benchmarks/graph/mod.rs` | modified | 1 | 0/1 | HEAD |
| `rust/src/fix/anomaly.rs` | `rust/fix/src/anomaly.rs` | deleted | 1 | 1/1 | HEAD |
| `rust/src/fix/batch.rs` | `rust/fix/src/batch.rs` | modified | 3 | 2/3 | HEAD |
| `rust/src/fix/build.rs` | `rust/fix/src/build.rs` | modified | 13 | 12/13 | HEAD |
| `rust/src/fix/codec.rs` | `rust/fix/src/codec.rs` | modified | 12 | 10/12 | HEAD |
| `rust/src/fix/enrich.rs` | `rust/fix/src/enrich.rs` | modified | 25 | 17/25 | HEAD |
| `rust/src/fix/identity.rs` | `rust/fix/src/identity.rs` | modified | 13 | 12/13 | HEAD |
| `rust/src/fix/latest.rs` | `rust/fix/src/latest.rs` | modified | 5 | 5/5 | HEAD |
| `rust/src/fix/market.rs` | `rust/fix/src/market.rs` | modified | 34 | 27/34 | HEAD |
| `rust/src/fix/messages.rs` | `rust/fix/src/messages.rs` | modified | 4 | 4/4 | HEAD |
| `rust/src/fix/mod.rs` | `rust/fix/src/lib.rs` | modified | 3 | 2/3 | HEAD |
| `rust/src/fix/msg.rs` | `rust/fix/src/msg.rs` | modified | 144 | 111/144 | HEAD |
| `rust/src/fix/native_derivations.rs` | `rust/fix/src/native_derivations.rs` | modified | 2 | 2/2 | HEAD |
| `rust/src/fix/schema.rs` | `rust/fix/src/schema.rs` | modified | 3 | 2/3 | HEAD |
| `rust/src/graph/anomaly.rs` | `rust/market/src/graph/anomaly.rs` | new | 1 | 1/1 | new (absent, as expected) |
| `rust/src/graph/arrow.rs` | `rust/market/src/graph/arrow.rs` | modified | 6 | 5/6 | HEAD |
| `rust/src/graph/book.rs` | `rust/market/src/graph/book.rs` | modified | 4 | 4/4 | HEAD |
| `rust/src/graph/iterator.rs` | `rust/market/src/graph/iterator.rs` | modified | 4 | 4/4 | HEAD |
| `rust/src/graph/kind.rs` | `rust/market/src/graph/kind.rs` | modified | 7 | 6/7 | HEAD |
| `rust/src/graph/market_data.rs` | `rust/market/src/graph/market_data.rs` | modified | 11 | 9/11 | HEAD |
| `rust/src/graph/message.rs` | `rust/market/src/graph/message.rs` | new | 1 | 1/1 | new (absent, as expected) |
| `rust/src/isin_registry.rs` | `rust/market/src/instrument.rs` | modified | 4 | 0/4 | P9 (working tree, not at HEAD) |
| `rust/src/typed.rs` | `(unchanged)` | modified | 1 | 1/1 | HEAD |
| `rust/tests/fix.rs` | `rust/fix/tests/root.rs` | modified | 5 | 3/5 | HEAD |
| `rust/tests/fix/alias_rule.rs` | `rust/fix/tests/root/alias_rule.rs` | modified | 4 | 4/4 | HEAD |
| `rust/tests/fix/anomaly.rs` | `rust/fix/tests/root/anomaly.rs` | deleted | 1 | 1/1 | HEAD |
| `rust/tests/fix/batch.rs` | `rust/fix/tests/root/batch.rs` | modified | 41 | 38/41 | HEAD |
| `rust/tests/fix/cfi.rs` | `rust/fix/tests/root/cfi.rs` | modified | 3 | 3/3 | HEAD |
| `rust/tests/fix/codec.rs` | `rust/fix/tests/root/codec.rs` | modified | 49 | 48/49 | HEAD |
| `rust/tests/fix/component.rs` | `rust/fix/tests/root/component.rs` | modified | 1 | 1/1 | HEAD |
| `rust/tests/fix/crated.rs` | `rust/fix/tests/root/crated.rs` | modified | 26 | 25/26 | HEAD |
| `rust/tests/fix/digest.rs` | `rust/fix/tests/root/digest.rs` | modified | 7 | 5/7 | HEAD |
| `rust/tests/fix/enrich.rs` | `rust/fix/tests/root/enrich.rs` | modified | 61 | 57/61 | HEAD |
| `rust/tests/fix/entry.rs` | `rust/fix/tests/root/entry.rs` | modified | 9 | 9/9 | HEAD |
| `rust/tests/fix/forex.rs` | `rust/fix/tests/root/forex.rs` | modified | 7 | 7/7 | HEAD |
| `rust/tests/fix/identity.rs` | `rust/fix/tests/root/identity.rs` | modified | 17 | 12/17 | HEAD |
| `rust/tests/fix/idmap.rs` | `rust/fix/tests/root/idmap.rs` | modified | 8 | 7/8 | HEAD |
| `rust/tests/fix/latest.rs` | `rust/fix/tests/root/latest.rs` | modified | 4 | 4/4 | HEAD |
| `rust/tests/fix/market.rs` | `rust/fix/tests/root/market.rs` | modified | 92 | 67/92 | HEAD |
| `rust/tests/fix/mod_.rs` | `rust/fix/tests/root/lib.rs` | modified | 4 | 4/4 | HEAD |
| `rust/tests/fix/msg.rs` | `rust/fix/tests/root/msg.rs` | modified | 115 | 99/115 | HEAD |
| `rust/tests/fix/schema.rs` | `rust/fix/tests/root/schema.rs` | modified | 17 | 17/17 | HEAD |
| `rust/tests/fix/securityids.rs` | `rust/fix/tests/root/securityids.rs` | modified | 26 | 17/26 | HEAD |
| `rust/tests/fix/store.rs` | `rust/fix/tests/root/store.rs` | modified | 1 | 1/1 | HEAD |
| `rust/tests/fix/ulbridge.rs` | `rust/fix/tests/root/ulbridge.rs` | modified | 24 | 18/24 | HEAD |
| `rust/tests/graph/anomaly.rs` | `rust/market/tests/graph/anomaly.rs` | new | 1 | 1/1 | new (absent, as expected) |
| `rust/tests/graph/kind.rs` | `rust/market/tests/graph/kind.rs` | modified | 6 | 4/6 | HEAD |
| `rust/tests/graph/message.rs` | `rust/market/tests/graph/message.rs` | new | 1 | 1/1 | new (absent, as expected) |
| `skills/yggdryl-fix/SKILL.md` | `(unchanged)` | modified | 5 | 4/5 | HEAD |
| `skills/yggdryl-fix/references/javascript.md` | `(unchanged)` | modified | 1 | 1/1 | HEAD |
| `skills/yggdryl-fix/references/python.md` | `(unchanged)` | modified | 2 | 2/2 | HEAD |
| `skills/yggdryl-fix/references/rust.md` | `(unchanged)` | modified | 14 | 12/14 | HEAD |
| `skills/yggdryl-market-data/SKILL.md` | `(unchanged)` | modified | 2 | 2/2 | HEAD |
| `skills/yggdryl-market-data/references/python.md` | `(unchanged)` | modified | 1 | 1/1 | HEAD |
| `skills/yggdryl-market-data/references/rust.md` | `(unchanged)` | modified | 1 | 1/1 | HEAD |

## Merge preview (`git apply -3`'s merge, against the tree as it stands)

58 of 94 modified files merge clean; the rest:

| Destination | Conflicts |
| --- | --- |
| `.api-bindings.txt` | 1 |
| `.api-inventory.txt` | 7 |
| `AGENTS.md` | 2 |
| `docs/fix/lifecycle.md` | 1 |
| `docs/fix/message.md` | 1 |
| `docs/graph/market-data.md` | 1 |
| `node/src/fix.rs` | 1 |
| `node/src/graph/market_data.rs` | 1 |
| `python/src/fix.rs` | 1 |
| `python/src/graph/market_data.rs` | 1 |
| `python/src/graph/mod.rs` | 3 |
| `rust/fix/benchmarks/fix/pipeline.rs` | 3 |
| `rust/fix/src/batch.rs` | 1 |
| `rust/fix/src/codec.rs` | 1 |
| `rust/fix/src/enrich.rs` | 8 |
| `rust/fix/src/identity.rs` | 1 |
| `rust/fix/src/market.rs` | 6 |
| `rust/fix/src/msg.rs` | 24 |
| `rust/fix/tests/root.rs` | 2 |
| `rust/fix/tests/root/batch.rs` | 3 |
| `rust/fix/tests/root/codec.rs` | 1 |
| `rust/fix/tests/root/crated.rs` | 1 |
| `rust/fix/tests/root/digest.rs` | 1 |
| `rust/fix/tests/root/enrich.rs` | 3 |
| `rust/fix/tests/root/idmap.rs` | 1 |
| `rust/fix/tests/root/market.rs` | 19 |
| `rust/fix/tests/root/msg.rs` | 8 |
| `rust/fix/tests/root/securityids.rs` | 10 |
| `rust/fix/tests/root/ulbridge.rs` | 4 |
| `rust/market/benchmarks/graph.rs` | 1 |
| `rust/market/benchmarks/graph/mod.rs` | 1 |
| `rust/market/src/graph/arrow.rs` | 1 |
| `rust/market/src/graph/market_data.rs` | 1 |
| `rust/market/src/instrument.rs` | 4 |
| `rust/market/tests/graph/kind.rs` | 1 |
| `skills/yggdryl-fix/references/rust.md` | 1 |

## Routed by hunk (S4 split the file; `git apply --reject`)

| Patch path | Destination | Hunks | Old side verbatim | Destination is |
| --- | --- | --- | --- | --- |
| `rust/tests/allocations.rs` | `rust/fix/tests/allocations.rs` | 7 | 4/7 | HEAD |
| `rust/tests/graph/iterator.rs` | `rust/fix/tests/graph/iterator.rs` | 4 | 3/4 | HEAD |
| `rust/tests/graph/market_data.rs` | `rust/fix/tests/graph/market_data.rs` | 2 | 1/2 | HEAD |
| `rust/src/graph/mod.rs` | `rust/market/src/graph/mod.rs` | 5 | 2/5 | HEAD |
| `rust/tests/allocations.rs` | `rust/market/tests/allocations.rs` | 1 | 0/1 | HEAD |
| `rust/tests/graph.rs` | `rust/market/tests/graph.rs` | 2 | 2/2 | HEAD |
| `rust/tests/graph/market_data.rs` | `rust/market/tests/graph/market_data.rs` | 1 | 0/1 | HEAD |
| `rust/src/implementer.rs` | `rust/src/implementer.rs` | 3 | 3/3 | HEAD |

## Unrouted hunks (no candidate holds the old side; by hand)

- `rust/src/lib.rs` `@@ -203,22 +203,22 @@ pub use fix::{` - scores: rust/fix/src/lib.rs 0.14, rust/src/lib.rs 0.05, rust/market/src/lib.rs 0.00

## Dropped

- `.handoff/split/DESIGN.md` (modified, 1 hunks): this lane writes its own record
- `rust/examples/fix_capture.rs` (modified, 2 hunks): S4 deleted `rust/examples/`

## Not mapped: paths missing from both HEAD and the working tree

None.

## Old names in the added lines (re-spell after apply; never write back)

| Patch path | Names |
| --- | --- |
| `.api-bindings.txt` | P7: cficode x3, P7: countrycode x1, P7: eusipacode x1, P7: isincode x3, P7: miccode x3, P9: IsinRegistry x2, P9: isin_registry x1 |
| `.api-inventory.txt` | P7: countrycode x2, P7: eusipacode x2, P9: IsinRegistry x4 |
| `AGENTS.md` | P7: cficode x1, P7: countrycode x1, P7: eusipacode x3, P7: forexcode x1, P7: isincode x2, P7: miccode x2, P9: IsinEntry x1, P9: IsinRegistry x4, P9: IsinTable x1, P9: YGGDRYL_ISIN_REGISTRY_URI x1, P9: isin_registry x6, deleted: BookService x1, deleted: FixLifted x1 |
| `docs/fix/capture.md` | P7: miccode x2 |
| `docs/fix/lifecycle.md` | P7: isincode x2 |
| `docs/fix/message.md` | P7: cficode x1, P7: miccode x1 |
| `docs/graph/market-data.md` | P7: countrycode x1, P7: eusipacode x1 |
| `node/src/fix.rs` | P7: isincode x1 |
| `python/src/fix.rs` | P7: cficode x1, P7: isincode x1, P7: miccode x1 |
| `python/src/graph/message.rs` | P7: countrycode x3, P7: eusipacode x3 |
| `python/tests/graph/test_message.py` | P7: countrycode x1, P7: eusipacode x1 |
| `python/tests/typing_bindings.py` | P7: countrycode x1 |
| `python/yggdryl/_native.pyi` | P7: cficode x1, P7: countrycode x1, P7: eusipacode x1, P7: isincode x1, P7: miccode x1 |
| `rust/benchmarks/fix/ulbridge.rs` | P7: bloombergcode x1 |
| `rust/src/fix/codec.rs` | P7: cficode x1, P7: miccode x1, P9: IsinRegistry x1 |
| `rust/src/fix/enrich.rs` | P7: cficode x6, P7: miccode x6, P9: IsinRegistry x1 |
| `rust/src/fix/identity.rs` | P7: cficode x1, P7: miccode x1 |
| `rust/src/fix/msg.rs` | P7: BLOOMBERGCODE x1, P7: FIGICODE x1, P7: FOREXCODE x1, P7: ISINCODE x1, P7: cficode x7, P7: countrycode x1, P7: eusipacode x1, P7: miccode x3, P9: IsinRegistry x7 |
| `rust/src/fix/schema.rs` | P7: cficode x1 |
| `rust/src/graph/message.rs` | P7: cficode x2, P7: countrycode x9, P7: eusipacode x9, P7: miccode x2, P9: IsinRegistry x2 |
| `rust/src/graph/mod.rs` | P7: cficode x2, P7: miccode x2 |
| `rust/src/isin_registry.rs` | P7: countrycode x2, P7: eusipacode x2 |
| `rust/src/lib.rs` | P7: ISINCODE x1, P7: MICCODE x1 |
| `rust/tests/fix/batch.rs` | P7: isincode x1, P7: miccode x3 |
| `rust/tests/fix/cfi.rs` | P7: cficode x4 |
| `rust/tests/fix/codec.rs` | P7: isincode x2, debug: "SCRATCH wire x1 |
| `rust/tests/fix/enrich.rs` | P7: cficode x1, P7: isincode x5, P7: miccode x1 |
| `rust/tests/fix/forex.rs` | P7: cficode x1, P7: miccode x1 |
| `rust/tests/fix/identity.rs` | P7: cficode x3, P7: miccode x1 |
| `rust/tests/fix/msg.rs` | P7: miccode x5 |
| `rust/tests/fix/schema.rs` | P7: miccode x2 |
| `rust/tests/fix/securityids.rs` | P7: isincode x7 |
| `rust/tests/fix/ulbridge.rs` | P7: cficode x1, P7: miccode x2 |
| `rust/tests/graph/message.rs` | P7: countrycode x2, P7: eusipacode x3 |
| `skills/yggdryl-fix/references/rust.md` | P7: cficode x2, P7: isincode x1 |
