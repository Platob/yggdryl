# P5 define - reconciler report

Tree: `/home/user/yggdryl-p5` (branch `wip/p5`, base `53e7d3c21`), uncommitted. 97 tracked files changed (+7864/-4861), 10 new files, 2 deleted. No cargo was run.

## Top three

1. **C1 blocks green.** W2 kept FIX's arrival record as the message's entries, so `Side(54)`, `Price(44)`, `OrderQty(38)`, `Currency(15)` and the other idmap-mapped tags are still entry cells, and the book children are built at `into_message`, not at parse. This breaks D37's idmap rule. With D37's flat row form, `MarketMessage::from_scalar` refuses every FIX-derived message at `$.price`, so pickle fails. The W4 and W6 pins written to the design rule also fail. W2 asked to nest the entries under one `entries` column; that was not applied because it contradicts D37's flat form. The lane manager must choose.
2. **C2: the round trip does not restate the stated bits.** `FixCodec::parse(render(m))` does not restate `StatedFacts` for a native message. `laid_out` unmarks the bits it writes under standard tags, and the parse leaves the facts it reads off fields unmarked (W1's own rule). The pin in `rust/tests/fix/codec.rs` `mod round_trip` and the `docs/fix/message.md:541` example `assert_eq!(again.stated(), native.stated())` will fail.
3. **Reconciled for compile, still unverified by cargo.**
   - Added W1's four missing crate-private doors (`set_entries_unchecked`, `clear_children`, `anomalies_mut`, `settle_orders`).
   - Fixed a duplicate `fn msgtype_of` in `fix/codec.rs` (E0428): the new one is renamed `msgtype_for_kind`.
   - Removed `FixAnomaly` and `FixLifted` from `lib.rs`. Re-spelled `latest.rs` and `messages.rs`.
   - Swept 12 unowned FIX test files: 305 call sites, plus one by hand.
   - Wired Node's `MarketMessage` into `mod.rs`, `lib.rs`, `binding.js` and `binding.d.ts`.
   - Re-pointed the Python and Node `isin_registry` doors to `message()`.
   - Results: `rustfmt` clean on all 68 changed `.rs`; `check_api_inventory.py` "inventories are current"; `mkdocs build --strict` built.

## Files edited by the reconciler (beyond W1-W6's sets)

| File | Change |
| --- | --- |
| `rust/src/graph/message.rs` | Added the crate-private `set_entries_unchecked`, `clear_children`, `anomalies_mut` and `settle_orders` (W2, W3). `stable_hash_of`, `folds_equal` and `write_named_bytes` now go through `crate::implementer` (W3). `record()` uses `FieldRecord::from_checked`, one pass with no `expect` (W1's optional request). |
| `rust/src/typed.rs` | Added `pub(crate) FieldRecord::from_checked(field, cells)` inside `mod record`. |
| `rust/src/lib.rs` | `FixAnomaly` and `FixLifted` dropped from `pub use fix::{..}`; rustfmt reflowed the list. |
| `rust/src/fix/latest.rs` | `lifted().clordid()` becomes `get_by_tag(11)` (W2). |
| `rust/src/fix/messages.rs` | `place_naming_sources(message.message_mut())` (W2). |
| `rust/src/fix/codec.rs` | W2's new `msgtype_of(kind)` collided with the existing `msgtype_of(pairs)` at the same module level. It is renamed `msgtype_for_kind`: definition, 2 calls and 1 doc link. |
| `rust/src/fix/msg.rs` | The `from_message` doc said "`D` a new order"; the code and D37 say `8` for an order or an execution. Doc fixed (W6). |
| `rust/src/fix/enrich.rs` | `learn_and_fill` doc: the origin currency links `Market::get_origccy`, and the `- then fills` line that rendered as a list item is rewrapped (W3). |
| `rust/tests/graph/iterator.rs` | `Vec<MarketMessage>`, `MarketMessage::try_from`, `as_message()` (W6, stage-2 request). |
| `rust/tests/fix/{alias_rule,batch,cfi,crated,digest,entry,forex,identity,idmap,mod_,securityids,ulbridge}.rs` | W6's `sweep_fixmsg_traits.py`: 4, 77, 4, 43, 16, 37, 8, 30, 11, 13, 40 and 22 sites, plus `batch.rs:1657` by hand (`expired.message().clone().with_previous(live.message())`); rustfmt. The sweep re-spelled 0 sites in every other FIX-using file (CLI, `scale_ulbridge.rs`, the benches). |
| `node/src/graph/mod.rs`, `node/src/lib.rs` | `mod message;` and `pub use message::JsMarketMessage;`; `JsMarketMessage` added to the `graph` re-export (W5). |
| `node/src/isin_registry.rs` | `learn(..message())`, `fill/enrich(..message_mut())` (W5). |
| `node/binding.js` | `NativeMarketMessage`, `graph.MarketMessage`, and `'MarketMessage'` in the flat-name delete list; the `MARKET_ITEMS` comment (W5). |
| `node/binding.d.ts` | `MarketMessage` in both lists, `Graph.MarketMessage` with its doc, `type Anomaly` (W5). |
| `python/src/isin_registry.rs` | `learn` / `fill` / `enrich` through `message()` / `message_mut()` (W4). |
| `python/tests/typing_bindings.py` | W4's two edits plus its optional mypy-verified block; byte-identical to `scratchpad/p5_w4_typing/typing_bindings_w4.py`. |
| `python/tests/enums/test_init.py` | `MARKET_KINDS` ends `"message"` (W4). |
| `python/tests/test_logging.py` | `anomaly.field` (W4). |
| `python/benchmarks/fix.py` | `PARSED.into_message().into_market_data()`. Python's `FixMsg.market_data()` is gone, and this caller was not in any worker's set. |
| `AGENTS.md` | The `isin_registry.rs` row's `learn_stating` sentence (W3). |
| `docs/fix/message.md` | The bindings row (Python `FixMsg.into_message().into_market_data()`, JS `marketData()`); Python tabs at 110, 124 and 423; the `from_message` row's MsgType wording (W4, W6). |
| `docs/types/enum/timeinforce.md` | `message.message().get_timeinforce()` (W6). |
| `docs/graph/index.md`, `docs/graph/event.md`, `docs/graph/operation.md` | `FixAnomaly` becomes `Anomaly` on a `MarketMessage`; "A FIX message's market message implements all four traits" (W6). |
| `skills/yggdryl-fix/references/python.md`, `skills/yggdryl-fix/SKILL.md` | The Python `FixMsg.market_data()` cells re-spelled (W4). |
| `.api-bindings.txt` | The Node `graph.MarketMessage` line no longer names `asFix()`. |
| `.api-inventory.txt` | The `from_message` MsgType wording (`8`, `S`, `W`, `AE`; the state under its crate tag). |

Workers' own sets, as reported:
- W1: `graph/{message,anomaly,mod,market_data,kind,arrow,book,iterator}.rs`.
- W2: `fix/{msg,market,enrich,codec,identity,build,batch,schema,mod}.rs`; `fix/anomaly.rs` deleted.
- W3: `isin_registry.rs`, `implementer.rs`, `benchmarks/fix/{pipeline,resolve,ulbridge}.rs`, `benchmarks/graph/message.rs` (new), and `benchmarks/graph{,/mod}.rs`. The last two lines fall just outside W3's glob; W3 flagged them.
- W4: `python/src/graph/{message,anomaly}.rs` (new), `python/src/graph/{market_data,mod}.rs`, `python/src/fix.rs`, `_native.pyi`, `yggdryl/graph/__init__.py`, `yggdryl/fix.py`, `python/tests/graph/{test_message,test_anomaly}.py` (new), `test_market_data.py`, `test_init.py`, `test_fix.py`, and the Python section of `.api-bindings.txt`.
- W5: `node/src/fix.rs`, `node/src/graph/{market_data,message}.rs`, `node/tests/graph/{market_data.test.js,index.test.js,index.types.ts}`, and the JS section of `.api-bindings.txt`.
- W6:
  - Rust tests: `rust/tests/graph/{message,anomaly}.rs` (new), `graph.rs`, `graph/{kind,market_data}.rs`, `fix.rs`, `fix/{msg,market,codec,schema,enrich}.rs`, `allocations.rs`; `fix/anomaly.rs` deleted.
  - Docs and skills: `docs/graph/market-data.md`, `docs/fix/{message,lifecycle,capture,arrow,encode,index,registry}.md`, both skills.
  - `AGENTS.md` and `.api-inventory.txt`.

## Public names

**Added (Rust):**
- `yggdryl::graph::MarketMessage`:
  - `new`, `set_marketdatakind`, `stated`, `set_stated`
  - `root`, `row`, `record`, `entry`, `set_entries`, `try_with_entries`, `set_entry`
  - `children`, `children_mut`, `push_child`
  - `anomalies`, `note_anomaly`, `set_anomalies`
  - `instrument`, `set_instrument`, `book`, `set_book`, `is_acknowledgement`, `set_acknowledgement`
  - `stable_hash`, `is_market_data`, `into_market_data` (returns `Vec`), `into_market_leaf`
  - `field(root, child_root)`, `into_scalar`, `from_scalar(root, child_root, &row)`
  - impls: `Element`, `Event`, `Market`, `Operation`, `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`, and `From<MarketMessage> for Result<MarketMessage>`.
- `graph::StatedFacts`: 28 fact bits plus `TRANSUNIX`, `HELD_EXECUNIX`, `HELD_SENDUNIX`, `EMPTY` and `ALL`, with the set operations.
- `graph::InstrumentStatement { countrycode, underlyingisin, eusipacode }` and `is_empty`.
- `graph::MessageIterator<I>`.
- `graph::Anomaly` (`new`, `field`, `reason`, `Display`).
- The modules `graph::message` and `graph::anomaly`.
- `MarketData::Message`, `as_message() -> Option<&MarketMessage>`, `as_message_mut`, and `From`/`TryFrom` between `MarketData` and `MarketMessage`.
- `MarketKind::Message` (spelled `message`, last).
- `FixMsg::{message, message_mut, into_message, from_message}`.
- `FixMsg::is_execution`, now inherent.
- `FixCodec::render`.

**Changed signatures:** `FixMsg::metadata -> &Metadata`, `FixMsg::anomalies -> &[Anomaly]`, and the crate-private `IsinRegistry::learn_stating(event, &InstrumentStatement)`.

**Added (Python):** `graph.MarketMessage`, `graph.Anomaly`, `MarketData.as_message()`, `FixMsg.into_message()`, `FixMsg.from_message(registry, message)`; `FixMsg.anomalies` now answers `list[graph.Anomaly]`.

**Added (Node):** `graph.MarketMessage` (getters only), `MarketData.asMessage()`; `FixMsg.anomalies` now answers `Anomaly[]`.

**Removed:**
- Rust:
  - the S3 `trait MarketMessage` and its Box impls, plus the `market_data::seam` module;
  - `MarketData::Fix`, `MarketKind::Fix` (`fix`) and `as_message::<T>()`;
  - `FixAnomaly` and `fix/anomaly.rs`;
  - `FixLifted` with all its getters, `LiftedFx` and `FixMsg::lifted()`;
  - `FixMarketIterator`;
  - `impl Element, Event, Market, Operation for FixMsg`, `impl TryFrom<MarketData> for FixMsg`, `impl From<FixMsg> for OperationEventFacts`;
  - `FixMsg::stated_origccy`.
- Python: `MarketData.as_fix()`, `FixMsg.market_data()`, the `fix` kind spelling, and the tuple shape of `FixMsg.anomalies`.
- Node: `MarketData.asFix()`, `FixAnomalyView`, the `fix` kind spelling.

**Reconciler:** no public name added. `clear_children` is `pub(crate)`: W2 asked for `pub`, W6 for `pub(crate)`, and only `fix/` calls it. `FieldRecord::from_checked` is `pub(crate)`.

## Requests: applied, and not applied

**Applied as written:**
- W1 → lane manager: `lib.rs` `FixAnomaly`; the optional `typed.rs` request.
- W2 → W1: `set_entries_unchecked`, `anomalies_mut`, `settle_orders`, `clear_children` (the last as `pub(crate)`, see above).
- W2 → lane manager: `lib.rs` `FixLifted`, `latest.rs`, `messages.rs`.
- W3:
  - → W1: the three `crate::implementer` paths;
  - → W2: the `enrich.rs` doc (the body was already in place); `stated_origccy` was already deleted;
  - → W6: the AGENTS `isin_registry` row.
- W4 → lane manager: `typing_bindings.py`, `enums/test_init.py`, `test_logging.py`, `python/src/isin_registry.rs`.
- W4 → W6: the docs and skills Python tabs W6 had not done (`message.md` 23, 110, 124, 423; `skills/yggdryl-fix` `python.md:583` and `SKILL.md:93`). Lines 128, 132, 847 and 893, `lifecycle.md:347/350` and `market-data SKILL.md:191` were already done.
- W5 → lane manager: `node/src/graph/mod.rs`, `node/src/lib.rs`, `node/src/isin_registry.rs`, `node/binding.js`, `node/binding.d.ts`.
- W6 → stage 2: `rust/tests/graph/iterator.rs`, the unowned `rust/tests/fix` sweep and `batch.rs:1657`.
- W6 → W2: the `from_message` doc's `D`/`8`.
- W6, docs outside its set: `timeinforce.md`, `graph/index.md`, `event.md`, `operation.md`.

**Not applied:**
- **W2 → W1, nest the entries under one `entries` column.** D37 states the flat form ("then the entry root's children"). The collision comes from W2's own deviation (C1).
- **W1 → W2, delete `FixMsg::{market_data, into_market_data, into_market_leaf}`, and switch `contributes_to_market` to `m.message().is_market_data()`.** W2 kept the three as wrappers. `into_market_leaf` remaps the error path to `$.MsgType(35)` / `$.NoMDEntries(268)`. W6's tests (about 45 sites in `rust/tests/fix/{market,crated,idmap}.rs`, including `market.rs:2085`'s `NoMDEntries(268)` pin), `.api-inventory.txt:1068-1070`, the Rust docs tabs and Node's `FixMsg.marketData()` (`node/src/fix.rs:1540`) all rely on them. `contributes_to_market` cannot read `is_market_data()` before `into_message`, because W2 sets the acknowledgement, the book control and the children there. The design does not name the deletion. Decision C3.
- **W1 → W2, fill the facts, children, book control and acknowledgement at parse in `build.rs`.** Not done by W2: it builds them in `into_message`. Part of C1.
- **W3 → W6, the AGENTS "S4 note".** No S4 note exists in AGENTS.md. W3's count belongs in DESIGN.md "### P5 results" or the handoff: of the 31 market-owned items FIX reached, 8 are gone and 23 stay.
  - Gone: `delegate_market!`, `delegate_operation!`, `OperationEventFacts`, `base_crosscode`, `MarketData::as_event`, `OperationEventFacts::into_event` / `from_facts`, `set_marketdatakind`.
  - Stay: `iterator::order`, `EventIterator::with_placing`, `restating_operation`, `MarketDataKind::stored_side`, `merge_operation_event`, `settle_orders`; the identifier vocabulary readers (`identifier::{WORD_PAIR_WIDTH, fold_into, folded_len, is_word}`, `idtype::names_another_instrument`, `IdKey::infer`, `IdKey`/`IdType::value_into`, `IdSource::from_namespace`, `IdType::identifier_names`, `IdType::underlying_security`, `Identifiers::held_at`); `isin_registry::{EconomicMemo, IsinTable, warn_full}`, `IsinRegistry::learn_stating`, `IsinTable::{fill_identifiers, fill_unsettled}`.
  - P5 adds two reaches from `graph/message.rs` for S4's implementer: `FieldScalar::from_checked` and `FieldRecord::from_checked` (both crate-private).

## Conflicts and decisions for the lane manager

- **C1 - W2 against D37's idmap rule.** The FIX dictionary names these fields exactly as the facts they map onto: `44 price`, `54 side`, `15 currency`, `53 quantity`, `31 lastpx`, `32 lastqty`, `6 avgpx`, `14 cumqty`, `151 leavesqty`, `84 cxlqty`, `99 stoppx`, `132 bidpx`, `461 cficode`, `59 timeinforce`, `1138 displayqty`. The crate band adds `crosscode`, `state`, `marketdatakind` and the rest at 650xx, which a parsed record does not hold.
  - **Failing:**
    - the pickle round trip of any FIX-derived message (`python/tests/graph/test_market_data.py:139`);
    - `MarketMessage::field(fix_root)`;
    - `rust/tests/fix/msg.rs` `mod message_handle` (the idmap pins: `entry("Side")` none or null, `|54=2|` after `set_side`);
    - `python/tests/test_fix.py` (`message.entry("side") is None`, `54` no row child, `set(54)` does not grow the row, the lifecycle adds no `54` entry).
  - **Options:**
    - (a) implement the rule in `fix/`: drop the idmap-mapped tags from the record and render them from the facts at `into_row` and on the wire, keeping `into_bytes`, the FIX digest and the equivalence snapshot byte-identical. W2's objection is arrival order.
    - (b) amend D37: nest the entries (W2's request) and re-pin the W4/W6 idmap pins to the arrival-record rule. AGENTS' `fix/` row, `docs/fix/message.md` and `.api-inventory.txt:1024` state the design rule and move with the choice.
- **C2 - the StatedFacts round trip.** The contract and W6's pin require `parse(render(m)).stated() == m.stated()`. W1's rule (codec-read facts unmarked) plus W2's `laid_out` `unmark_stated` make it false for a native message stating `price` or `side`.
  - Options: publish the facts read off fields onto the message's `StatedFacts` at `into_message`, keeping FixMsg's shadow word for the fixed row's crate band so the snapshot does not move; or narrow the contract to the bits under crate tags. The `docs/fix/message.md:541` example fails until one of these lands.
- **C3 - the three `FixMsg` market_data wrappers.** Keep them (the current tree) or retire them (one spelling). Python already dropped `FixMsg.market_data()`; Node keeps `marketData()`.
- **C4 - identity after a registry fill through `message_mut()`.**
  - `IsinRegistry::fill`/`enrich` call `element.finalize()`. Through the Python and Node `fill`/`enrich` and W3's benches (`message_mut()`), that is `MarketMessage::finalize`, the generic digest, not `FixMsg::settle`. A filled `FixMsg`'s `uuid` and `hashcode` move off the FIX digest, and its indexes and derived overlay are not refreshed.
  - The same holds for a FIX message walked by the graph `EventIterator` as `MarketData::Message` (W1 noted it).
  - Options: a `FixMsg`-level fill door, or the bindings settle afterwards.
- **C5 - the round trip loses parts of a FIX message (W2's list).** Across `into_message` / `from_message`:
  - the capture's carried row cells;
  - the plugin side of a message with no `msgpluginside` cell;
  - the arrival/idmap split of anomalies;
  - an unstated header sending clock;
  - a bridge key lifted into an identifier comes back as that identifier, not as the key.

## Residue left by the grep (excluding `.handoff/` and history)

- `docs/assets/fix.json` (46) and `node/index.d.ts` (89, including `asFix()` and `FixAnomalyView`): generated files. Regenerate with `npm run --prefix node build:debug`, then `node scripts/build_docs_fix.js` and `node scripts/build_docs_playground.js`.
- `rust/tests/fix/equivalence.snapshot` (1283 `currunix` keys of the text-line columns, unchanged since `2ae975674`): P4's landing commit owns its re-pin; P5 leaves it unmoved.
- `rust/tests/fix/store.rs` (17): history sentences of the dictionary hash ("It last moved when ...").
- `refrecdunix`, pinned absent: `node/tests/fix.test.js` (2), `node/tests/fix.types.ts`, `python/tests/test_fix.py` (2), `rust/tests/fix/{digest,schema}.rs`, `rust/tests/graph/column.rs`; plus a free capture name in `rust/tests/text/line.rs` (9). These are not residue.
- `python/tests/graph/test_init.py:55` pins `"FixAnomaly"` absent. `AGENTS.md:410` says "there is no `FixLifted` ... no `FixAnomaly`", a statement of absence.

## Assumptions to verify at the compiler

- **W1:**
  - the macro forms: the `@impl` arms, the `stating = ...` marking arm, `[$($deref:tt)?]`, hygiene of `fn note_conflict` passed through `$item`;
  - the implicit reborrow in `from_cells`;
  - `Field::canonicalize_value` answering a run for the empty `entries` root;
  - `Zip<Map<..>, slice::Iter<Scalar>>` being `ExactSizeIterator` for `implementer::write_named_bytes`;
  - the doctest of an exact row-form round trip;
  - `size_of::<MarketData>() == 912`.
- **Reconciler:**
  - `FieldRecord::from_checked(&self.root, &cow)`: deref coercion from `&Arc<Field>`, and the temporary `Cow` living through the call;
  - `get_by_tag(11)` answering `None` for an empty `ClOrdID` exactly as `FixLifted::clordid` did;
  - `as_inner_mut()?.message_mut()` borrows in `python/src/isin_registry.rs`.
- **W2:**
  - `FixMsg::adopted`/`recache`/`unmark_stated`, `FixHeader::unknown()`, `Unmapped::occurrence`;
  - the crate band of the render (65_037 metadata) parsing back;
  - `msgtype_for_kind` replacing every new call (3 sites; the 6 older `msgtype_of(pairs)` calls untouched).
- **W3:** NLL ends the borrow in `let facts = message.message_mut(); ...; message`; `MdUpdateAction::Snapshot`; `BookRef: Default + Clone`.
- **W4:** PyO3 0.29 accepts the `MarketMessage` constructor signature with `book=ellipsis()`; `Option<Vec<PyRef<PyMarketMessage>>>`; the `#[expect(clippy::needless_pass_by_value)]` attributes are fulfilled.
- **W5:** `#[napi(object, js_name = "Anomaly")]`; the getter macros over `JsMarketMessage`; `JsMarketData::into_leaf` being infallible.
- **Cost pins:**
  - `allocations.rs` @8715: bytes per parsed message within 2%, after W2's `FixIndexes` rework;
  - `into_message` 0 allocations on `35=D|55=AAPL|`;
  - `a_market_message_adopted_back_rebuilds_its_indexes_alone`: equal counts at 4 and 64 pairs;
  - the `W` children row: linear;
  - rekey 2/18/0 may fall (a re-pin with its sentence, not a defect).
- `rust/tests/fix/equivalence.snapshot` and the dictionary hash must stay green without edit.

## The inverse idmap (W2, `fix/codec.rs` `standard_writes`)

| Fact | Written under |
| --- | --- |
| `side` | `Side(54)`, its wire char (`Side::fix_code`) |
| `price` | `Price(44)` |
| `quantity` | `Quantity(53)` |
| `ordqty` | `OrderQty(38)` |
| `currency` | `Currency(15)` |
| `cficode` | `CFICode(461)` |
| `timeinforce` | `TimeInForce(59)` (`TimeInForce::fix_code`) |
| `stoppx` | `StopPx(99)` |
| `strikepx` | `StrikePrice(202)` |
| `displayqty` | `DisplayQty(1138)` |
| `cxlqty` | `CxlQty(84)` |
| `prevpx` | `PrevClosePx(140)` |
| `lastpx`, `spotrate`, `forwardpoints` | `LastPx(31)`, `LastSpotRate(194)`, `LastForwardPoints(195)`; `LASTPX` stays the crate word while `lastpx` is absent |
| `lastqty`, `avgpx`, `cumqty`, `leavesqty` | `LastQty(32)`, `AvgPx(6)`, `CumQty(14)`, `LeavesQty(151)` |
| `bidpx`, `bidqty`, `askpx`, `askqty` | `BidPx(132)`, `BidSize(134)`, `OfferPx(133)`, `OfferSize(135)`; `BIDASK` stays the crate word while `bidccy`/`askccy` are held |
| `ticker` | `Symbol(55)` |
| `miccode` | `LastMkt(30)` |
| `exprunix` | `ExpireTime(126)` |
| `unit` | `UnitOfMeasure(996)` |

Under the crate tag only, in the render's crate band (`FixMsg::stated_band`):

| Fact | Crate tag |
| --- | --- |
| `crosscode` | 65_003 |
| `srcuuids` | 65_006 |
| `transunix` (where `TRANSUNIX` is stated) | 65_007 |
| `creaunix` | 65_008 |
| `sendunix` (the row word) | 65_009 |
| `prevunix` | 65_011 |
| `snapunix` | 65_012 |
| `prevuuid` | 65_013 |
| `seqnum` | 65_014 |
| `state` | 65_015 |
| `marketdatakind` | 65_016 |
| `marketdatatype` | 65_017 |
| `origccy` | 65_018 |
| `hiddenqty` | 65_019 |
| `securityids` | 65_021 |
| `execunix` | 65_024 |
| `prevqty` | 65_026 |
| `bidccy` | 65_030 |
| `askccy` | 65_033 |
| `fxrates` | 65_034 |
| `metadata` | 65_037 |
| `tradable` | 65_039 |
| `identifiers` | 65_040 |
| `partyids` | 65_041 |

Never rendered: `uuid`, `crossuuid`, `hashcode`, `crosshashcode`, the views (`isincode`, `bloombergcode`, `figicode`, `forexcode`), the capture cells and `fixmsg`.

## The MsgType(35) table (`fix/codec.rs` `msgtype_for_kind`)

Written only where no entry states 35. The state rides its crate tag. The table is the inverse of `fix::state::from_msgtype` for these categories.

| Category | MsgType(35) |
| --- | --- |
| `ORDR` | `8` |
| `EXEC` | `8` |
| `QUOT` | `S` |
| `BOOK` | `W` |
| `TRAD` | `AE` |
| any other | none (its crate tag 65_016 says what it is) |

## size_of (W1's estimate)

`size_of::<MarketMessage>()` is 192 bytes:

| Part | Bytes |
| --- | --- |
| facts box | 8 |
| `StatedFacts` | 8 |
| `Arc<Field>` | 8 |
| `Scalar` | 48 |
| children `Vec` | 24 |
| anomalies `Vec` | 24 |
| `InstrumentStatement` | 56 |
| `Option<Box<BookRef>>` | 8 |
| `bool` | 1 |

The parts sum to 185, aligned to 192. `MarketData` stays 912 (`SnapshotEvent` is the widest leaf). W6 pins `size_of::<MarketMessage>() < size_of::<SnapshotEvent>()`.

## Expected conflicts when merging the program branch (S3's final settle, then P4's landing commit)

- **Whole-line rows.** `AGENTS.md`'s `graph/`, `fix/` and `isin_registry.rs` rows are one line each. P4 renamed the instants in them and P5 rewrote them. Resolve by hand: P5's content with P4's names (`transunix`, `sendunix`, `uuid`, `hashcode`). The same goes for `.api-inventory.txt`, `.api-bindings.txt`, `docs/graph/market-data.md`, `docs/fix/{message,lifecycle,capture,arrow}.md` and both skills.
- **`rust/src/graph/market_data.rs` and `graph/mod.rs`.** S3's settle may still touch the S3 trait and the `seam` module P5 deletes. Keep the deletion, then re-apply any non-trait fix S3 made.
- **`rust/src/implementer.rs`** (S3's file; W3 moved `write_named_bytes`) and **`rust/src/isin_registry.rs`** (the `learn_stating` signature and doc beside P4's `transunix` wording).
- **`rust/src/fix/{msg,market,codec,enrich,identity,build,batch,schema}.rs`.** W2 rewrote large spans of `msg.rs` (+1430/-1406) and `market.rs`. Any edit P4's landing made beyond the sweep (cargo-check fixes, doc sentences) conflicts. Take P5's structure and re-apply P4's renames.
- **Test files.** `rust/tests/fix/*.rs` (the swept files, plus `msg.rs`, `market.rs` and `codec.rs`), `rust/tests/allocations.rs` (P4's re-pins against the new rows), `python/tests/test_fix.py`, `python/yggdryl/_native.pyi`, `python/src/fix.rs` and `node/src/fix.rs`.
- **Take P4's side, then regenerate.**
  - Take P4's: `rust/tests/fix/equivalence.snapshot` and the dictionary hash pin in `rust/tests/fix/store.rs` (P5 must not move them).
  - Regenerate rather than resolve: `node/index.js`, `node/index.d.ts`, `docs/assets/fix.json`, `docs/assets/playground.json`.
- **After the merge:** re-run the `rust/tests/fix` sweep only through the compiler's E0599 list (`scratchpad/p5_w6/fix_e0599.py` over `cargo check -p yggdryl --all-targets --keep-going --message-format=short`).

## Checks run

- `rustfmt --edition 2024 --check` over the 68 changed or new `.rs` files: clean. `rust/src/lib.rs`, `node/src/lib.rs`, the 12 swept test files and `rust/tests/graph/iterator.rs` were formatted. A `--check` of `rust/src/lib.rs` walks the whole crate and is clean.
- `python3 scripts/check_api_inventory.py`: "inventories are current; 180 source file(s) and 568 `pub` name(s) are not described yet".
- `mkdocs build --strict` (the main checkout's `python/.venv`): "Documentation built in 18.07 seconds".
- `git diff --stat`: 97 files changed, 7864 insertions(+), 4861 deletions(-); plus 10 new files.
- Not run: cargo (contract), pytest, `node --test`, mypy, the docs example runner.
