# P5R (D37) - the plan, startable the moment P8's results commit is pushed

Brief: `$S/p5r_manager_prompt.md` (`$S` = `.handoff/split/scratch`). Order (`$S/user_decisions.md`
7): P9 -> P7 -> P8 -> **P5R**. Inputs here: `path_map.py` (stdlib, paths only), its outputs
`p5_on_s4p9.patch` / `.routed.patch` / `.unrouted.patch` and `path_map_report.md`, all written
2026-10-10 against HEAD `bbbe10ae6` plus P9's working tree (mid-flight). Every cargo command one at
a time on the one target directory, `CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0` (regroup.md).

## What the map did (read `path_map_report.md` for every row)

115 sections in `p5_on_p4.patch` (bf5921bf4 over P4 4e46b5ab7, P4's blobs in the object store):

| Output | What | Apply with |
| --- | --- | --- |
| `p5_on_s4p9.patch` | 106 sections, one destination each, `index` kept (3-way from P4's blob) | `git apply -3` |
| `p5_on_s4p9.routed.patch` | 25 hunks of 5 files S4 split, each routed to the candidate whose text holds its old side, `index` dropped | `git apply --reject` |
| `p5_on_s4p9.unrouted.patch` | 1 hunk: `rust/src/lib.rs` `pub use fix::{..}` (S4 moved the re-exports to `rust/fix/src/lib.rs:179,207`) | by hand |
| dropped | `.handoff/split/DESIGN.md` (this lane writes its record), `rust/examples/fix_capture.rs` (S4 deleted `rust/examples/`) | - |

Path rules: `rust/src/graph/X` -> `rust/market/src/graph/X` (`element`, `column`, `element_column`
stay); `rust/src/graph/mod.rs` routed (all 5 hunks -> `rust/market/src/graph/mod.rs`, where the
`delegate_*` macros live); `rust/src/fix/mod.rs` -> `rust/fix/src/lib.rs`, `rust/src/fix/X` ->
`rust/fix/src/X`; `rust/tests/fix.rs` -> `rust/fix/tests/root.rs`, `rust/tests/fix/mod_.rs` ->
`root/lib.rs`, `rust/tests/fix/X` -> `rust/fix/tests/root/X`; `rust/tests/graph/X` ->
`rust/market/tests/graph/X`, routed where `rust/fix/tests/graph/X` also exists (`iterator.rs` 4
hunks -> fix; `market_data.rs` 2 -> fix, 1 -> market); the new `anomaly.rs`/`message.rs` tests and
bench name no FIX item -> market; `rust/tests/graph.rs` 2 hunks -> `rust/market/tests/graph.rs`;
`rust/tests/allocations.rs` 7 hunks -> `rust/fix/tests/allocations.rs`, 1 -> market;
`rust/src/implementer.rs` 3 hunks -> core (the `write_named_bytes` forwarder moved up, its doc
"for the market and FIX crates"); `rust/src/typed.rs` stays core (`FieldRecord::from_checked`);
`rust/benchmarks/{graph,fix}/` -> `rust/{market,fix}/benchmarks/..`; P9:
`rust/src/isin_registry.rs` -> `rust/market/src/instrument.rs`, `python/src/isin_registry.rs` ->
`python/src/instrument.rs`, `node/src/isin_registry.rs` -> `node/src/instrument.rs`. No path is
missing from both HEAD and P9's tree.

## Phase 0 - gate, then the map again on the landed tree

```bash
git fetch origin ccr-0fe6f9d0-ruymat && git log --oneline -3   # HEAD = P8's results commit
git status --short                                           # clean; pgrep cargo empty
python3 -I .handoff/split/scratch/p5r/path_map.py --repo . --preview /tmp/p5r-preview
```

The re-run re-checks every destination against the landed tree and rewrites the merge preview
(`git merge-file` of P4's blob -> section into the file as it stands, in `/tmp`, nothing in the
repository touched). Today 58 of 94 modified files merge clean; the preview's table is the work
list, P7 and P8 adding theirs (below). The outputs are regenerated in place; nothing else moves.

## Phase 1 - apply

```bash
P=.handoff/split/scratch/p5r
git apply -3 --check $P/p5_on_s4p9.patch        # "Applied patch to X with conflicts" per file, writes nothing
git apply -3 $P/p5_on_s4p9.patch
git apply --reject $P/p5_on_s4p9.routed.patch   # *.rej beside each file a hunk missed
git diff --name-only --diff-filter=U; find rust -name '*.rej'
```

- `git apply -3` stages what it merges and leaves conflicted paths unmerged; resolve, `git add`.
- Routed hunks today: `rust/src/implementer.rs` 3/3 and `rust/market/tests/graph.rs` 2/2 apply;
  `rust/fix/tests/allocations.rs` 3 of 7, `rust/fix/tests/graph/iterator.rs` 1 of 4,
  `rust/fix/tests/graph/market_data.rs` 1 of 2, `rust/market/src/graph/mod.rs` 3 of 5 (the
  delegation macros, which P9 grew by `instcode`), `rust/market/tests/allocations.rs` 1,
  `rust/market/tests/graph/market_data.rs` 1 reject: re-apply each `.rej` by hand, then delete it.
- Unrouted: in `rust/fix/src/lib.rs` drop `FixLifted` from `pub use identity::{..}` (:207) and
  `pub use anomaly::FixAnomaly;` (:179) if the mapped `lib.rs` section has not already; nothing
  else of that hunk applies (the core root re-exports no FIX name since S4).
- Drop the debug block in `rust/fix/tests/root/codec.rs` `mod round_trip`,
  `a_native_message_rendered_and_parsed_again_restates_its_facts`: the 5-line
  `eprintln!("SCRATCH wire {} meta {:?}", ..)`. `git grep -n 'SCRATCH wire'` empty.

Conflicts the preview names today (S4 + mid-P9), by file and count:

| File | Conflicts | Cause |
| --- | --- | --- |
| `rust/fix/src/msg.rs` | 24 | S4's `crate::` -> `yggdryl::` beside P5's slots and lend; P9's `fill_instrument`/`refill_instrument_ids` over `InstrumentTable`, `instcode` |
| `rust/fix/tests/root/market.rs` | 19 | P5's `.message()` re-spelling beside S4's install lines and P9's `instcode` cells |
| `rust/fix/tests/root/securityids.rs` | 10 | P9's `derived:isin` lines (D42.17) |
| `rust/fix/src/enrich.rs` | 8 | P9's `Codes::{Walk,Shared}` over `Instruments`, `Stated`, `learn_and_fill` |
| `rust/fix/tests/root/msg.rs` | 8 | as `msg.rs` |
| `.api-inventory.txt` | 7 | S4's per-crate sections; re-express, never merge (below) |
| `rust/fix/src/market.rs` | 6 | `impl MarketMessage for FixMsg` (the D36 trait) against P5's struct |
| `rust/market/src/instrument.rs` | 4 | P5's `InstrumentStatement` in `learn_stating` against P9's `Stated<'_>` (open point 1) |
| `rust/fix/tests/root/ulbridge.rs` | 4 | |
| `python/src/graph/mod.rs`, `rust/fix/benchmarks/fix/pipeline.rs`, `rust/fix/tests/root/{batch,enrich}.rs` | 3 each | |
| `AGENTS.md`, `rust/fix/tests/root.rs` | 2 each | AGENTS: P5 rewrote the pre-S4 `graph/` row whole (it still names `BookService`, `isin_registry.rs`); take ours and write P5's sentences into the `graph/`, `rust/market/src/graph/` and `rust/fix/src/` rows. `root.rs`: `#[path = "fix/anomaly.rs"]` is `root/anomaly.rs` |
| `.api-bindings.txt`, `docs/fix/{lifecycle,message}.md`, `docs/graph/market-data.md`, `node/src/{fix,graph/market_data}.rs`, `python/src/{fix,graph/market_data}.rs`, `rust/fix/src/{batch,codec,identity}.rs`, `rust/fix/tests/root/{codec,crated,digest,idmap}.rs`, `rust/market/benchmarks/graph{.rs,/mod.rs}` (P9's `instrument` beside the new `message`), `rust/market/src/graph/{arrow,market_data}.rs`, `rust/market/tests/graph/kind.rs`, `skills/yggdryl-fix/references/rust.md` | 1 each | |

Added by P7 (D40) - files the patch edits that P7 rewrites: `rust/fix/src/{schema,crated,identity}.rs`
(D40.1 `dictionary_event_tag`; 65_007/65_009 and the nine other derived definitions retired),
`rust/market/src/graph/{arrow,market_column}.rs` + `LiftedColumn` (D40.2), `graph/{facts,market}.rs`
(D40.3: `cficode` into `securityids`, `get_isin`/`get_cfi`/`set_cfi`/`get_mic`), `graph/book.rs`
(a book's sources), `rust/src/graph/element.rs` (D40.5, `crossuuid` = XXH3-128), every name of
D40.4 (the residue table: 36 patch files spell `isincode`/`cficode`/`miccode`/`countrycode`/
`eusipacode`/`*CODE_TAG_NAME`). Added by P8 (D41, user decision 12):
`rust/market/src/graph/iterator.rs` (4 P5 hunks: `note_conflict` an `Anomaly`) and
`rust/fix/src/enrich.rs` (the lifecycle), `docs/fix/lifecycle.md`, their tests.

## Phase 2 - re-spell inside the applied hunks (one script, asserted anchors)

`$P/p5r_respell.py`, driven by `cargo check -p yggdryl -p yggdryl-market -p yggdryl-fix
--all-targets --keep-going --message-format=short`, re-run until clean. Every path the added lines
spell, by destination (counted from the mapped patches):

| In | Spelled | Becomes |
| --- | --- | --- |
| `rust/fix/src` | `crate::fix::X` (36) | `crate::X` |
| `rust/fix/src` | `crate::graph::{Element, Event}` | `yggdryl::graph::..` |
| `rust/fix/src` | `crate::graph::{Anomaly, InstrumentStatement, Market, MarketMessage, MdUpdateAction, Metadata, Operation, StatedFacts, book, market}` | `yggdryl_market::graph::..` |
| `rust/fix/src` | `crate::implementer::{expected_got, feed_event_facts, field_new_with_metadata, struct_type_from_unique_fields}`, `crate::json::into_utf8` | `yggdryl::implementer::..`, `yggdryl::json::..` |
| `rust/fix/tests`, `rust/fix/benchmarks` | `yggdryl::graph::{Anomaly, Market, MarketMessage, MessageIterator, message}` | `yggdryl_market::graph::..`; `Element`/`Event` stay `yggdryl::graph` |
| `rust/fix/tests` | `yggdryl::internals::fix_enrich` | `yggdryl_fix::internals::enrich` |
| `rust/fix/tests` | `yggdryl::FixX`, `yggdryl::MarketMessage`, ... at the core root | `yggdryl_fix::`/`yggdryl_market::` (S4's rule) |
| `rust/market/src` | `crate::graph::*` (217) | unchanged |
| `rust/market/src` | `crate::implementer::{folds_equal, stable_hash_of, warned, write_named_bytes}`, `crate::xxhash::Xxh3` | `yggdryl::implementer::..` (core-private; add a forwarder for `Xxh3` if none) |
| `rust/market/tests`, `rust/market/benchmarks` | `yggdryl::graph::{Anomaly, InstrumentStatement, MarketMessage, StatedFacts}` | `yggdryl_market::graph::..`; `Element` stays |
| `rust/market/benchmarks` | `crate::bench_profile::corpus` | as S4's market benches reach it |

Then the later lanes' names over what P5 wrote, by their own idempotent sweeps:

```bash
python3 -I .handoff/split/scratch/p9/p9_sweep.py . --check      # IsinRegistry/isin_registry/IsinTable in P5's lines
python3 -I .handoff/split/scratch/p9/p9_sweep.py .              # no flag applies; idempotent, its moves already done
python3 .handoff/split/scratch/p7/p7_sweep.py . --check $P/p7_on_p5.manifest.json --manifest
python3 .handoff/split/scratch/p7/p7_sweep.py . --apply $P/p7_on_p5.manifest.json
git grep -nE 'SCRATCH wire|FixLifted|MarketData::Fix|BookService|currunix|recdunix|curruuid|currhashcode|IsinRegistry|isin_registry|instuuid' -- ':!.handoff'
```

The greps end empty (`detailedcficode` stays; a REVIEW site of the P7 sweep is settled by its
rule: intake spellings and hashed `feed(..)` names keep theirs).

Smoke: `cargo check -p yggdryl -p yggdryl-market -p yggdryl-fix --all-targets --keep-going
--message-format=short` clean; `cargo check --workspace --all-targets --keep-going
--message-format=short` (the bindings' broken sites listed).

## Phase 3 - the core (two files)

`rust/src/implementer.rs` (the forwarder's doc), `rust/src/typed.rs` (`FieldRecord::from_checked`).

```bash
cargo check -p yggdryl --all-targets
cargo test -p yggdryl --test root typed
```

## Phase 4 - market: the message, the anomaly, the delegation macros

Files: `rust/market/src/graph/{message,anomaly,mod,market_data,kind,arrow,book,iterator}.rs`,
`rust/market/src/instrument.rs`, `rust/market/tests/graph/{message,anomaly,kind,market_data}.rs`,
`rust/market/tests/graph.rs`, `rust/market/tests/allocations.rs`,
`rust/market/benchmarks/graph{.rs,/mod.rs,/message.rs}`.

Re-derive from the tree, never from the patch: `MarketMessage::field(root, child_root)`'s fact
columns follow `MarketColumn::ALL` and P7's `LiftedColumn` (P5 wrote "56 fact columns in
`marketdata` order" at P4; P9 added `instcode`, P7 moved four into the lifted band); a
`StatedFacts` bit for `instcode` does not exist - it is filled, never stated (C2: a derivation
marks nothing).

```bash
cargo test -p yggdryl-market --test graph message
cargo test -p yggdryl-market --test graph anomaly
cargo test -p yggdryl-market --test graph kind
cargo test -p yggdryl-market --test graph market_data   # the size pins as P7 left them, green without edit
cargo test -p yggdryl-market --test graph iterator
cargo test -p yggdryl-market --test root instrument
cargo test -p yggdryl-market --doc graph::message
cargo test -p yggdryl-market --test allocations         # a fall re-pinned with its sentence, a rise a defect
cargo bench -p yggdryl-market --bench graph -- message --quick
```

## Phase 5 - FIX: C1, C2, the round trip, the lifecycle

Files: `rust/fix/src/{build,msg,codec,identity,batch}.rs` (worker A) against
`rust/fix/src/{market,enrich}.rs` + bindings + tests (worker B), as the brief splits them.

**The one failing test, red first**: `codec::round_trip::a_native_message_rendered_and_parsed_again_restates_its_facts`.

```bash
cargo test -p yggdryl-fix --test root codec::round_trip::a_native_message_rendered_and_parsed_again_restates_its_facts
```

Cause (`$S/p5r_report.md` 17:05, confirmed against the patch): the native order's entry
`desknote` resolves to no field, so `laid_out` files it in the message's metadata; the frame
writes metadata only through `stated_band` (`rust/fix/src/msg.rs` after the apply, patch line
14539), whose `METADATA_TAG_NAME` (65_037) arm renders the map through `wire_text_under`/
`wire_text` - and `wire_text` answers `None` for `Scalar::Map`/`SortedMap`
(`rust/fix/src/entry.rs:198-212`), so the band drops the entry and the wire
(`..|58=hold|54=2|55=AAPL|65003=..|65016=10|`) carries no `desknote`; parsed back,
`back.get_metadata()` is empty. Fix at cause: 65_037 has no wire form - take it out of
`stated_band` beside `derived`, and write each metadata pair the content row does not already
hold as an unmapped `key=value` entry (tag 0 under its own name, as a bridge's `OMS_ListID=L1`
arrives - `rust/fix/tests/root/idmap.rs:380`), which the parse reads back into the metadata
(`for_each_unmapped`, `msg.rs:3426`). A parsed bridge key is in the row already and is not
written twice.

`stated_band` after P7 and P9: its `derived` list gains `INSTCODE_TAG_NAME` (65_054, the parse
fills it, never a source statement) and spells P7's `ISIN_`/`BBG_`/`FIGI_`/`FOREX_TAG_NAME`; its
`TRANSUNIX_TAG_NAME` arm goes with 65_007 (D40.1: the stated instant is `TransactTime(60)`, read
through `dictionary_event_tag`). `restatable` in `rust/fix/tests/root.rs` lists the tags the
facts took (`[6, 14, 31, 32, 38, 44, 53, 151, 194, 195]` at P4): re-derive it from D40.1's pairing
table. `identity::FACT_TAGS` (27, the inverse idmap) re-derived the same way.

```bash
cargo test -p yggdryl-fix --test root codec
cargo test -p yggdryl-fix --test root msg
cargo test -p yggdryl-fix --test root market
cargo test -p yggdryl-fix --test root idmap
cargo test -p yggdryl-fix --test root identity
cargo test -p yggdryl-fix --test root enrich              # the lifecycle partyids/identifiers regression stays gone
cargo test -p yggdryl-fix --test root codec::equivalence   # green without edit
cargo test -p yggdryl-fix --test root store                # the dump and the hash green without edit: D37 moves no crate field, fixmsg stays 65_053
cargo test -p yggdryl-fix --test graph
cargo test -p yggdryl-fix --features internals --test root -- enrich batch
cargo test -p yggdryl-fix --test allocations               # FIX_LINE_COSTS (4,28),(16,29),(64,31) never rise; into_message 0
cargo test -p yggdryl-fix --test iobase_calls
cargo test -p yggdryl-fix --test instrument
cargo test -p yggdryl-fix --doc
```

## Phase 6 - Python

```bash
VIRTUAL_ENV=python/.venv python/.venv/bin/python -m maturin develop -m python/Cargo.toml
python/.venv/bin/python -m pytest python/tests/graph/test_message.py python/tests/graph/test_anomaly.py python/tests/graph/test_market_data.py python/tests/graph/test_init.py -x -q
python/.venv/bin/python -m pytest python/tests/test_fix.py python/tests/test_instrument.py -x -q
python/.venv/bin/python -m mypy --strict --config-file python/pyproject.toml python/yggdryl python/tests/typing_bindings.py python/tests/typing_fields.py
```

`python/src/instrument.rs`: P5's `held.fill(registry)`/`held.enrich(registry)` onto P9's
`Instruments`; the classes stay in the one native module.

## Phase 7 - Node (no new door)

```bash
npm run --prefix node build:debug          # regenerates node/index.js, node/index.d.ts
node --test node/tests/graph/index.test.js node/tests/graph/market_data.test.js node/tests/fix.test.js node/tests/instrument.test.js
npm run --prefix node test:package:debug
node scripts/build_docs_fix.js && node scripts/build_docs_playground.js && node scripts/build_docs_fix.js --check && node scripts/build_docs_playground.js --check
```

## Phase 8 - docs, skills, inventories

`.api-inventory.txt`: P5's 8 hunks are pre-S4 sections; write them under S4's headers
(`### yggdryl_market::graph::MarketMessage`, `Anomaly`, `StatedFacts`, `InstrumentStatement`,
`MessageIterator`; `FixMsg::{from_message, into_message}` under the FIX crate's), drop `FixLifted`,
`FixAnomaly`, `MarketData::Fix`. `docs/fix/message.md` keeps the "What crosses" table.

```bash
python -m mkdocs build --strict --config-file mkdocs.yml
python scripts/check_api_inventory.py
python scripts/generate_internals.py --check
```

The three `check_docs_examples.py --lang {rust,python,javascript}` run as chain steps.

## Then

`cargo fmt --all` once after the last worker; the greps (no test code under any `src/`, no model
identifier outside `.handoff/`, the old names above); commit with `$S/p5_commit_message.txt`
verbatim; the chain (`$S/logs/chain.sh`), one push, CI read to `CI result`; the results commit.

## Open points

1. **`InstrumentStatement` (P5) against P9's `Stated<'_>`/`Body`** (`rust/market/src/instrument.rs:2982`):
   two types for one fact set. Recommended: one owned `InstrumentStatement` in
   `graph/message.rs` holding P9's facts under P7's names (`country`, `underlying`, `eusipa`, the
   `Body`, the metadata pairs), filled at parse in `build.rs` (D37), and `learn_stating(event,
   &InstrumentStatement)` - `Stated` deleted, the origin currency the event's own read before the
   fill (P5's rule; P9 passes it explicitly). Decide before phase 4; it moves `enrich.rs:780`.
2. P8's match rule (decision 12) may change the two-chains case P5's `note_conflict` records as an
   `Anomaly`: re-read P8's `iterator.rs` before resolving its 4 hunks.
3. The FIX row's `cficode` vs `cfi` (D40 interpretation 6) in P5's added lines: one decision per
   site, by the P7 sweep's REVIEW list.
4. The map was computed mid-P9; phase 0's re-run is the authority for the conflict list.
