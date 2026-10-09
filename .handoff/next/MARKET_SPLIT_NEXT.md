# Handoff: split yggdryl into seven crates

Read `.handoff/next/MARKET_SPLIT_PROMPT.md` (the program) and
`.handoff/split/DESIGN.md` (the decisions, the ledger, the pins, the bench
baselines, what S1 and S2 built) before any edit. Where this file and those differ,
the committed files hold; where the prompt text a session was given differs
from `MARKET_SPLIT_PROMPT.md`, the committed file holds.

## Goal

Split the core into seven crates - `yggdryl`, `yggdryl-market`,
`yggdryl-fix`, `yggdryl-avro`, `yggdryl-parquet`, `yggdryl-iceberg` (with
`s3tables`) and `yggdryl-excel` (the user's third instruction, D24) - the
split first and the adaptations after, one slice per session, one commit per
slice, on one program branch under one draft PR, nothing published and no
live AWS resource touched.

## Invariants

- The program branch is `ccr-0fe6f9d0-ruymat` (harness-assigned, D1) and
  the draft PR is #209. Never push to `main`, never tag, never run a
  release, never mark the PR ready or merge it, never publish (no
  `cargo publish` without `--dry-run`, no `npm`/`maturin`/`twine` publish).
- Each session starts with `git fetch origin && git switch
  ccr-0fe6f9d0-ruymat && git merge origin/main`, never rebasing a pushed
  slice; a conflict is resolved by re-running the slice's sweep over the
  merged tree and the merge is recorded here.
- No back-compat: a replaced name, spelling, test or doc is deleted in the
  same commit; no alias, shim or dual reader. One owner per fact; the core's
  registers are the one place a kind, a medium, a table format, a catalog
  factory or a locator is claimed (`plugin::Register`; `market.rs`,
  `media/codec.rs`, `media/format.rs`, `warehouse/catalog.rs`,
  `holder/locator.rs`), each seeded by the core with its own until the
  leaving crate's `install()` claims them (S4, S6, S6b).
- The seventeen codes are the core's own flat variants (D25); the market
  register holds enum kinds alone. S4 moves the four enums, `graph/` and the
  ISIN registry into `yggdryl-market`, never a code.
- The wire contracts S0 pinned are byte-identical through every slice: the
  `DataTypeId` bytes, the ranks, the `Shape` positions, the hash feeds, the
  serde documents, the value-stream bytes, the Arrow extension names and the
  FIX dictionary hash `14_542_711_836_201_211_247`. A moved pin is a
  defect, never a re-pin; a cost pin (`allocations.rs`, `iobase_calls.rs`,
  a bench) is never re-pinned from a sweep.
- No test code under any `src/` (`grep -rn '#\[cfg(test)\]\|#\[test\]\|mod tests' rust/src rust/*/src python/src node/src cli/src` is empty before every commit).
- Node gains no door; the slices re-spell and re-pin the doors that exist.
- Layer order inside a slice: Rust core -> Python -> Node -> docs, settled by
  the smoke loop, then one whole run in the `--all-features` lane leading
  the background chain, then push, then the CI run read to the end.
- The model fits the step (the user, mid-S1: "use sonnet when you scan so
  much"): every scan, classification, check-and-report and mechanical sweep
  runs on `sonnet`; a merge of a core file and a review on `opus`; the
  design and the edits no script makes stay in the foreground.

## State

| Fact | Value |
| --- | --- |
| Program branch | `ccr-0fe6f9d0-ruymat` |
| HEAD | `f9f665198` ("Open the media extension point for record media, table formats and catalogs", S2) |
| Draft PR | #209, draft |
| Base | `origin/main` at `2ae975674`; no merge of `origin/main` was needed this session (nothing landed on `main` since) |
| Slices done | P0 `3d8bf84d9` (D22), P1 `08ae4c6b7` (D23), S0 `6d71a36ee` (the pins), S1 `eeb4ec14d` (the market extension point), S1's handoff `719299cf6`, S2 `f9f665198` (the media extension point) - each pushed alone, its CI read green before the next |
| Next | S3, the remaining seams in place (D5, D6, D9, D10) |

What S1 built is the "S1: what was built" section of DESIGN.md. What S2
built is its "S2: design" (D26-D32), "The review's amendments (S2)" and "S2:
what was built" sections: `MediaCodec` statics claimed under their MIME types
(`media/codec.rs`; `codec_of`/`codec_for` the intake lookups,
`RecordOptions::codec()` the one dispatcher past them), `RecordOptions::
Registered(RegisteredOptions)` over `MediumOptions`/`MediumSettings` with the
typed `settings`/`require_settings(_mut)` doors replacing the Parquet, Avro
and Excel variants and accessors, `Media::Registered(Box<dyn MediaWrapper>)`
and `Media::medium()`, the four media series deleted for `GenericMediaSerie`,
`TableFormat`/`LocatedTable` (`media/format.rs`, `ICEBERG_FORMAT`,
`TableFormat::table` for the folder catalog), `Catalog`/`Namespace`/`Table::
Registered` with `downcast_ref`, `CatalogFactory` (`HADOOP_FACTORY`,
`S3TABLES_FACTORY`), `Locator` (`holder/locator.rs`, `S3TABLES_LOCATOR`),
`Site::Store` under `s3`, `Error::External`, `IOMedia::as_any` with
`ParquetFooter`, `parquet::read_media_statistics`/`read_media_geospatial_statistics`,
`filter_phases` public; the bindings keep every name; the S2 pins and every
cost pin unmoved; `rust/tests/media_register.rs` pins the four registers.

## Checks

Every command below ran on the tree committed as S2 (`f9f665198`), from
`/home/user/yggdryl`, with `CARGO_INCREMENTAL=0` and the debug info off; one
background chain held the cargo lock for the long steps and cleaned the
workspace's own artifacts between lanes, because one lane's test binaries
are about 11 GB of the session's disk allowance and a first chain died on a
full disk at its first step (nothing of that chain counts).

| Check | Command | Result |
| --- | --- | --- |
| it builds | `cargo check -p yggdryl --all-targets --keep-going --message-format=short`, and with `--all-features` | clean in both lanes, 0 warnings |
| the registers | `cargo test -p yggdryl --test media_register`, and with `--all-features` | 13 and 14 passed (the Parquet row under its feature) |
| the S2 pins | `cargo test -p yggdryl --test media s2_pins`, both lanes | every default hash, the shared-section and own-setting feeds and the variant order unmoved |
| the whole run | `cargo test -p yggdryl --all-targets --all-features --no-fail-fast` | 63 targets, 9170 passed, 0 failed; `iobase_calls`, `allocations`, the Iceberg `call_counts` and the S3 Tables request counts unmoved, no cost pin re-pinned |
| the whole run, default features | `cargo test -p yggdryl --all-targets --no-fail-fast` | 63 targets, 6522 passed, 0 failed |
| the CLI | `cargo test -p yggdryl-cli --all-targets --no-fail-fast` | 35 passed |
| rustdoc examples | `cargo test -p yggdryl --doc` | 628 passed |
| clippy | `cargo clippy --workspace --all-targets --all-features --no-deps -- -D warnings`; `cargo clippy -p yggdryl --all-targets --no-deps -- -D warnings` | exit 0 both, after one `matches!` rewrite |
| the API pages | `RUSTDOCFLAGS="-D warnings" cargo doc -p yggdryl --no-deps --all-features` | exit 0, after six intra-doc link lints |
| formatting | `cargo fmt --all -- --check` | clean |
| Python | `VIRTUAL_ENV=python/.venv python/.venv/bin/python -m maturin develop -m python/Cargo.toml`, `-m pytest python/tests --deselect python/tests/test_spark_interop.py`, `-m mypy --strict ...` | the extension installed; 2780 passed, 4 skipped (pyspark, two PEP 649 tests, one free-threaded test); mypy no issues in 70 files |
| Node | `npm run --prefix node build:debug`; `cargo build --locked -p yggdryl-cli`; `npm test --prefix node`; `npx tsc --noEmit`; `git diff --stat -- node/index.js node/index.d.ts` | 1122 tests, 1120 passed, the 2 failing ones the sandbox `TextDecoder` pair below; tsc exit 0; the generated loader and declarations unchanged |
| the docs manifests | `node scripts/build_docs_fix.js --check`; `node scripts/build_docs_playground.js --check` | both current |
| the page examples | `python scripts/check_docs_examples.py --lang rust`, `--lang python`, `--lang javascript` | Rust 940 passed; Python 837 run, 3 skipped, 0 failed; JavaScript 789 run, 2 skipped, 0 failed |
| the site | `python -m mkdocs build --strict --config-file mkdocs.yml` | clean |
| the inventories | `python scripts/check_api_inventory.py`; `python scripts/generate_internals.py --check` | current (180 source files and 586 `pub` names not described yet, as before); `yggdryl::internals` current |
| no test code under `src/` | `grep -rn '#\[cfg(test)\]\|#\[test\]\|mod tests' rust/src rust/*/src python/src node/src cli/src` | empty |
| the review | an independent read of `git diff -- rust/src` (opus) after the suites passed | seven findings, six fixed before the commit (DESIGN.md "The review's amendments (S2)"), the seventh - duplicate-name refusal order - fixed with its test |
| CI | the run on `ccr-0fe6f9d0-ruymat` for PR #209 | run 37888869824 on `f9f665198` success: all 17 jobs green - Rust quality (default features, all features), Iceberg Rust 1.94, the S3, Azure, Google, ZIP, Avro, Excel and PyIceberg exchanges, Spark interop, Python binding wheel, Python binding (`pyarrow==18.*`, `pyarrow>=18`), Python free-threaded, Node.js binding, Documentation examples; docs run 37888869830 success |

Not run, as the slice made nothing of theirs stale: the charset table and
interop checks, the ISIN seed check, the country and MIC table checks, every
`cargo bench` and `npm run bench:*`, the Python boundary benchmarks, the
scale run and the free-threaded lane. The two Node charset tests that compare
the package to the runtime's `TextDecoder` (`node/tests/charset.test.js`:
`decoding agrees with TextDecoder over the same names`, `iso-8859-1 is not a
spelling of windows-1252 here`) fail in this sandbox alone, whose Node 22.22
decodes the C1 range of `windows-1252` as ISO 8859-1 does; they failed at P0,
P1, S0 and S1 the same way and CI's Node proves them.

## Blockers

None.

## Next

S3, the remaining seams in place (the prompt's "S3" items; D5 the market
type's own `dtype()`/`field(name)` items, D6 the one public `implementer`
module of forwarders for what the leaving crates reach, D9 the market trait
for a message that splits and the protocol-view builder, D10 the vocabulary
that stays in the core and the `log` targets' rule), each settled by its
smoke row and the whole run, one commit. The first command, from a fresh
checkout of the program branch:

```bash
git fetch origin && git switch ccr-0fe6f9d0-ruymat && git merge origin/main
git grep -nE 'pub\(crate\)' -- rust/src/market.rs rust/src/graph rust/src/fix rust/src/iceberg rust/src/parquet rust/src/avro rust/src/excel | wc -l
```

Then list, per leaving crate, every `pub(crate)` item it reaches (a scratch
`git mv` of its folder and the compiler's list) before deciding D6's module;
S2's `Handle::bound`, `Site::Store`, `MediaTable::listed`, `FolderLayout`
and the `plugin::CORE` claimant are already on that list (`iceberg/mod.rs`
and `s3tables/` read them).

## The decision ledger

The whole ledger is `.handoff/split/DESIGN.md`, section "The ledger". The
rows decided or changed this session:

| D# | decision | evidence | slice |
| --- | --- | --- | --- |
| D2 | closed shape over `&'static MarketDescriptor`; `Scalar` 48 and `Serie` 40 bytes | the spike; `allocations.rs` | S0, built S1 |
| D3 | `DataTypeId(u8)` newtype with CamelCase consts; the four enum kinds' ids leave it and the seventeen codes' stay (D25); `all()` answers the claimed order | `datatype_id.rs` | S0, built S1 |
| D7 | explicit idempotent `install()`; intake alone reads the register; the refusal list; the core seeds itself until S4 | `market.rs` | S0, built S1 |
| D8 | `plugin.rs`: one claim-once `Register<K, V>`; market and `LOGICAL_NAMES` keys on it, the media in S2 | `plugin.rs`, `vocabulary.rs` | S0, built S1/S2 |
| D19 | nothing moves: ranks, `Shape` positions and feeds as listed; a kind's marker hashes nothing and `DataType` writes its `shape` | the pins; the dictionary hash | S0, held S1 |
| D20 | `MarketValue` with an owned `from_scalar`; `Value` unchanged; the kind's ZST marker over `MarketType`; `SideField = FieldOf<SideType>` | `market.rs`, `typed.rs` | S0, built S1 |
| D25 | the seventeen codes stay core and flat; the register holds enum kinds alone (`Code8`/`Code16`); `yggdryl-market` carries the enums, `graph/` and the ISIN registry | the user's instruction; the S0 pins; one free Code byte | S1 |
| D22 | `pluginside` deleted; the plugin's role is a `Side` | the user's instruction | P0 |
| D23 | the medium holds its `RecordOptions`; the media serie keeps no copy | the user's instruction | P1 |
| D24 | `yggdryl-excel`, a seventh crate through the media point, claimed in S2, its own commit after S6 | the user's instruction | S0, built S6b |
| D21 | one trait per role on the register, refined: the whole options struct behind the box, `open(handle) -> Media`, `as_any` answering a state object | `media/codec.rs`, `media/format.rs` | S0, built S2 |
| D26 | `RecordOptions::Registered(RegisteredOptions)`; the Parquet/Avro/Excel variants and accessors deleted; `Ord`/`Hash` by rank; the typed `settings` door; `registered` answers a core struct as its variant | the S2 pins | S2 |
| D27 | `MediaCodec` statics claimed under their MIME types; `codec_of`/`codec_for`; `RecordOptions::codec()` the one dispatcher; `Media::Registered`, `Media::medium()`; one name per medium | `transfer.rs`, `media/mod.rs`, `holder/mod.rs` | S2 |
| D28 | `Serie::{Parquet, Avro, Excel, IcebergTable}` deleted; `require_kind` by MIME set | `media_serie.rs` | S2 |
| D29 | `TableFormat`/`LocatedTable`; `TableFormat::table` for the folder catalog; the layout detection stays core | `iceberg/mod.rs`, `warehouse/folder.rs` | S2 |
| D30 | `Catalog`/`Namespace`/`Table::Registered`; `CatalogFactory`; `Locator`; `Site::Store` under `s3`; the `From` impls stay, answering `Registered` | `warehouse/`, `holder/locator.rs` | S2 |
| D31 | `Error::External`; `From<ParquetError>` deleted; `IOMedia::as_any` + `ParquetFooter`; the statistics as free functions | `error.rs`, `parquet/mod.rs` | S2 |
| D32 | `filter_phases` published; logging and D17 unchanged | `expression/mod.rs` | S2 |

## Questions for the user

1. The npm name (D12): `yggdryl-market` is recommended (one string
   preflight checks on all three registries); `yggdryl.market`, the user's
   spelling, is valid on npm and the alternative. Decide before S5 publishes.
2. The artifact sizes (D11): option (d) - one native module per runtime
   linking every crate - leaves the `yggdryl` wheels at 41-50 MB each
   (PyPI 0.1.21: 32 files, 1.40 GB in all) and the npm package at 337 MB
   unpacked. Is the unchanged size acceptable?
3. S6 as one commit (Avro, Parquet, Iceberg with `s3tables`, in that order),
   or the reversed order S6a-c if its diff is too large to review.
4. A persisted std hash: not needed - D19's plan held through S1 with no
   pin moved.
5. The go for S5's rehearsal and the repository configuration it needs
   (crates.io, PyPI trusted publishing and npm tokens for the new names).
6. Registry and network checks skipped: none - D11's and D12's reads were
   made on 2026-10-08 (crates.io, PyPI and npm all answered).
