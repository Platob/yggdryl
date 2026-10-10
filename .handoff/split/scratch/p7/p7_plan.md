# P7 (D40) - the plan, startable the moment P9 is pushed

Design: DESIGN.md "## P7: design" = `$S/p7/d40_design.md` (amended for P9, 2026-10-10). Order
(`$S/user_decisions.md` 7): P9 -> **P7** -> P8 -> P5R. One commit, one push, CI read to `CI
result`. `$S` = `.handoff/split/scratch`. Phases in AGENTS order, each settled by its build check
and the one suite it touched; the whole run leads the chain once the last cargo phase settles.

Starting state (read 2026-10-10 against the mid-P9 tree, to be re-read on P9's commit):
`fix_schema_tags` 153, the schema 154, `held.len()` 53 (`rust/fix/tests/root/{schema.rs:96,232,
crated.rs:723,1022}`), `MarketColumn::ALL` 37, the `marketdata` row 66 (`[60..]` nested,
`rust/market/src/graph/arrow.rs:142`), `MarketData` 928 / `BookEvent` 912
(`rust/market/tests/graph/market_data.rs:154-155`), the dictionary hash 857_326_152_662_128_339
(`rust/fix/tests/root/store.rs:3557`), `Element::cross_uuid` = `Uuid::from_v8(crosshashcode)`
(`rust/src/graph/element.rs:297-302`), `instrument.rs` `cross_uuid_of` (`:1603`) and `mint_digest`
(`:931`).

## Phase 0 - the sweep (D40.4), mechanical

```bash
python3 .handoff/split/scratch/p7/p7_sweep.py . --check .handoff/split/scratch/p7/p7_sweep.manifest.json --manifest
# read REVIEW / RESIDUE / SHADOW; settle each by hand (below), re-run --check until RESIDUE 0
python3 .handoff/split/scratch/p7/p7_sweep.py . --apply .handoff/split/scratch/p7/p7_sweep.manifest.json
cargo check --workspace --all-targets --all-features --keep-going --message-format=short
python3 scripts/check_instruments_seed.py
cargo test -p yggdryl-market --test graph market_column
cargo test -p yggdryl-market --features internals --test root -- instrument listing
cargo test -p yggdryl-market --features "internals iceberg parquet" --test instrument
```

Mid-P9 dry run: 1,735 lines in 114 files, REVIEW 78, RESIDUE 0, SHADOW 2. By hand, from REVIEW:
- `idtype.rs` doc lines 32-33 (a view's column `isincode`/`cficode` -> `isin`/`cfi`); the word
  tables and `Identifier::from_key` spellings stay (intake);
- the crate column stated by name on the wire (`|isincode=..|miccode=..|bloombergcode=..|` in
  `rust/fix/tests/root/batch.rs:1394,1514,1999-2118`, `python/tests/test_fix.py:4554-4709`,
  `node/tests/fix.test.js:2621-2622`): re-spell to `isin=`/`mic=`/`bbg=` where the test means the
  crate's column - phase 3's risk item decides;
- quoted `cficode` under the FIX scope: D40 interpretation 6 (`cficode` on the FIX row's 461
  column, `cfi` on `marketdata`); `docs/graph/schemas.md:323` is the `marketdata` table -> `cfi`;
- `eusipacode` beside `sspa*` in prose that names our column (AGENTS.md:485, `docs/graph/
  instrument.md:1138`, the skills) -> `eusipa`; the intake lists (`rust/fix/src/msg.rs:453,4264`,
  `SKILL.md:438`) keep it;
- `crated.rs:716` "displayed `ForexCode`" -> `Forex`;
- SHADOW `python/tests/test_instrument.py:682,684` (kwarg `cficode=` of a helper) - read, rename.
- `rust/market/src/graph/market.rs:1161,1164` `feed("cficode")`/`feed("miccode")`: **kept** (the
  name is hashed; D40 moves no market `hashcode`).

Pins moving in phase 0: none by value - names only; the instrument row's digest feeds read
`NAMES[..]`, so every `Instrument::hashcode` moves (phase 1's sentence covers it: the instruments
are rebuilt from bronze at P7).

## Phase 1 - D40.5, the cross identity (core, then market)

`rust/src/graph/element.rs:297-302`: `cross_uuid` = `Uuid::new(xxh128(crosscode))` (one core
function the market crate reaches through `implementer`, e.g. `implementer::cross_uuid_of`);
`rust/market/src/instrument.rs:1603` `cross_uuid_of` calls it; `mint`/`is_own_mint` read
`crossuuid.as_u128()`, `mint_digest` goes.

```bash
cargo check -p yggdryl --all-targets
cargo test -p yggdryl --test graph
cargo test -p yggdryl --test text -- line plan options
cargo test -p yggdryl --doc graph::element
cargo check -p yggdryl-market --all-targets
cargo test -p yggdryl-market --features internals --test root instrument
cargo test -p yggdryl-market --test graph
```

Moves, sentence "`crossuuid` is the raw XXH3-128 of the cross code, D40.5": the core's pinned
`crossuuid`s (`rust/tests/graph/{element,column,element_column}.rs`, `rust/tests/text/{line,plan,
options}.rs`, the `element.rs` rustdoc), panel D42.2's `crossuuid`/`uuid` columns in
`rust/market/tests/root/instrument.rs`, every market test naming a `crossuuid`. Must not move: any
event's `uuid` or `hashcode` (`time_uuid` reads `crosshashcode`), any `QY` number, any
`crosshashcode`.

## Phase 2 - D40.3 + D40.2 in the market crate

`MarketFacts` drops `cfi: Option<Cfi>` (`facts.rs`); `get_cfi`/`set_cfi` over `securityids`
(`IdType::Cfi`, `Cfi::refined`); `LiftedColumn` (`instcode`, `isin`, `cfi`, `mic`) - a root file
of its own, `rust/market/src/graph/lifted_column.rs`, mirrored by
`rust/market/tests/graph/lifted_column.rs`; `MarketColumn::ALL` 37 -> 33; `arrow.rs` writes the
band after the book controls; `book_crosscode` reads the lifted `isin`.

```bash
cargo check -p yggdryl-market --all-targets
cargo test -p yggdryl-market --test graph -- market_column lifted_column arrow facts market market_data book element
cargo test -p yggdryl-market --test allocations
```

Moves: `MarketColumn::ALL` 37 -> 33 ("`instcode`, `isin`, `cfi` and `mic` left the market band
for the lifted band, D40.2"); the band positions in `rust/market/tests/graph/arrow.rs` (339, 351,
449, 1918 by P9's count) - the row stays 66, `[60..]` stays; `MarketData`/`BookEvent` fall or
stay ("the CFI moved into `securityids`, D40.3"), a rise is a defect. **Open (O3)**: the
`securityids` cell of every row stating a CFI gains `cfi=..`; to keep D40's "hash feeds do not
move", `feed_market` skips `IdType::Cfi` in its `securityids` loop and feeds `"cficode"` where it
does today (`market.rs:1156-1161`); `--test graph element market` must stay green unedited.
Allocation rows: unchanged (a `get` on `securityids` is one binary search).

## Phase 3 - D40.1 + D40.2 in the FIX crate

`crated.rs`: the eleven derived definitions retired - `transunix` 65_007, `sendunix` 65_009,
`unit` 65_020, `spotrate` 65_027, `forwardpoints` 65_028, `bidqty` 65_029, `askpx` 65_031,
`askqty` 65_032, `ticker` 65_035, `strikepx` 65_036, `ordqty` 65_038 - `is_derived_tag` deleted,
"Fifty-three definitions" -> "Forty-two", "a retired definition leaves no gap" -> "a retired
number is never reused" (`crated.rs:102,116`, `docs/fix/capture.md:640`); `schema.rs`
`dictionary_event_tag` (60, 52) and the pairs (202, 133, 134, 135, 55, 996, 194, 195, 38) in the
one table, `cfi` -> 461 moved to the lifted band's table, `shared_prefix_len` (`schema.rs:369`)
reading the pairing table rather than the column names (the event band now opens `transacttime`,
which `EventColumn::of_name` does not answer); `fix_schema_tags` the lifted band
`instcode, isin, cfi(461), mic, bbg, figi, forex` after the groups and the frame, before
`fixentries`; `identity.rs`/`msg.rs`/`build.rs` read the settled facts off the registry tags.

```bash
cargo check -p yggdryl-fix --all-targets
cargo test -p yggdryl-fix --test root -- schema crated digest identity msg build
YGGDRYL_FIX_DUMP_WRITE=1 cargo test --locked -p yggdryl-fix --test root the_committed_store_carries_the_crate_dump
cargo test --locked -p yggdryl-fix --test root the_committed_dictionary_hashes_to_one_pinned_value
YGGDRYL_FIX_EQUIVALENCE_WRITE=1 cargo test --locked -p yggdryl-fix --test root equivalence
git diff -U0 rust/fix/tests/root/equivalence.snapshot | grep -E '^[-+].*"(hashcode|uuid)"'   # BOOK rows only (phase 4)
cargo test -p yggdryl-fix --test root -- batch codec direction securityids enrich ulbridge forex market
cargo test -p yggdryl-fix --test allocations && cargo test -p yggdryl-fix --test iobase_calls
```

Moves, each with its sentence: `fix_schema_tags` 153 -> 142, the schema 154 -> 143, `held.len()`
53 -> 42, `shared` 6+9+37+5 -> 6+9+33+5 (`schema.rs`); the dump (eleven definitions gone, five
renamed with their displays, the fixed row's order); the hash once - "It last moved when the
eleven derived definitions FIX states under a name of its own left the dictionary, the five crate
instrument codes dropped their `code` suffix (`isin`, `mic`, `forex`, `bbg`, `figi`) and the
lifted band closed the fixed row (D40)" - and the census (eleven definitions fewer) in the same
edit; the equivalence snapshot's keys (`transunix` -> `transacttime` and the ten others,
`isincode` -> `isin` and the four others) and its `crossuuid` cells (phase 1). Must not move: any
non-book `hashcode`/`uuid` cell, the FIX allocation and call-count rows.
**Risk (O2)**: the renamed crate names enter the registry's name index - a bridge key `ISIN=`,
`MIC=`, `FOREX=` would now resolve to 65_022/65_023/65_049, `isincode=` would stop; the suites
above and the snapshot's `metadata`/`fixentries` cells show it. If they move, the old spellings
become intake (`.also_called(&["isincode"])`, as `instrument[exchange]` is on 65_023), never a
second definition.

## Phase 4 - D40.6, the book's sources, and the `with_previous` refinement

`rust/market/src/graph/book.rs`: `get_srcuuids` (`:3505`) answers the unique sorted uuids of the
`delta` entries, the `events` items and - after `with_previous` (`:3538`) - the previous book's
own `uuid`, never its sources; `set_srcuuids` (`:3510`) keeps nothing; `finalize_book_event`
(`:3417`) becomes `XXH3-64(transunix BE ++ XXH3-64(sorted srcuuids bytes) BE)`;
`MarketData::from_arrow_reader` reads past a `BOOK` row's `srcuuids` cell. Red first: two cycles
over one book, the second's `srcuuids` = its delta + its events + the first's `uuid` and nothing
of the first's sources; two books of one instant over one source set after one predecessor are
one book whatever facts they settled on.

```bash
cargo test -p yggdryl-market --test graph -- book market_data arrow
cargo test -p yggdryl-fix --test root market
YGGDRYL_FIX_EQUIVALENCE_WRITE=1 cargo test --locked -p yggdryl-fix --test root equivalence
```

Moves, sentence "a book's code is its instant over its sources, D40.6": every book `uuid` and
`hashcode` (`rust/market/tests/graph/book.rs`, `rust/fix/tests/root/market.rs`, the snapshot's
BOOK rows, `docs/graph/book.md`); nothing else.

## Phase 5 - Python (§3)

```bash
VIRTUAL_ENV=python/.venv python/.venv/bin/python -m maturin develop -m python/Cargo.toml
python/.venv/bin/python -m pytest python/tests/test_fix.py python/tests/test_instrument.py python/tests/graph -x -q
python/.venv/bin/python -m pytest python/tests/test_fix.py -x -q -k medallion
```

`python/tests/medallion.py`: the FIX tables window and partition by `transacttime`, the line,
book and event tables by `transunix` (`PRIMARY_KEY`, `PARTUNIX` per table); a lake written before
P7 is replayed from bronze (instruments and silver, D40.5). Properties `msg.isin`, `msg.cfi`,
`msg.mic`, `msg.bbg`, `msg.figi`, `msg.forex` come from the sweep; the stubs and
`typing_bindings.py` likewise.

## Phase 6 - Node (§4, re-spelled only)

```bash
npm run --prefix node build:debug
node --test node/tests/fix.test.js node/tests/instrument.test.js
node --test node/tests/graph/*.test.js
```

`node/index.js`/`index.d.ts` regenerated; no door added.

## Phase 7 - manifests, docs, skills, inventories

```bash
node scripts/build_docs_fix.js && node scripts/build_docs_playground.js
python -m mkdocs build --strict --config-file mkdocs.yml
python scripts/check_api_inventory.py
python scripts/generate_internals.py --check
```

`docs/fix/capture.md` (the crate's columns table, the derived-column section retired),
`docs/graph/schemas.md` (both rows' tables: 142/143, the lifted band, 33 market columns),
`docs/graph/{event,market,book,instrument}.md`; `.api-inventory.txt` (`LiftedColumn`, the eleven
`*_TAG_NAME` retired, `is_derived_tag` gone), `.api-bindings.txt`; AGENTS.md's Layout rows.

## Phase 8 - the chain, then the commit

The whole runs `--all-features --no-fail-fast` of the three crates, clippy both lanes, `cargo doc
-D warnings`, the rustdoc examples, the CLI tests, `pytest python/tests`, Node, the manifests'
`--check`, mkdocs, the three docs runners - one background script, one log, read once; then
`cargo fmt --all`, the commit (the program's two attribution lines), push, CI to `CI result`.

## Open

- **O1** D40 interpretation 6: the FIX row's 461 column keeps the registry's name `cficode`
  (D40.1) and `cfi` is the `marketdata` name - or the FIX row says `cfi` too, which D40.1's rule
  forbids. Taken: `cficode` on the FIX row. Put to the user.
- **O2** the bridge-key spelling risk of phase 3 (renamed crate names in the name index).
- **O3** D40.3 vs "the hash feeds do not move": `feed_market` keeps feeding the CFI under
  `"cficode"` outside the `securityids` loop; every row's `securityids` cell gains `cfi=..` -
  a cell move D40 did not list. P9 chose the opposite (the fill skips `cfi` in `securityids`,
  `impl_log.md` 04:59Z); D40.3 reverses it.
- **O4** P9's own phase 4 (decisions 10 and 11: the instrument's `metadata`, the shared `instcode`
  allocation) is still open: run `--check` on P9's commit, not before.
- **O5** outside this lane's files: DESIGN.md's D40 decisions row (`instuuid`, `:2970`) and
  `$S/p5r_manager_prompt.md:9` are re-spelled by P9's results commit (D42 plan step 4).
