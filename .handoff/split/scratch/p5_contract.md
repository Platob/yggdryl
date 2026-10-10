# P5 contract - D37: `MarketMessage`, the concrete generic message, and the FIX doors onto it

Design: `scratchpad/d37_design.md` (normative). Inputs: `scratchpad/message_map/design_inputs.md`
(Part A, anchors) and the four maps beside it. Vocabulary: P4 (D38) landed first, so every
instant is `transunix`/`sendunix` and the element's own names are `uuid`/`hashcode`; never write
`currunix`, `recdunix`, `curruuid` or `currhashcode`. Program rules (AGENTS.md
is in your context): no test code under any `src/`; no back-compat (S3's trait, `as_message::<T>()`,
`into_any`, `clone_box`, `dyn_eq`, `FixAnomaly`, `FixLifted`, `MarketData::Fix`, `MarketKind::Fix`
gone in the same commit); one commit; the exact trailer; no model identifier in the repo;
never push to main, never publish, never touch live AWS.

## Invariants the slice proves
- The dictionary hash, the census, the crate dump, `fixmsg` (65_053), the fixed row's columns
  and order, and `rust/tests/fix/equivalence.snapshot` are unmoved: no `config/fix` document,
  crate tag, fixed-row column or `FIX:` key changes. `size_of::<MarketData>() == 912` holds.
- Cost pins hold: `allocations.rs` @922 (`into_market_leaf` 0 allocs), @937, @1695, @8595 (rekey
  2 / 18 / 0), @8715 (bytes per parsed message within 2%); `iobase_calls` @127. New rows:
  `into_message` 0 allocs, `from_message` on the registry's own root allocates its indexes alone,
  a `W` fixture's children cost stated. A count that rises is a defect to design away.
- `FixCodec::parse(FixCodec::render(m))` restates `m` (facts, `StatedFacts`, entries,
  metadata, children) for a parsed message and for a native one.

## Stage 1 - define workers, disjoint files, no cargo
Worktree: `/home/user/yggdryl-p5` (branch `wip/p5`) ALREADY EXISTS at commit `53e7d3c21` - the
settled S3 tree (`wip/s3` b8ddef717) with the P4 sweep applied and committed as a wip commit, so it
speaks D38's names (`transunix`, `sendunix`, `uuid`, `hashcode`); do not re-cut it and do not run
the sweep again. The lane manager re-bases onto the program branch once P4 lands. Each worker edits only its set; a request to
another worker is written in the report. `rustfmt --edition 2024 --check` is the one tool run.

- **W1 the message (graph/)**: `rust/src/graph/message.rs` (new: `MarketMessage` as designed,
  `StatedFacts` - FixMsg's bit names moved, `pub` consts, `InstrumentStatement`; constructors
  `new(kind: MarketDataKind)`, `with_entries(root: Arc<Field>, row: Scalar) -> Result<Self>`,
  readers `record()`, `entry(name)`, `root()`, `row()`, `children()`, `metadata()`,
  `anomalies()`, `instrument()`, `stated()`; mutators `entries_mut() -> Result<..>` canonicalizing,
  `push_child`, `note_anomaly`, `state_instrument`, `metadata_mut`; the four trait impls through
  the delegation macros; `with_previous`/`merge_with`/`restating` concrete; `note_conflict`
  pushing an `Anomaly`; `into_market_data`, `into_market_leaf`, `MessageIterator` and the
  helpers moved from `fix/market.rs` and made fact-only - the FIX readings they made named in
  the report with the fact that replaces each; `field(root)`, `into_scalar`, `from_scalar`;
  `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash` (over facts, root field, row, children, metadata,
  anomalies, instrument), `stable_hash`), `rust/src/graph/anomaly.rs` (`Anomaly` = `FixAnomaly`
  moved), `rust/src/graph/market_data.rs` (`Message(MarketMessage)` inline; `From`/`TryFrom`;
  `as_message`/`as_message_mut`; the (Message, Message) arms; `delegate_by_variant!` arm;
  `marketdatakind()`), `rust/src/graph/kind.rs` (`Message`, `message`, last), `graph/mod.rs`
  exports, `graph/arrow.rs` and `graph/book.rs` intake arms, `graph/iterator.rs` restating arm,
  `graph/facts.rs` only if a delegation macro needs a form it lacks. Report the struct's
  `size_of` estimate.
