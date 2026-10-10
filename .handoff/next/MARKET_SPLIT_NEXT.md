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
  serde documents, the value-stream bytes and the Arrow extension names. The
  FIX dictionary hash (`12_613_356_107_921_639_431` since P4, unmoved by P6
  and S4) moves only in a slice that says why, in the sentence beside the
  pin - P7 moves it, with the snapshot keys and the book identities, on
  purpose. Any other moved pin is a defect, never a re-pin; a cost pin
  (`allocations.rs`, `iobase_calls.rs`, a bench) is never re-pinned from a
  sweep.
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
| HEAD | the handoff commits over `f860e6b67` (the handoff correction), `8d1c6b733` (the CLI's `parquet` dev-dependency), `f6d6a6970` (the P4, P6 and S4 results), S4 `9f69d7141` "Move the market vocabulary into yggdryl-market and FIX into yggdryl-fix", P6 `1ad29bfa6` "Delete the market book service", the handoff `7b566b3b2` and P4 `4e46b5ab7` |
| Draft PR | #209, draft |
| Base | `origin/main` at `2ae975674`; nothing landed on `main` since, no merge needed |
| Slices done | P0 `3d8bf84d9`, P1 `08ae4c6b7`, S0 `6d71a36ee`, S1 `eeb4ec14d` (+ `719299cf6`), S2 `f9f665198` (+ `2d800d51b`), S2b `c015226b6` (+ `2048b4681`), the CI structure `1f909739b` (+ `b656e765b`), the S3 design `d100439d3`, P2 `f7c3c1b58`, the D36-D38 record `49bcab3e3`, S3 `a46a2177b`, P3 `ef4e241a4`, P4 `4e46b5ab7` (proven green by run `37963283466` on the handoff commit `7b566b3b2` over it, 32 jobs), P6 `1ad29bfa6` and S4 `9f69d7141` with the CLI's `parquet` dev-dependency `8d1c6b733` (proven green by run `38007212559` on the handoff correction `f860e6b67` over them, 39 jobs) |
| Crates | `yggdryl` (`rust/`), `yggdryl-market` (`rust/market/`), `yggdryl-fix` (`rust/fix/`); the CLI and both bindings link and install both; `cargo package --list -p yggdryl` lists nothing under `market/` or `fix/` |
| Next | P7 (D40, the user's 2026-10-09 instruction: the FIX row named by the registry, the lifted band last, `securityids` the one hold map with `instuuid`, the code columns renamed, `crossuuid` the XXH3-128 of the cross code, a book's sources and its code over them), then P8 (D41, the lifecycle matching on one common identifier with propagation - evidence first), then P5R, M6, B5, S7, S8 |

## Checks (S4, on the tree committed as `9f69d7141`)
See DESIGN.md "### S4 results" - the whole table - and "### P6 results". The user's named validation: `pytest python/tests/test_fix.py -k medallion` 1 passed (65.6 s, and 68.7 s on the settled tree), the pipeline over two local Iceberg warehouse folders; `pytest python/tests` 2784 passed, 4 skipped, as at HEAD.

## Blockers
None.

## Next

The slices, in the order they land, each one commit on the program branch:

1. **P7** (D40) - the FIX row named by the registry, the lifted band last, `securityids` the one hold map with `instuuid`, the code columns renamed, `crossuuid` the XXH3-128 of the cross code, a book's sources and its code over them; inside the crates; the dump, the hash, the snapshot keys and the book identities move once with their sentences. Design: DESIGN.md "## P7: design"; the instruction `scratch/p7/user_instruction.md`.
2. **P8** (D41) - the lifecycle matching on one common identifier, indexed, with propagation: an evidence workflow over the capture's lifecycle first, then the design in the foreground, then the implementation. `scratch/p8/user_instruction.md`.
3. **P5R** (D37) - `MarketMessage` finished inside the crates, the parked patch `scratch/p5_on_p4.patch` re-targeted onto S4's paths and P7's names (`scratch/p5r_manager_prompt.md`, `scratch/p5_state.md`).
4. **M6** - excel and xmla each a commit under one push, then the avro+parquet+s3+iceberg batch (`scratch/m6_manager_prompt.md`; every script re-derived on the tree it runs on).
5. **B5** (S5), **S7**, **S8** (designed in the foreground first), **S9** on the user's go.

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
| D36 | `yggdryl-s3` through a storage-backend extension point: `StorageBackend` claimed per scheme on the register (`claim_backend`, `backend_for`, `backends`), asked by `Holder::from_url` after lowering, its answer described; `Holder::Registered(Box<dyn RegisteredHandle>)`; the verbs the wildcards specialized on S3 (`upload_from`, `discard`, `as_leaf`, `as_container`, `set_known_size`) as `IOBase` defaults and `into_byte_stream` over `owned_stream_bytes`; `Site::Opened` with an opener; `aws/` and `auth/` stay core under `aws`; `yggdryl-iceberg[s3tables]` depends on `yggdryl-s3`; CI leaf `s3` with the three exchanges | the backend map, `s3_backend_map/design_inputs.md` | built in place P3; S6d moves |
| D33 | `yggdryl-xmla`, an eighth crate through the media point, `soap/` with it; registered in place in S2b - the core's own media are three, `RecordOptions::Xmla`, `Media::Xmla`, `Media::xmla`, `Serie::Xmla` and `XmlaSerie` deleted, `Xmla<Holder>` a `MediaWrapper` - and moved in S6c; the Python `Xmla` class stays in the one native module, the CLI's `xmla serve` depends on the crate | the user's instruction; DESIGN.md D33; crates.io 404 | S2b, built S6c |
| D39 | a leaving medium keeps its rank: `media::codec::RESERVED_RANKS` (`parquet` 1, `avro` 2, `xmla` 4, `excel` 6) admitted by `claim` under the codec's own name, every other medium at or above `EXTERNAL_RANK`, so the `s2_pins` hashes and the order pins are byte-identical through S6; `implementer` grows once per move by S3's routes, Avro and Parquet carry their own hidden `implementer` for what Iceberg reaches; the Iceberg field view built by `protocol_field_types!` in `yggdryl-iceberg` (`IcebergField::new`), `as_iceberg` gone; core tests building a leaving crate's objects move to that crate's tests; `install()` at every init; order avro, parquet, excel, xmla, then iceberg after `yggdryl-s3` | the five media maps | S6 |
| D38 | `currunix` -> `transunix` (the transaction instant, required, the identity and order axis), `recdunix` -> `sendunix` (the technical wire clock, optional, the merge reference), and the element's own `curruuid` -> `uuid`, `currhashcode` -> `hashcode` (`prevuuid`, `crossuuid`, `crosshashcode`, `srcuuids` keep their prefix); precedences, values, derivations, positions and tags unchanged - the carrier's clock first, else `SendingTime(52)`; crate fields 65_001, 65_004, 65_007 and 65_009 re-spelled, so the dump, the dictionary hash (once, with its sentence), the snapshot's keys and `fix.json` move and the census does not | the instants map | P4 |
| D37 | `MarketMessage` a concrete public struct in `graph/message.rs` - boxed facts, `StatedFacts`, the entries as an `Arc<Field>` root and a `Scalar` row, `children`, `Metadata`, `Vec<Anomaly>`, `InstrumentStatement` - the four traits implemented once on it; `MarketData::Message`, `MarketKind::Message` (`message`); `FixMsg` the codec's handle over a message (`into_message`, `from_message`), an idmap-mapped tag never an entry, a native message rendered by the inverse idmap else its crate tags; S3's trait, `as_message::<T>()` and `Box<dyn MarketMessage>` deleted | the message map, `message_map/design_inputs.md` | P5 |
| D40 | the FIX row named by the registry where one field states the fact whole, the lifted band last, `securityids` the one hold map with `instuuid`, the code columns renamed, `crossuuid` the XXH3-128 of the cross code, a book's sources (its delta, its events, the previous book's uuid) and its code over them | the user's instruction, DESIGN.md "## P7: design" | P7 |
| D41 | the lifecycle matches the previous alive and the current element on one common identifier through an index and propagates values; the identifier types and the two-chains case decided on the capture's evidence | the user's instruction, DESIGN.md "## P8" | P8 |

