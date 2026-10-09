# S4 residue

Tree: `/home/user/yggdryl-s4`

## Done

- tables: 34 market root names, 121 FIX names, 14 graph modules leave
- rust/src/lib.rs: 296 items read, the moved modules' declarations deleted
- graph/mod.rs split: 6 items stay, 29 go to the market crate
- rust/tests/allocations.rs: 79 items moved up to yggdryl-market, yggdryl-fix
- rust/tests/arrow/extension.rs: 2 items moved up to yggdryl-market
- rust/tests/expression/filter.rs: 1 items moved up to yggdryl-market
- rust/tests/graph/iterator.rs: 2 items moved up to yggdryl-fix
- rust/tests/graph/market_data.rs: 13 items moved up to yggdryl-fix
- rust/tests/iceberg/mod_.rs: 1 items moved up to yggdryl-market
- rust/tests/iceberg/partition.rs: 1 items moved up to yggdryl-market
- rust/tests/iceberg/value.rs: 1 items moved up to yggdryl-market
- rust/tests/iobase_calls.rs: 3 items moved up to yggdryl-market, yggdryl-fix
- rust/tests/isin_registry/env.rs: 1 items moved up to yggdryl-fix
- rust/tests/json/field.rs: 3 items moved up to yggdryl-market
- rust/tests/root/ascii.rs: 1 items moved up to yggdryl-market
- rust/tests/root/cast.rs: 7 items moved up to yggdryl-market
- rust/tests/root/code.rs: 2 items moved up to yggdryl-market
- rust/tests/root/datatype.rs: 4 items moved up to yggdryl-market
- rust/tests/root/datatype_id.rs: 3 items moved up to yggdryl-market
- rust/tests/root/enums.rs: 6 items moved up to yggdryl-market
- rust/tests/root/fisn.rs: 1 items moved up to yggdryl-market
- rust/tests/root/implementer.rs: 5 items moved up to yggdryl-market, yggdryl-fix
- rust/tests/root/integer.rs: 1 items moved up to yggdryl-market
- rust/tests/root/scalar.rs: 2 items moved up to yggdryl-market
- rust/tests/root/string.rs: 2 items moved up to yggdryl-fix
- rust/tests/root/temporal.rs: 1 items moved up to yggdryl-fix
- rust/tests/root/unit.rs: 1 items moved up to yggdryl-market
- rust/tests/root/valuestream.rs: 2 items moved up to yggdryl-market
- rust/tests/root/version.rs: 1 items moved up to yggdryl-market
- rust/tests/root/vocabulary.rs: 8 items moved up to yggdryl-fix
- rust/tests/serie/order.rs: 1 items moved up to yggdryl-market
- rust/tests/value/canonical.rs: 2 items moved up to yggdryl-market
- rust/tests/xmla/dbtype.rs: 7 items moved up to yggdryl-market
- rust/tests/xxhash/arrow.rs: 12 items moved up to yggdryl-market
- rust/tests/xxhash/scalar.rs: 5 items moved up to yggdryl-market
- moves: 167 files moved by git, 2 examples deleted
- rust/market/tests/root.rs: 13 root test files declared
- rust/market/src/lib.rs and implementer.rs written
- rust/fix/src/lib.rs: crate attributes, `fix_category`, `install()`
- rust/src/market.rs: the seed of the four kinds replaced by `HELD`
- rust/src/vocabulary.rs: the FIX Latest names leave the seed
- market items FIX reaches raised and routed through its implementer: graph::facts::OperationEventFacts, idtype::names_another_instrument, isin_registry::EconomicMemo, isin_registry::IsinTable, isin_registry::warn_full
- arrow/extension.rs: the four kinds' extension types move to their files through `market_extension!`
- .api-inventory.txt: the moved sections re-homed, the graph section split
- paths: 1099 use statements and 1937 paths re-owned; file paths in 133 files
- install: 21 harnesses, 1838 tests, the benches' mains, every moved rustdoc example, 204 page blocks, the bindings and the CLI
- manifests: rust/market, rust/fix, the workspace, the bindings and the CLI
- tooling: generate_internals.py and check_api_inventory.py per crate, the docs runner in cli/
- generate_internals.py: rust/src/lib.rs: internals re-exports 116 module(s)
rust/fix/src/lib.rs: internals re-exports 16 module(s)
rust/market/src/lib.rs: internals re-exports 7 module(s)

## Residue

