# P5 state (paused for S4) - 2026-10-09

Preserved: branch `wip/p5-impl` = bf5921bf4 on P4 (4e46b5ab7): the define (`$S/p5_define.patch`, `git apply -3`, clean) plus everything below. `wip/p5` (the define alone, on 53e7d3c21) kept. `/home/user/yggdryl-p5` left in place.

## Done (on wip/p5-impl)
- Define applied; compile fixes (11 lib errors: trait imports, Scalar text in `laid_out`, `recache` borrow, const `as_field`, `enrich` sort over `message()`); 73 sites re-spelled (`$S/p45/scripts/c3_sites.py`), 122 trait calls (`p5_w6/fix_e0599.py`); batch.rs's direct session-event merge moved under `internals` (forwarders `fix_enrich::{is_followed_identifier, with_previous}`).
- C3: `FixMsg::{market_data, into_market_data, into_market_leaf}` deleted; tests, docs tabs (`docs/fix/message.md`, `docs/graph/{index,market,trade}.md`, `docs/fix/arrow.md`, skills), rustdoc links and `.api-inventory.txt` re-spelled; Node `marketData()` via `into_message()`.
- C4: `MarketMessage::finalize` keeps a held hashcode; `FixMsg::fill`/`enrich` (settle after the registry; doc example); Python/Node `fill`/`enrich` call them; inventory lines added.
- C1 (as a slot design, see DESIGN.md "#### As landed" on the branch): `identity::FACT_TAGS` (27, the inverse idmap) + `fact_of_tag`/`fact_reading`; builder/`assemble`/`replace_content` move a fact field's cell into `FixIndexes::slots` (`Consumed {tag, before, cell}`, sorted by place); entries/wire/digest interleave slots; reads/writes/removes reach them; writes placed where staged among appends; appended children land in front of the trailer cells; `message_mut` lends (fact readings + the codec word without read bits + the security set) and `absorb_lent` writes fact changes into slots and re-applies the overlay rule; `stating` door for session-event folds; book group by message type (`book_group`); `state_market_data` at parse, in `FixMessages`, `with_registry` and row reads; `contributes_to_market` = `is_market_data()`.
- C2: read bits per fact published beside the codec word; derived fields (native rules, FX detection) mark nothing (`derived_tags`, `note_derived`).
- C5: "What crosses" table in `docs/fix/message.md`; round-trip tests made order-insensitive; `render` writes `Text(58)`.
- Docs: StatedFacts wording (graph/message.rs rustdoc, `docs/graph/market-data.md`); AGENTS.md `graph/`/`fix/` rows (C1/C2/C4); `.api-bindings.txt` two stale `FixMsg.market_data` mentions; DESIGN.md "### Decisions after the define" + "#### As landed" under "## P5: design".
- Bench: `fix/pipeline/market` gains `message_out_and_back` and `render_native_message` (`rust/benchmarks/fix/pipeline.rs`).

## Last run (run8, `$S/p45/p5_run8.log`): graph 411/411, fix 905/906
- Equivalence snapshot unmoved (green since run5); 912 pin (graph) green; content-code pin green.
- The one failure: `codec::round_trip::a_native_message_rendered_and_parsed_again_restates_its_facts` - the native order's unresolved entry `desknote` (laid out into metadata) is not on the rendered wire (`...|58=hold|54=2|55=AAPL|65003=..|65016=10|`, no metadata crate tag), so `back.get_metadata()` is empty. Look at `stated_band`'s metadata rule after `laid_out` / `FixMsg::adopted`.
- A temporary `eprintln!("SCRATCH wire ...")` sits in that test (rust/tests/fix/codec.rs, mod round_trip): remove it.

## Unfinished
- Not yet run: `--test root`, `--features internals` (fix), `allocations` (FIX_LINE_COSTS (4,28),(16,29),(64,31) must not rise; the define's rows `into_message` 0 allocs, `from_message` adopted, W children linear), `iobase_calls`, `cargo check --workspace` (bindings), clippy, rustdoc.
- Bindings: Python (maturin develop; pytest `graph/test_message.py`, `test_anomaly.py`, `test_market_data.py`, `test_fix.py`, isin_registry; mypy), Node (build:debug regenerates index.js/index.d.ts; graph tests, `asMessage`; tsc; the two docs manifests).
- Docs/skills sweep for the define's remaining pages; inventories check; mkdocs strict; check_docs_examples (the message.md examples changed).
- The lifecycle identifiers/partyids regression of the baseline is gone since run5 (lend word + overlay rule); the round-trip and idmap pins pass but the one above.

## Files touched beyond the define
`.handoff/split/DESIGN.md`, `docs/graph/market.md`, `docs/graph/trade.md`, `rust/examples/fix_capture.rs`, `rust/src/fix/native_derivations.rs`, `rust/tests/fix/component.rs`, `rust/tests/fix/latest.rs`, `rust/tests/fix/store.rs`; and, inside the define's set, further edits to `rust/src/fix/{msg,build,codec,identity,market,messages,enrich,latest,schema}.rs`, `rust/src/graph/message.rs`, `rust/benchmarks/fix/pipeline.rs`, `python/src/isin_registry.rs`, `node/src/{isin_registry,fix}.rs`, `rust/tests/fix.rs` (`restatable` carries taken fields), `rust/tests/fix/{codec,msg,market,batch,digest,forex,cfi,ulbridge,securityids}.rs`, `rust/tests/allocations.rs`, `docs/fix/{message,arrow}.md`, `docs/graph/{index,market-data}.md`, `skills/yggdryl-fix/{SKILL.md,references/rust.md}`, `AGENTS.md`, `.api-inventory.txt`, `.api-bindings.txt` (`git diff 4e46b5ab7 wip/p5-impl --stat` for the whole).

## Re-targeting after S4
Path map: `rust/src/graph/` -> `rust/market/src/graph/`; `rust/src/fix/` -> `rust/fix/src/`; `rust/tests/graph/` -> `rust/market/tests/`, `rust/tests/fix/` -> `rust/fix/tests/` (harness entries likewise); `crate::` inside moved files -> `yggdryl::` for core items and `yggdryl_market::` for graph items, core-private items through `yggdryl::implementer::..` (add forwarders there for anything the slots/lend code reaches: `field_new_with_metadata`, `struct_type_from_unique_fields`, `warned!`, ...). Suggested route: `git diff 4e46b5ab7 wip/p5-impl > p5_impl.patch`, rewrite paths with the map (a script of exact-path renames on the `diff --git`/`---`/`+++` lines), `git apply -3`, then the compiler's `--keep-going --message-format=short` list for the `crate::` re-spellings.