## Questions for the user - decided 2026-10-09 ("Do what's recommended", `.handoff/split/scratch/user_decisions.md`)
1. D12 npm name: `yggdryl-market` on all three registries (`yggdryl.market` not used).
2. D11 artifact sizes: option (d), one native module per runtime linking every crate, the sizes unchanged.
3. S6 landing shape: excel and xmla each a commit under one push, then avro, parquet, s3 and iceberg (+s3tables) one dependency-closed commit (the reversed split not taken).
8. D38: `sendunix` keeps the carrier-first precedence.
The CLI links `yggdryl-s3` always (S6d; its `s3` feature goes). Still open, on the user's explicit go: the manual `release.yml` rehearsal and the registry configuration a real publish needs (a PyPI pending trusted publisher for `yggdryl-market`, the npm bootstrap publish, `CARGO_REGISTRY_TOKEN` with `publish-new` over `yggdryl-*`).

## New since the handoff (the user's instructions of 2026-10-09, recorded verbatim under `.handoff/split/scratch/p7/` and `p8/`)
- P6: "Delete also the market served service its useless" - landed (`1ad29bfa6`).
- "Focus on landing market and fix split validated by medaillon python test" - landed (`9f69d7141`), the medallion test green.
- P7 (D40): the six items on the FIX row, the market row, the cross identity and the book's sources, plus the refinement on `with_previous` (the previous book's uuid among the sources, never its sources). Designed; next.
- P8 (D41): the lifecycle matches the previous alive and the current element on one common identifier, through an index, and propagates values. Evidence first, then the design.

## State at the handoff (2026-10-09, after S4)
P6 (`1ad29bfa6`) and S4 (`9f69d7141`) are pushed, with the results commit `f6d6a6970`, the CLI's `parquet` dev-dependency `8d1c6b733` (the Rust docs examples' one S4 defect, red in run `38006511421`) and the handoff correction `f860e6b67`; run `38007212559` on `f860e6b67` is read to `CI result`: green, 39 of 39 jobs. P7 (D40) is designed in DESIGN.md "## P7: design" and is next; P8 (D41) is recorded under "## P8" with the evidence to gather first; P5R follows them, then M6, B5, S7, S8 and S9 on the user's go. The former scratchpad is `.handoff/split/scratch/` (the S4 lane's decisions in `s4_decisions.md`, the user's P7 and P8 instructions under `p7/` and `p8/`; the results tables are DESIGN.md's).