- **W2 the FIX handle (fix/)**: `rust/src/fix/msg.rs` (`FixMsg { registry, message, header,
  indexes, capture }`; `into_message`, `from_message`, `message()`, `message_mut()`; the four
  hand-written trait impls deleted - sites inside `fix/` re-spelled onto `message()`; `FixLifted`
  deleted and `LIFTED_TAGS`/`typed_fact` reading the record; `stated`/`stale`/`row_stated` moved
  to `StatedFacts`; `into_row`/`from_row` interleaving by a per-registry column index, the
  entries' run shared; the children's `NoMDEntries(268)` occurrences rendered into `fixentries`
  byte for byte), `rust/src/fix/schema.rs` (the entry root beside `fix_schema`: `fix_entry_root
  (&FixRegistry) -> Arc<Field>` = the fixed row less the fact columns, less `metadata`, less the
  elements group; the per-registry column index `FixedRowLayout { fact_at: Vec<(usize, Fact)>,
  entry_at: Vec<(usize, usize)> }` or equivalent, built once and memoized on the registry),
  `rust/src/fix/build.rs` (the parse filling facts, entries, children for 268, `instrument`),
  `rust/src/fix/identity.rs` (`Typed::fact` unchanged; the idmap-mapped tags never entries),
  `rust/src/fix/market.rs` (reduced: `From<FixMsg> for MarketData`, the parse-time FIX
  derivations that stay FIX's, listed), `rust/src/fix/anomaly.rs` deleted, `rust/src/fix/enrich.rs`
  (`LifecycleMessage` over the message; `learn_stating` through `instrument()`),
  `rust/src/fix/codec.rs` (the render of a native message: the inverse idmap table - every fact
  to its one standard tag, listed in the report from `Typed`/`identity::record` - else the crate
  tag; `MsgType(35)` from kind and state, the table beside `fix::state::from_msgtype`; the
  round-trip door), `rust/src/fix/batch.rs`, `rust/src/fix/ulbridge.rs`, `rust/src/fix/mod.rs`.
- **W3 the rest of the core and the CLI**: `rust/src/isin_registry.rs` (`learn_stating` over a
  `&MarketMessage`'s `instrument()` - or over `Market + Event` plus an `InstrumentStatement`
  argument; choose the form that keeps the FIX enrichment call site and report it),
  `cli/src/market.rs`, `rust/benchmarks/fix/*.rs`, `rust/benchmarks/graph/*.rs` if any,
  `rust/tests/scale_ulbridge.rs`, `rust/src/implementer.rs` (the market-owned items FIX reached
  that the public message now covers - delete their forwarders; list the ones that stay).
- **W4 Python**: `python/src/graph/message.rs` (new class `MarketMessage`, frozen, hashable,
  `__reduce__` through `into_scalar`/`from_scalar`; properties for the facts the leaf classes
  expose; `record()`, `entry()`, `children`, `metadata`, `anomalies`, `instrument`),
  `python/src/graph/market_data.rs` (intake of a `MarketMessage`, `as_message()`, kinds tuple
  `message`), `python/src/graph/mod.rs`, `python/src/fix.rs` (`FixMsg.into_message()`,
  `FixMsg.from_message(registry, message)`, `Anomaly` class replacing `FixAnomaly`),
  `python/yggdryl/graph/*.py`, `python/yggdryl/fix*.py`, `python/yggdryl/_native.pyi`,
  `python/tests/graph/*.py`, `python/tests/test_fix.py` where the API moved,
  `.api-bindings.txt` (Python section).
- **W5 Node**: `node/src/graph/market_data.rs` (`asMessage()` replacing `asFix()`, intake of a
  `FixMsg` converting inside, kinds `message`), `node/src/graph/message.rs` (new `JsMarketMessage`,
  the re-spelled door's class: getters only, no new verbs), `node/src/fix.rs` (`FixAnomaly` ->
  `Anomaly`), `node/tests/**` re-spelled, `.api-bindings.txt` (JS section: the re-spelling, no
  addition beyond the class the door needs - say so in a line).
- **W6 tests, docs, AGENTS.md, skills**: `rust/tests/graph/message.rs` (new: construction, the
  record, children expansion, `into_market_leaf` 0 allocs row in `allocations.rs`, the row form
  round trip, equality/hash), `rust/tests/graph/anomaly.rs`, `rust/tests/graph/market_data.rs`
  (the 912 pin unchanged, the `Message` arms), `rust/tests/graph/kind.rs` (the order pin
  re-spelled with its sentence), `rust/tests/fix/msg.rs`, `market.rs`, `codec.rs`, `schema.rs`,
  `enrich.rs` (the API moves; the round-trip pin; `from_message` re-rooting; the idmap rule: a
  stated `Side(54)` reads into the fact and renders from it), `rust/tests/allocations.rs` (the
  new rows), `docs/graph/market-data.md`, the FIX pages under `docs/fix/`, `skills/yggdryl-market-data/`,
  `skills/yggdryl-fix/`, AGENTS.md rows (`graph/`: `message.rs`, `anomaly.rs`, `market_data.rs`;
  `fix/`: `FixMsg` the handle, the idmap rule, `FixLifted` gone; the `Ownership` bullet on parse
  enrichment unchanged), `.api-inventory.txt`.

## Stage 2 - the lane manager (after P4 is landed and chain_p4 done)
1. Under the git lock: merge the program branch into `wip/p5` (P4's sweep already applied in
   the worktree; S3's settle lines may conflict - resolve, note), merge into the program branch.
2. `cargo check -p yggdryl --all-targets --keep-going --message-format=short` until clean;
   `cargo fmt --all`. 3. Phase suites: `--test graph`, `--test fix`, `--test root
   isin_registry`, `--test allocations` filtered to the fix/graph rows, then the chain
   `$S/logs/chain_p5.sh` as the others. The snapshot and the dictionary hash must be green
   without edit; a moved value is a defect. 4. DESIGN.md "### P5 results"; the handoff's D37
   row built; one commit with `$S/p5_commit_message.txt`; push; read CI to `CI result`. Report.
