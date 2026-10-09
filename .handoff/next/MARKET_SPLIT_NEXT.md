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
| HEAD | the commit holding this file, P2: "Hold the origin field and its cache on the medium; compose the pushdown once" (D34, D35), on `d100439d3` ("Record the S3 design") |
| Draft PR | #209, draft |
| Base | `origin/main` at `2ae975674`; no merge of `origin/main` was needed this session (nothing landed on `main` since) |
| Slices done | P0 `3d8bf84d9` (D22), P1 `08ae4c6b7` (D23), S0 `6d71a36ee` (the pins), S1 `eeb4ec14d` (the market extension point), S1's handoff `719299cf6`, S2 `f9f665198` (the media extension point), S2's handoff `2d800d51b`, S2b `c015226b6` (the XMLA medium registered in place, D33) and `2048b4681` (its CI's red census race fixed in the test), the CI structure `1f909739b` and its handoff `b656e765b`, the S3 design `d100439d3`, P2 (this commit: the medium holds its origin and its cache, the serie composes the pushdown, D34 and D35) - each pushed alone, its CI read green before the next |
| CI structure | `1f909739b`, "Build the core first and run only the CI jobs a change reaches": run 37904365402 success, 33 jobs green and the empty `Leaf` matrix skipped as planned; 10m41s wall against 18m09s before (the old workflow's last run, 37902279246, 18m57s); the critical path `Changes` 12s, `Python binding wheel` 5m12s, `Documentation examples (Python)` 4m57s, `CI result` 9s; the core path `Core build (all features)` 1m40s then its slowest shard, `rest`, 5m37s; the exchanges 30s to 1m18s, compiling nothing; the gate proved 20 rows into the ledger; docs run 37904365351 success |
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

## Checks

Every command below ran on the tree committed as P2, from
`/home/user/yggdryl`, with `CARGO_INCREMENTAL=0` and the debug info off: the
smoke rows after the review's fixes, then one background chain
(`logs/chain_p2.sh` under the scratchpad) that cleaned the workspace's own
artifacts between lanes, then a confirmation chain (`logs/chain_p2b.sh`)
over the unchanged tree for the format, the build checks and the Node
package audit.

| Check | Command | Result |
| --- | --- | --- |
| it builds | `cargo check -p yggdryl --all-targets --keep-going --message-format=short`; `cargo check --workspace --all-targets --all-features --keep-going --message-format=short` | clean, 0 diagnostics |
| the smoke, all-features lane | `cargo test -p yggdryl --all-features --test <t>` for media, root, ipc, parquet, avro, csv, xmla, excel, text, iobase_calls, iceberg, expression, http, warehouse, allocations, media_register | 205, 1871, 52, 88, 142, 102, 761, 332, 236, 67, 528, 244, 503, 139, 184, 14 passed; 0 failed |
| the S2 pins | `cargo test -p yggdryl --test media s2_pins` | green in both lanes, byte-identical (`CacheTtl` hashes nothing) |
| the cost rows | `--test iobase_calls`; `--test allocations` | 67 and 184 passed; no cost pin moved or re-pinned; the media-serie construction row unmoved (`pstream_bytes=1 media_type=2 is_container=2`) |
| the whole run | `cargo test -p yggdryl --all-targets --all-features --no-fail-fast` | 63 targets, 9298 passed, 0 failed, 3 ignored |
| the whole run, default features | `cargo test -p yggdryl --all-targets --no-fail-fast` | 63 targets, 6580 passed, 0 failed |
| the CLI | `cargo test -p yggdryl-cli --all-targets --no-fail-fast` | 6 targets, 35 passed, 0 failed, 6 ignored |
| rustdoc examples | `cargo test -p yggdryl --doc` | 628 passed |
| clippy | `cargo clippy --workspace --all-targets --all-features --no-deps -- -D warnings`; `cargo clippy -p yggdryl --all-targets --no-deps -- -D warnings` | exit 0 both |
| the API pages | `RUSTDOCFLAGS="-D warnings" cargo doc -p yggdryl --no-deps --all-features` | exit 0 |
| formatting | `cargo fmt --all -- --check` | clean |
| Python | `VIRTUAL_ENV=python/.venv python/.venv/bin/python -m maturin develop -m python/Cargo.toml`, `-m pytest python/tests --deselect python/tests/test_spark_interop.py`, `-m mypy --strict ...` | the extension installed; 2784 passed, 4 skipped; mypy "no issues found in 70 source files" |
| Node | `npm run --prefix node build:debug`; `cargo build --locked -p yggdryl-cli`; `npm test --prefix node`; `npx tsc --noEmit`; `npm run --prefix node test:package:debug`; `git diff --exit-code -- node/index.js node/index.d.ts` | 1122 tests, 1120 passed, the 2 failing ones the sandbox `TextDecoder` pair below; tsc exit 0; the package audit passed; the generated loader and declarations unchanged |
| the docs manifests | `node scripts/build_docs_fix.js --check`; `node scripts/build_docs_playground.js --check` | both current |
| the page examples | `python scripts/check_docs_examples.py --lang rust`, `--lang python`, `--lang javascript` | Rust 940 passed; Python 837 run, 3 skipped, 0 failed; JavaScript 789 run, 2 skipped, 0 failed |
| the site | `python -m mkdocs build --strict --config-file mkdocs.yml` | clean |
| the inventories | `python scripts/check_api_inventory.py`; `python scripts/generate_internals.py --check` | current (180 source files and 587 `pub` names not described yet); `yggdryl::internals` current |
| no test code under `src/` | `grep -rn '#\[cfg(test)\]\|#\[test\]\|mod tests' rust/src python/src node/src cli/src` | empty |
| the review | the `code-review` skill at high effort over the whole diff, and the lane manager's pass over the brief's checklist | fourteen points: eleven fixed before the commit (the headerless projection, `clear` over a container, the encoding check, `size()` under a TTL, the counts kept under a TTL, `add` beside `update`, one `keeps`, the Python docstrings, one `container_origin`, `cache_ttl` taking `__index__` and `None`), three kept with their reason; DESIGN.md "P2 results" lists them |
| CI | the run on `ccr-0fe6f9d0-ruymat` for PR #209 | read after the push; the lane's report states it, and the next handoff records it |

Not run, as the slice made nothing of theirs stale: the charset table and
interop checks, the ISIN seed check, the country and MIC table checks, every
`cargo bench` and `npm run bench:*`, the Python boundary benchmarks, the
scale run and the free-threaded lane. The two Node charset tests that compare
the package to the runtime's `TextDecoder` (`node/tests/charset.test.js`:
`decoding agrees with TextDecoder over the same names`, `iso-8859-1 is not a
spelling of windows-1252 here`) fail in this sandbox alone, whose Node 22.22
decodes the C1 range of `windows-1252` as ISO 8859-1 does; they failed at P0,
P1, S0, S1, S2, S2b and P2 the same way and CI's Node proves them.

## Blockers

None.

## Next

The slices, in the order they land, each one commit on the program branch:

1. **S3** (in flight: its worktree `wip/s3` merging and settling) - the
   remaining seams in place (D5, D6, D9, D10).
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
| D5, D6, D9, D10 (refined for S3) | the inherent `dtype()`/`field(name)` in `enum_leaf!`'s market arm and the four `DataType::<kind>()` blocks deleted; `implementer.rs` with 115 items by route R/F/A/M/X and the eleven `pub(super)` raises, proven by the re-point script and `refs.py`; `MarketMessage` behind `MarketData::Fix`, `protocol_field_types!` exported as the builder, `FixField::new(&field)` the one spelling; `fix/state.rs` free functions, `Scheme::FIX` and `STATE_CODES` core, a logger named by its module path whatever crate holds it | DESIGN.md "## S3: design"; `s3_map/design_inputs.md` | S3 |
| D34 | the medium holds its origin (`IOMedia::read_origin_field`), `read_arrow_field` one rule (declared else origin, narrowed by the select), one projection rule (declared ∩ the columns the select and the early filter read), one composer in `media_serie.rs` that `read_record_serie` also calls - the serie the one wrapper composing pushdown - `source_field` and the five residual copies gone, S8 amended | the user's sixth instruction; DESIGN.md D34, "P2 results" | S2b, built P2 |
| D35 | `MediaCache` on every medium wrapper under `cache_ttl` - milliseconds, 0 realtime, a shared options section outside the hash feed as `file_threads` is - served while the handle is open or the entry younger than the TTL, every write door updating it with what it knows or invalidating it | the user's seventh instruction; DESIGN.md D35, "P2 results" | S2b, built P2 |
| D36 | `yggdryl-s3` through a storage-backend extension point: `StorageBackend` claimed per scheme on the register (`claim_backend`, `backend_for`, `backends`), asked by `Holder::from_url` after lowering, its answer described; `Holder::Registered(Box<dyn RegisteredHandle>)`; the verbs the wildcards specialized on S3 (`upload_from`, `discard`, `as_leaf`, `as_container`, `set_known_size`) as `IOBase` defaults and `into_byte_stream` over `owned_stream_bytes`; `Site::Opened` with an opener; `aws/` and `auth/` stay core under `aws`; `yggdryl-iceberg[s3tables]` depends on `yggdryl-s3`; CI leaf `s3` with the three exchanges | the backend map, `s3_backend_map/design_inputs.md` | P3 in place, S6d the move |
| D33 | `yggdryl-xmla`, an eighth crate through the media point, `soap/` with it; registered in place in S2b - the core's own media are three, `RecordOptions::Xmla`, `Media::Xmla`, `Media::xmla`, `Serie::Xmla` and `XmlaSerie` deleted, `Xmla<Holder>` a `MediaWrapper` - and moved in S6c; the Python `Xmla` class stays in the one native module, the CLI's `xmla serve` depends on the crate | the user's instruction; DESIGN.md D33; crates.io 404 | S2b, built S6c |
| D38 | `currunix` -> `transunix` (the transaction instant, required, the identity and order axis), `recdunix` -> `sendunix` (the technical wire clock, optional, the merge reference), and the element's own `curruuid` -> `uuid`, `currhashcode` -> `hashcode` (`prevuuid`, `crossuuid`, `crosshashcode`, `srcuuids` keep their prefix); precedences, values, derivations, positions and tags unchanged - the carrier's clock first, else `SendingTime(52)`; crate fields 65_001, 65_002, 65_007 and 65_009 re-spelled, so the dump, the dictionary hash (once, with its sentence), the snapshot's keys and `fix.json` move and the census does not | the instants map | P4 |
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