- `rust/fix/src/enrich.rs -> yggdryl_market::graph::iterator::order`: `pub(crate)` in a published module: needs a forwarder in `yggdryl_market::implementer` (S3's F or A route)
- `rust/fix/src/market.rs -> yggdryl_market::graph::market::base_crosscode`: `pub(crate)` in a published module: needs a forwarder in `yggdryl_market::implementer` (S3's F or A route)
- `rust/fix/src/msg.rs -> yggdryl_market::graph::market::merge_operation_event`: `pub(crate)` in a published module: needs a forwarder in `yggdryl_market::implementer` (S3's F or A route)
- `rust/fix/src/msg.rs -> yggdryl_market::graph::market::restating_operation`: `pub(crate)` in a published module: needs a forwarder in `yggdryl_market::implementer` (S3's F or A route)
- `rust/fix/src/msg.rs -> yggdryl_market::identifier::WORD_PAIR_WIDTH`: `pub(crate)` in a published module: needs a forwarder in `yggdryl_market::implementer` (S3's F or A route)
- `rust/fix/src/msg.rs -> yggdryl_market::identifier::fold_into`: `pub(crate)` in a published module: needs a forwarder in `yggdryl_market::implementer` (S3's F or A route)
- `rust/fix/src/msg.rs -> yggdryl_market::identifier::folded_len`: `pub(crate)` in a published module: needs a forwarder in `yggdryl_market::implementer` (S3's F or A route)
- `rust/fix/src/msg.rs -> yggdryl_market::identifier::is_word`: `pub(crate)` in a published module: needs a forwarder in `yggdryl_market::implementer` (S3's F or A route)
- `rust/market/src/graph/arrow.rs`: 1 doc link(s) to FIX unlinked to their paths (the market cannot name `yggdryl_fix`): reword
- `rust/market/src/idtype.rs`: 1 doc link(s) to FIX unlinked to their paths (the market cannot name `yggdryl_fix`): reword
- `rust/market/src/isin_registry/env.rs`: 1 doc link(s) to FIX unlinked to their paths (the market cannot name `yggdryl_fix`): reword
- `rust/market/src/isin_registry.rs`: 1 doc link(s) to FIX unlinked to their paths (the market cannot name `yggdryl_fix`): reword
- `rust/market/src/marketdatatype.rs`: 1 doc link(s) to FIX unlinked to their paths (the market cannot name `yggdryl_fix`): reword
- `rust/market/src/timeinforce.rs`: 1 doc link(s) to FIX unlinked to their paths (the market cannot name `yggdryl_fix`): reword
- `rust/src/fisn.rs`: 1 doc link(s) to moved items unlinked to their paths: reword (D14)
- `rust/src/graph/element.rs`: 1 doc link(s) to moved items unlinked to their paths: reword (D14)
- `rust/src/hashing/mod.rs`: 1 doc link(s) to moved items unlinked to their paths: reword (D14)
- `rust/src/isin.rs`: 2 doc link(s) to moved items unlinked to their paths: reword (D14)
- `rust/src/string.rs`: 4 doc link(s) to moved items unlinked to their paths: reword (D14)
- `rust/src/text/options.rs`: 1 doc link(s) to moved items unlinked to their paths: reword (D14)
- `.github/ci/rows.toml`: absent in this tree: uncomment `market` and `fix` under `[leaves]` and move the shards by hand
- `rust/src/enums.rs:637`: a core rustdoc example names `yggdryl::Side`, which `yggdryl-market` holds now: the core's doctests cannot name it - re-fixture the example on a core type or drop the line
- `rust/src/graph/column.rs:26`: a core rustdoc example names `yggdryl::graph::OrderEvent`, which `yggdryl-market` holds now: the core's doctests cannot name it - re-fixture the example on a core type or drop the line
- `rust/src/graph/element_column.rs:24`: a core rustdoc example names `yggdryl::graph::OrderEvent`, which `yggdryl-market` holds now: the core's doctests cannot name it - re-fixture the example on a core type or drop the line
- `rust/benchmarks/media/iceberg.rs`: a core test or bench names a moved item at 2 line(s) (1880, 1995): move the test to the crate's own `tests/` (D39) or re-fixture it on the test-only kind (D14)
- `rust/benchmarks/text/line.rs`: a core test or bench names a moved item at 1 line(s) (452): move the test to the crate's own `tests/` (D39) or re-fixture it on the test-only kind (D14)
- `rust/benchmarks/types/datatype/serie.rs`: a core test or bench names a moved item at 1 line(s) (18): move the test to the crate's own `tests/` (D39) or re-fixture it on the test-only kind (D14)
- `rust/benchmarks/types/datatype/value.rs`: a core test or bench names a moved item at 4 line(s) (4, 5, 12, 14): move the test to the crate's own `tests/` (D39) or re-fixture it on the test-only kind (D14)
