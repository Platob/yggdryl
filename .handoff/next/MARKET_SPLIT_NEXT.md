# Handoff: split yggdryl into eight crates

Read `.handoff/next/MARKET_SPLIT_PROMPT.md` (the program) and
`.handoff/split/DESIGN.md` (the decisions, the ledger, the pins, the bench
baselines, what S1 and S2 built) before any edit. Where this file and those differ,
the committed files hold; where the prompt text a session was given differs
from `MARKET_SPLIT_PROMPT.md`, the committed file holds.

## Goal

Split the core into eight crates - `yggdryl`, `yggdryl-market`,
`yggdryl-fix`, `yggdryl-avro`, `yggdryl-parquet`, `yggdryl-iceberg` (with
`s3tables`), `yggdryl-excel` (the user's third instruction, D24) and
`yggdryl-xmla` with `soap/` (the user's fifth instruction, D33) - the
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
  leaving crate's `install()` claims them (S4, S6, S6b, S6c).
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
  design and the edits no script makes stay in the foreground - and, from
  S2b on (the user: "reduce usages of fable's model and use it only when
  need deep thinking or design, until the end"), the foreground model is
  reached for a design, a synthesis or a decision alone: implementation,
  the smoke and chain runs, the fixes a check names, the commit, the push
  and the CI read are an `opus` or `sonnet` agent's, briefed exactly.

## State

| Fact | Value |
| --- | --- |
| Program branch | `ccr-0fe6f9d0-ruymat` |
| HEAD | the commit holding this file, S3: "Settle the remaining seams before the crates leave" (D5, D6, D9, D10), on `49bcab3e3` ("Record the D36, D37 and D38 designs") |
| Draft PR | #209, draft |
| Base | `origin/main` at `2ae975674`; no merge of `origin/main` was needed this session (nothing landed on `main` since) |
| Slices done | P0 `3d8bf84d9` (D22), P1 `08ae4c6b7` (D23), S0 `6d71a36ee` (the pins), S1 `eeb4ec14d` (the market extension point), S1's handoff `719299cf6`, S2 `f9f665198` (the media extension point), S2's handoff `2d800d51b`, S2b `c015226b6` (the XMLA medium registered in place, D33) and `2048b4681` (its CI's red census race fixed in the test), the CI structure `1f909739b` and its handoff `b656e765b`, the S3 design `d100439d3`, P2 `f7c3c1b58` (the medium holds its origin and its cache, the serie composes the pushdown, D34 and D35), the D36-D38 record `49bcab3e3`, S3 (this commit: the remaining seams in place, D5, D6, D9, D10) - each pushed alone, its CI read green before the next but P2's, whose run 37923405641 the record push cancelled: the S3 run proves P2 and S3 together |
| CI structure | `1f909739b`, "Build the core first and run only the CI jobs a change reaches": run 37904365402 success, 33 jobs green and the empty `Leaf` matrix skipped as planned; 10m41s wall against 18m09s before (the old workflow's last run, 37902279246, 18m57s); the critical path `Changes` 12s, `Python binding wheel` 5m12s, `Documentation examples (Python)` 4m57s, `CI result` 9s; the core path `Core build (all features)` 1m40s then its slowest shard, `rest`, 5m37s; the exchanges 30s to 1m18s, compiling nothing; the gate proved 20 rows into the ledger; docs run 37904365351 success |
| Next | P4 (D38), then P5 (D37), P3 (D36) and S4, as "## Next" orders them |

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
What S2b built is DESIGN.md's D33 and "S2b results": XMLA a registered
medium in place - `RecordOptions::Xmla`, `Media::Xmla`, `Media::xmla`,
`Serie::Xmla` and `XmlaSerie` deleted, `Xmla<Holder>` a `MediaWrapper`,
the core's own media three (`Ipc`, `Text`, `Csv`).
What P2 built is DESIGN.md's D34, D35 and "P2 results":
`IOMedia::read_origin_field` (the whole root the origin holds) and
`read_arrow_field` one rule in the trait default; one projection rule (the
declared children, else the origin's, that the `select` and the early
`where` read, handed whole to a headerless medium); `compose` and
`Residual` in `media_serie.rs`, the one composition every record read
takes, the five residual copies and `RecordOptions::apply_stream` gone;
`media/cache.rs` `MediaCache` on the seven wrappers under the options'
`cache_ttl` (milliseconds, `0` realtime, outside the options' identity),
every write door updating or dropping it; Python's `cache_ttl` property;
Node gains no door.
What S3 built is DESIGN.md's "S3: what was built": each market kind's own
`dtype()`/`field(name)` (the four `DataType::<kind>()` deleted),
`fix/state.rs` with State's FIX doors, `fix::LOGICAL_NAMES`, the logging
facade's per-crate table and the `warned!` key by logger name,
`MarketMessage` behind `MarketData::Fix` with `as_message::<T>()`,
`protocol_field_types!` exported and `FixField::new(&field)` the one
spelling, and `implementer.rs` - 114 names by route, `#[doc(hidden)]` -
every market and FIX reach of a crate-private core item spelled through it.

## Checks

Every command below ran on the tree committed as S3, from
`/home/user/yggdryl`, with `CARGO_INCREMENTAL=0` and the debug info off: the phase suites, then
`logs/chain_s3.sh` under the scratchpad, then `logs/chain_s3b.sh` over the
review's last edits (two doc links, the unused `pub(crate) use warned`, two
test assertions).

| check | result |
| --- | --- |
| `cargo check --workspace --all-targets --all-features --keep-going --message-format=short` (the worktree before the merge, then the merged tree) | 2 errors and 3 warnings in the worktree (two `FixField::new(&document)` borrows, three unused imports), then clean |
| phase suites, all features: `--test root` filtered to the kinds, `state`, `vocabulary`, `datatype`, `implementer`, `protocol`, `market`; `--test graph`; `--test fix`; `--test market_register`; `--test logging`; `--test allocations` and `--test iobase_calls` filtered | 424 of 426 (two of the test worker's assumptions, corrected: 34 of 34 `implementer`), 439, 1026 (the dictionary hash and the crate dump among them, unedited), 7, 88 then 89, 16, 5 |
| `refs.py` over the merged tree | 0 market or FIX references to a crate-private core path; 26 FIX references over 15 crate-private market names |
| `cargo test -p yggdryl --all-targets --all-features --no-fail-fast` | 63 targets, 9357 passed, 0 failed, 3 ignored |
| `cargo test -p yggdryl --all-targets --no-fail-fast` (default features) | 63 targets, 6637 passed, 1 failed - the `implementer` test asserting Iceberg's refusal, which no claimed format makes without the feature; the assertion dropped and the target re-run in `chain_s3b` |
| `cargo test -p yggdryl-cli --all-targets --no-fail-fast` | 6 targets, 35 passed, 0 failed, 6 ignored |
| `cargo test -p yggdryl --doc` | 628 passed |
| `cargo clippy --workspace --all-targets --all-features --no-deps -- -D warnings`; `cargo clippy -p yggdryl --all-targets --no-deps -- -D warnings` | the first clean; the second refused the then-unused `pub(crate) use warned` (deleted) |
| `RUSTDOCFLAGS="-D warnings" cargo doc -p yggdryl --no-deps --all-features` | four redundant explicit link targets the sweep's imports made (`fix/mod.rs`, `graph/market_column.rs`), fixed |
| `cargo fmt --all -- --check` | clean |
| `maturin develop`, `pytest python/tests --deselect python/tests/test_spark_interop.py`, `mypy --strict` | installed; 2784 passed, 4 skipped; no issues in 70 files |
| `npm run --prefix node test:package:debug`, `cargo build -p yggdryl-cli`, `npm test --prefix node`, `tsc --noEmit`, `git diff -- node/index.js node/index.d.ts` | the package audit passed; 1122 tests, 1120 passed, the two sandbox `TextDecoder` tests failing as at every slice; tsc clean; the generated files unchanged |
| `node scripts/build_docs_fix.js --check`, `build_docs_playground.js --check` | current |
| `mkdocs build --strict` | clean |
| `check_api_inventory.py`; `generate_internals.py --check` | current (180 source files and 570 `pub` names not described yet); `internals` current |
| `check_docs_examples.py --lang python` / `javascript` / `rust` | 837 run, 3 skipped, 0 failed; 789 run, 2 skipped, 0 failed; 940 passed |
| `grep -rn '#\[cfg(test)\]\|#\[test\]\|mod tests' rust/src python/src node/src cli/src` | empty |
| `chain_s3b`: fmt, clippy in both lanes, `cargo doc` at `-D warnings`, `--test root --test logging` in both lanes and `--test s3` | fmt clean; clippy clean in both lanes; `cargo doc` clean; default features `--test root` 1727 and `--test logging` 82 passed; all features `--test root` 1911, `--test logging` 89, `--test s3` 264 passed; 0 failed |
| the review: the `code-review` skill at high effort over the whole diff | five findings: three fixed, two kept with their reason ("S3 results" above) |
| local-only checks (charset tables and interop, ISIN seed, country and MIC tables), benches | not run: nothing the slice touches makes them stale |

Not run, as the slice made nothing of theirs stale: every `cargo bench` and
`npm run bench:*`, the Python boundary benchmarks, the scale run and the
free-threaded lane. The two Node charset tests that compare the package to
the runtime's `TextDecoder` (`node/tests/charset.test.js`: `decoding agrees
with TextDecoder over the same names`, `iso-8859-1 is not a spelling of
windows-1252 here`) fail in this sandbox alone, whose Node 22.22 decodes the
C1 range of `windows-1252` as ISO 8859-1 does; they failed at every slice
the same way and CI's Node proves them. The CI run of this push is read to
its `CI result` and recorded by the next handoff.

## Blockers

None.

## Next

The slices, in the order they land, each one commit on the program branch:

1. **S3** - done (this commit): the remaining seams in place (D5, D6, D9,
   D10); DESIGN.md "S3: what was built".
2. **P4** (D38) - `transunix` and `sendunix` at every door; the dump, the
   dictionary hash, the snapshot's keys and `fix.json` regenerated once each.
   Design: "## P4: design" in DESIGN.md; contract `scratchpad/p4_contract.md`;
   the sweep `scratchpad/p4_sweep.py` applied to a worktree cut after S3 lands.
3. **P5** (D37) - `MarketMessage` the concrete generic message, `FixMsg` the
   codec's handle over it, both doors (`into_message`, `from_message`, the
   render of a native message), `MarketData::Message`; after P4 so it speaks
   the new names. Design: "## P5: design"; contract `scratchpad/p5_contract.md`.
4. **P3** (D36) - the storage-backend extension point in place: the register,
   `Holder::Registered`, the `IOBase` capabilities, `Site::Opened`, the S3 trio
   claimed by the core itself; disjoint files from P4 and P5, so it runs
   beside them and lands when its chain is clean. Design: "## P3: design";
   contract `scratchpad/p3_contract.md`.
5. **S4** - `yggdryl-market` (the `graph/` vocabulary, the four enum kinds,
   the identifiers, `isin_registry.rs`) and `yggdryl-fix` move out, as the
   prompt's S4 section says, over the `implementer` door S3 opened.
6. S5, S6 (a-d: Avro, Parquet, Iceberg with `s3tables`, XMLA, the object
   stores), S7-S9 as the prompt states them.

The first command of every slice, from a fresh checkout of the program branch:

```bash
git fetch origin && git switch ccr-0fe6f9d0-ruymat && git merge origin/main
```

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
| D5, D6, D9, D10 (refined for S3) | the inherent `dtype()`/`field(name)` in `enum_leaf!`'s market arm and the four `DataType::<kind>()` blocks deleted; `implementer.rs` with 115 items by route R/F/A/M/X and the eleven `pub(super)` raises, proven by the re-point script and `refs.py`; `MarketMessage` behind `MarketData::Fix`, `protocol_field_types!` exported as the builder, `FixField::new(&field)` the one spelling; `fix/state.rs` free functions, `Scheme::FIX` and `STATE_CODES` core, a logger named by its module path whatever crate holds it | DESIGN.md "## S3: design"; `s3_map/design_inputs.md` | S3, built S3 |
| D34 | the medium holds its origin (`IOMedia::read_origin_field`), `read_arrow_field` one rule (declared else origin, narrowed by the select), one projection rule (declared ∩ the columns the select and the early filter read), one composer in `media_serie.rs` that `read_record_serie` also calls - the serie the one wrapper composing pushdown - `source_field` and the five residual copies gone, S8 amended | the user's sixth instruction; DESIGN.md D34, "P2 results" | S2b, built P2 |
| D35 | `MediaCache` on every medium wrapper under `cache_ttl` - milliseconds, 0 realtime, a shared options section outside the hash feed as `file_threads` is - served while the handle is open or the entry younger than the TTL, every write door updating it with what it knows or invalidating it | the user's seventh instruction; DESIGN.md D35, "P2 results" | S2b, built P2 |
| D36 | `yggdryl-s3` through a storage-backend extension point: `StorageBackend` claimed per scheme on the register (`claim_backend`, `backend_for`, `backends`), asked by `Holder::from_url` after lowering, its answer described; `Holder::Registered(Box<dyn RegisteredHandle>)`; the verbs the wildcards specialized on S3 (`upload_from`, `discard`, `as_leaf`, `as_container`, `set_known_size`) as `IOBase` defaults and `into_byte_stream` over `owned_stream_bytes`; `Site::Opened` with an opener; `aws/` and `auth/` stay core under `aws`; `yggdryl-iceberg[s3tables]` depends on `yggdryl-s3`; CI leaf `s3` with the three exchanges | the backend map, `s3_backend_map/design_inputs.md` | P3 in place, S6d the move |
| D33 | `yggdryl-xmla`, an eighth crate through the media point, `soap/` with it; registered in place in S2b - the core's own media are three, `RecordOptions::Xmla`, `Media::Xmla`, `Media::xmla`, `Serie::Xmla` and `XmlaSerie` deleted, `Xmla<Holder>` a `MediaWrapper` - and moved in S6c; the Python `Xmla` class stays in the one native module, the CLI's `xmla serve` depends on the crate | the user's instruction; DESIGN.md D33; crates.io 404 | S2b, built S6c |
| D39 | a leaving medium keeps its rank: `media::codec::RESERVED_RANKS` (`parquet` 1, `avro` 2, `xmla` 4, `excel` 6) admitted by `claim` under the codec's own name, every other medium at or above `EXTERNAL_RANK`, so the `s2_pins` hashes and the order pins are byte-identical through S6; `implementer` grows once per move by S3's routes, Avro and Parquet carry their own hidden `implementer` for what Iceberg reaches; the Iceberg field view built by `protocol_field_types!` in `yggdryl-iceberg` (`IcebergField::new`), `as_iceberg` gone; core tests building a leaving crate's objects move to that crate's tests; `install()` at every init; order avro, parquet, excel, xmla, then iceberg after `yggdryl-s3` | the five media maps | S6 |
| D38 | `currunix` -> `transunix` (the transaction instant, required, the identity and order axis), `recdunix` -> `sendunix` (the technical wire clock, optional, the merge reference), and the element's own `curruuid` -> `uuid`, `currhashcode` -> `hashcode` (`prevuuid`, `crossuuid`, `crosshashcode`, `srcuuids` keep their prefix); precedences, values, derivations, positions and tags unchanged - the carrier's clock first, else `SendingTime(52)`; crate fields 65_001, 65_004, 65_007 and 65_009 re-spelled, so the dump, the dictionary hash (once, with its sentence), the snapshot's keys and `fix.json` move and the census does not | the instants map | P4 |
| D37 | `MarketMessage` a concrete public struct in `graph/message.rs` - boxed facts, `StatedFacts`, the entries as an `Arc<Field>` root and a `Scalar` row, `children`, `Metadata`, `Vec<Anomaly>`, `InstrumentStatement` - the four traits implemented once on it; `MarketData::Message`, `MarketKind::Message` (`message`); `FixMsg` the codec's handle over a message (`into_message`, `from_message`), an idmap-mapped tag never an entry, a native message rendered by the inverse idmap else its crate tags; S3's trait, `as_message::<T>()` and `Box<dyn MarketMessage>` deleted | the message map, `message_map/design_inputs.md` | P5 |

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
7. Branch protection: `main` has none today. When it gets some, require
   `CI result` alone - the old job names ("Rust quality (...)",
   "Documentation examples") are gone, and the gate fails unless every
   planned job passed and every other was skipped (DESIGN.md "The CI
   structure").
8. D38: `sendunix` keeps `recdunix`'s precedence - the carrier's clock (the
   capture's write time) first, else the stated `SendingTime(52)` - because
   `SendingTime(52)` already has its own fixed-row column and the capture's
   clock has no other. If "the technical sending time" means the sender's
   clock first, say so: it is one precedence line (`fix/build.rs`'s carrier
   fill and `fix/msg.rs`'s `record_at_sending`) and its tests, and the
   snapshot's `sendunix` values would move with it.
